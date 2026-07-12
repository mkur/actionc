use super::values::offset_mem;
use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirCallTarget, MirCond, MirMem, MirMemoryEffect, MirOp,
    MirPointerPair, MirTerminator, MirValue,
};

pub(super) fn mem_is_read_after(
    ops: &[MirOp],
    start: usize,
    terminator: &MirTerminator,
    mem: &MirMem,
) -> bool {
    ops[start..].iter().any(|op| op_reads_mem(op, mem)) || terminator_reads_mem(terminator, mem)
}

fn terminator_reads_mem(terminator: &MirTerminator, mem: &MirMem) -> bool {
    match terminator {
        MirTerminator::Branch {
            cond: MirCond::BoolValue(value),
            ..
        } => value_reads_mem(value, mem),
        MirTerminator::Jump(_)
        | MirTerminator::Branch { .. }
        | MirTerminator::Return
        | MirTerminator::Exit
        | MirTerminator::Unreachable => false,
    }
}

pub(super) fn op_reads_mem(op: &MirOp, mem: &MirMem) -> bool {
    match op {
        MirOp::Load { src, .. } => addr_reads_mem(src, mem),
        MirOp::Store { dst, src, .. } => {
            store_addr_reads_mem(dst, mem) || value_reads_mem(src, mem)
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. } => value_reads_mem(src, mem),
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            value_reads_mem(left, mem) || value_reads_mem(right, mem)
        }
        MirOp::UpdateMem { mem: op_mem, .. } => op_mem == mem,
        MirOp::AddByteToWordMem { mem: op_mem, value }
        | MirOp::SubByteFromWordMem { mem: op_mem, value } => {
            op_mem == mem || offset_mem(op_mem, 1) == *mem || value_reads_mem(value, mem)
        }
        MirOp::Call { target, args, .. } => {
            let target_reads = match target {
                MirCallTarget::Indirect { target, .. } => value_reads_mem(target, mem),
                MirCallTarget::Routine(_)
                | MirCallTarget::Builtin { .. }
                | MirCallTarget::Runtime { .. } => false,
            };
            target_reads || args.iter().any(|arg| value_reads_mem(&arg.value, mem))
        }
        MirOp::LoadIndirect { consumer, .. } => address_consumer_reads_mem(consumer, mem),
        MirOp::StoreIndirect { consumer, src, .. } => {
            address_consumer_reads_mem(consumer, mem) || value_reads_mem(src, mem)
        }
        MirOp::AdvanceAddress {
            consumer, index, ..
        } => address_consumer_reads_mem(consumer, mem) || value_reads_mem(index, mem),
        MirOp::MaterializeIndexedAddress {
            consumer,
            base,
            index,
            ..
        } => {
            address_consumer_reads_mem(consumer, mem)
                || value_reads_mem(base, mem)
                || value_reads_mem(index, mem)
        }
        MirOp::IndirectByteCompound { target, source, .. } => {
            address_consumer_reads_mem(target, mem) || address_consumer_reads_mem(source, mem)
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}

fn addr_reads_mem(addr: &MirAddr, mem: &MirMem) -> bool {
    match addr {
        MirAddr::Direct(op_mem)
        | MirAddr::AbsoluteIndexedX { base: op_mem }
        | MirAddr::AbsoluteIndexedY { base: op_mem } => op_mem == mem,
        MirAddr::PointerCell { ptr, .. } | MirAddr::PointerIndex { ptr, .. } => ptr == mem,
        MirAddr::ComputedIndex { base, index, .. } => {
            value_reads_mem(base, mem) || value_reads_mem(index, mem)
        }
        MirAddr::Deref { ptr, .. } => value_reads_mem(ptr, mem),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => false,
    }
}

fn address_consumer_reads_mem(consumer: &MirAddressConsumer, mem: &MirMem) -> bool {
    match consumer {
        MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(slot)) => {
            *mem == MirMem::ZeroPage(*slot) || *mem == offset_mem(&MirMem::ZeroPage(*slot), 1)
        }
        MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo }) => {
            *mem == MirMem::FixedZeroPage(*lo) || *mem == offset_mem(&MirMem::FixedZeroPage(*lo), 1)
        }
    }
}

fn store_addr_reads_mem(addr: &MirAddr, mem: &MirMem) -> bool {
    match addr {
        MirAddr::PointerCell { ptr, .. } | MirAddr::PointerIndex { ptr, .. } => ptr == mem,
        MirAddr::ComputedIndex { base, index, .. } => {
            value_reads_mem(base, mem) || value_reads_mem(index, mem)
        }
        MirAddr::Deref { ptr, .. } => value_reads_mem(ptr, mem),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => false,
    }
}

fn value_reads_mem(value: &MirValue, mem: &MirMem) -> bool {
    match value {
        MirValue::PointerCell(op_mem) => op_mem == mem,
        MirValue::Word { lo, hi } => value_reads_mem(lo, mem) || value_reads_mem(hi, mem),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => false,
    }
}

pub(super) fn op_definitely_writes_mem(op: &MirOp, mem: &MirMem) -> bool {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(dst),
            ..
        } => dst == mem,
        MirOp::UpdateMem { mem: dst, .. } => dst == mem,
        _ => false,
    }
}

pub(super) fn op_may_have_unknown_memory_effects(op: &MirOp) -> bool {
    match op {
        MirOp::Call { effects, .. } | MirOp::RuntimeHelper { effects, .. } => {
            !matches!(effects.memory_reads, MirMemoryEffect::None)
                || !matches!(effects.memory_writes, MirMemoryEffect::None)
        }
        MirOp::Barrier { effects } | MirOp::MachineBlock { effects, .. } => {
            effects.opaque
                || !matches!(effects.memory_reads, MirMemoryEffect::None)
                || !matches!(effects.memory_writes, MirMemoryEffect::None)
        }
        MirOp::StoreIndirect { .. } | MirOp::IndirectByteCompound { .. } => true,
        _ => false,
    }
}

pub(super) fn op_may_write_mem(op: &MirOp, mem: &MirMem) -> bool {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(dst),
            ..
        } => dst == mem,
        MirOp::UpdateMem { mem: dst, .. } => dst == mem,
        MirOp::AddByteToWordMem { mem: dst, .. } | MirOp::SubByteFromWordMem { mem: dst, .. } => {
            dst == mem || &offset_mem(dst, 1) == mem
        }
        MirOp::Call { effects, .. } | MirOp::RuntimeHelper { effects, .. } => {
            !matches!(effects.memory_writes, MirMemoryEffect::None)
        }
        MirOp::Barrier { effects } | MirOp::MachineBlock { effects, .. } => {
            effects.opaque || !matches!(effects.memory_writes, MirMemoryEffect::None)
        }
        MirOp::Store { .. } | MirOp::StoreIndirect { .. } | MirOp::IndirectByteCompound { .. } => {
            true
        }
        MirOp::Load { .. }
        | MirOp::LoadImm { .. }
        | MirOp::Move { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::Extend { .. }
        | MirOp::Truncate { .. }
        | MirOp::Unary { .. }
        | MirOp::Binary { .. }
        | MirOp::Compare { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::LoadIndirect { .. } => false,
    }
}
