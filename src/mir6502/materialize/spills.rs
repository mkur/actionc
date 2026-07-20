use super::peepholes::private_scratch_store_removal_is_safe_after;
use super::*;
use crate::mir6502::ir::{
    MirAddr, MirBlock, MirBlockId, MirCallTarget, MirDef, MirMem, MirOp, MirRoutine, MirSpillId,
    MirTerminator, MirValue, MirZpSlot, RoutineId,
};
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
    let mut homes = BTreeSet::new();
    match op {
        MirOp::Load { src, .. } => collect_addr_read_homes(src, &mut homes),
        MirOp::Store { dst, src, .. } => {
            collect_store_addr_read_homes(dst, &mut homes);
            collect_value_read_homes(src, &mut homes);
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. } => collect_value_read_homes(src, &mut homes),
        MirOp::MaterializeAddress { value, .. } => collect_value_read_homes(value, &mut homes),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            collect_value_read_homes(base, &mut homes);
            collect_value_read_homes(index, &mut homes);
        }
        MirOp::UpdateMem { mem, .. } => collect_mem_home(mem, &mut homes),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            collect_mem_home(mem, &mut homes);
            collect_value_read_homes(value, &mut homes);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            collect_value_read_homes(left, &mut homes);
            collect_value_read_homes(right, &mut homes);
        }
        MirOp::Call { target, args, .. } => {
            if let MirCallTarget::Indirect { target, .. } = target {
                collect_value_read_homes(target, &mut homes);
            }
            for arg in args {
                collect_value_read_homes(&arg.value, &mut homes);
            }
        }
        MirOp::AdvanceAddress { index, .. } | MirOp::StoreIndirect { src: index, .. } => {
            collect_value_read_homes(index, &mut homes);
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
    homes
}

fn op_write_homes(op: &MirOp) -> BTreeSet<MirHomeStorage> {
    let mut homes = BTreeSet::new();
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            ..
        }
        | MirOp::UpdateMem { mem, .. }
        | MirOp::AddByteToWordMem { mem, .. }
        | MirOp::SubByteFromWordMem { mem, .. } => collect_mem_home(mem, &mut homes),
        MirOp::Load { .. }
        | MirOp::Move { .. }
        | MirOp::Extend { .. }
        | MirOp::Truncate { .. }
        | MirOp::Unary { .. }
        | MirOp::Binary { .. }
        | MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::Compare { .. }
        | MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::Store { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
    homes
}

fn collect_addr_read_homes(addr: &MirAddr, homes: &mut BTreeSet<MirHomeStorage>) {
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => collect_mem_home(mem, homes),
        MirAddr::ComputedIndex { base, index, .. } => {
            collect_value_read_homes(base, homes);
            collect_value_read_homes(index, homes);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            collect_mem_home(ptr, homes);
            collect_value_read_homes(index, homes);
        }
        MirAddr::Deref { ptr, .. } => collect_value_read_homes(ptr, homes),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
}

