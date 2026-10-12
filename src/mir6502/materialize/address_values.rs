//! Full-pair address materialization and bounded final-image transactions.
use super::*;
use crate::mir6502::MirRoutine;
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::rewrite::context::PreHomeRewriteContext;
use crate::mir6502::rewrite::context::{MirExitStateChange, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::{
    MirChangeSet, MirEffectDelta, MirRemovedDefinition, MirRewritePlan,
};
use std::collections::{BTreeMap, BTreeSet};

/// A complete-pair constructor preserves Y. Keep a captured unsigned byte
/// index there when its private saved byte has no other reader. Final placement
/// also proves that the removed store cannot change a base input.
pub(in crate::mir6502) fn discover_saved_byte_indexes(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<crate::mir6502::rewrite::plan::MirPostHomeRewritePlan> {
    let physical = |mem: &MirMem| match mem {
        MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0)),
        MirMem::ZeroPage(slot) => routine
            .frame
            .zero_page_allocations
            .iter()
            .find(|allocation| allocation.slot == *slot)
            .map(|allocation| u16::from(allocation.start.0)),
        _ => layout.mem_address(routine.id, mem),
    };
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (start, ops) in block.ops.windows(4).enumerate() {
            let [
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(home),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
                constructor @ MirOp::MaterializeIndexedAddress {
                    consumer,
                    base,
                    index: MirValue::ConstU8(_) | MirValue::ConstU16(_),
                    scale: 1 | 2,
                },
                MirOp::AdvanceAddress {
                    consumer: advance,
                    index: MirValue::Word { lo, hi },
                    scale,
                },
            ] = ops
            else {
                continue;
            };
            if consumer != advance
                || consumer.preserves_index_in_y()
                || !peepholes::mem_is_private_scratch(home)
                || !matches!(lo.as_ref(), MirValue::PointerCell(mem) if mem == home)
                || !matches!(hi.as_ref(), MirValue::ConstU8(0))
                || classify_op(constructor).may_clobber_reg_compat(MirReg::Y)
            {
                continue;
            }
            fn reads_a(value: &MirValue) -> bool {
                match value {
                    MirValue::Def(MirDef::Reg(MirReg::A)) => true,
                    MirValue::Word { lo, hi } => reads_a(lo) || reads_a(hi),
                    _ => false,
                }
            }
            let Some(address) = physical(home) else {
                continue;
            };
            if reads_a(base) || (0xAC..=0xAF).contains(&address) {
                continue;
            }
            let effects = classify_op(constructor);
            if effects.memory.direct_reads.iter().any(|range| {
                physical(&range.base).is_none_or(|base| address.wrapping_sub(base) < range.bytes)
            }) {
                continue;
            }
            // Logical home deadness does not recognize a separately named
            // absolute alias of a spill. Check all surviving direct reads too.
            if routine.blocks.iter().any(|other| {
                other.ops.iter().enumerate().any(|(i, op)| {
                    !(other.id == block.id && (start..start + 4).contains(&i))
                        && classify_op(op).memory.direct_reads.iter().any(|range| {
                            range.base != *home
                                && physical(&range.base)
                                    .is_none_or(|base| address.wrapping_sub(base) < range.bytes)
                        })
                })
            }) {
                continue;
            }
            if let Some(plan) = crate::mir6502::rewrite::posthome::structural_plan(
                routine,
                context,
                block.id,
                start..start + 4,
                vec![
                    constructor.clone(),
                    MirOp::AdvanceAddress {
                        consumer: *advance,
                        index: MirValue::Word {
                            lo: Box::new(MirValue::Def(MirDef::Reg(MirReg::Y))),
                            hi: Box::new(MirValue::ConstU8(0)),
                        },
                        scale: *scale,
                    },
                ],
                MirExitStateChange::default(),
                "address-value-index-y",
                0,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(super) fn forward_saved_byte_indexes(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
    known_callees: &MirKnownCalleeSummaries,
    stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let result = MirPostHomeRewriteDriver::default()
        .run_fixed_point_with_known_callees(routine, known_callees, |r, c| {
            discover_saved_byte_indexes(r, c, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("address index forwarding failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, stats);
    Ok(())
}

pub(in crate::mir6502) fn discover_address_stores(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (start, ops) in block.ops.windows(2).enumerate() {
            let [
                MirOp::ComputeAddress {
                    dst: MirDef::VTemp(temp),
                    base,
                    index: Some(index),
                    elem_size,
                    offset,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(dst),
                    src: MirValue::Def(MirDef::VTemp(source)),
                    width: MirWidth::Word,
                },
            ] = ops
            else {
                continue;
            };
            if temp != source || !(1..=255).contains(elem_size) {
                continue;
            }
            let site = MirSite::Op {
                block: block.id,
                op_index: start + 1,
            };
            if !context
                .exclusive_word_temp_definition(*temp, site)
                .is_proven()
                || !context.flags_dead_after(
                    classify_op(&ops[0]).machine.flag_writes,
                    MirSite::Op {
                        block: block.id,
                        op_index: start,
                    },
                )
                || !context
                    .pointer_pair_dead_after(DEFAULT_POINTER_PAIR, context.point(site))
                    .is_proven()
                || !context
                    .pointer_pair_dead_after(INDEX_POINTER_PAIR, context.point(site))
                    .is_proven()
            {
                continue;
            }
            let mut replacement = Vec::new();
            if *offset == 0 {
                if matches!(elem_size, 1 | 2) {
                    replacement.push(MirOp::MaterializeIndexedAddress {
                        consumer: DEFAULT_POINTER_PAIR,
                        base: base.clone(),
                        index: index.clone(),
                        scale: *elem_size as u8,
                    });
                } else {
                    replacement.push(MirOp::MaterializeAddress {
                        consumer: DEFAULT_POINTER_PAIR,
                        value: base.clone(),
                    });
                    replacement.push(MirOp::AdvanceAddress {
                        consumer: DEFAULT_POINTER_PAIR,
                        index: index.clone(),
                        scale: *elem_size as u8,
                    });
                }
            } else {
                // Construct the full displaced base in one operation. A
                // constant index avoids a separate base copy and increment.
                replacement.push(MirOp::MaterializeIndexedAddress {
                    consumer: DEFAULT_POINTER_PAIR,
                    base: base.clone(),
                    index: MirValue::ConstU16(*offset),
                    scale: 1,
                });
                replacement.push(MirOp::AdvanceAddress {
                    consumer: DEFAULT_POINTER_PAIR,
                    index: index.clone(),
                    scale: *elem_size as u8,
                });
            }
            // This is the complete address pair, with no residual Y component.
            // Keep the source/public word store at its original semantic point.
            replacement.push(MirOp::Store {
                dst: MirAddr::Direct(dst.clone()),
                src: MirValue::Word {
                    lo: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                        MirFixedZpSlot(POINTER_SCRATCH_LO),
                    ))),
                    hi: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                        MirFixedZpSlot(POINTER_SCRATCH_HI),
                    ))),
                },
                width: MirWidth::Word,
            });
            let mut end = start + 2;
            let mut delta = MirEffectDelta::MaterializedIndexConsumer;
            if crate::mir6502::rewrite::address_values::ordinary_pointer_local(routine, dst)
                && layout
                    .mem_address(routine.id, dst)
                    .is_some_and(|a| a >= 0x100 && a < 0xFFFF)
            {
                for field in start + 2..block.ops.len().min(start + 10) {
                    if let Some(field_ops) = prepared_local_field(
                        &block.ops[field],
                        dst,
                        &collect_routine_temp_widths(routine),
                    ) {
                        let field_site = MirSite::Op {
                            block: block.id,
                            op_index: field,
                        };
                        if crate::mir6502::analysis::address_values::local_pointer_store(
                            block, field, dst,
                        )
                        .is_ok_and(|(store, _)| store == start + 1)
                            && context.flags_dead_after(
                                classify_op(&block.ops[field]).machine.flag_writes,
                                field_site,
                            )
                            && context
                                .pointer_pair_dead_after(
                                    DEFAULT_POINTER_PAIR,
                                    context.point(field_site),
                                )
                                .is_proven()
                            && context
                                .pointer_pair_dead_after(
                                    INDEX_POINTER_PAIR,
                                    context.point(field_site),
                                )
                                .is_proven()
                        {
                            replacement.extend_from_slice(&block.ops[start + 2..field]);
                            replacement.extend(field_ops);
                            end = field + 1;
                            delta = MirEffectDelta::MaterializedRetainedPointerField;
                        }
                        break;
                    }
                    if !retained_field_gap(&block.ops[field]) {
                        break;
                    }
                }
            }
            let Some(definitions) =
                crate::mir6502::rewrite::pilots::prove_removed_window_definitions(
                    block.id,
                    &block.ops,
                    start,
                    end,
                    &replacement,
                    context,
                )
            else {
                continue;
            };
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: start..end,
                replacement,
                removed_defs: definitions
                    .into_iter()
                    .map(|definition| MirRemovedDefinition { definition })
                    .collect(),
                exit_effect_delta: delta,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: if end > start + 2 {
                    "address-value-retained-field"
                } else {
                    "address-value-store"
                },
                observations: vec![],
                family_priority: 10,
                estimated_byte_saving: 0,
                estimated_cycle_saving: 0,
            });
        }
    }
    plans
}

