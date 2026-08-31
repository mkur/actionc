use super::dead_spills::block_successor_indices;
#[cfg(test)]
use super::peepholes::private_scratch_store_removal_is_safe_after;
use super::temps::temp_def_spill;
use super::*;
use crate::mir6502::analysis::effects::{MirFlagSet, MirHomeByte, classify_op};
use crate::mir6502::analysis::home_liveness::MirHomeLiveness;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{
    MirAddr, MirBlock, MirBlockId, MirCallTarget, MirDef, MirFixedZpSlot, MirMem, MirOp,
    MirRegisterSet, MirRoutine, MirSpillId, MirTerminator, MirValue, MirZpSlot, RoutineId,
};
use crate::mir6502::rewrite::context::{MirExitStateChange, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::MirPostHomeRewritePlan;
use crate::mir6502::rewrite::posthome::structural_plan;
use std::collections::{BTreeMap, BTreeSet};

#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirSpillAccounting {
    pub allocated: usize,
    pub emitted_into_storage: usize,
    pub written: usize,
    pub read: usize,
    pub one_write_one_immediate_read: usize,
    pub live_across_calls: usize,
    pub live_across_block_joins: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MirHomeStorage {
    Spill(MirSpillId),
    ZeroPage(MirZpSlot),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct MirHomeAccessCount {
    pub(super) reads: usize,
    pub(super) writes: usize,
}

pub(super) fn home_access_counts(
    routine: &MirRoutine,
) -> BTreeMap<MirHomeStorage, MirHomeAccessCount> {
    let mut counts = BTreeMap::<MirHomeStorage, MirHomeAccessCount>::new();
    for block in &routine.blocks {
        for op in &block.ops {
            for home in op_read_homes(op) {
                counts.entry(home).or_default().reads += 1;
            }
            for home in op_write_homes(op) {
                counts.entry(home).or_default().writes += 1;
            }
        }
        if let MirTerminator::Branch {
            cond: MirCond::BoolValue(value),
            ..
        } = &block.terminator
        {
            let mut reads = BTreeSet::new();
            collect_value_read_homes(value, &mut reads);
            for home in reads {
                counts.entry(home).or_default().reads += 1;
            }
        }
    }
    counts
}

fn op_read_homes(op: &MirOp) -> BTreeSet<MirHomeStorage> {
    if matches!(op, MirOp::RuntimeHelper { .. }) {
        return BTreeSet::new();
    }
    classify_op(op)
        .homes
        .reads
        .into_iter()
        .filter_map(home_storage)
        .collect()
}

fn op_write_homes(op: &MirOp) -> BTreeSet<MirHomeStorage> {
    if matches!(op, MirOp::RuntimeHelper { .. }) {
        return BTreeSet::new();
    }
    classify_op(op)
        .homes
        .writes
        .into_iter()
        .filter_map(home_storage)
        .collect()
}

fn home_storage(home: MirHomeByte) -> Option<MirHomeStorage> {
    match home {
        MirHomeByte::Spill { id, .. } => Some(MirHomeStorage::Spill(id)),
        MirHomeByte::VirtualZeroPage(slot) => Some(MirHomeStorage::ZeroPage(slot)),
        MirHomeByte::FixedZeroPage(_) => None,
    }
}

fn collect_value_read_homes(value: &MirValue, homes: &mut BTreeSet<MirHomeStorage>) {
    match value {
        MirValue::PointerCell(mem) | MirValue::StorageAddrByte { mem, .. } => {
            collect_mem_home(mem, homes)
        }
        MirValue::Word { lo, hi } => {
            collect_value_read_homes(lo, homes);
            collect_value_read_homes(hi, homes);
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => {}
    }
}

fn collect_mem_home(mem: &MirMem, homes: &mut BTreeSet<MirHomeStorage>) {
    match mem {
        MirMem::Spill { id, .. } => {
            homes.insert(MirHomeStorage::Spill(*id));
        }
        MirMem::ZeroPage(slot) => {
            homes.insert(MirHomeStorage::ZeroPage(*slot));
        }
        MirMem::Param { .. }
        | MirMem::Local { .. }
        | MirMem::Static { .. }
        | MirMem::Global { .. }
        | MirMem::Absolute(_)
        | MirMem::FixedZeroPage(_) => {}
    }
}

#[allow(dead_code)]
pub(in crate::mir6502) fn spill_accounting_for_routine(routine: &MirRoutine) -> MirSpillAccounting {
    let mut accounting = MirSpillAccounting {
        allocated: routine.frame.spills.len(),
        emitted_into_storage: routine.frame.spills.len(),
        ..MirSpillAccounting::default()
    };
    let mut writes = BTreeMap::<MirSpillId, usize>::new();
    let mut reads = BTreeMap::<MirSpillId, usize>::new();
    let mut immediate_read_pairs = BTreeMap::<MirSpillId, usize>::new();
    let mut live_across_calls = BTreeSet::<MirSpillId>::new();
    let mut live_across_block_joins = BTreeSet::<MirSpillId>::new();
    let predecessors = block_predecessor_counts(&routine.blocks);

    for block in &routine.blocks {
        let mut written_in_block = BTreeSet::<MirSpillId>::new();
        for (index, op) in block.ops.iter().enumerate() {
            let op_reads = op_read_spills(op);
            let op_writes = op_write_spills(op);
            for spill in &op_reads {
                *reads.entry(*spill).or_insert(0) += 1;
                if predecessors.get(&block.id).copied().unwrap_or(0) > 1
                    && !written_in_block.contains(spill)
                {
                    live_across_block_joins.insert(*spill);
                }
            }
            for spill in &op_writes {
                *writes.entry(*spill).or_insert(0) += 1;
                written_in_block.insert(*spill);
                if op_reads_spill(ops_get_next(&block.ops, index), *spill) {
                    *immediate_read_pairs.entry(*spill).or_insert(0) += 1;
                }
            }
            if op_is_call_barrier(op) {
                let later_reads = block.ops[index + 1..]
                    .iter()
                    .flat_map(op_read_spills)
                    .collect::<BTreeSet<_>>();
                for spill in written_in_block.intersection(&later_reads) {
                    live_across_calls.insert(*spill);
                }
            }
        }
    }

    accounting.written = writes.values().sum();
    accounting.read = reads.values().sum();
    accounting.one_write_one_immediate_read = routine
        .frame
        .spills
        .iter()
        .filter(|spill| {
            writes.get(spill).copied().unwrap_or(0) == 1
                && reads.get(spill).copied().unwrap_or(0) == 1
                && immediate_read_pairs.get(spill).copied().unwrap_or(0) == 1
        })
        .count();
    accounting.live_across_calls = live_across_calls.len();
    accounting.live_across_block_joins = live_across_block_joins.len();
    accounting
}

fn ops_get_next(ops: &[MirOp], index: usize) -> Option<&MirOp> {
    ops.get(index + 1)
}

fn block_predecessor_counts(blocks: &[MirBlock]) -> BTreeMap<MirBlockId, usize> {
    let mut counts = BTreeMap::new();
    for block in blocks {
        match &block.terminator {
            MirTerminator::Jump(edge) => {
                *counts.entry(edge.target).or_insert(0) += 1;
            }
            MirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                *counts.entry(then_edge.target).or_insert(0) += 1;
                *counts.entry(else_edge.target).or_insert(0) += 1;
            }
            MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
        }
    }
    counts
}

pub(super) fn op_is_call_barrier(op: &MirOp) -> bool {
    matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. })
}

fn op_reads_spill(op: Option<&MirOp>, spill: MirSpillId) -> bool {
    op.is_some_and(|op| op_read_spills(op).contains(&spill))
}

fn op_read_spills(op: &MirOp) -> BTreeSet<MirSpillId> {
    classify_op(op).projected_spill_reads
}

fn op_write_spills(op: &MirOp) -> BTreeSet<MirSpillId> {
    if matches!(op, MirOp::Compare { .. } | MirOp::Call { .. }) {
        BTreeSet::new()
    } else {
        classify_op(op).projected_spill_writes
    }
}

