//! Conservative program-wide eligibility for the first scalar leaf inliner.
use std::collections::{BTreeMap, BTreeSet};

use super::cfg::MirCfg;
use super::effects::{MirTempAccess, classify_op, classify_terminator};
use crate::mir6502::ir::*;
use crate::mir6502::standalone::{
    visit_data_image_routines, visit_global_init_routines, visit_op_routines,
    visit_storage_init_routines, visit_terminator_routines,
};
use crate::nir::ParamId;

const MAX_LEAF_BLOCKS: usize = 4;
const MAX_LEAF_OPS: usize = 12;
const MAX_REQUESTED_BLOCKS: usize = 8;
const MAX_REQUESTED_OPS: usize = 128;

#[derive(Debug, Clone)]
pub(in crate::mir6502) struct LeafRoutine {
    pub routine: MirRoutine,
    pub params: Vec<LeafParam>,
    pub return_widths: Vec<MirWidth>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::mir6502) struct LeafParam {
    pub id: ParamId,
    pub offset: u16,
    pub width: MirWidth,
}

#[derive(Debug, Default)]
pub(in crate::mir6502) struct LeafCensus {
    pub leaves: BTreeMap<RoutineId, LeafRoutine>,
    pub rejected: BTreeMap<RoutineId, &'static str>,
}

pub(in crate::mir6502) fn analyze(program: &MirProgram) -> LeafCensus {
    let recursive = recursive_routines(program);
    let mut escaped = BTreeSet::new();
    for data in &program.statics {
        visit_data_image_routines(&data.image, &mut escaped);
    }
    for global in &program.globals {
        if let Some(init) = &global.init {
            visit_global_init_routines(init, &mut escaped);
        }
    }
    for helper in &program.runtime_helpers {
        if let MirRuntimeHelperTarget::Routine(id) = helper.target {
            escaped.insert(id);
        }
    }
    let mut unresolved_machine_reference = false;
    for machine in &program.machine_blocks {
        for item in &machine.items {
            match item {
                MirMachineItem::Relocation {
                    target: MirInlineAsmTarget::Routine(id),
                    ..
                } => {
                    escaped.insert(*id);
                }
                MirMachineItem::Name(_)
                | MirMachineItem::AddressByte { .. }
                | MirMachineItem::AddressExpr {
                    atom: MirMachineAtom::Name(_),
                    ..
                } => {
                    unresolved_machine_reference = true;
                }
                _ => {}
            }
        }
    }
    let mut calls = BTreeMap::<RoutineId, Vec<(&MirOp, bool)>>::new();
    for routine in &program.routines {
        let mut byte_temps = BTreeSet::new();
        let mut nonbyte_temps = BTreeSet::new();
        for block in &routine.blocks {
            for param in &block.params {
                if param.width == MirWidth::Byte {
                    byte_temps.insert(param.dest);
                } else {
                    nonbyte_temps.insert(param.dest);
                }
            }
            for access in block
                .ops
                .iter()
                .flat_map(|op| classify_op(op).logical.temp_defs)
            {
                match access {
                    MirTempAccess::Exact { temp, byte: 0 } => {
                        byte_temps.insert(temp);
                    }
                    other => {
                        nonbyte_temps.insert(other.temp());
                    }
                }
            }
        }
        byte_temps.retain(|id| !nonbyte_temps.contains(id));
        for slot in routine.frame.params.iter().chain(&routine.frame.locals) {
            if let Some(init) = &slot.init {
                visit_storage_init_routines(init, &mut escaped);
            }
        }
        for block in &routine.blocks {
            visit_terminator_routines(&block.terminator, &mut escaped);
            for op in &block.ops {
                visit_op_routines(op, &mut escaped);
                if let MirOp::Call {
                    target: MirCallTarget::Routine(id),
                    args,
                    ..
                } = op
                {
                    let byte_args = args.iter().all(|a| match &a.value {
                        MirValue::Def(MirDef::VTemp(id)) => byte_temps.contains(id),
                        value => byte_value(value),
                    });
                    calls.entry(*id).or_default().push((op, byte_args));
                }
            }
        }
    }
    let mut census = LeafCensus::default();
    for routine in &program.routines {
        if !calls.contains_key(&routine.id) {
            continue;
        }
        let candidate = (|| {
            if recursive.contains(&routine.id) {
                return Err("recursion");
            }
            if escaped.contains(&routine.id) || unresolved_machine_reference {
                return Err("escape");
            }
            let leaf = classify_leaf(routine)?;
            if calls[&routine.id].iter().any(|(op, byte_args)| {
                (!routine.inline.requested() && !byte_args) || !call_matches(op, &leaf)
            }) {
                return Err("arguments-or-abi");
            }
            Ok(leaf)
        })();
        match candidate {
            Ok(leaf) => {
                census.leaves.insert(routine.id, leaf);
            }
            Err(reason) => {
                census.rejected.insert(routine.id, reason);
            }
        }
    }
    census
}