fn complete_pair_value() -> MirValue {
    MirValue::Word {
        lo: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
            MirFixedZpSlot(POINTER_SCRATCH_LO),
        ))),
        hi: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
            MirFixedZpSlot(POINTER_SCRATCH_HI),
        ))),
    }
}

/// Recognize a full constructor feeding an explicit value store, rather than
/// changing the read-deferral policy of existing indirect field consumers.
pub(super) fn complete_value_constructor(ops: &[MirOp], index: usize) -> bool {
    if !matches!(ops.get(index),Some(MirOp::MaterializeAddress { consumer, .. }
        | MirOp::MaterializeIndexedAddress { consumer, .. }) if *consumer == DEFAULT_POINTER_PAIR)
    {
        return false;
    }
    let next = if matches!(ops.get(index + 1),Some(MirOp::AdvanceAddress { consumer, .. }) if *consumer == DEFAULT_POINTER_PAIR)
    {
        index + 2
    } else {
        index + 1
    };
    matches!(ops.get(next),Some(MirOp::Store { dst: MirAddr::Direct(_),src,width:MirWidth::Word })
        if *src == complete_pair_value())
}

/// These operations cannot prepare another pointer or introduce a call/helper.
/// In particular, arithmetic and indirect reads are outside the initial slice.
fn retained_field_gap(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Load {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            src: MirAddr::Direct(_),
            ..
        } | MirOp::LoadImm {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            ..
        }
    )
}