#[cfg(test)]
pub(super) fn fold_indirect_load_spill_consumers(
    ops: Vec<MirOp>,
    live_out: &MirTempLiveSet,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < ops.len() {
        if let Some(consumed) =
            try_fold_indirect_load_spill_consumer(&ops, index, live_out, &mut out)
        {
            index += consumed;
            continue;
        }
        out.push(ops[index].clone());
        index += 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AccumulatorSpillValue {
    id: MirSpillId,
    offset: u16,
}

#[cfg(test)]
pub(super) fn forward_block_local_spill_accumulator(
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
) -> Vec<MirOp> {
    let mut out = Vec::with_capacity(ops.len());
    let mut a_value: Option<AccumulatorSpillValue> = None;
    let mut index = 0usize;
    while index < ops.len() {
        if let Some(consumed) =
            try_forward_immediate_register_spill_consumer(&ops, index, terminator, &mut out)
        {
            a_value = None;
            index += consumed;
            continue;
        }
        if can_remove_spill_store_reload_pair_at(&ops, index, terminator) {
            a_value = None;
            index += 2;
            continue;
        }

        let op = &ops[index];
        if let MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src:
                MirAddr::Direct(MirMem::Spill {
                    id: load_id,
                    offset: load_offset,
                }),
            width: MirWidth::Byte,
        } = &op
            && a_value
                == Some(AccumulatorSpillValue {
                    id: *load_id,
                    offset: *load_offset,
                })
            && (can_remove_spill_reload_at(&ops, index, terminator)
                || can_remove_spill_reload_before_later_a_use(&ops, index, terminator))
        {
            index += 1;
            continue;
        }

        update_accumulator_spill_value(&mut a_value, op);
        out.push(op.clone());
        index += 1;
    }
    out
}

pub(in crate::mir6502) fn discover_spill_forwards(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        let mut a_value = None;
        for index in 0..block.ops.len() {
            for (replacement, kept_stores) in indirect_load_spill_forward_shapes(&block.ops, index)
            {
                let consumed =
                    if matches!(block.ops.get(index + 2), Some(MirOp::LoadIndirect { .. })) {
                        8
                    } else {
                        4
                    };
                if let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    "indirect-load-spill-consumer",
                    kept_stores,
                ) {
                    plans.push(plan);
                }
            }

            if let Some((consumed, replacement)) =
                immediate_register_spill_forward_shape(&block.ops, index)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    accumulator_exit_change(),
                    "immediate-register-spill-forward",
                    10,
                )
            {
                plans.push(plan);
            }

            if spill_store_reload_shape_at(&block.ops, index).is_some()
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + 2,
                    Vec::new(),
                    zn_exit_change(),
                    "spill-store-reload-pair",
                    11,
                )
            {
                plans.push(plan);
            }

            if let Some(spill) = load_a_spill_byte(block.ops.get(index))
                && a_value == Some(spill)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + 1,
                    Vec::new(),
                    zn_exit_change(),
                    "spill-accumulator-reload",
                    12,
                )
            {
                plans.push(plan);
            }

            update_accumulator_spill_value(&mut a_value, &block.ops[index]);
        }
    }
    plans
}

