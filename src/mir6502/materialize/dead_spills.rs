use super::values::offset_mem;
use crate::mir6502::ir::{
    MirAddr, MirCallTarget, MirCond, MirDef, MirMem, MirOp, MirRoutine, MirSpillId, MirTerminator,
    MirValue, MirWidth,
};
use std::collections::BTreeSet;

pub(super) fn remove_dead_spill_stores(routine: &mut MirRoutine) {
    let mut remove = BTreeSet::<(usize, usize)>::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let Some((spill, offset)) = op_store_spill_byte(op) else {
                continue;
            };
            let mut visited = BTreeSet::new();
            if !spill_byte_may_be_read_after(
                routine,
                block_index,
                op_index.saturating_add(1),
                spill,
                offset,
                &mut visited,
            ) {
                remove.insert((block_index, op_index));
            }
        }
    }

    if remove.is_empty() {
        return;
    }
    for (block_index, block) in routine.blocks.iter_mut().enumerate() {
        let mut op_index = 0usize;
        block.ops.retain(|_| {
            let keep = !remove.contains(&(block_index, op_index));
            op_index = op_index.saturating_add(1);
            keep
        });
    }
}

fn spill_byte_may_be_read_after(
    routine: &MirRoutine,
    block_index: usize,
    start: usize,
    spill: MirSpillId,
    offset: u16,
    visited: &mut BTreeSet<usize>,
) -> bool {
    if !visited.insert(block_index) {
        return false;
    }
    let Some(block) = routine.blocks.get(block_index) else {
        return true;
    };
    for op in block.ops.iter().skip(start) {
        if op_reads_spill_byte(op, spill, offset) {
            return true;
        }
        if op_writes_spill_byte(op, spill, offset) {
            return false;
        }
        if op_may_read_unknown_spill_byte(op) {
            return true;
        }
    }
    if terminator_reads_spill_byte(&block.terminator, spill, offset) {
        return true;
    }
    block_successor_indices(routine, &block.terminator)
        .into_iter()
        .any(|successor_index| {
            spill_byte_may_be_read_after(routine, successor_index, 0, spill, offset, visited)
        })
}