fn prepared_local_field(
    op: &MirOp,
    cell: &MirMem,
    widths: &BTreeMap<MirTempId, MirWidth>,
) -> Option<Vec<MirOp>> {
    let (ptr, displacement, width) = match op {
        MirOp::Load {
            src: MirAddr::PointerCell { ptr, offset },
            width,
            ..
        }
        | MirOp::Store {
            dst: MirAddr::PointerCell { ptr, offset },
            width,
            ..
        } => (ptr, *offset, *width),
        _ => return None,
    };
    if ptr != cell {
        return None;
    }
    let window =
        crate::mir6502::analysis::address_uses::MirIndirectWindow::constant(displacement, width);
    let offset = u16::from(window.y_displacement);
    let mut out = Vec::new();
    if window.base_adjustment != 0 {
        out.push(MirOp::AdvanceAddress {
            consumer: DEFAULT_POINTER_PAIR,
            index: MirValue::ConstU16(window.base_adjustment),
            scale: 1,
        });
    }
    match op {
        MirOp::Load {
            dst: MirDef::Reg(_),
            ..
        } => return None,
        MirOp::Load {
            dst,
            width: MirWidth::Byte,
            ..
        } => out.push(MirOp::LoadIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            dst: dst.clone(),
            offset,
        }),
        MirOp::Load {
            dst,
            width: MirWidth::Word,
            ..
        } => {
            let (lo, hi) = split_def(dst.clone())?;
            out.push(MirOp::LoadIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                dst: lo,
                offset,
            });
            out.push(MirOp::LoadIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                dst: hi,
                offset: offset + 1,
            });
        }
        MirOp::Store { src, width, .. } => {
            fn captured_byte(value: &MirValue) -> bool {
                matches!(
                    value,
                    MirValue::ConstU8(_)
                        | MirValue::Def(MirDef::VTemp(_))
                        | MirValue::Def(MirDef::VTempByte { .. })
                )
            }
            let values = match (width, src) {
                (MirWidth::Byte, src) if captured_byte(src) => vec![src.clone()],
                (MirWidth::Word, MirValue::ConstU16(v)) => {
                    v.to_le_bytes().into_iter().map(MirValue::ConstU8).collect()
                }
                (MirWidth::Word, MirValue::Def(MirDef::VTemp(id)))
                    if widths.get(id) == Some(&MirWidth::Word) =>
                {
                    [0, 1]
                        .into_iter()
                        .map(|byte| MirValue::Def(MirDef::VTempByte { id: *id, byte }))
                        .collect()
                }
                (MirWidth::Word, MirValue::Word { lo, hi })
                    if captured_byte(lo) && captured_byte(hi) =>
                {
                    vec![*lo.clone(), *hi.clone()]
                }
                _ => return None,
            };
            out.extend(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(lane, src)| MirOp::StoreIndirect {
                        consumer: DEFAULT_POINTER_PAIR,
                        src,
                        offset: offset + lane as u16,
                    }),
            );
        }
        _ => return None,
    }
    Some(out)
}

/// Validate this narrow delta without discarding observable memory effects.
/// The complete local store and every intervening read are retained exactly;
/// the field is expanded through the identical complete pair and Y window.
pub(in crate::mir6502) fn retained_pointer_field_is_justified(
    original: &[MirOp],
    replacement: &[MirOp],
    widths: &BTreeMap<MirTempId, MirWidth>,
) -> bool {
    let [
        MirOp::ComputeAddress {
            dst: MirDef::VTemp(temp),
            base,
            index: Some(index),
            elem_size,
            offset,
        },
        MirOp::Store {
            dst: MirAddr::Direct(cell @ MirMem::Local { offset: 0, .. }),
            src: MirValue::Def(MirDef::VTemp(source)),
            width: MirWidth::Word,
        },
        rest @ ..,
    ] = original
    else {
        return false;
    };
    if temp != source || !(1..=255).contains(elem_size) || rest.is_empty() {
        return false;
    }
    let (field, gap) = rest.split_last().unwrap();
    if !gap.iter().all(retained_field_gap) {
        return false;
    }
    let mut expected = if *offset != 0 {
        vec![
            MirOp::MaterializeIndexedAddress {
                consumer: DEFAULT_POINTER_PAIR,
                base: base.clone(),
                index: MirValue::ConstU16(*offset),
                scale: 1,
            },
            MirOp::AdvanceAddress {
                consumer: DEFAULT_POINTER_PAIR,
                index: index.clone(),
                scale: *elem_size as u8,
            },
        ]
    } else if matches!(elem_size, 1 | 2) {
        vec![MirOp::MaterializeIndexedAddress {
            consumer: DEFAULT_POINTER_PAIR,
            base: base.clone(),
            index: index.clone(),
            scale: *elem_size as u8,
        }]
    } else {
        vec![
            MirOp::MaterializeAddress {
                consumer: DEFAULT_POINTER_PAIR,
                value: base.clone(),
            },
            MirOp::AdvanceAddress {
                consumer: DEFAULT_POINTER_PAIR,
                index: index.clone(),
                scale: *elem_size as u8,
            },
        ]
    };
    expected.push(MirOp::Store {
        dst: MirAddr::Direct(cell.clone()),
        src: complete_pair_value(),
        width: MirWidth::Word,
    });
    expected.extend_from_slice(gap);
    let Some(field_ops) = prepared_local_field(field, cell, widths) else {
        return false;
    };
    expected.extend(field_ops);
    expected == replacement
}