fn indirect_load_spill_forward_shapes(ops: &[MirOp], index: usize) -> Vec<(Vec<MirOp>, u16)> {
    if matches!(ops.get(index + 2), Some(MirOp::LoadIndirect { .. })) {
        let Some(MirOp::LoadIndirect {
            consumer: lo,
            offset: lo_offset,
            dst: MirDef::Reg(MirReg::A),
        }) = ops.get(index)
        else {
            return Vec::new();
        };
        let Some(MirOp::Store {
            dst:
                MirAddr::Direct(MirMem::Spill {
                    id: lo_spill,
                    offset: 0,
                }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }) = ops.get(index + 1)
        else {
            return Vec::new();
        };
        let Some(MirOp::LoadIndirect {
            consumer: hi,
            offset: hi_offset,
            dst: MirDef::Reg(MirReg::A),
        }) = ops.get(index + 2)
        else {
            return Vec::new();
        };
        let Some(MirOp::Store {
            dst:
                MirAddr::Direct(MirMem::Spill {
                    id: hi_spill,
                    offset: 0,
                }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }) = ops.get(index + 3)
        else {
            return Vec::new();
        };
        let Some(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src:
                MirAddr::Direct(MirMem::Spill {
                    id: lo_reload,
                    offset: 0,
                }),
            width: MirWidth::Byte,
        }) = ops.get(index + 4)
        else {
            return Vec::new();
        };
        let Some(
            lo_store @ MirOp::Store {
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
                ..
            },
        ) = ops.get(index + 5)
        else {
            return Vec::new();
        };
        let Some(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src:
                MirAddr::Direct(MirMem::Spill {
                    id: hi_reload,
                    offset: 0,
                }),
            width: MirWidth::Byte,
        }) = ops.get(index + 6)
        else {
            return Vec::new();
        };
        let Some(
            hi_store @ MirOp::Store {
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
                ..
            },
        ) = ops.get(index + 7)
        else {
            return Vec::new();
        };
        if lo != hi
            || *hi_offset != lo_offset.saturating_add(1)
            || lo_reload != lo_spill
            || hi_reload != hi_spill
        {
            return Vec::new();
        }
        return (0u16..4)
            .map(|mask| {
                let mut replacement = vec![ops[index].clone()];
                if mask & 1 != 0 {
                    replacement.push(ops[index + 1].clone());
                }
                replacement.push(lo_store.clone());
                replacement.push(ops[index + 2].clone());
                if mask & 2 != 0 {
                    replacement.push(ops[index + 3].clone());
                }
                replacement.push(hi_store.clone());
                (replacement, mask.count_ones() as u16)
            })
            .collect();
    }

    let Some(
        load @ MirOp::LoadIndirect {
            dst: MirDef::Reg(MirReg::A),
            ..
        },
    ) = ops.get(index)
    else {
        return Vec::new();
    };
    let Some(
        store @ MirOp::Store {
            dst:
                MirAddr::Direct(MirMem::Spill {
                    id: spill,
                    offset: 0,
                }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ) = ops.get(index + 1)
    else {
        return Vec::new();
    };
    let Some(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src:
            MirAddr::Direct(MirMem::Spill {
                id: reload,
                offset: 0,
            }),
        width: MirWidth::Byte,
    }) = ops.get(index + 2)
    else {
        return Vec::new();
    };
    let Some(
        final_store @ MirOp::Store {
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
            ..
        },
    ) = ops.get(index + 3)
    else {
        return Vec::new();
    };
    if spill != reload {
        return Vec::new();
    }
    vec![
        (vec![load.clone(), final_store.clone()], 0),
        (vec![load.clone(), store.clone(), final_store.clone()], 1),
    ]
}

fn immediate_register_spill_forward_shape(
    ops: &[MirOp],
    index: usize,
) -> Option<(usize, Vec<MirOp>)> {
    let MirOp::Store {
        dst: MirAddr::Direct(store),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let MirOp::Load {
        dst: MirDef::Reg(reg @ (MirReg::X | MirReg::Y)),
        src: MirAddr::Direct(load),
        width: MirWidth::Byte,
    } = ops.get(index + 2)?
    else {
        return None;
    };
    if !matches!(store, MirMem::Spill { .. }) || store != load {
        return None;
    }
    let replacement = match ops.get(index)? {
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value,
            width: MirWidth::Byte,
        } => MirOp::LoadImm {
            dst: MirDef::Reg(*reg),
            value: *value,
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(src),
            width: MirWidth::Byte,
        } => MirOp::Load {
            dst: MirDef::Reg(*reg),
            src: MirAddr::Direct(src.clone()),
            width: MirWidth::Byte,
        },
        _ => return None,
    };
    Some((3, vec![replacement]))
}

fn spill_store_reload_shape_at(ops: &[MirOp], index: usize) -> Option<()> {
    let MirOp::Store {
        dst: MirAddr::Direct(store @ MirMem::Spill { .. }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(load),
        width: MirWidth::Byte,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    (store == load).then_some(())
}

fn load_a_spill_byte(op: Option<&MirOp>) -> Option<AccumulatorSpillValue> {
    let MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(MirMem::Spill { id, offset }),
        width: MirWidth::Byte,
    } = op?
    else {
        return None;
    };
    Some(AccumulatorSpillValue {
        id: *id,
        offset: *offset,
    })
}

fn accumulator_exit_change() -> MirExitStateChange {
    MirExitStateChange {
        registers: MirRegisterSet {
            a: true,
            ..MirRegisterSet::default()
        },
        ..MirExitStateChange::default()
    }
}

fn zn_exit_change() -> MirExitStateChange {
    MirExitStateChange {
        flags: MirFlagSet {
            z: true,
            n: true,
            ..MirFlagSet::default()
        },
        ..MirExitStateChange::default()
    }
}

#[cfg(test)]
fn try_forward_immediate_register_spill_consumer(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let Some(MirOp::Store {
        dst:
            MirAddr::Direct(MirMem::Spill {
                id: store_id,
                offset: store_offset,
            }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }) = ops.get(index + 1)
    else {
        return None;
    };
    let Some(MirOp::Load {
        dst: MirDef::Reg(reg @ (MirReg::X | MirReg::Y)),
        src:
            MirAddr::Direct(MirMem::Spill {
                id: load_id,
                offset: load_offset,
            }),
        width: MirWidth::Byte,
    }) = ops.get(index + 2)
    else {
        return None;
    };
    if store_id != load_id || store_offset != load_offset {
        return None;
    }
    let stored = MirMem::Spill {
        id: *store_id,
        offset: *store_offset,
    };
    if !private_scratch_store_removal_is_safe_after(ops, index + 3, terminator, &stored) {
        return None;
    }

    match ops.get(index)? {
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value,
            width: MirWidth::Byte,
        } => {
            out.push(MirOp::LoadImm {
                dst: MirDef::Reg(*reg),
                value: *value,
                width: MirWidth::Byte,
            });
            Some(3)
        }
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(src),
            width: MirWidth::Byte,
        } => {
            out.push(MirOp::Load {
                dst: MirDef::Reg(*reg),
                src: MirAddr::Direct(src.clone()),
                width: MirWidth::Byte,
            });
            Some(3)
        }
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn can_remove_spill_store_reload_pair_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> bool {
    let Some(MirOp::Store {
        dst:
            MirAddr::Direct(MirMem::Spill {
                id: store_id,
                offset: store_offset,
            }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }) = ops.get(index)
    else {
        return false;
    };
    let Some(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src:
            MirAddr::Direct(MirMem::Spill {
                id: load_id,
                offset: load_offset,
            }),
        width: MirWidth::Byte,
    }) = ops.get(index + 1)
    else {
        return false;
    };
    if store_id != load_id || store_offset != load_offset {
        return false;
    }
    if !can_remove_spill_reload_at(ops, index + 1, terminator) {
        return false;
    }
    private_scratch_store_removal_is_safe_after(
        ops,
        index + 2,
        terminator,
        &MirMem::Spill {
            id: *store_id,
            offset: *store_offset,
        },
    )
}

#[cfg(test)]
pub(super) fn can_remove_spill_reload_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> bool {
    match ops.get(index + 1) {
        Some(MirOp::Store { .. })
        | Some(MirOp::Compare { .. })
        | Some(MirOp::CompareDirectIndexedBytes { .. })
        | Some(MirOp::CompareIndirectBytes { .. })
        | Some(MirOp::CompareIndirectWords { .. })
        | Some(MirOp::PackedRealCompare { .. })
        | Some(MirOp::PackedRealCopy { .. })
        | Some(MirOp::Unary { .. })
        | Some(MirOp::Binary { .. })
        | Some(MirOp::Call { .. })
        | Some(MirOp::CopyIndirectWord { .. })
        | Some(MirOp::CopyDirectWordToIndirect { .. })
        | Some(MirOp::CopyIndirectBytesToFixedZp { .. })
        | Some(MirOp::AbsoluteWordSubToIndirect { .. })
        | Some(MirOp::IndirectWordCompound { .. })
        | Some(MirOp::RuntimeHelper { .. }) => true,
        Some(MirOp::Load { .. })
        | Some(MirOp::LoadImm { .. })
        | Some(MirOp::Move { .. })
        | Some(MirOp::LeaAddr { .. })
        | Some(MirOp::Extend { .. })
        | Some(MirOp::Truncate { .. })
        | Some(MirOp::UpdateMem { .. })
        | Some(MirOp::UpdateReg { .. })
        | Some(MirOp::UpdateIndexedMem { .. })
        | Some(MirOp::AddByteToWordMem { .. })
        | Some(MirOp::SubByteFromWordMem { .. })
        | Some(MirOp::OffsetPointerByIndirectByte { .. })
        | Some(MirOp::IndirectByteCompound { .. })
        | Some(MirOp::MaterializeAddress { .. })
        | Some(MirOp::MaterializeIndexedAddress { .. })
        | Some(MirOp::AdvanceAddress { .. })
        | Some(MirOp::LoadIndirect { .. })
        | Some(MirOp::StoreIndirect { .. })
        | Some(MirOp::Barrier { .. })
        | Some(MirOp::MachineBlock { .. }) => false,
        None => !terminator_consumes_flags(terminator),
    }
}

#[cfg(test)]
pub(super) fn can_remove_spill_reload_before_later_a_use(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> bool {
    let mut flags_overwritten = false;
    for op in ops.iter().skip(index.saturating_add(1)) {
        let op_writes_flags = op_writes_flags(op);
        if op_reads_reg(op, MirReg::A) {
            return !terminator_consumes_flags(terminator) || flags_overwritten || op_writes_flags;
        }
        if op_may_clobber_reg(op, MirReg::A) {
            return false;
        }
        flags_overwritten |= op_writes_flags;
    }
    false
}

pub(super) fn op_may_clobber_reg(op: &MirOp, reg: MirReg) -> bool {
    classify_op(op).may_clobber_reg_compat(reg)
}

fn update_accumulator_spill_value(a_value: &mut Option<AccumulatorSpillValue>, op: &MirOp) {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src:
                MirAddr::Direct(MirMem::Spill {
                    id: load_id,
                    offset: load_offset,
                }),
            width: MirWidth::Byte,
        } => {
            *a_value = Some(AccumulatorSpillValue {
                id: *load_id,
                offset: *load_offset,
            });
        }
        MirOp::Store {
            dst:
                MirAddr::Direct(MirMem::Spill {
                    id: store_id,
                    offset: store_offset,
                }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } => {
            *a_value = Some(AccumulatorSpillValue {
                id: *store_id,
                offset: *store_offset,
            });
        }
        MirOp::Store {
            dst:
                MirAddr::Direct(MirMem::Spill {
                    id: store_id,
                    offset: store_offset,
                }),
            ..
        } => {
            if a_value.is_some_and(|value| value.id == *store_id && value.offset == *store_offset) {
                *a_value = None;
            }
        }
        MirOp::UpdateMem {
            mem:
                MirMem::Spill {
                    id: store_id,
                    offset: store_offset,
                },
            ..
        } => {
            if a_value.is_some_and(|value| value.id == *store_id && value.offset == *store_offset) {
                *a_value = None;
            }
        }
        MirOp::UpdateIndexedMem { .. } => {
            *a_value = None;
        }
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            ..
        }
        | MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            ..
        }
        | MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            ..
        }
        | MirOp::Unary {
            dst: MirDef::Reg(MirReg::A),
            ..
        }
        | MirOp::Binary {
            dst: MirDef::Reg(MirReg::A),
            ..
        }
        | MirOp::LoadIndirect {
            dst: MirDef::Reg(MirReg::A),
            ..
        }
        | MirOp::LeaAddr {
            dst: MirDef::Reg(MirReg::A),
            ..
        }
        | MirOp::Compare { .. }
        | MirOp::CompareDirectIndexedBytes { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::PackedRealCopy { .. }
        | MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyDirectWordToIndirect { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::AbsoluteWordSubToIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::OffsetPointerByIndirectByte { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {
            *a_value = None;
        }
        MirOp::Load { .. }
        | MirOp::LoadImm { .. }
        | MirOp::Move { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::Extend { .. }
        | MirOp::Truncate { .. }
        | MirOp::Unary { .. }
        | MirOp::Binary { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::Store { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateReg { .. } => {}
    }
}

#[cfg(test)]
fn try_fold_indirect_load_spill_consumer(
    ops: &[MirOp],
    index: usize,
    live_out: &MirTempLiveSet,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    if let Some(consumed) = try_fold_indirect_load_spill_pair_consumer(ops, index, live_out, out) {
        return Some(consumed);
    }
    let Some(MirOp::LoadIndirect {
        consumer,
        dst: MirDef::Reg(MirReg::A),
        offset,
    }) = ops.get(index)
    else {
        return None;
    };
    let Some(MirOp::Store {
        dst:
            MirAddr::Direct(MirMem::Spill {
                id: spill_id,
                offset: 0,
            }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }) = ops.get(index + 1)
    else {
        return None;
    };
    let Some(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src:
            MirAddr::Direct(MirMem::Spill {
                id: load_spill_id,
                offset: 0,
            }),
        width: MirWidth::Byte,
    }) = ops.get(index + 2)
    else {
        return None;
    };
    if load_spill_id != spill_id {
        return None;
    }
    let Some(MirOp::Store {
        dst,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }) = ops.get(index + 3)
    else {
        return None;
    };
    out.push(MirOp::LoadIndirect {
        consumer: *consumer,
        dst: MirDef::Reg(MirReg::A),
        offset: *offset,
    });
    if spill_value_needed_after(ops, index + 4, *spill_id, live_out) {
        out.push(ops[index + 1].clone());
    }
    out.push(MirOp::Store {
        dst: dst.clone(),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    Some(4)
}

#[cfg(test)]
fn try_fold_indirect_load_spill_pair_consumer(
    ops: &[MirOp],
    index: usize,
    live_out: &MirTempLiveSet,
    out: &mut Vec<MirOp>,
) -> Option<usize> {
    let Some(MirOp::LoadIndirect {
        consumer: lo_consumer,
        dst: MirDef::Reg(MirReg::A),
        offset: lo_offset,
    }) = ops.get(index)
    else {
        return None;
    };
    let Some(MirOp::Store {
        dst:
            MirAddr::Direct(MirMem::Spill {
                id: lo_spill,
                offset: 0,
            }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }) = ops.get(index + 1)
    else {
        return None;
    };
    let Some(MirOp::LoadIndirect {
        consumer: hi_consumer,
        dst: MirDef::Reg(MirReg::A),
        offset: hi_offset,
    }) = ops.get(index + 2)
    else {
        return None;
    };
    let Some(MirOp::Store {
        dst:
            MirAddr::Direct(MirMem::Spill {
                id: hi_spill,
                offset: 0,
            }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }) = ops.get(index + 3)
    else {
        return None;
    };
    let Some(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src:
            MirAddr::Direct(MirMem::Spill {
                id: lo_load_spill,
                offset: 0,
            }),
        width: MirWidth::Byte,
    }) = ops.get(index + 4)
    else {
        return None;
    };
    let Some(MirOp::Store {
        dst: lo_dst,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }) = ops.get(index + 5)
    else {
        return None;
    };
    let Some(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src:
            MirAddr::Direct(MirMem::Spill {
                id: hi_load_spill,
                offset: 0,
            }),
        width: MirWidth::Byte,
    }) = ops.get(index + 6)
    else {
        return None;
    };
    let Some(MirOp::Store {
        dst: hi_dst,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }) = ops.get(index + 7)
    else {
        return None;
    };
    if lo_consumer != hi_consumer
        || hi_offset != &lo_offset.saturating_add(1)
        || lo_load_spill != lo_spill
        || hi_load_spill != hi_spill
    {
        return None;
    }
    out.push(MirOp::LoadIndirect {
        consumer: *lo_consumer,
        dst: MirDef::Reg(MirReg::A),
        offset: *lo_offset,
    });
    if spill_value_needed_after(ops, index + 8, *lo_spill, live_out) {
        out.push(ops[index + 1].clone());
    }
    out.push(MirOp::Store {
        dst: lo_dst.clone(),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    out.push(MirOp::LoadIndirect {
        consumer: *hi_consumer,
        dst: MirDef::Reg(MirReg::A),
        offset: *hi_offset,
    });
    if spill_value_needed_after(ops, index + 8, *hi_spill, live_out) {
        out.push(ops[index + 3].clone());
    }
    out.push(MirOp::Store {
        dst: hi_dst.clone(),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    Some(8)
}

#[cfg(test)]
fn spill_value_needed_after(
    ops: &[MirOp],
    start: usize,
    spill: MirSpillId,
    live_out: &MirTempLiveSet,
) -> bool {
    for op in &ops[start..] {
        if op_read_spills(op).contains(&spill) {
            return true;
        }
        if op_write_spills(op).contains(&spill) {
            return false;
        }
    }

    let temp = MirTempId(spill.0 / 2);
    let byte = (spill.0 % 2) as u8;
    live_out.full_temp_live(temp) || live_out.exact_lane_live(temp, byte)
}

pub(super) fn prune_unused_spills(routine: &mut MirRoutine) {
    let mut used = Vec::new();
    for block in &routine.blocks {
        for op in &block.ops {
            collect_op_spills(op, &mut used);
        }
        collect_terminator_spills(&block.terminator, &mut used);
    }
    routine.frame.spills.retain(|spill| used.contains(spill));
}

/// Reuse an existing, dead private zero-page pair for an earlier known-call
/// word result which must survive a later call.
///
/// This is deliberately narrower than general spill promotion. The source
/// pair must be the public word-result slots copied immediately after a known
/// routine call, and every call crossed by either lane must have an exact
/// fixed-pair preservation proof. Unknown calls, machine blocks, and opaque
/// writes retain the ordinary RAM spill homes.
pub(super) fn lower_known_call_result_spills_to_reused_zero_page(
    program: &mut MirProgram,
    known_callees: &MirKnownCalleeSummaries,
) -> BTreeMap<RoutineId, BTreeMap<MirSpillId, MirZpSlot>> {
    let mut remaps = BTreeMap::new();
    for routine in &mut program.routines {
        let Ok(cfg) = MirCfg::from_routine(routine) else {
            continue;
        };
        let liveness = MirHomeLiveness::analyze(routine, &cfg);
        let result_pairs = known_call_result_spill_pairs(routine, known_callees);
        if result_pairs.is_empty() {
            continue;
        }
        let zero_page_pairs = allocated_private_zero_page_pairs(routine);
        let mut routine_remap = BTreeMap::new();
        let mut claimed_slots = BTreeSet::new();

        for [spill_lo, spill_hi] in result_pairs {
            let spill_lo_home = MirHomeByte::Spill {
                id: spill_lo,
                offset: 0,
            };
            let spill_hi_home = MirHomeByte::Spill {
                id: spill_hi,
                offset: 0,
            };
            let Some((zp_lo, zp_hi, _fixed_lo)) =
                zero_page_pairs
                    .iter()
                    .copied()
                    .find(|(zp_lo, zp_hi, fixed_lo)| {
                        !claimed_slots.contains(zp_lo)
                            && !claimed_slots.contains(zp_hi)
                            && !homes_interfere(
                                routine,
                                &liveness,
                                spill_lo_home,
                                MirHomeByte::VirtualZeroPage(*zp_lo),
                            )
                            && !homes_interfere(
                                routine,
                                &liveness,
                                spill_hi_home,
                                MirHomeByte::VirtualZeroPage(*zp_hi),
                            )
                            && live_range_preserves_fixed_pair(
                                routine,
                                &liveness,
                                [spill_lo_home, spill_hi_home],
                                *fixed_lo,
                                known_callees,
                            )
                    })
            else {
                continue;
            };

            claimed_slots.insert(zp_lo);
            claimed_slots.insert(zp_hi);
            routine_remap.insert(spill_lo, zp_lo);
            routine_remap.insert(spill_hi, zp_hi);
        }

        if routine_remap.is_empty() {
            continue;
        }
        for block in &mut routine.blocks {
            for op in &mut block.ops {
                remap_op_spills_to_zero_page(op, &routine_remap);
            }
            remap_terminator_spills_to_zero_page(&mut block.terminator, &routine_remap);
        }
        prune_unused_spills(routine);
        remaps.insert(routine.id, routine_remap);
    }
    remaps
}

fn known_call_result_spill_pairs(
    routine: &MirRoutine,
    known_callees: &MirKnownCalleeSummaries,
) -> Vec<[MirSpillId; 2]> {
    let mut pairs = Vec::new();
    for block in &routine.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            let MirOp::Call { target, .. } = op else {
                continue;
            };
            if known_callees.for_target(target).is_none() {
                continue;
            }
            let mut lanes = [None, None];
            let mut cursor = index + 1;
            let end = block.ops.len().min(index.saturating_add(8));
            while cursor < end {
                if let Some((lane, spill)) = copied_return_slot_to_spill(&block.ops, cursor) {
                    lanes[usize::from(lane)] = Some(spill);
                    cursor += 2;
                    continue;
                }
                if block
                    .ops
                    .get(cursor)
                    .and_then(store_a_direct_mem)
                    .is_some_and(|mem| matches!(mem, MirMem::FixedZeroPage(_)))
                {
                    cursor += 1;
                    continue;
                }
                break;
            }
            let [Some(lo), Some(hi)] = lanes else {
                continue;
            };
            if lo == hi
                || !spill_uses_zero_offset_only(routine, lo)
                || !spill_uses_zero_offset_only(routine, hi)
                || spill_write_count(routine, lo) != 1
                || spill_write_count(routine, hi) != 1
                || spill_read_count(routine, lo) == 0
                || spill_read_count(routine, hi) == 0
            {
                continue;
            }
            let pair = [lo, hi];
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
        }
    }
    pairs
}

fn copied_return_slot_to_spill(ops: &[MirOp], index: usize) -> Option<(u8, MirSpillId)> {
    let source = load_a_direct_mem(ops.get(index)?)?;
    let lane = match source {
        MirMem::FixedZeroPage(MirFixedZpSlot(0xA0)) => 0,
        MirMem::FixedZeroPage(MirFixedZpSlot(0xA1)) => 1,
        _ => return None,
    };
    let destination = store_a_direct_mem(ops.get(index + 1)?)?;
    let MirMem::Spill { id, offset: 0 } = destination else {
        return None;
    };
    Some((lane, id))
}

fn load_a_direct_mem(op: &MirOp) -> Option<MirMem> {
    let MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(mem),
        width: MirWidth::Byte,
    } = op
    else {
        return None;
    };
    Some(mem.clone())
}

fn store_a_direct_mem(op: &MirOp) -> Option<MirMem> {
    let MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    } = op
    else {
        return None;
    };
    Some(mem.clone())
}

fn spill_write_count(routine: &MirRoutine, spill: MirSpillId) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter(|op| op_write_spills(op).contains(&spill))
        .count()
}

fn spill_read_count(routine: &MirRoutine, spill: MirSpillId) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter(|op| op_read_spills(op).contains(&spill))
        .count()
}

fn allocated_private_zero_page_pairs(
    routine: &MirRoutine,
) -> Vec<(MirZpSlot, MirZpSlot, MirFixedZpSlot)> {
    let mut allocations = routine
        .frame
        .zero_page_allocations
        .iter()
        .filter(|allocation| allocation.size == 1)
        .collect::<Vec<_>>();
    allocations.sort_by_key(|allocation| allocation.start);
    let mut pairs = Vec::new();
    for left in &allocations {
        for right in &allocations {
            if right.start.0 == left.start.0.saturating_add(1) {
                pairs.push((left.slot, right.slot, left.start));
            }
        }
    }
    pairs
}

fn homes_interfere(
    routine: &MirRoutine,
    liveness: &MirHomeLiveness,
    left: MirHomeByte,
    right: MirHomeByte,
) -> bool {
    for block in &routine.blocks {
        for (op_index, op) in block.ops.iter().enumerate() {
            let effects = classify_op(op);
            let site = MirSite::Op {
                block: block.id,
                op_index,
            };
            if effect_writes_home(&effects, left) && liveness.live_after(right, site) != Ok(false) {
                return true;
            }
            if effect_writes_home(&effects, right) && liveness.live_after(left, site) != Ok(false) {
                return true;
            }
        }
    }
    false
}

fn effect_writes_home(
    effects: &crate::mir6502::analysis::effects::MirOpEffectSummary,
    home: MirHomeByte,
) -> bool {
    effects.homes.writes.contains(&home) || effects.addresses.pair_writes.contains(&home)
}

fn live_range_preserves_fixed_pair(
    routine: &MirRoutine,
    liveness: &MirHomeLiveness,
    spills: [MirHomeByte; 2],
    fixed_lo: MirFixedZpSlot,
    known_callees: &MirKnownCalleeSummaries,
) -> bool {
    for block in &routine.blocks {
        for (op_index, op) in block.ops.iter().enumerate() {
            let site = MirSite::Op {
                block: block.id,
                op_index,
            };
            let live_after = spills
                .iter()
                .any(|spill| liveness.live_after(*spill, site) != Ok(false));
            if !live_after {
                continue;
            }
            match op {
                MirOp::Call { target, .. } => {
                    if let Some(summary) = known_callees.for_target(target) {
                        if !summary.preserves_fixed_pair(fixed_lo) {
                            return false;
                        }
                    } else if matches!(target, MirCallTarget::AtariFpp(_)) {
                        // The Atari FPP target has a verifier-enforced audited
                        // contract, so its physical workspace can prove
                        // preservation without a source-routine summary.
                        if op_may_write_fixed_pair(op, fixed_lo) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                MirOp::RuntimeHelper { .. }
                | MirOp::MachineBlock { .. }
                | MirOp::Barrier { .. } => return false,
                _ if op_may_write_fixed_pair(op, fixed_lo) => return false,
                _ => {}
            }
        }
    }
    true
}

fn op_may_write_fixed_pair(op: &MirOp, fixed_lo: MirFixedZpSlot) -> bool {
    let fixed_hi = MirFixedZpSlot(fixed_lo.0.saturating_add(1));
    let effects = classify_op(op);
    if effects.memory.indirect_writes
        || effects.memory.opaque
        || effects.memory.may_write_any
        || effects.memory.has_unknown_effects
        || effects.homes.unknown_writes
    {
        return true;
    }
    if effects
        .homes
        .writes
        .iter()
        .chain(effects.addresses.pair_writes.iter())
        .any(|home| {
            matches!(
                home,
                MirHomeByte::FixedZeroPage(slot)
                    if *slot == fixed_lo || *slot == fixed_hi
            )
        })
    {
        return true;
    }
    effects.memory.direct_writes.iter().any(|range| {
        (0..range.bytes).any(|offset| {
            let address = match &range.base {
                MirMem::Absolute(address) => Some(address.saturating_add(offset)),
                MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0).saturating_add(offset)),
                // Named source globals can resolve to fixed zero page. The
                // allocator normally reserves those bytes, but retain the
                // conservative answer here because this proof is physical.
                MirMem::Global { .. } => return true,
                MirMem::Static { .. }
                | MirMem::Local { .. }
                | MirMem::Param { .. }
                | MirMem::Spill { .. }
                | MirMem::ZeroPage(_) => None,
            };
            address.is_some_and(|address| {
                address == u16::from(fixed_lo.0) || address == u16::from(fixed_hi.0)
            })
        })
    })
}

pub(super) fn lower_block_local_byte_spills_to_zero_page(
    program: &mut MirProgram,
) -> BTreeMap<RoutineId, BTreeMap<MirSpillId, MirZpSlot>> {
    let source_zero_page = source_zero_page_slots(program);
    let mut remaps = BTreeMap::new();
    for routine in &mut program.routines {
        let mut used = [false; 256];
        for fixed in &source_zero_page {
            used[fixed.0 as usize] = true;
        }
        for fixed in &routine.frame.fixed_zero_page {
            used[fixed.0 as usize] = true;
        }
        for allocation in &routine.frame.zero_page_allocations {
            mark_zp_range(&mut used, allocation.start.0, allocation.size);
        }

        let mut next_virtual_slot = routine
            .frame
            .virtual_zero_page
            .iter()
            .map(|slot| slot.0)
            .max()
            .map_or(0, |slot| slot.saturating_add(1));
        let mut remap = BTreeMap::<MirSpillId, MirZpSlot>::new();
        let mut intervals = basic_block_spill_intervals(routine);
        intervals.sort_by_key(|interval| {
            (
                interval.first_read.unwrap_or(usize::MAX),
                interval.first,
                interval.last,
                interval.spill,
            )
        });

        for interval in intervals {
            if interval.first_read.is_none()
                || spill_crosses_call(routine, &interval)
                || !spill_uses_zero_offset_only(routine, interval.spill)
            {
                continue;
            }
            let Some(start) = find_zp_range(&used, 0xE0, 0xEF, 1) else {
                break;
            };
            mark_zp_range(&mut used, start, 1);
            let slot = MirZpSlot(next_virtual_slot);
            next_virtual_slot = next_virtual_slot.saturating_add(1);
            routine.frame.virtual_zero_page.push(slot);
            remap.insert(interval.spill, slot);
        }

        if remap.is_empty() {
            continue;
        }
        remaps.insert(routine.id, remap.clone());
        for block in &mut routine.blocks {
            for op in &mut block.ops {
                remap_op_spills_to_zero_page(op, &remap);
            }
            remap_terminator_spills_to_zero_page(&mut block.terminator, &remap);
        }
        prune_unused_spills(routine);
    }
    remaps
}

/// Places a small number of loop-carried word indexes in private zero page.
///
/// NIR promotion turns an induction variable into a pair of MIR spill lanes.
/// This target pass recognizes pairs which feed indexed-address
/// materialization and remain live across a CFG cycle. Pairs crossing calls or
/// opaque barriers stay in ordinary spill storage because the private
/// `$E0-$EF` pool is shared by caller and callee materialization.
pub(super) fn lower_hot_induction_address_spills_to_zero_page(
    routine: &mut MirRoutine,
    source_zero_page: &[MirFixedZpSlot],
) -> BTreeMap<MirSpillId, MirZpSlot> {
    const MAX_HOT_PAIRS: usize = 2;

    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return BTreeMap::new();
    };
    let liveness = MirHomeLiveness::analyze(routine, &cfg);
    let accesses = home_access_counts(routine);
    let mut candidates = indexed_word_spill_pairs(routine)
        .into_iter()
        .filter(|pair| {
            pair.iter().all(|spill| {
                routine.frame.spills.contains(spill) && spill_uses_zero_offset_only(routine, *spill)
            })
        })
        .filter(|pair| pair_live_across_cycle(&cfg, &liveness, *pair))
        .filter(|pair| !pair_live_across_private_zp_barrier(routine, &liveness, *pair))
        .map(|pair| {
            let indexed_uses = indexed_word_spill_pair_use_count(routine, pair);
            let traffic = pair
                .iter()
                .map(|spill| {
                    accesses
                        .get(&MirHomeStorage::Spill(*spill))
                        .map_or(0, |count| count.reads.saturating_add(count.writes))
                })
                .sum::<usize>();
            (pair, indexed_uses.saturating_mul(8).saturating_add(traffic))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(pair, score)| (std::cmp::Reverse(*score), *pair));

    let available_pairs = available_private_zero_page_lanes(routine, source_zero_page) / 2;
    let pair_limit = MAX_HOT_PAIRS.min(available_pairs);
    if pair_limit == 0 {
        return BTreeMap::new();
    }

    let mut next_virtual_slot = routine
        .frame
        .virtual_zero_page
        .iter()
        .map(|slot| slot.0)
        .max()
        .map_or(0, |slot| slot.saturating_add(1));
    let mut remap = BTreeMap::new();
    for (pair, _) in candidates.into_iter().take(pair_limit) {
        for spill in pair {
            let slot = MirZpSlot(next_virtual_slot);
            next_virtual_slot = next_virtual_slot.saturating_add(1);
            routine.frame.virtual_zero_page.push(slot);
            remap.insert(spill, slot);
        }
    }
    if remap.is_empty() {
        return remap;
    }
    for block in &mut routine.blocks {
        for op in &mut block.ops {
            remap_op_spills_to_zero_page(op, &remap);
        }
        remap_terminator_spills_to_zero_page(&mut block.terminator, &remap);
    }
    prune_unused_spills(routine);
    remap
}

fn indexed_word_spill_pairs(routine: &MirRoutine) -> Vec<[MirSpillId; 2]> {
    let mut pairs = Vec::new();
    for block in &routine.blocks {
        for op in &block.ops {
            let MirOp::MaterializeIndexedAddress { index, .. } = op else {
                continue;
            };
            let Some(pair) = word_spill_pair(index) else {
                continue;
            };
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
        }
    }
    pairs
}

fn indexed_word_spill_pair_use_count(routine: &MirRoutine, pair: [MirSpillId; 2]) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter(|op| {
            matches!(
                op,
                MirOp::MaterializeIndexedAddress { index, .. }
                    if word_spill_pair(index) == Some(pair)
            )
        })
        .count()
}

fn word_spill_pair(value: &MirValue) -> Option<[MirSpillId; 2]> {
    let MirValue::Word { lo, hi } = value else {
        return None;
    };
    let (
        MirValue::PointerCell(MirMem::Spill { id: lo, offset: 0 }),
        MirValue::PointerCell(MirMem::Spill { id: hi, offset: 0 }),
    ) = (lo.as_ref(), hi.as_ref())
    else {
        return None;
    };
    (hi.0 == lo.0.saturating_add(1)).then_some([*lo, *hi])
}

fn pair_live_across_cycle(cfg: &MirCfg, liveness: &MirHomeLiveness, pair: [MirSpillId; 2]) -> bool {
    let homes = pair.map(|id| MirHomeByte::Spill { id, offset: 0 });
    cfg.reachable().iter().any(|source| {
        cfg.successors(*source).iter().any(|target| {
            path_exists_between(cfg, *target, *source)
                && liveness
                    .live_out(*source)
                    .is_some_and(|live| homes.iter().all(|home| live.contains(*home)))
                && liveness
                    .live_in(*target)
                    .is_some_and(|live| homes.iter().all(|home| live.contains(*home)))
        })
    })
}

fn path_exists_between(cfg: &MirCfg, start: MirBlockId, target: MirBlockId) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(block) = pending.pop() {
        if block == target {
            return true;
        }
        if !visited.insert(block) {
            continue;
        }
        pending.extend(cfg.successors(block));
    }
    false
}

fn pair_live_across_private_zp_barrier(
    routine: &MirRoutine,
    liveness: &MirHomeLiveness,
    pair: [MirSpillId; 2],
) -> bool {
    let homes = pair.map(|id| MirHomeByte::Spill { id, offset: 0 });
    routine.blocks.iter().any(|block| {
        block.ops.iter().enumerate().any(|(op_index, op)| {
            matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) && homes.iter().any(|home| {
                liveness.live_after(
                    *home,
                    MirSite::Op {
                        block: block.id,
                        op_index,
                    },
                ) != Ok(false)
            })
        })
    })
}

fn available_private_zero_page_lanes(
    routine: &MirRoutine,
    source_zero_page: &[MirFixedZpSlot],
) -> usize {
    let mut used = [false; 256];
    for slot in source_zero_page
        .iter()
        .chain(&routine.frame.fixed_zero_page)
    {
        used[usize::from(slot.0)] = true;
    }
    for allocation in &routine.frame.zero_page_allocations {
        mark_zp_range(&mut used, allocation.start.0, allocation.size);
    }
    (0xE0..=0xEF).filter(|slot| !used[*slot]).count()
}

fn spill_crosses_call(routine: &MirRoutine, interval: &SpillUseInterval) -> bool {
    let Some(block) = routine.blocks.get(interval.block_index) else {
        return true;
    };
    block.ops.iter().enumerate().any(|(index, op)| {
        index > interval.first && index < interval.last && op_is_call_barrier(op)
    })
}

fn spill_uses_zero_offset_only(routine: &MirRoutine, spill: MirSpillId) -> bool {
    for block in &routine.blocks {
        for op in &block.ops {
            if !op_spill_uses_zero_offset_only(op, spill) {
                return false;
            }
        }
        if !terminator_spill_uses_zero_offset_only(&block.terminator, spill) {
            return false;
        }
    }
    true
}

fn op_spill_uses_zero_offset_only(op: &MirOp, spill: MirSpillId) -> bool {
    let mut ok = true;
    visit_op_mems(op, &mut |mem| {
        if matches!(mem, MirMem::Spill { id, offset } if *id == spill && *offset != 0) {
            ok = false;
        }
    });
    ok
}

fn terminator_spill_uses_zero_offset_only(terminator: &MirTerminator, spill: MirSpillId) -> bool {
    let mut ok = true;
    if let MirTerminator::Branch {
        cond: MirCond::BoolValue(value),
        ..
    } = terminator
    {
        visit_value_mems(value, &mut |mem| {
            if matches!(mem, MirMem::Spill { id, offset } if *id == spill && *offset != 0) {
                ok = false;
            }
        });
    }
    ok
}

#[derive(Debug, Clone)]
struct SpillUseInterval {
    spill: MirSpillId,
    block_index: usize,
    first: usize,
    last: usize,
    first_read: Option<usize>,
}

#[derive(Debug, Clone)]
struct SpillUseBuilder {
    block_index: usize,
    first: usize,
    last: usize,
    first_write: Option<usize>,
    first_read: Option<usize>,
    blocks: BTreeSet<usize>,
    terminator_use: bool,
}

pub(super) fn color_basic_block_spills(
    routine: &mut MirRoutine,
) -> BTreeMap<MirSpillId, MirSpillId> {
    let intervals = basic_block_spill_intervals(routine);
    if intervals.len() <= 1 {
        return BTreeMap::new();
    }

    let mut remap = BTreeMap::<MirSpillId, MirSpillId>::new();
    for block_index in 0..routine.blocks.len() {
        let mut block_intervals = intervals
            .iter()
            .filter(|interval| interval.block_index == block_index)
            .cloned()
            .collect::<Vec<_>>();
        block_intervals.sort_by_key(|interval| {
            (
                interval.first,
                interval.last,
                interval.first_read.unwrap_or(usize::MAX),
                interval.spill,
            )
        });

        let mut colors = Vec::<(MirSpillId, usize)>::new();
        for interval in block_intervals {
            let color_index = colors
                .iter()
                .position(|(_, active_until)| *active_until < interval.first);
            if let Some(color_index) = color_index {
                let (color, active_until) = &mut colors[color_index];
                remap.insert(interval.spill, *color);
                *active_until = interval.last;
            } else {
                remap.insert(interval.spill, interval.spill);
                colors.push((interval.spill, interval.last));
            }
        }
    }

    remap.retain(|from, to| from != to);
    if remap.is_empty() {
        return remap;
    }

    for block in &mut routine.blocks {
        for op in &mut block.ops {
            remap_op_spills(op, &remap);
        }
        remap_terminator_spills(&mut block.terminator, &remap);
    }
    remap
}

pub(super) fn color_routine_spills(routine: &mut MirRoutine) -> BTreeMap<MirSpillId, MirSpillId> {
    let already_block_local = basic_block_spill_intervals(routine)
        .into_iter()
        .map(|interval| interval.spill)
        .collect::<BTreeSet<_>>();
    let used = routine
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .ops
                .iter()
                .flat_map(|op| {
                    op_direct_read_spills(op)
                        .into_iter()
                        .chain(op_direct_write_spills(op))
                })
                .chain(terminator_spills(&block.terminator))
        })
        .collect::<BTreeSet<_>>();
    let eligible = routine
        .frame
        .spills
        .iter()
        .copied()
        .filter(|spill| {
            used.contains(spill)
                && !already_block_local.contains(spill)
                && spill_uses_zero_offset_only(routine, *spill)
                && !spill_has_unremappable_temp_identity(routine, *spill)
        })
        .collect::<BTreeSet<_>>();
    if eligible.len() <= 1 {
        return BTreeMap::new();
    }

    let block_facts = routine
        .blocks
        .iter()
        .map(|block| routine_spill_block_facts(block, &eligible))
        .collect::<Vec<_>>();
    let mut live_in = vec![BTreeSet::<MirSpillId>::new(); routine.blocks.len()];
    let mut live_out = live_in.clone();
    loop {
        let mut changed = false;
        for block_index in (0..routine.blocks.len()).rev() {
            let mut next_out = BTreeSet::new();
            for successor in
                block_successor_indices(routine, &routine.blocks[block_index].terminator)
            {
                next_out.extend(live_in[successor].iter().copied());
            }
            let mut next_in = block_facts[block_index].uses.clone();
            next_in.extend(
                next_out
                    .iter()
                    .filter(|spill| !block_facts[block_index].defs.contains(spill))
                    .copied(),
            );
            changed |= live_in[block_index] != next_in || live_out[block_index] != next_out;
            live_in[block_index] = next_in;
            live_out[block_index] = next_out;
        }
        if !changed {
            break;
        }
    }

    let mut graph = eligible
        .iter()
        .copied()
        .map(|spill| (spill, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        let mut live = live_out[block_index].clone();
        live.extend(
            terminator_spills(&block.terminator)
                .into_iter()
                .filter(|spill| eligible.contains(spill)),
        );
        for op in block.ops.iter().rev() {
            let reads = op_direct_read_spills(op)
                .into_iter()
                .filter(|spill| eligible.contains(spill))
                .collect::<BTreeSet<_>>();
            let writes = op_direct_write_spills(op)
                .into_iter()
                .filter(|spill| eligible.contains(spill))
                .collect::<BTreeSet<_>>();
            for spill in &writes {
                for other in live.iter().chain(reads.iter()).chain(writes.iter()) {
                    if spill != other {
                        add_spill_interference(&mut graph, *spill, *other);
                    }
                }
            }
            for spill in &writes {
                live.remove(spill);
            }
            live.extend(reads);
        }
    }

    let mut nodes = eligible.into_iter().collect::<Vec<_>>();
    nodes.sort_by_key(|spill| {
        (
            std::cmp::Reverse(graph.get(spill).map_or(0, BTreeSet::len)),
            *spill,
        )
    });
    let mut assigned = BTreeMap::<MirSpillId, MirSpillId>::new();
    let mut colors = Vec::<MirSpillId>::new();
    for spill in nodes {
        let color = colors
            .iter()
            .copied()
            .find(|color| {
                graph[&spill]
                    .iter()
                    .all(|neighbor| assigned.get(neighbor) != Some(color))
            })
            .unwrap_or_else(|| {
                colors.push(spill);
                spill
            });
        assigned.insert(spill, color);
    }

    let mut remap = assigned
        .into_iter()
        .filter(|(from, to)| from != to)
        .collect::<BTreeMap<_, _>>();
    if remap.is_empty() {
        return remap;
    }
    for block in &mut routine.blocks {
        for op in &mut block.ops {
            remap_op_spills(op, &remap);
        }
        remap_terminator_spills(&mut block.terminator, &remap);
    }
    remap.retain(|from, to| from != to);
    remap
}

fn spill_has_unremappable_temp_identity(routine: &MirRoutine, spill: MirSpillId) -> bool {
    routine.blocks.iter().any(|block| {
        block.ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Compare {
                    dst: MirCondDest::Temp(temp),
                    ..
                } if MirSpillId(temp.0.saturating_mul(2)) == spill
            )
        })
    })
}