fn collect_store_addr_read_homes(addr: &MirAddr, homes: &mut BTreeSet<MirHomeStorage>) {
    match addr {
        MirAddr::PointerCell { ptr, .. } => collect_mem_home(ptr, homes),
        MirAddr::ComputedIndex { base, index, .. } => {
            collect_value_read_homes(base, homes);
            collect_value_read_homes(index, homes);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            collect_mem_home(ptr, homes);
            collect_value_read_homes(index, homes);
        }
        MirAddr::Deref { ptr, .. } => collect_value_read_homes(ptr, homes),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
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
    let mut spills = BTreeSet::new();
    collect_op_read_spills(op, &mut spills);
    spills
}

fn op_write_spills(op: &MirOp) -> BTreeSet<MirSpillId> {
    let mut spills = BTreeSet::new();
    collect_op_write_spills(op, &mut spills);
    spills
}

fn collect_op_read_spills(op: &MirOp, spills: &mut BTreeSet<MirSpillId>) {
    match op {
        MirOp::Load { src, .. } => collect_addr_read_spills(src, spills),
        MirOp::Store { dst, src, .. } => {
            collect_store_addr_read_spills(dst, spills);
            collect_value_read_spills(src, spills);
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. } => collect_value_read_spills(src, spills),
        MirOp::MaterializeAddress { value, .. } => collect_value_read_spills(value, spills),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            collect_value_read_spills(base, spills);
            collect_value_read_spills(index, spills);
        }
        MirOp::LoadImm { .. } | MirOp::LeaAddr { .. } => {}
        MirOp::UpdateMem { mem, .. } => collect_mem_read_spills(mem, spills),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            collect_mem_read_spills(mem, spills);
            collect_value_read_spills(value, spills);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            collect_value_read_spills(left, spills);
            collect_value_read_spills(right, spills);
        }
        MirOp::Call { target, args, .. } => {
            if let MirCallTarget::Indirect { target, .. } = target {
                collect_value_read_spills(target, spills);
            }
            for arg in args {
                collect_value_read_spills(&arg.value, spills);
            }
        }
        MirOp::AdvanceAddress { index, .. } | MirOp::StoreIndirect { src: index, .. } => {
            collect_value_read_spills(index, spills);
        }
        MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn collect_op_write_spills(op: &MirOp, spills: &mut BTreeSet<MirSpillId>) {
    match op {
        MirOp::Load { dst, .. }
        | MirOp::Move { dst, .. }
        | MirOp::Extend { dst, .. }
        | MirOp::Truncate { dst, .. }
        | MirOp::Unary { dst, .. }
        | MirOp::Binary { dst, .. }
        | MirOp::LoadImm { dst, .. }
        | MirOp::LeaAddr { dst, .. }
        | MirOp::LoadIndirect { dst, .. } => collect_def_write_spills(dst, spills),
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill { id, .. }),
            ..
        } => {
            spills.insert(*id);
        }
        MirOp::UpdateMem {
            mem: MirMem::Spill { id, .. },
            ..
        } => {
            spills.insert(*id);
        }
        MirOp::AddByteToWordMem {
            mem: MirMem::Spill { id, .. },
            ..
        }
        | MirOp::SubByteFromWordMem {
            mem: MirMem::Spill { id, .. },
            ..
        } => {
            spills.insert(*id);
        }
        MirOp::Compare { .. }
        | MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::Store { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn collect_def_write_spills(def: &MirDef, spills: &mut BTreeSet<MirSpillId>) {
    if let Some(spill) = temp_def_spill(def) {
        spills.insert(spill);
    }
}

pub(super) fn collect_addr_read_spills(addr: &MirAddr, spills: &mut BTreeSet<MirSpillId>) {
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => collect_mem_read_spills(mem, spills),
        MirAddr::ComputedIndex { base, index, .. } => {
            collect_value_read_spills(base, spills);
            collect_value_read_spills(index, spills);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            collect_mem_read_spills(ptr, spills);
            collect_value_read_spills(index, spills);
        }
        MirAddr::Deref { ptr, .. } => collect_value_read_spills(ptr, spills),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
}

pub(super) fn collect_store_addr_read_spills(addr: &MirAddr, spills: &mut BTreeSet<MirSpillId>) {
    match addr {
        MirAddr::PointerCell { ptr, .. } => collect_mem_read_spills(ptr, spills),
        MirAddr::ComputedIndex { base, index, .. } => {
            collect_value_read_spills(base, spills);
            collect_value_read_spills(index, spills);
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            collect_mem_read_spills(ptr, spills);
            collect_value_read_spills(index, spills);
        }
        MirAddr::Deref { ptr, .. } => collect_value_read_spills(ptr, spills),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {}
    }
}

pub(super) fn collect_value_read_spills(value: &MirValue, spills: &mut BTreeSet<MirSpillId>) {
    match value {
        MirValue::Def(def) => {
            if let Some(spill) = temp_def_spill(def) {
                spills.insert(spill);
            }
        }
        MirValue::PointerCell(mem) => collect_mem_read_spills(mem, spills),
        MirValue::StorageAddrByte { mem, .. } => collect_mem_read_spills(mem, spills),
        MirValue::Word { lo, hi } => {
            collect_value_read_spills(lo, spills);
            collect_value_read_spills(hi, spills);
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => {}
    }
}

pub(super) fn collect_mem_read_spills(mem: &MirMem, spills: &mut BTreeSet<MirSpillId>) {
    if let MirMem::Spill { id, .. } = mem {
        spills.insert(*id);
    }
}

fn temp_def_spill(def: &MirDef) -> Option<MirSpillId> {
    match def {
        MirDef::VTemp(id) => Some(MirSpillId(id.0.saturating_mul(2))),
        MirDef::VTempByte { id, byte } if *byte <= 1 => Some(MirSpillId(
            id.0.saturating_mul(2).saturating_add(*byte as u32),
        )),
        MirDef::VTempByte { .. } | MirDef::Reg(_) => None,
    }
}

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

pub(super) fn can_remove_spill_reload_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> bool {
    match ops.get(index + 1) {
        Some(MirOp::Store { .. })
        | Some(MirOp::Compare { .. })
        | Some(MirOp::Unary { .. })
        | Some(MirOp::Binary { .. })
        | Some(MirOp::Call { .. })
        | Some(MirOp::RuntimeHelper { .. }) => true,
        Some(MirOp::Load { .. })
        | Some(MirOp::LoadImm { .. })
        | Some(MirOp::Move { .. })
        | Some(MirOp::LeaAddr { .. })
        | Some(MirOp::Extend { .. })
        | Some(MirOp::Truncate { .. })
        | Some(MirOp::UpdateMem { .. })
        | Some(MirOp::AddByteToWordMem { .. })
        | Some(MirOp::SubByteFromWordMem { .. })
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
    if op_writes_reg(op, reg) {
        return true;
    }
    match op {
        MirOp::Call { .. } => true,
        MirOp::RuntimeHelper { .. } | MirOp::Barrier { .. } | MirOp::MachineBlock { .. } => true,
        MirOp::AddByteToWordMem { .. } | MirOp::SubByteFromWordMem { .. } if reg == MirReg::A => {
            true
        }
        MirOp::IndirectByteCompound { .. } if reg == MirReg::A => true,
        MirOp::MaterializeIndexedAddress { .. } if reg == MirReg::A => true,
        MirOp::Load { .. }
        | MirOp::Store { .. }
        | MirOp::Move { .. }
        | MirOp::Extend { .. }
        | MirOp::Truncate { .. }
        | MirOp::Unary { .. }
        | MirOp::Binary { .. }
        | MirOp::Compare { .. }
        | MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::StoreIndirect { .. } => false,
        MirOp::IndirectByteCompound { .. } => false,
    }
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
        | MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
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
        | MirOp::UpdateMem { .. } => {}
    }
}

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
    let mut spills = BTreeSet::new();
    match op {
        MirOp::Load { src, .. } => collect_addr_read_spills(src, &mut spills),
        MirOp::Store { dst, src, .. } => {
            collect_store_addr_read_spills(dst, &mut spills);
            collect_value_read_spills(src, &mut spills);
        }
        MirOp::UpdateMem { mem, .. } => collect_mem_read_spills(mem, &mut spills),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            collect_mem_read_spills(mem, &mut spills);
            collect_value_read_spills(value, &mut spills);
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. } => collect_value_read_spills(src, &mut spills),
        MirOp::MaterializeAddress { value, .. } => collect_value_read_spills(value, &mut spills),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            collect_value_read_spills(base, &mut spills);
            collect_value_read_spills(index, &mut spills);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            collect_value_read_spills(left, &mut spills);
            collect_value_read_spills(right, &mut spills);
        }
        MirOp::Call { target, args, .. } => {
            if let MirCallTarget::Indirect { target, .. } = target {
                collect_value_read_spills(target, &mut spills);
            }
            for arg in args {
                collect_value_read_spills(&arg.value, &mut spills);
            }
        }
        MirOp::AdvanceAddress { index, .. } | MirOp::StoreIndirect { src: index, .. } => {
            collect_value_read_spills(index, &mut spills);
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
    spills
}

fn op_direct_write_spills(op: &MirOp) -> BTreeSet<MirSpillId> {
    let mut spills = BTreeSet::new();
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill { id, .. }),
            ..
        }
        | MirOp::UpdateMem {
            mem: MirMem::Spill { id, .. },
            ..
        } => {
            spills.insert(*id);
        }
        MirOp::AddByteToWordMem {
            mem: MirMem::Spill { id, .. },
            ..
        }
        | MirOp::SubByteFromWordMem {
            mem: MirMem::Spill { id, .. },
            ..
        } => {
            spills.insert(*id);
        }
        MirOp::Load { .. }
        | MirOp::Move { .. }
        | MirOp::Extend { .. }
        | MirOp::Truncate { .. }
        | MirOp::Unary { .. }
        | MirOp::Binary { .. }
        | MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::Compare { .. }
        | MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::Store { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
    spills
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
        MirOp::UpdateMem { mem, .. } => remap_mem_spills(mem, remap),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            remap_mem_spills(mem, remap);
            remap_value_spills(value, remap);
        }
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
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
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
        MirOp::UpdateMem { mem, .. } => remap_mem_spills_to_zero_page(mem, remap),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            remap_mem_spills_to_zero_page(mem, remap);
            remap_value_spills_to_zero_page(value, remap);
        }
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
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
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

fn visit_op_mems<F>(op: &MirOp, visitor: &mut F)
where
    F: FnMut(&MirMem),
{
    match op {
        MirOp::Load { src, .. } => visit_addr_mems(src, visitor),
        MirOp::Store { dst, src, .. } => {
            visit_addr_mems(dst, visitor);
            visit_value_mems(src, visitor);
        }
        MirOp::UpdateMem { mem, .. } => visitor(mem),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            visitor(mem);
            visit_value_mems(value, visitor);
        }
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
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
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
        MirOp::UpdateMem { mem, .. } => collect_mem_spills(mem, spills),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            collect_mem_spills(mem, spills);
            collect_value_spills(value, spills);
        }
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
        MirOp::Compare { left, right, .. } => {
            collect_value_spills(left, spills);
            collect_value_spills(right, spills);
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
        MirOp::RuntimeHelper { .. } | MirOp::Barrier { .. } | MirOp::MachineBlock { .. } => {}
        MirOp::IndirectByteCompound { .. } => {}
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