pub(super) fn materialize_address_stores(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
    stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |r, c| discover_address_stores(r, c, layout),
            crate::mir6502::rewrite::address_values::address_value_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("address-store materialization failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, stats);
    Ok(())
}

pub(super) struct Cost {
    pub(super) bytes: usize,
    routine_bytes: BTreeMap<RoutineId, usize>,
    homes: BTreeMap<RoutineId, usize>,
}
impl Cost {
    pub(super) fn measure(program: &MirProgram, origin: u16) -> Option<Self> {
        let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
        let summary = crate::mir6502::emit::emit_program(program, origin, &mut emitter).ok()?;
        let image = emitter.finish_with_relocations().ok()?;
        let mut routine_bytes = BTreeMap::new();
        for (routine, _, range) in summary.block_ranges {
            *routine_bytes.entry(routine).or_insert(0) += range.len();
        }
        let homes = program
            .routines
            .iter()
            .map(|r| {
                (
                    r.id,
                    r.frame.spills.len()
                        + r.frame
                            .params
                            .iter()
                            .chain(&r.frame.locals)
                            .filter(|s| {
                                matches!(
                                    s.base,
                                    MirStorageBase::Local(_) | MirStorageBase::Param(_)
                                )
                            })
                            .map(|s| usize::from(s.storage_size))
                            .sum::<usize>(),
                )
            })
            .collect();
        Some(Self {
            bytes: image.bytes.len(),
            routine_bytes,
            homes,
        })
    }
    pub(super) fn does_not_grow(&self, old: &Self, id: RoutineId) -> bool {
        self.routine_bytes.get(&id).zip(old.routine_bytes.get(&id)).is_some_and(|(a,b)|a<=b)
            && self.homes.get(&id).zip(old.homes.get(&id)).is_some_and(|(a,b)|a<=b)
    }
    pub(super) fn routine(&self, id: RoutineId) -> Option<usize> {
        Some(*self.routine_bytes.get(&id)? + *self.homes.get(&id)?)
    }
}

/// Avoid introducing writes to source-owned physical scratch, including aliases
/// and absolute-backed local cells. Other inputs remain captured virtual values;
/// the allocator reserves all four carrier bytes in the materialized operation.
fn scratch_is_private(program: &MirProgram, origin: u16) -> bool {
    if origin < 0x100 {
        return false;
    }
    let layout = MaterializeLayout::new(program, origin);
    let overlaps = |address: u16, size: u16| {
        (0..size).any(|n| (0xAC..=0xAF).contains(&address.wrapping_add(n)))
    };
    for global in &program.globals {
        if layout
            .mem_address(
                RoutineId(0),
                &MirMem::Global {
                    id: global.id,
                    offset: 0,
                },
            )
            .is_none_or(|a| overlaps(a, global.storage_size))
        {
            return false;
        }
    }
    for r in &program.routines {
        for slot in r.frame.locals.iter().chain(&r.frame.params) {
            if let MirStorageBase::Absolute(a) = slot.base
                && overlaps(a.wrapping_add(slot.offset), slot.storage_size)
            {
                return false;
            }
        }
    }
    true
}

pub(in crate::mir6502) fn materialize_costed(
    original: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let mut baseline_config = config.clone();
    baseline_config.enable_indexed_address_values = false;
    if !scratch_is_private(&original, origin) {
        return crate::mir6502::materialize_resolved_program_without_address_costing(
            original,
            &baseline_config,
            origin,
            runtime,
            report,
        );
    }
    let mut selected = original.clone();
    let mut candidates = BTreeSet::new();
    for routine in &mut selected.routines {
        let mut driver = MirPreHomeRewriteDriver::default();
        let result = driver
            .run_fixed_point_by_key(
                routine,
                crate::mir6502::rewrite::address_values::discover_indexed_address_values,
                crate::mir6502::rewrite::address_values::address_value_rank,
            )
            .map_err(|error| {
                vec![MirDiagnostic::routine(
                    &routine.name,
                    format!("address-value provenance failed: {error:?}"),
                )]
            })?;
        if result.applied > 0
            || routine.blocks.iter().flat_map(|b| &b.ops).any(|op| {
                matches!(
                    op,
                    MirOp::ComputeAddress {
                        index: Some(_),
                        elem_size: 1..=255,
                        ..
                    }
                )
            })
        {
            candidates.insert(routine.id);
        }
    }
    if candidates.is_empty() {
        return crate::mir6502::materialize_resolved_program_without_address_costing(
            original,
            &baseline_config,
            origin,
            runtime,
            report,
        );
    }
    let baseline = crate::mir6502::materialize_resolved_program_without_address_costing(
        original.clone(),
        &baseline_config,
        origin,
        runtime,
        false,
    )?;
    let mut trial = crate::mir6502::materialize_resolved_program_without_address_costing(
        selected.clone(),
        config,
        origin,
        runtime,
        false,
    )?;
    let baseline_cost = Cost::measure(&baseline, origin);
    let trial_cost = Cost::measure(&trial, origin);
    let mut accepted = BTreeSet::new();
    for id in &candidates {
        if baseline_cost
            .as_ref()
            .and_then(|c| c.routine(*id))
            .zip(trial_cost.as_ref().and_then(|c| c.routine(*id)))
            .is_some_and(|(old, new)| new <= old)
        {
            accepted.insert(*id);
        }
    }
    if accepted != candidates && !accepted.is_empty() {
        for routine in &mut selected.routines {
            if candidates.contains(&routine.id) && !accepted.contains(&routine.id) {
                *routine = original
                    .routines
                    .iter()
                    .find(|r| r.id == routine.id)
                    .unwrap()
                    .clone();
            }
        }
        trial = crate::mir6502::materialize_resolved_program_without_address_costing(
            selected.clone(),
            config,
            origin,
            runtime,
            false,
        )?;
    }
    // Shared homes, runtime helpers and relaxed branches are charged again at
    // the whole-image boundary. A local saving is not a pooled-home saving.
    if accepted.is_empty()
        || !baseline_cost
            .as_ref()
            .zip(Cost::measure(&trial, origin).as_ref())
            .is_some_and(|(old, new)| new.bytes <= old.bytes)
    {
        accepted.clear();
    }
    let applied = !accepted.is_empty();
    if report {
        let mut stats = MirPeepholeStats::default();
        for id in &candidates {
            stats.record(
                *id,
                if accepted.contains(id) {
                    "address-value-applied"
                } else {
                    "address-value-declined-size"
                },
            );
        }
        for id in &candidates {
            if let Some((old, new)) = baseline_cost
                .as_ref()
                .and_then(|c| c.routine(*id))
                .zip(trial_cost.as_ref().and_then(|c| c.routine(*id)))
            {
                stats.record_dynamic(*id, format!("address-value-code-home-{old}-to-{new}"));
            }
        }
        maybe_report_peepholes(&original, &stats, config);
    }
    if report && stats::reporting_enabled(config) {
        return crate::mir6502::materialize_resolved_program_without_address_costing(
            if applied { selected } else { original },
            if applied { config } else { &baseline_config },
            origin,
            runtime,
            true,
        );
    }
    Ok(if applied { trial } else { baseline })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{MirBlock, MirEdge};
    pub(super) fn lower(source: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        crate::mir6502::lower_program(&nir).unwrap()
    }
    #[test]
    fn production_stride_twenty_values_and_locals_admit_helper_free_code() {
        let source = include_str!("../../../surveys/tn/reproducers/escaping-record-address.act")
            .replace("\r\n", "\n");
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            let config = Mir6502Config::optimized();
            let mut off = config.clone();
            off.enable_indexed_address_values = false;
            let baseline = crate::mir6502::materialize_program_with_origin_and_runtime(
                lower(&source),
                &off,
                0x3000,
                runtime,
            )
            .unwrap();
            let selected = crate::mir6502::materialize_program_with_origin_and_runtime(
                lower(&source),
                &config,
                0x3000,
                runtime,
            )
            .unwrap();
            let old = Cost::measure(&baseline, 0x3000).unwrap();
            let new = Cost::measure(&selected, 0x3000).unwrap();
            assert!(new.bytes <= old.bytes);
            for name in ["Address", "ReadViaLocal"] {
                let r = selected.routines.iter().find(|r| r.name == name).unwrap();
                assert!(
                    new.routine(r.id).unwrap() < old.routine(r.id).unwrap(),
                    "{runtime:?}/{name}"
                );
                assert!(
                    !r.blocks
                        .iter()
                        .flat_map(|b| &b.ops)
                        .any(|op| matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. })),
                    "{runtime:?}/{name}: no multiply call"
                );
                assert!(
                    r.frame.spills.is_empty(),
                    "{runtime:?}/{name}: no spill bytes"
                );
                if name == "ReadViaLocal" {
                    let stores = r
                        .blocks
                        .iter()
                        .flat_map(|b| &b.ops)
                        .filter(|op| {
                            matches!(
                                op,
                                MirOp::Store {
                                    dst: MirAddr::Direct(MirMem::Local { .. }),
                                    width: MirWidth::Byte,
                                    ..
                                }
                            )
                        })
                        .count();
                    assert_eq!(stores, 2, "{runtime:?}: retain both source pointer bytes");
                }
            }
        }
    }
    #[test]
    fn complete_index_loads_defer_only_from_ordinary_disjoint_storage() {
        let mut p =
            lower("CARD FUNC F(CARD index) RETURN(index) PROC Main() CARD value value=F(1) RETURN");
        let mem = MirMem::Param {
            id: crate::nir::ParamId(0),
            offset: 0,
        };
        let temp = MirTempId(0);
        let ops = vec![
            MirOp::Load {
                dst: MirDef::VTemp(temp),
                src: MirAddr::Direct(mem.clone()),
                width: MirWidth::Word,
            },
            MirOp::MaterializeAddress {
                consumer: DEFAULT_POINTER_PAIR,
                value: MirValue::ConstU16(0x5000),
            },
            MirOp::AdvanceAddress {
                consumer: DEFAULT_POINTER_PAIR,
                index: MirValue::Def(MirDef::VTemp(temp)),
                scale: 20,
            },
        ];
        p.routines[0].temps = vec![MirTemp { id: temp }];
        p.routines[0].blocks[0].ops = ops.clone();
        let layout = MaterializeLayout::new(&p, 0x3000);
        assert_eq!(
            word_values::forward_unique_word_load_address_consumers(
                &mut p.routines[0],
                &layout,
                false
            ),
            0
        );
        assert_eq!(
            word_values::forward_unique_word_load_address_consumers(
                &mut p.routines[0],
                &layout,
                true
            ),
            1
        );
        for address in [0xAC, 0xAD, 0xAF, 0xD000] {
            p.routines[0].blocks[0].ops = ops.clone();
            p.routines[0].frame.params[0].base = MirStorageBase::Absolute(address);
            let layout = MaterializeLayout::new(&p, 0x3000);
            assert_eq!(
                word_values::forward_unique_word_load_address_consumers(
                    &mut p.routines[0],
                    &layout,
                    true
                ),
                0
            );
        }
        // A hardware or unknown-pointer read can have ordering effects even
        // without an explicit store. Preserve the original index snapshot.
        p.routines[0].frame.params[0].base = MirStorageBase::Param(crate::nir::ParamId(0));
        for read in [
            MirOp::Load {
                dst: MirDef::VTempByte {
                    id: MirTempId(1),
                    byte: 0,
                },
                src: MirAddr::Direct(MirMem::Absolute(0xD000)),
                width: MirWidth::Byte,
            },
            MirOp::LoadIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                dst: MirDef::VTempByte {
                    id: MirTempId(1),
                    byte: 0,
                },
                offset: 0,
            },
        ] {
            p.routines[0].temps = vec![MirTemp { id: temp }, MirTemp { id: MirTempId(1) }];
            p.routines[0].blocks[0].ops = ops.clone();
            p.routines[0].blocks[0].ops.insert(1, read);
            let layout = MaterializeLayout::new(&p, 0x3000);
            assert_eq!(
                word_values::forward_unique_word_load_address_consumers(
                    &mut p.routines[0],
                    &layout,
                    true
                ),
                0
            );
        }
        p.routines[0].blocks[0].ops = vec![
            ops[0].clone(),
            MirOp::Load {
                dst: MirDef::VTempByte {
                    id: MirTempId(1),
                    byte: 0,
                },
                src: MirAddr::Direct(MirMem::Absolute(0xD000)),
                width: MirWidth::Byte,
            },
            MirOp::MaterializeIndexedAddress {
                consumer: DEFAULT_POINTER_PAIR,
                base: MirValue::Def(MirDef::VTemp(temp)),
                index: MirValue::ConstU16(38),
                scale: 1,
            },
            MirOp::AdvanceAddress {
                consumer: DEFAULT_POINTER_PAIR,
                index: MirValue::Word {
                    lo: Box::new(MirValue::Def(MirDef::VTempByte {
                        id: MirTempId(1),
                        byte: 0,
                    })),
                    hi: Box::new(MirValue::ConstU8(0)),
                },
                scale: 20,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                src: complete_pair_value(),
                width: MirWidth::Word,
            },
        ];
        let layout = MaterializeLayout::new(&p, 0x3000);
        assert_eq!(
            word_values::forward_unique_word_load_address_consumers(
                &mut p.routines[0],
                &layout,
                true
            ),
            0,
            "complete value construction preserves a base snapshot across a hardware index read"
        );
    }
    #[test]
    fn retained_pointer_fields_preserve_the_store_and_reject_forged_deltas() {
        let mut p = lower(
            "TYPE E=[CARD value BYTE ARRAY pad(18)] TYPE T=[BYTE ARRAY pad(38) E ARRAY rows(2)] CARD FUNC ReadViaLocal(T POINTER table CARD index) E POINTER item item=@table.rows(index) RETURN(item.value) PROC Main() RETURN",
        );
        let layout = MaterializeLayout::new(&p, 0x3000);
        let r = &mut p.routines[0];
        MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                r,
                crate::mir6502::rewrite::address_values::discover_indexed_address_values,
                crate::mir6502::rewrite::address_values::address_value_rank,
            )
            .unwrap();
        let original = r.clone();
        let snapshot = crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot::new(
            r,
            crate::mir6502::analysis::sites::MirRoutineGeneration::initial(),
        )
        .unwrap();
        let context = PreHomeRewriteContext::new(&snapshot);
        let plan = discover_address_stores(r, &context, &layout)
            .into_iter()
            .find(|p| p.exit_effect_delta == MirEffectDelta::MaterializedRetainedPointerField)
            .unwrap();
        let ops = &r.blocks[0].ops[plan.range.clone()];
        let widths = collect_routine_temp_widths(r);
        assert!(retained_pointer_field_is_justified(
            ops,
            &plan.replacement,
            &widths
        ));
        let mut forged = plan.replacement.clone();
        forged.retain(|op| {
            !matches!(
                op,
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Local { .. }),
                    ..
                }
            )
        });
        assert!(!retained_pointer_field_is_justified(ops, &forged, &widths));
        let mut forged = plan.replacement.clone();
        if let MirOp::LoadIndirect { offset, .. } = forged.last_mut().unwrap() {
            *offset += 1;
        }
        assert!(!retained_pointer_field_is_justified(ops, &forged, &widths));
        drop(snapshot);
        assert_eq!(
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    r,
                    |r, c| discover_address_stores(r, c, &layout),
                    crate::mir6502::rewrite::address_values::address_value_rank
                )
                .unwrap()
                .applied,
            1
        );
        assert!(r.blocks[0].ops.iter().any(|op| matches!(
            op,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Local { .. }),
                width: MirWidth::Word,
                ..
            }
        )));
        for barrier in [
            MirOp::Barrier {
                effects: MirEffects::default(),
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                src: MirValue::ConstU8(9),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Local {
                    id: crate::nir::LocalId(0),
                    offset: 1,
                }),
                src: MirValue::ConstU8(9),
                width: MirWidth::Byte,
            },
        ] {
            *r = original.clone();
            r.blocks[0].ops.insert(plan.range.start + 2, barrier);
            let snapshot = crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot::new(
                r,
                crate::mir6502::analysis::sites::MirRoutineGeneration::initial(),
            )
            .unwrap();
            assert!(
                !discover_address_stores(r, &PreHomeRewriteContext::new(&snapshot), &layout)
                    .iter()
                    .any(
                        |p| p.exit_effect_delta == MirEffectDelta::MaterializedRetainedPointerField
                    )
            );
        }
    }
    #[test]
    fn saved_byte_index_requires_dead_home_and_disjoint_physical_inputs() {
        let mut p = lower("PROC Main() RETURN");
        let r = &mut p.routines[0];
        r.frame.spills = vec![MirSpillId(0)];
        let home = MirMem::Spill {
            id: MirSpillId(0),
            offset: 0,
        };
        let window = vec![
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(home.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::MaterializeIndexedAddress {
                consumer: DEFAULT_POINTER_PAIR,
                base: MirValue::ConstU16(0x5000),
                index: MirValue::ConstU16(38),
                scale: 1,
            },
            MirOp::AdvanceAddress {
                consumer: DEFAULT_POINTER_PAIR,
                index: MirValue::Word {
                    lo: Box::new(MirValue::PointerCell(home.clone())),
                    hi: Box::new(MirValue::ConstU8(0)),
                },
                scale: 20,
            },
        ];
        r.blocks[0].ops = window.clone();
        let layout = MaterializeLayout::new(&p, 0x3000);
        let address = layout.mem_address(p.routines[0].id, &home).unwrap();
        let run = |r: &mut MirRoutine| {
            MirPostHomeRewriteDriver::default()
                .run_fixed_point(r, |r, c| discover_saved_byte_indexes(r, c, &layout))
                .unwrap()
                .applied
        };
        let mut r = p.routines[0].clone();
        assert_eq!(run(&mut r), 1);
        assert_eq!(r.blocks[0].ops.len(), 2);
        // A was changed by the removed TYA; it cannot be an original base input.
        r.blocks[0].ops = window.clone();
        if let MirOp::MaterializeIndexedAddress { base, .. } = &mut r.blocks[0].ops[2] {
            *base = MirValue::Word {
                lo: Box::new(MirValue::Def(MirDef::Reg(MirReg::A))),
                hi: Box::new(MirValue::ConstU8(0x50)),
            };
        }
        assert_eq!(run(&mut r), 0);
        // Distinct symbolic names can refer to the exact same physical byte.
        r.blocks[0].ops = window.clone();
        if let MirOp::MaterializeIndexedAddress { base, .. } = &mut r.blocks[0].ops[2] {
            *base = MirValue::Word {
                lo: Box::new(MirValue::PointerCell(MirMem::Absolute(address))),
                hi: Box::new(MirValue::ConstU8(0x50)),
            };
        }
        assert_eq!(run(&mut r), 0);
        for mem in [home.clone(), MirMem::Absolute(address)] {
            r.blocks[0].ops = window.clone();
            r.blocks[0].ops.push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            });
            assert_eq!(run(&mut r), 0);
        }
        // A successor's use is included in the shared home-deadness proof.
        r.blocks[0].ops = window;
        r.blocks[0].terminator = MirTerminator::Jump(MirEdge {
            target: MirBlockId(1),
            args: vec![],
        });
        r.blocks.push(MirBlock {
            id: MirBlockId(1),
            label: "successor".into(),
            params: vec![],
            ops: vec![MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(home),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Return,
        });
        assert_eq!(run(&mut r), 0);
    }
    #[test]
    fn address_value_cost_includes_final_code_and_homes_for_two_layouts() {
        for (stride, offset) in [(2u16, 38u16), (20, 256), (255, 254), (256, 255)] {
            let pad = if stride == 2 {
                String::new()
            } else {
                format!("BYTE ARRAY padding({})", stride - 2)
            };
            let source = format!(
                "TYPE E=[CARD value {pad}] TYPE T=[BYTE ARRAY prefix({offset}) E ARRAY rows(2)] CARD result=$6000 CARD FUNC Address(T POINTER p CARD index) RETURN(@p.rows(index)) PROC Main() result=Address($5000,257) RETURN"
            );
            for runtime in [
                crate::runtime::Runtime::ActionCart,
                crate::runtime::Runtime::Standalone,
            ] {
                let input = lower(&source);
                let config = Mir6502Config::optimized();
                let mut off = config.clone();
                off.enable_indexed_address_values = false;
                let baseline = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input.clone(),
                    &off,
                    0x3000,
                    runtime,
                )
                .unwrap();
                let output = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input, &config, 0x3000, runtime,
                )
                .unwrap();
                assert!(
                    Cost::measure(&output, 0x3000).unwrap().bytes
                        <= Cost::measure(&baseline, 0x3000).unwrap().bytes,
                    "stride={stride}/offset={offset}/{runtime:?}"
                );
            }
        }
    }
    #[test]
    fn address_value_selection_rejects_source_scratch_aliases_and_absolute_locals() {
        for cell in [0xACu16, 0xAD, 0xAE, 0xAF] {
            let p = lower(&format!(
                "CARD input=${cell:04X},result=$6000 PROC Main() result=input*20+100 RETURN"
            ));
            assert!(!scratch_is_private(&p, 0x3000));
        }
        let mut p = lower(
            "TYPE E=[CARD value] E object CARD result PROC Main() E POINTER p p=@object result=p.value RETURN",
        );
        p.routines[0].frame.locals[0].base = MirStorageBase::Absolute(0xAE);
        assert!(!scratch_is_private(&p, 0x3000));
        assert!(!scratch_is_private(&p, 0x80));
    }
}