#[derive(Debug, Default)]
struct RoutineSpillBlockFacts {
    uses: BTreeSet<MirSpillId>,
    defs: BTreeSet<MirSpillId>,
}

fn routine_spill_block_facts(
    block: &MirBlock,
    eligible: &BTreeSet<MirSpillId>,
) -> RoutineSpillBlockFacts {
    let mut facts = RoutineSpillBlockFacts::default();
    for op in &block.ops {
        for spill in op_direct_read_spills(op) {
            if eligible.contains(&spill) && !facts.defs.contains(&spill) {
                facts.uses.insert(spill);
            }
        }
        facts.defs.extend(
            op_direct_write_spills(op)
                .into_iter()
                .filter(|spill| eligible.contains(spill)),
        );
    }
    for spill in terminator_spills(&block.terminator) {
        if eligible.contains(&spill) && !facts.defs.contains(&spill) {
            facts.uses.insert(spill);
        }
    }
    facts
}

fn add_spill_interference(
    graph: &mut BTreeMap<MirSpillId, BTreeSet<MirSpillId>>,
    left: MirSpillId,
    right: MirSpillId,
) {
    graph.entry(left).or_default().insert(right);
    graph.entry(right).or_default().insert(left);
}

fn basic_block_spill_intervals(routine: &MirRoutine) -> Vec<SpillUseInterval> {
    let mut builders = BTreeMap::<MirSpillId, SpillUseBuilder>::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let reads = op_direct_read_spills(op);
            let writes = op_direct_write_spills(op);
            for spill in reads {
                note_spill_use(&mut builders, spill, block_index, op_index, false);
            }
            for spill in writes {
                note_spill_use(&mut builders, spill, block_index, op_index, true);
            }
        }
        for spill in terminator_spills(&block.terminator) {
            let entry = builders.entry(spill).or_insert_with(|| SpillUseBuilder {
                block_index,
                first: usize::MAX,
                last: 0,
                first_write: None,
                first_read: None,
                blocks: BTreeSet::new(),
                terminator_use: false,
            });
            entry.blocks.insert(block_index);
            entry.terminator_use = true;
        }
    }

    builders
        .into_iter()
        .filter_map(|(spill, builder)| {
            if builder.terminator_use || builder.blocks.len() != 1 {
                return None;
            }
            let first_write = builder.first_write?;
            if builder
                .first_read
                .is_some_and(|first_read| first_read < first_write)
            {
                return None;
            }
            Some(SpillUseInterval {
                spill,
                block_index: builder.block_index,
                first: builder.first,
                last: builder.last,
                first_read: builder.first_read,
            })
        })
        .collect()
}

