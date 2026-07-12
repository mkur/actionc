use crate::mir6502::ir::{MirAddr, MirCallTarget, MirDef, MirOp, MirReg, MirValue};

pub(super) fn op_reads_reg(op: &MirOp, reg: MirReg) -> bool {
    match op {
        MirOp::Store { dst, src, .. } => addr_reads_reg(dst, reg) || value_reads_reg(src, reg),
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => value_reads_reg(src, reg),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            value_reads_reg(base, reg) || value_reads_reg(index, reg)
        }
        MirOp::Load { src, .. } => addr_reads_reg(src, reg),
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            value_reads_reg(left, reg) || value_reads_reg(right, reg)
        }
        MirOp::AddByteToWordMem { value, .. } | MirOp::SubByteFromWordMem { value, .. } => {
            value_reads_reg(value, reg)
        }
        MirOp::Call { target, args, .. } => {
            call_target_reads_reg(target, reg)
                || args.iter().any(|arg| value_reads_reg(&arg.value, reg))
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}

pub(super) fn op_writes_reg(op: &MirOp, reg: MirReg) -> bool {
    match op {
        MirOp::Load { dst, .. }
        | MirOp::LoadImm { dst, .. }
        | MirOp::Move { dst, .. }
        | MirOp::LeaAddr { dst, .. }
        | MirOp::Extend { dst, .. }
        | MirOp::Truncate { dst, .. }
        | MirOp::Unary { dst, .. }
        | MirOp::Binary { dst, .. }
        | MirOp::LoadIndirect { dst, .. } => def_writes_reg(dst, reg),
        MirOp::Call { result, .. } => result
            .as_ref()
            .is_some_and(|result| def_writes_reg(&result.dst, reg)),
        MirOp::Store { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::Compare { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}

fn def_writes_reg(def: &MirDef, reg: MirReg) -> bool {
    matches!(def, MirDef::Reg(def_reg) if *def_reg == reg)
}

pub(super) fn value_reads_reg(value: &MirValue, reg: MirReg) -> bool {
    match value {
        MirValue::Def(MirDef::Reg(value_reg)) => *value_reg == reg,
        MirValue::Word { lo, hi } => value_reads_reg(lo, reg) || value_reads_reg(hi, reg),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => false,
    }
}

fn addr_reads_reg(addr: &MirAddr, reg: MirReg) -> bool {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            value_reads_reg(base, reg) || value_reads_reg(index, reg)
        }
        MirAddr::PointerIndex { index, .. } => value_reads_reg(index, reg),
        MirAddr::Deref { ptr, .. } => value_reads_reg(ptr, reg),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::PointerCell { .. } => false,
    }
}

fn call_target_reads_reg(target: &MirCallTarget, reg: MirReg) -> bool {
    match target {
        MirCallTarget::Indirect { target, .. } => value_reads_reg(target, reg),
        MirCallTarget::Routine(_)
        | MirCallTarget::Builtin { .. }
        | MirCallTarget::Runtime { .. } => false,
    }
}