fn recursive_routines(program: &MirProgram) -> BTreeSet<RoutineId> {
    let edges: BTreeMap<_, BTreeSet<_>> = program
        .routines
        .iter()
        .map(|routine| {
            let calls = routine
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter_map(|op| match op {
                    MirOp::Call {
                        target: MirCallTarget::Routine(id),
                        ..
                    } => Some(*id),
                    _ => None,
                })
                .collect();
            (routine.id, calls)
        })
        .collect();
    edges
        .keys()
        .copied()
        .filter(|start| {
            let mut seen = BTreeSet::new();
            let mut pending: Vec<_> = edges[start].iter().copied().collect();
            while let Some(id) = pending.pop() {
                if id == *start {
                    return true;
                }
                if seen.insert(id) {
                    pending.extend(edges.get(&id).into_iter().flatten().copied());
                }
            }
            false
        })
        .collect()
}

pub(in crate::mir6502) fn byte_value(value: &MirValue) -> bool {
    matches!(
        value,
        MirValue::ConstU8(_) | MirValue::Def(MirDef::VTemp(_))
    ) || matches!(value, MirValue::ConstU16(value) if *value <= 255)
}

#[cfg(test)]
pub(in crate::mir6502) fn return_store(op: &MirOp) -> Option<&MirValue> {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(slot)),
            src,
            width: MirWidth::Byte,
        } if slot.0 == crate::codegen::runtime_zp::ARGS.address() && byte_value(src) => Some(src),
        _ => None,
    }
}

fn scalar_value(value: &MirValue, width: MirWidth) -> bool {
    match value {
        MirValue::ConstU8(_) | MirValue::Def(MirDef::VTemp(_)) => true,
        MirValue::ConstU16(n) => width == MirWidth::Word || *n <= 255,
        MirValue::Def(MirDef::VTempByte { byte: 0..=1, .. }) => width == MirWidth::Byte,
        MirValue::Word { lo, hi } => {
            width == MirWidth::Word
                && scalar_value(lo, MirWidth::Byte)
                && scalar_value(hi, MirWidth::Byte)
        }
        _ => false,
    }
}

pub(in crate::mir6502) fn return_values(block: &MirBlock) -> Vec<(MirWidth, &MirValue)> {
    if block.terminator != MirTerminator::Return {
        return Vec::new();
    }
    let mut lanes: Vec<_> = block
        .ops
        .iter()
        .rev()
        .take_while(|op| {
            matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0 | 0xA2))),
                    ..
                }
            )
        })
        .filter_map(|op| match op {
            MirOp::Store { src, width, .. } => Some((*width, src)),
            _ => None,
        })
        .collect();
    lanes.reverse();
    lanes
}