fn note_spill_use(
    builders: &mut BTreeMap<MirSpillId, SpillUseBuilder>,
    spill: MirSpillId,
    block_index: usize,
    op_index: usize,
    is_write: bool,
) {
    let entry = builders.entry(spill).or_insert_with(|| SpillUseBuilder {
        block_index,
        first: op_index,
        last: op_index,
        first_write: None,
        first_read: None,
        blocks: BTreeSet::new(),
        terminator_use: false,
    });
    entry.block_index = entry.block_index.min(block_index);
    entry.first = entry.first.min(op_index);
    entry.last = entry.last.max(op_index);
    entry.blocks.insert(block_index);
    if is_write {
        entry.first_write = Some(
            entry
                .first_write
                .map_or(op_index, |first| first.min(op_index)),
        );
    } else {
        entry.first_read = Some(
            entry
                .first_read
                .map_or(op_index, |first| first.min(op_index)),
        );
    }
}

fn op_direct_read_spills(op: &MirOp) -> BTreeSet<MirSpillId> {
    classify_op(op).projected_spill_reads
}

fn op_direct_write_spills(op: &MirOp) -> BTreeSet<MirSpillId> {
    if matches!(
        op,
        MirOp::Compare { .. }
            | MirOp::CompareDirectIndexedBytes { .. }
            | MirOp::CompareIndirectBytes { .. }
            | MirOp::CompareIndirectWords { .. }
            | MirOp::Call { .. }
    ) {
        BTreeSet::new()
    } else {
        classify_op(op).projected_spill_writes
    }
}

