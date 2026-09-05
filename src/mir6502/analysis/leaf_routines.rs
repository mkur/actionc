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

#[derive(Debug, Clone)]
pub(in crate::mir6502) struct LeafRoutine {
    pub routine: MirRoutine,
    pub params: Vec<ParamId>,
    pub returns_value: bool,
}

#[derive(Debug, Default)]
pub(in crate::mir6502) struct LeafCensus {
    pub leaves: BTreeMap<RoutineId, LeafRoutine>,
    pub rejected: BTreeMap<RoutineId, &'static str>,
}

pub(in crate::mir6502) fn analyze(program: &MirProgram) -> LeafCensus {
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
            if escaped.contains(&routine.id) || unresolved_machine_reference {
                return Err("escape");
            }
            let leaf = classify_leaf(routine)?;
            if calls[&routine.id]
                .iter()
                .any(|(op, byte_args)| !byte_args || !call_matches(op, &leaf))
            {
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

pub(in crate::mir6502) fn byte_value(value: &MirValue) -> bool {
    matches!(
        value,
        MirValue::ConstU8(_) | MirValue::Def(MirDef::VTemp(_))
    ) || matches!(value, MirValue::ConstU16(value) if *value <= 255)
}

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

fn classify_leaf(routine: &MirRoutine) -> Result<LeafRoutine, &'static str> {
    if routine.abi != MirRoutineAbi::Action {
        return Err("observable-entry");
    }
    if routine.blocks.is_empty()
        || routine.blocks.len() > MAX_LEAF_BLOCKS
        || routine.blocks.iter().map(|b| b.ops.len()).sum::<usize>() > MAX_LEAF_OPS
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
    let params = frame
        .params
        .iter()
        .map(|p| match p.base {
            MirStorageBase::Param(id)
                if p.storage == MirStorageClass::Scalar
                    && p.scalar_width == Some(MirWidth::Byte)
                    && p.storage_size == 1
                    && p.offset == 0
                    && p.init.is_none() =>
            {
                Ok(id)
            }
            _ => Err("parameter-storage"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cfg = MirCfg::from_routine(routine).map_err(|_| "cfg")?;
    // Acyclicity is checked by topological removal, independent of layout order.
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
    let mut result_kind = None;
    for block in &routine.blocks {
        if block.params.iter().any(|p| p.width != MirWidth::Byte) {
            return Err("width");
        }
        for (index, op) in block.ops.iter().enumerate() {
            let supported = match op {
                MirOp::Load {
                    dst: MirDef::VTemp(_),
                    src: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
                    width: MirWidth::Byte,
                } => params.contains(id),
                MirOp::LoadImm {
                    dst: MirDef::VTemp(_),
                    value,
                    width: MirWidth::Byte,
                } => *value <= 255,
                MirOp::Move {
                    dst: MirDef::VTemp(_),
                    src,
                    width: MirWidth::Byte,
                }
                | MirOp::Unary {
                    dst: MirDef::VTemp(_),
                    src,
                    width: MirWidth::Byte,
                    ..
                } => byte_value(src),
                MirOp::Binary {
                    dst: MirDef::VTemp(_),
                    op,
                    left,
                    right,
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: MirCarryOut::Ignore,
                } => {
                    byte_value(left)
                        && byte_value(right)
                        && match op {
                            MirBinaryOp::Add
                            | MirBinaryOp::Sub
                            | MirBinaryOp::And
                            | MirBinaryOp::Or
                            | MirBinaryOp::Xor => true,
                            MirBinaryOp::Lsh | MirBinaryOp::Rsh => matches!(
                                right,
                                MirValue::ConstU8(0..=7) | MirValue::ConstU16(0..=7)
                            ),
                            _ => false,
                        }
                }
                MirOp::Compare {
                    dst: MirCondDest::Temp(_),
                    left,
                    right,
                    width: MirWidth::Byte,
                    ..
                } => byte_value(left) && byte_value(right),
                _ => {
                    index + 1 == block.ops.len()
                        && block.terminator == MirTerminator::Return
                        && return_store(op).is_some()
                }
            };
            if !supported {
                return Err("operation-or-effects");
            }
        }
        match &block.terminator {
            MirTerminator::Return => {
                let returns = block.ops.last().and_then(return_store).is_some();
                if result_kind
                    .replace(returns)
                    .is_some_and(|old| old != returns)
                {
                    return Err("mixed-returns");
                }
            }
            MirTerminator::Jump(edge)
                if edge
                    .args
                    .iter()
                    .all(|a| a.width == MirWidth::Byte && byte_value(&a.value)) => {}
            MirTerminator::Branch {
                cond: MirCond::BoolValue(value),
                then_edge,
                else_edge,
            } if byte_value(value)
                && then_edge
                    .args
                    .iter()
                    .chain(&else_edge.args)
                    .all(|a| a.width == MirWidth::Byte && byte_value(&a.value)) => {}
            _ => return Err("control"),
        }
    }
    Ok(LeafRoutine {
        routine: routine.clone(),
        params,
        returns_value: result_kind.ok_or("no-return")?,
    })
}

fn call_matches(op: &MirOp, leaf: &LeafRoutine) -> bool {
    let MirOp::Call {
        args,
        result,
        abi,
        effects,
        ..
    } = op
    else {
        return false;
    };
    args.len() == leaf.params.len()
        && args.len() == abi.params.len()
        && args.iter().enumerate().all(|(i, a)| {
            a.width == MirWidth::Byte
                && byte_value(&a.value)
                && a.home == MirArgHome::Reg(if i == 0 { MirReg::A } else { MirReg::X })
                && abi.params.get(i) == Some(&a.home)
        })
        && abi.result
            == leaf
                .returns_value
                .then_some(MirResultHome::ReturnSlot { offset: 0 })
        && result.as_ref().is_none_or(|r| {
            leaf.returns_value
                && r.width == MirWidth::Byte
                && matches!(r.dst, MirDef::VTemp(_))
                && r.home == MirResultHome::ReturnSlot { offset: 0 }
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
                    && !classify_op(op).machine.flag_reads.any()
            })
    })
}