fn classify_leaf(routine: &MirRoutine) -> Result<LeafRoutine, &'static str> {
    if routine.abi != MirRoutineAbi::Action {
        return Err("observable-entry");
    }
    let signature = routine
        .scalar_signature
        .as_ref()
        .ok_or("non-scalar-signature")?;
    let requested = routine.inline.requested();
    let (max_blocks, max_ops) = if requested {
        (MAX_REQUESTED_BLOCKS, MAX_REQUESTED_OPS)
    } else {
        (MAX_LEAF_BLOCKS, MAX_LEAF_OPS)
    };
    if routine.blocks.is_empty()
        || routine.blocks.len() > max_blocks
        || routine.blocks.iter().map(|b| b.ops.len()).sum::<usize>() > max_ops
    {
        return Err("body-size");
    }
    let frame = &routine.frame;
    if !frame.locals.is_empty()
        || !frame.spills.is_empty()
        || !frame.virtual_zero_page.is_empty()
        || !frame.zero_page_allocations.is_empty()
        || frame.params.len() > 2
    {
        return Err("storage");
    }
    let mut params = Vec::new();
    for slot in &frame.params {
        let MirStorageBase::Param(id) = slot.base else {
            return Err("parameter-storage");
        };
        if slot.storage != MirStorageClass::Scalar || slot.offset != 0 || slot.init.is_some() {
            return Err("parameter-storage");
        }
        let widths: &[MirWidth] = match (slot.scalar_width, slot.storage_size) {
            (Some(MirWidth::Byte), 1) => &[MirWidth::Byte],
            (Some(MirWidth::Word), 2) if requested => &[MirWidth::Word],
            (None, 4) if requested => &[MirWidth::Word, MirWidth::Word],
            _ => return Err("parameter-storage"),
        };
        for (index, &width) in widths.iter().enumerate() {
            params.push(LeafParam {
                id,
                offset: index as u16 * 2,
                width,
            });
        }
    }
    let cfg = MirCfg::from_routine(routine).map_err(|_| "cfg")?;
    let mut remaining = cfg.reachable().clone();
    while !remaining.is_empty() {
        let Some(id) = remaining
            .iter()
            .copied()
            .find(|id| cfg.predecessors(*id).is_disjoint(&remaining))
        else {
            return Err("cycle");
        };
        remaining.remove(&id);
    }
    if cfg.reachable().len() != routine.blocks.len() || !routine.blocks[0].params.is_empty() {
        return Err("cfg");
    }
    let value = |v: &MirValue, w| {
        if requested {
            scalar_value(v, w)
        } else {
            w == MirWidth::Byte && byte_value(v)
        }
    };
    let def = |d: &MirDef, w| {
        matches!(d, MirDef::VTemp(_))
            || (requested
                && w == MirWidth::Byte
                && matches!(d, MirDef::VTempByte { byte: 0..=1, .. }))
    };
    let mut result_kind = None;
    for block in &routine.blocks {
        if !requested && block.params.iter().any(|p| p.width != MirWidth::Byte) {
            return Err("width");
        }
        let returns = return_values(block);
        let widths: Vec<_> = returns.iter().map(|(w, _)| *w).collect();
        if block.terminator == MirTerminator::Return {
            if !matches!(widths.as_slice(), [] | [MirWidth::Byte])
                && !(requested
                    && matches!(
                        widths.as_slice(),
                        [MirWidth::Word] | [MirWidth::Word, MirWidth::Word]
                    ))
            {
                return Err("return-lanes");
            }
            if result_kind
                .replace(widths.clone())
                .is_some_and(|old| old != widths)
            {
                return Err("mixed-returns");
            }
        }
        let return_start = block.ops.len() - returns.len();
        for (index, op) in block.ops.iter().enumerate() {
            let supported = match op {
                MirOp::Load {
                    dst,
                    src: MirAddr::Direct(MirMem::Param { id, offset }),
                    width,
                } => {
                    def(dst, *width)
                        && params
                            .iter()
                            .any(|p| p.id == *id && p.offset == *offset && p.width == *width)
                }
                MirOp::LoadImm {
                    dst,
                    value: n,
                    width,
                } => def(dst, *width) && value(&MirValue::ConstU16(*n), *width),
                MirOp::Move { dst, src, width }
                | MirOp::Unary {
                    dst, src, width, ..
                } => def(dst, *width) && value(src, *width),
                MirOp::Extend {
                    dst,
                    src,
                    from_width,
                    to_width,
                    ..
                }
                | MirOp::Truncate {
                    dst,
                    src,
                    from_width,
                    to_width,
                } => requested && def(dst, *to_width) && value(src, *from_width),
                MirOp::Binary {
                    dst,
                    op,
                    left,
                    right,
                    width,
                    carry_in,
                    carry_out,
                } => {
                    def(dst, *width)
                        && value(left, *width)
                        && value(right, *width)
                        && (requested || (carry_in.is_none() && *carry_out == MirCarryOut::Ignore))
                        && match op {
                            MirBinaryOp::Add
                            | MirBinaryOp::Sub
                            | MirBinaryOp::And
                            | MirBinaryOp::Or
                            | MirBinaryOp::Xor => true,
                            MirBinaryOp::Lsh | MirBinaryOp::Rsh => {
                                matches!(
                                    right,
                                    MirValue::ConstU8(0..=7) | MirValue::ConstU16(0..=7)
                                ) || (requested
                                    && matches!(
                                        right,
                                        MirValue::ConstU8(8) | MirValue::ConstU16(8)
                                    ))
                            }
                            _ => false,
                        }
                }
                MirOp::Compare {
                    dst: MirCondDest::Temp(_),
                    left,
                    right,
                    width,
                    ..
                } => value(left, *width) && value(right, *width),
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(slot)),
                    src,
                    width,
                } if index >= return_start && block.terminator == MirTerminator::Return => {
                    let lane = index - return_start;
                    slot.0 == 0xA0 + lane as u8 * 2 && value(src, *width)
                }
                _ => false,
            };
            if !supported {
                return Err("operation-or-effects");
            }
        }
        let edge = |e: &MirEdge| e.args.iter().all(|a| value(&a.value, a.width));
        match &block.terminator {
            MirTerminator::Return => {}
            MirTerminator::Jump(e) if edge(e) => {}
            MirTerminator::Branch {
                cond: MirCond::BoolValue(v),
                then_edge,
                else_edge,
            } if value(v, MirWidth::Byte) && edge(then_edge) && edge(else_edge) => {}
            _ => return Err("control"),
        }
    }
    let return_widths = result_kind.ok_or("no-return")?;
    if return_widths != signature.result
        || signature
            .params
            .iter()
            .flatten()
            .copied()
            .ne(params.iter().map(|p| p.width))
    {
        return Err("signature");
    }
    Ok(LeafRoutine {
        routine: routine.clone(),
        params,
        return_widths,
    })
}