fn terminator_spills(terminator: &MirTerminator) -> Vec<MirSpillId> {
    let mut spills = Vec::new();
    collect_terminator_spills(terminator, &mut spills);
    spills
}

fn remap_op_spills(op: &mut MirOp, remap: &BTreeMap<MirSpillId, MirSpillId>) {
    match op {
        MirOp::Load { src, .. } => remap_addr_spills(src, remap),
        MirOp::Store { dst, src, .. } => {
            remap_addr_spills(dst, remap);
            remap_value_spills(src, remap);
        }
        MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => {
            remap_addr_spills(source, remap);
            remap_addr_spills(destination, remap);
        }
        MirOp::UpdateMem { mem, .. } | MirOp::UpdateIndexedMem { base: mem, .. } => {
            remap_mem_spills(mem, remap)
        }
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            remap_mem_spills(mem, remap);
            remap_value_spills(value, remap);
        }
        MirOp::OffsetPointerByIndirectByte { dst, .. } => remap_mem_spills(dst, remap),
        MirOp::CopyDirectWordToIndirect { source, .. } => remap_mem_spills(source, remap),
        MirOp::AbsoluteWordSubToIndirect { rhs, .. } => remap_mem_spills(rhs, remap),
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. } => remap_value_spills(src, remap),
        MirOp::MaterializeAddress { value, .. } => remap_value_spills(value, remap),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            remap_value_spills(base, remap);
            remap_value_spills(index, remap);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            remap_value_spills(left, remap);
            remap_value_spills(right, remap);
        }
        MirOp::CompareDirectIndexedBytes { left, right, .. } => {
            remap_mem_spills(left, remap);
            remap_mem_spills(right, remap);
        }
        MirOp::Call { target, args, .. } => {
            remap_call_target_spills(target, remap);
            for arg in args {
                remap_value_spills(&mut arg.value, remap);
            }
        }
        MirOp::AdvanceAddress { index, .. } | MirOp::StoreIndirect { src: index, .. } => {
            remap_value_spills(index, remap);
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn remap_terminator_spills(
    terminator: &mut MirTerminator,
    remap: &BTreeMap<MirSpillId, MirSpillId>,
) {
    if let MirTerminator::Branch {
        cond: MirCond::BoolValue(value),
        ..
    } = terminator
    {
        remap_value_spills(value, remap);
    }
}

fn remap_call_target_spills(target: &mut MirCallTarget, remap: &BTreeMap<MirSpillId, MirSpillId>) {
    if let MirCallTarget::Indirect { target, .. } = target {
        remap_value_spills(target, remap);
    }
}

fn remap_addr_spills(addr: &mut MirAddr, remap: &BTreeMap<MirSpillId, MirSpillId>) {
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => remap_mem_spills(mem, remap),
        MirAddr::ComputedIndex { base, index, .. } => {
            remap_value_spills(base, remap);
            remap_value_spills(index, remap);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            remap_mem_spills(ptr, remap);
            remap_value_spills(index, remap);
        }
        MirAddr::Deref { ptr, .. } => remap_value_spills(ptr, remap),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
}

fn remap_value_spills(value: &mut MirValue, remap: &BTreeMap<MirSpillId, MirSpillId>) {
    match value {
        MirValue::PointerCell(mem) => remap_mem_spills(mem, remap),
        MirValue::Word { lo, hi } => {
            remap_value_spills(lo, remap);
            remap_value_spills(hi, remap);
        }
        MirValue::StorageAddrByte { mem, .. } => remap_mem_spills(mem, remap),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => {}
    }
}

fn remap_mem_spills(mem: &mut MirMem, remap: &BTreeMap<MirSpillId, MirSpillId>) {
    if let MirMem::Spill { id, .. } = mem
        && let Some(mapped) = remap.get(id)
    {
        *id = *mapped;
    }
}

fn remap_op_spills_to_zero_page(op: &mut MirOp, remap: &BTreeMap<MirSpillId, MirZpSlot>) {
    match op {
        MirOp::Load { src, .. } => remap_addr_spills_to_zero_page(src, remap),
        MirOp::Store { dst, src, .. } => {
            remap_addr_spills_to_zero_page(dst, remap);
            remap_value_spills_to_zero_page(src, remap);
        }
        MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => {
            remap_addr_spills_to_zero_page(source, remap);
            remap_addr_spills_to_zero_page(destination, remap);
        }
        MirOp::UpdateMem { mem, .. } | MirOp::UpdateIndexedMem { base: mem, .. } => {
            remap_mem_spills_to_zero_page(mem, remap)
        }
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            remap_mem_spills_to_zero_page(mem, remap);
            remap_value_spills_to_zero_page(value, remap);
        }
        MirOp::OffsetPointerByIndirectByte { dst, .. } => remap_mem_spills_to_zero_page(dst, remap),
        MirOp::CopyDirectWordToIndirect { source, .. } => {
            remap_mem_spills_to_zero_page(source, remap)
        }
        MirOp::AbsoluteWordSubToIndirect { rhs, .. } => remap_mem_spills_to_zero_page(rhs, remap),
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. } => remap_value_spills_to_zero_page(src, remap),
        MirOp::MaterializeAddress { value, .. } => remap_value_spills_to_zero_page(value, remap),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            remap_value_spills_to_zero_page(base, remap);
            remap_value_spills_to_zero_page(index, remap);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            remap_value_spills_to_zero_page(left, remap);
            remap_value_spills_to_zero_page(right, remap);
        }
        MirOp::CompareDirectIndexedBytes { left, right, .. } => {
            remap_mem_spills_to_zero_page(left, remap);
            remap_mem_spills_to_zero_page(right, remap);
        }
        MirOp::Call { target, args, .. } => {
            remap_call_target_spills_to_zero_page(target, remap);
            for arg in args {
                remap_value_spills_to_zero_page(&mut arg.value, remap);
            }
        }
        MirOp::AdvanceAddress { index, .. } | MirOp::StoreIndirect { src: index, .. } => {
            remap_value_spills_to_zero_page(index, remap);
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn remap_terminator_spills_to_zero_page(
    terminator: &mut MirTerminator,
    remap: &BTreeMap<MirSpillId, MirZpSlot>,
) {
    if let MirTerminator::Branch {
        cond: MirCond::BoolValue(value),
        ..
    } = terminator
    {
        remap_value_spills_to_zero_page(value, remap);
    }
}

fn remap_call_target_spills_to_zero_page(
    target: &mut MirCallTarget,
    remap: &BTreeMap<MirSpillId, MirZpSlot>,
) {
    if let MirCallTarget::Indirect { target, .. } = target {
        remap_value_spills_to_zero_page(target, remap);
    }
}

fn remap_addr_spills_to_zero_page(addr: &mut MirAddr, remap: &BTreeMap<MirSpillId, MirZpSlot>) {
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => remap_mem_spills_to_zero_page(mem, remap),
        MirAddr::ComputedIndex { base, index, .. } => {
            remap_value_spills_to_zero_page(base, remap);
            remap_value_spills_to_zero_page(index, remap);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            remap_mem_spills_to_zero_page(ptr, remap);
            remap_value_spills_to_zero_page(index, remap);
        }
        MirAddr::Deref { ptr, .. } => remap_value_spills_to_zero_page(ptr, remap),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
}

fn remap_value_spills_to_zero_page(value: &mut MirValue, remap: &BTreeMap<MirSpillId, MirZpSlot>) {
    match value {
        MirValue::PointerCell(mem) => remap_mem_spills_to_zero_page(mem, remap),
        MirValue::Word { lo, hi } => {
            remap_value_spills_to_zero_page(lo, remap);
            remap_value_spills_to_zero_page(hi, remap);
        }
        MirValue::StorageAddrByte { mem, .. } => remap_mem_spills_to_zero_page(mem, remap),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => {}
    }
}

fn remap_mem_spills_to_zero_page(mem: &mut MirMem, remap: &BTreeMap<MirSpillId, MirZpSlot>) {
    if let MirMem::Spill { id, offset: 0 } = mem
        && let Some(slot) = remap.get(id)
    {
        *mem = MirMem::ZeroPage(*slot);
    }
}

pub(super) fn visit_op_mems<F>(op: &MirOp, visitor: &mut F)
where
    F: FnMut(&MirMem),
{
    match op {
        MirOp::Load { src, .. } => visit_addr_mems(src, visitor),
        MirOp::Store { dst, src, .. } => {
            visit_addr_mems(dst, visitor);
            visit_value_mems(src, visitor);
        }
        MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => {
            visit_addr_mems(source, visitor);
            visit_addr_mems(destination, visitor);
        }
        MirOp::UpdateMem { mem, .. } | MirOp::UpdateIndexedMem { base: mem, .. } => visitor(mem),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            visitor(mem);
            visit_value_mems(value, visitor);
        }
        MirOp::OffsetPointerByIndirectByte { dst, .. } => visitor(dst),
        MirOp::CopyDirectWordToIndirect { source, .. } => visitor(source),
        MirOp::AbsoluteWordSubToIndirect { rhs, .. } => visitor(rhs),
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. } => visit_value_mems(src, visitor),
        MirOp::MaterializeAddress { value, .. } => visit_value_mems(value, visitor),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            visit_value_mems(base, visitor);
            visit_value_mems(index, visitor);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            visit_value_mems(left, visitor);
            visit_value_mems(right, visitor);
        }
        MirOp::CompareDirectIndexedBytes { left, right, .. } => {
            visitor(left);
            visitor(right);
        }
        MirOp::Call { target, args, .. } => {
            if let MirCallTarget::Indirect { target, .. } = target {
                visit_value_mems(target, visitor);
            }
            for arg in args {
                visit_value_mems(&arg.value, visitor);
            }
        }
        MirOp::AdvanceAddress { index, .. } | MirOp::StoreIndirect { src: index, .. } => {
            visit_value_mems(index, visitor);
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn visit_addr_mems<F>(addr: &MirAddr, visitor: &mut F)
where
    F: FnMut(&MirMem),
{
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => visitor(mem),
        MirAddr::ComputedIndex { base, index, .. } => {
            visit_value_mems(base, visitor);
            visit_value_mems(index, visitor);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            visitor(ptr);
            visit_value_mems(index, visitor);
        }
        MirAddr::Deref { ptr, .. } => visit_value_mems(ptr, visitor),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
}

fn visit_value_mems<F>(value: &MirValue, visitor: &mut F)
where
    F: FnMut(&MirMem),
{
    match value {
        MirValue::PointerCell(mem) => visitor(mem),
        MirValue::Word { lo, hi } => {
            visit_value_mems(lo, visitor);
            visit_value_mems(hi, visitor);
        }
        MirValue::StorageAddrByte { mem, .. } => visitor(mem),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => {}
    }
}

fn collect_op_spills(op: &MirOp, spills: &mut Vec<MirSpillId>) {
    match op {
        MirOp::Load { dst, src, .. } => {
            collect_def_spills(dst, spills);
            collect_addr_spills(src, spills);
        }
        MirOp::Store { dst, src, .. } => {
            collect_addr_spills(dst, spills);
            collect_value_spills(src, spills);
        }
        MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => {
            collect_addr_spills(source, spills);
            collect_addr_spills(destination, spills);
        }
        MirOp::UpdateMem { mem, .. } | MirOp::UpdateIndexedMem { base: mem, .. } => {
            collect_mem_spills(mem, spills)
        }
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            collect_mem_spills(mem, spills);
            collect_value_spills(value, spills);
        }
        MirOp::OffsetPointerByIndirectByte { dst, .. } => collect_mem_spills(dst, spills),
        MirOp::CopyDirectWordToIndirect { source, .. } => collect_mem_spills(source, spills),
        MirOp::AbsoluteWordSubToIndirect { rhs, .. } => collect_mem_spills(rhs, spills),
        MirOp::Move { dst, src, .. }
        | MirOp::Extend { dst, src, .. }
        | MirOp::Truncate { dst, src, .. }
        | MirOp::Unary { dst, src, .. } => {
            collect_def_spills(dst, spills);
            collect_value_spills(src, spills);
        }
        MirOp::MaterializeAddress { value, .. } => collect_value_spills(value, spills),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            collect_value_spills(base, spills);
            collect_value_spills(index, spills);
        }
        MirOp::LoadImm { dst, width, .. } | MirOp::LeaAddr { dst, width, .. } => {
            collect_def_spills_for_width(dst, *width, spills);
        }
        MirOp::Binary {
            dst, left, right, ..
        } => {
            collect_def_spills(dst, spills);
            collect_value_spills(left, spills);
            collect_value_spills(right, spills);
        }
        MirOp::Compare {
            dst, left, right, ..
        } => {
            if let MirCondDest::Temp(id) = dst {
                collect_spill(MirSpillId(id.0.saturating_mul(2)), spills);
            }
            collect_value_spills(left, spills);
            collect_value_spills(right, spills);
        }
        MirOp::CompareDirectIndexedBytes {
            dst, left, right, ..
        } => {
            if let MirCondDest::Temp(id) = dst {
                collect_spill(MirSpillId(id.0.saturating_mul(2)), spills);
            }
            collect_mem_spills(left, spills);
            collect_mem_spills(right, spills);
        }
        MirOp::Call { target, args, .. } => {
            collect_call_target_spills(target, spills);
            for arg in args {
                collect_value_spills(&arg.value, spills);
            }
        }
        MirOp::AdvanceAddress { index, .. } | MirOp::StoreIndirect { src: index, .. } => {
            collect_value_spills(index, spills);
        }
        MirOp::LoadIndirect { dst, .. } => collect_def_spills(dst, spills),
        MirOp::UpdateReg { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
        MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. } => {}
    }
}