pub(super) fn block_successor_indices(
    routine: &MirRoutine,
    terminator: &MirTerminator,
) -> Vec<usize> {
    let mut successors = Vec::new();
    match terminator {
        MirTerminator::Jump(target) => successors.push(*target),
        MirTerminator::Branch {
            then_block,
            else_block,
            ..
        } => {
            successors.push(*then_block);
            successors.push(*else_block);
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
    }
    successors
        .into_iter()
        .filter_map(|id| routine.blocks.iter().position(|block| block.id == id))
        .collect()
}

fn op_store_spill_byte(op: &MirOp) -> Option<(MirSpillId, u16)> {
    let MirOp::Store {
        dst: MirAddr::Direct(MirMem::Spill { id, offset }),
        width: MirWidth::Byte,
        ..
    } = op
    else {
        return None;
    };
    Some((*id, *offset))
}

fn op_reads_spill_byte(op: &MirOp, spill: MirSpillId, offset: u16) -> bool {
    match op {
        MirOp::Load { src, .. } => addr_reads_spill_byte(src, spill, offset),
        MirOp::Store { dst, src, .. } => {
            store_addr_reads_spill_byte(dst, spill, offset)
                || value_reads_spill_byte(src, spill, offset)
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => value_reads_spill_byte(src, spill, offset),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            value_reads_spill_byte(base, spill, offset)
                || value_reads_spill_byte(index, spill, offset)
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            value_reads_spill_byte(left, spill, offset)
                || value_reads_spill_byte(right, spill, offset)
        }
        MirOp::Call { target, args, .. } => {
            call_target_reads_spill_byte(target, spill, offset)
                || args
                    .iter()
                    .any(|arg| value_reads_spill_byte(&arg.value, spill, offset))
        }
        MirOp::UpdateMem { mem, .. } => mem_reads_spill_byte(mem, spill, offset),
        MirOp::AddByteToWordMem { mem, value } | MirOp::SubByteFromWordMem { mem, value } => {
            mem_reads_spill_byte(mem, spill, offset) || value_reads_spill_byte(value, spill, offset)
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}

fn op_writes_spill_byte(op: &MirOp, spill: MirSpillId, offset: u16) -> bool {
    match op {
        MirOp::Load { dst, .. }
        | MirOp::Move { dst, .. }
        | MirOp::Extend { dst, .. }
        | MirOp::Truncate { dst, .. }
        | MirOp::Unary { dst, .. }
        | MirOp::Binary { dst, .. }
        | MirOp::LoadImm { dst, .. }
        | MirOp::LeaAddr { dst, .. }
        | MirOp::LoadIndirect { dst, .. } => def_matches_spill_byte(dst, spill, offset),
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            ..
        } => mem_writes_spill_byte(mem, spill, offset),
        MirOp::UpdateMem { mem, .. } => mem_writes_spill_byte(mem, spill, offset),
        MirOp::AddByteToWordMem { mem, .. } | MirOp::SubByteFromWordMem { mem, .. } => {
            mem_writes_spill_byte(mem, spill, offset)
                || mem_writes_spill_byte(&offset_mem(mem, 1), spill, offset)
        }
        MirOp::Compare { .. }
        | MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::Store { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}

fn op_may_read_unknown_spill_byte(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::RuntimeHelper { .. } | MirOp::Barrier { .. } | MirOp::MachineBlock { .. }
    )
}

fn terminator_reads_spill_byte(terminator: &MirTerminator, spill: MirSpillId, offset: u16) -> bool {
    let MirTerminator::Branch {
        cond: MirCond::BoolValue(value),
        ..
    } = terminator
    else {
        return false;
    };
    value_reads_spill_byte(value, spill, offset)
}

fn addr_reads_spill_byte(addr: &MirAddr, spill: MirSpillId, offset: u16) -> bool {
    match addr {
        MirAddr::Direct(mem)
        | MirAddr::AbsoluteIndexedX { base: mem }
        | MirAddr::AbsoluteIndexedY { base: mem }
        | MirAddr::PointerCell { ptr: mem, .. } => mem_reads_spill_byte(mem, spill, offset),
        MirAddr::ComputedIndex { base, index, .. } => {
            value_reads_spill_byte(base, spill, offset)
                || value_reads_spill_byte(index, spill, offset)
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            mem_reads_spill_byte(ptr, spill, offset) || value_reads_spill_byte(index, spill, offset)
        }
        MirAddr::Deref { ptr, .. } => value_reads_spill_byte(ptr, spill, offset),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => false,
    }
}

fn store_addr_reads_spill_byte(addr: &MirAddr, spill: MirSpillId, offset: u16) -> bool {
    match addr {
        MirAddr::PointerCell { ptr, .. } => mem_reads_spill_byte(ptr, spill, offset),
        MirAddr::ComputedIndex { base, index, .. } => {
            value_reads_spill_byte(base, spill, offset)
                || value_reads_spill_byte(index, spill, offset)
        }
        MirAddr::PointerIndex { ptr, index, .. } => {
            mem_reads_spill_byte(ptr, spill, offset) || value_reads_spill_byte(index, spill, offset)
        }
        MirAddr::Deref { ptr, .. } => value_reads_spill_byte(ptr, spill, offset),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => false,
    }
}

fn call_target_reads_spill_byte(target: &MirCallTarget, spill: MirSpillId, offset: u16) -> bool {
    match target {
        MirCallTarget::Indirect { target, .. } => value_reads_spill_byte(target, spill, offset),
        MirCallTarget::Routine(_)
        | MirCallTarget::Builtin { .. }
        | MirCallTarget::Runtime { .. } => false,
    }
}

fn value_reads_spill_byte(value: &MirValue, spill: MirSpillId, offset: u16) -> bool {
    match value {
        MirValue::Def(def) => def_matches_spill_byte(def, spill, offset),
        MirValue::PointerCell(mem) => mem_reads_spill_byte(mem, spill, offset),
        MirValue::Word { lo, hi } => {
            value_reads_spill_byte(lo, spill, offset) || value_reads_spill_byte(hi, spill, offset)
        }
        MirValue::StorageAddrByte { mem, .. } => mem_reads_spill_byte(mem, spill, offset),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. } => false,
    }
}

fn mem_reads_spill_byte(mem: &MirMem, spill: MirSpillId, offset: u16) -> bool {
    mem_writes_spill_byte(mem, spill, offset)
}

fn mem_writes_spill_byte(mem: &MirMem, spill: MirSpillId, offset: u16) -> bool {
    matches!(mem, MirMem::Spill { id, offset: mem_offset } if *id == spill && *mem_offset == offset)
}

fn def_matches_spill_byte(def: &MirDef, spill: MirSpillId, offset: u16) -> bool {
    match def {
        MirDef::VTemp(id) if offset == 0 => MirSpillId(id.0.saturating_mul(2)) == spill,
        MirDef::VTempByte { id, byte } if offset == 0 => {
            MirSpillId(id.0.saturating_mul(2).saturating_add(*byte as u32)) == spill
        }
        MirDef::VTemp(_) | MirDef::VTempByte { .. } | MirDef::Reg(_) => false,
    }
}