fn call_matches(op: &MirOp, leaf: &LeafRoutine) -> bool {
    let MirOp::Call {
        args,
        result,
        additional_results,
        abi,
        effects,
        ..
    } = op
    else {
        return false;
    };
    let mut offset = 0;
    let arguments = args.len() == leaf.params.len()
        && args.len() == abi.params.len()
        && args
            .iter()
            .zip(&leaf.params)
            .enumerate()
            .all(|(i, (a, p))| {
                let home = crate::mir6502::abi::action_arg_home(offset, p.width);
                offset += crate::mir6502::abi::action_arg_width_bytes(p.width);
                a.width == p.width
                    && scalar_value(&a.value, a.width)
                    && a.home == home
                    && abi.params.get(i) == Some(&home)
            });
    let expected_additional: Vec<_> = leaf
        .return_widths
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, &width)| MirHelperResult {
            home: MirResultHome::ReturnSlot {
                offset: i as u16 * 2,
            },
            width,
        })
        .collect();
    arguments
        && abi.additional_results == expected_additional
        && (abi.result
            == (!leaf.return_widths.is_empty()).then_some(MirResultHome::ReturnSlot { offset: 0 })
            || (abi.result.is_none() && result.is_none() && additional_results.is_empty()))
        && result.iter().chain(additional_results).all(|r| {
            let MirResultHome::ReturnSlot { offset } = r.home else {
                return false;
            };
            offset % 2 == 0
                && leaf.return_widths.get(offset as usize / 2) == Some(&r.width)
                && matches!(r.dst, MirDef::VTemp(_))
        })
        && !effects.opaque
        && !effects.may_call_os
        && effects.stack_depth_delta.is_none_or(|delta| delta == 0)
        && effects.reads == MirRegisterSet::default()
        && abi.preserves == MirRegisterSet::default()
}

pub(in crate::mir6502) fn caller_supported(routine: &MirRoutine) -> bool {
    matches!(
        routine.abi,
        MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry
    ) && routine.blocks.iter().all(|block| {
        let machine = classify_terminator(&block.terminator).machine;
        machine.register_reads == MirRegisterSet::default()
            && !matches!(
                block.terminator,
                MirTerminator::Branch {
                    cond: MirCond::FlagTest(_)
                        | MirCond::AnyFlagTest(_)
                        | MirCond::FusedCompare { .. },
                    ..
                }
            )
            && block.ops.iter().all(|op| {
                !matches!(op, MirOp::MachineBlock { .. } | MirOp::Barrier { .. })
                    && classify_op(op).machine.register_reads == MirRegisterSet::default()
                    && (!classify_op(op).machine.flag_reads.any()
                        || matches!(
                            op,
                            MirOp::Binary {
                                carry_in: Some(MirCarryIn::FromPrevious),
                                ..
                            }
                        ))
            })
    })
}