fn collect_terminator_spills(terminator: &MirTerminator, spills: &mut Vec<MirSpillId>) {
    if let MirTerminator::Branch {
        cond: MirCond::BoolValue(value),
        ..
    } = terminator
    {
        collect_value_spills(value, spills);
    }
}

fn collect_call_target_spills(target: &MirCallTarget, spills: &mut Vec<MirSpillId>) {
    if let MirCallTarget::Indirect { target, .. } = target {
        collect_value_spills(target, spills);
    }
}

fn collect_addr_spills(addr: &MirAddr, spills: &mut Vec<MirSpillId>) {
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => collect_mem_spills(mem, spills),
        MirAddr::ComputedIndex { base, index, .. } => {
            collect_value_spills(base, spills);
            collect_value_spills(index, spills);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            collect_mem_spills(ptr, spills);
            collect_value_spills(index, spills);
        }
        MirAddr::Deref { ptr, .. } => collect_value_spills(ptr, spills),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
}

fn collect_value_spills(value: &MirValue, spills: &mut Vec<MirSpillId>) {
    match value {
        MirValue::Def(def) => collect_def_spills(def, spills),
        MirValue::PointerCell(mem) => collect_mem_spills(mem, spills),
        MirValue::StorageAddrByte { mem, .. } => collect_mem_spills(mem, spills),
        MirValue::Word { lo, hi } => {
            collect_value_spills(lo, spills);
            collect_value_spills(hi, spills);
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => {}
    }
}

fn collect_def_spills(def: &MirDef, spills: &mut Vec<MirSpillId>) {
    if let Some(spill) = temp_def_spill(def) {
        collect_spill(spill, spills);
    }
}

fn collect_def_spills_for_width(def: &MirDef, width: MirWidth, spills: &mut Vec<MirSpillId>) {
    if width == MirWidth::Word
        && let Some((lo, hi)) = split_def(def.clone())
    {
        collect_def_spills(&lo, spills);
        collect_def_spills(&hi, spills);
        return;
    }
    collect_def_spills(def, spills);
}

fn collect_mem_spills(mem: &MirMem, spills: &mut Vec<MirSpillId>) {
    if let MirMem::Spill { id, .. } = mem {
        collect_spill(*id, spills);
    }
}

fn collect_spill(spill: MirSpillId, spills: &mut Vec<MirSpillId>) {
    if !spills.contains(&spill) {
        spills.push(spill);
    }
}