#[cfg(test)]
mod execution_tests {
    use super::tests::lower;
    use super::*;
    #[test]
    fn retained_record_fields_execute_both_profiles_with_full_word_wrap() {
        let source = "TYPE E=[CARD value BYTE ARRAY pad(18)] TYPE T=[BYTE ARRAY prefix(38) E ARRAY rows(2)] T POINTER base=$6000 CARD indexValue=$6002,input=$6004,observed=$6006,pointerValue=$6008 PROC Main() E POINTER item item=@base.rows(indexValue) observed=item.value pointerValue=CARD(item) item.value=input RETURN";
        for config in [Mir6502Config::default(), Mir6502Config::optimized()] {
            for runtime in [
                crate::runtime::Runtime::ActionCart,
                crate::runtime::Runtime::Standalone,
            ] {
                let mut selected = lower(source);
                for r in &mut selected.routines {
                    MirPreHomeRewriteDriver::default().run_fixed_point_by_key(r,
                        crate::mir6502::rewrite::address_values::discover_indexed_address_values,
                        crate::mir6502::rewrite::address_values::address_value_rank).unwrap();
                }
                // Exercise construction even when the image cost gate would
                // decline this particular absolute-input test program.
                let mir = crate::mir6502::materialize_resolved_program_without_address_costing(
                    selected, &config, 0x3000, runtime, false,
                )
                .unwrap();
                let mut emitter =
                    crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
                let summary =
                    crate::mir6502::emit::emit_program(&mir, 0x3000, &mut emitter).unwrap();
                let output = crate::mir6502::codegen_output(
                    emitter.finish_with_relocations().unwrap(),
                    0x3000,
                    summary,
                    runtime,
                    &mir,
                );
                for base in [0x5000u16, 0x50F1, 0xFFD9] {
                    for index in [0u16, 1, 255, 256, 257, 65535] {
                        let pointer = base.wrapping_add(38).wrapping_add(index.wrapping_mul(20));
                        if pointer < 0x4000 {
                            continue;
                        }
                        let mut ram = [0u8; 65536];
                        if runtime == crate::runtime::Runtime::ActionCart {
                            let cart = include_bytes!("../../../roms/action.rom");
                            ram[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
                            ram[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
                        }
                        ram[0x3000..0x3000 + output.bytes.len()].copy_from_slice(&output.bytes);
                        ram[0x6000..0x6002].copy_from_slice(&base.to_le_bytes());
                        ram[0x6002..0x6004].copy_from_slice(&index.to_le_bytes());
                        ram[0x6004..0x6006].copy_from_slice(&0xB327u16.to_le_bytes());
                        ram[usize::from(pointer)] = 0xD9;
                        ram[usize::from(pointer.wrapping_add(1))] = 0x73;
                        super::super::leaf_test_cpu::run_memory(
                            &mut ram,
                            usize::from(output.run_address),
                        );
                        let label = format!(
                            "{runtime:?}/{}/{base:04X}/{index}",
                            config.enable_indexed_address_values
                        );
                        assert_eq!(
                            &ram[0x6006..0x6008],
                            &0x73D9u16.to_le_bytes(),
                            "{label}: full field read"
                        );
                        assert_eq!(
                            &ram[0x6008..0x600A],
                            &pointer.to_le_bytes(),
                            "{label}: retained pointer"
                        );
                        assert_eq!(
                            [
                                ram[usize::from(pointer)],
                                ram[usize::from(pointer.wrapping_add(1))]
                            ],
                            0xB327u16.to_le_bytes(),
                            "{label}: full field write"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn indexed_address_store_trials_preserve_complete_values_and_return_lanes() {
        for ty in ["BYTE", "CARD"] {
            for (stride, offset) in [
                (1u16, 38u16),
                (2, 256),
                (20, 0),
                (20, 38),
                (255, 0),
                (255, 255),
                (256, 254),
            ] {
                let element = if stride == 1 {
                    "BYTE value".to_string()
                } else {
                    format!(
                        "CARD value {}",
                        if stride == 2 {
                            String::new()
                        } else {
                            format!("BYTE ARRAY pad({})", stride - 2)
                        }
                    )
                };
                let source = format!(
                    "TYPE E=[{element}] TYPE T=[{} E ARRAY rows(2)] T POINTER base=$6000 {ty} indexValue=$6002 CARD result=$6004 CARD FUNC Address(T POINTER p {ty} index) RETURN(@p.rows(index)) PROC Main() result=Address(base,indexValue) RETURN",
                    if offset == 0 {
                        String::new()
                    } else {
                        format!("BYTE ARRAY prefix({offset})")
                    }
                );
                for runtime in [
                    crate::runtime::Runtime::ActionCart,
                    crate::runtime::Runtime::Standalone,
                ] {
                    let mut selected = lower(&source);
                    for r in &mut selected.routines {
                        MirPreHomeRewriteDriver::default().run_fixed_point_by_key(r,crate::mir6502::rewrite::address_values::discover_indexed_address_values,crate::mir6502::rewrite::address_values::address_value_rank).unwrap();
                    }
                    let mir = crate::mir6502::materialize_resolved_program_without_address_costing(
                        selected,
                        &Mir6502Config::optimized(),
                        0x3000,
                        runtime,
                        false,
                    )
                    .unwrap();
                    if ty == "BYTE"
                        && stride == 20
                        && runtime == crate::runtime::Runtime::ActionCart
                    {
                        if let Ok(directory) = std::env::var("ACTIONC_ADDRESS_TRIAL_DUMP") {
                            std::fs::create_dir_all(&directory).unwrap();
                            std::fs::write(
                                std::path::Path::new(&directory).join("stride20-trial.mir"),
                                crate::mir6502::format_program(&mir),
                            )
                            .unwrap();
                        }
                    }
                    let mut emitter =
                        crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
                    let summary =
                        crate::mir6502::emit::emit_program(&mir, 0x3000, &mut emitter).unwrap();
                    let emission = emitter.finish_with_relocations().unwrap();
                    let output =
                        crate::mir6502::codegen_output(emission, 0x3000, summary, runtime, &mir);
                    for base in [0x5000u16, 0x50F1, 0xFFF0] {
                        for index in [0u16, 1, 127, 255, 256, 257, 65535] {
                            if ty == "BYTE" && index > 255 {
                                continue;
                            }
                            let mut ram = [0u8; 65536];
                            if runtime == crate::runtime::Runtime::ActionCart {
                                let cart = include_bytes!("../../../roms/action.rom");
                                ram[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
                                ram[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
                            }
                            ram[0x3000..0x3000 + output.bytes.len()].copy_from_slice(&output.bytes);
                            ram[0x6000..0x6002].copy_from_slice(&base.to_le_bytes());
                            ram[0x6002..0x6004].copy_from_slice(&index.to_le_bytes());
                            super::super::leaf_test_cpu::run_memory(
                                &mut ram,
                                usize::from(output.run_address),
                            );
                            let expected = base
                                .wrapping_add(offset)
                                .wrapping_add(index.wrapping_mul(stride));
                            assert_eq!(
                                u16::from_le_bytes([ram[0x6004], ram[0x6005]]),
                                expected,
                                "{ty}/stride={stride}/offset={offset}/base={base:04X}/index={index}/{runtime:?}"
                            );
                            assert_eq!([ram[0xA0], ram[0xA1]], expected.to_le_bytes());
                        }
                    }
                }
            }
        }
    }
}
