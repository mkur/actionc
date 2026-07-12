use crate::mir6502::ir::{
    MirAddr, MirCallTarget, MirCond, MirDef, MirOp, MirTempId, MirTerminator, MirValue,
};

pub(super) fn value_uses_temp(value: &MirValue) -> bool {
    match value {
        MirValue::Def(MirDef::VTemp(_) | MirDef::VTempByte { .. }) => true,
        MirValue::Word { lo, hi } => value_uses_temp(lo) || value_uses_temp(hi),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(MirDef::Reg(_))
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => false,
    }
}

pub(super) fn terminator_uses_temp(terminator: &MirTerminator, temp: MirTempId) -> bool {
    match terminator {
        MirTerminator::Branch {
            cond: MirCond::BoolValue(value),
            ..
        } => value_uses_specific_temp(value, temp),
        MirTerminator::Jump(_)
        | MirTerminator::Branch { .. }
        | MirTerminator::Return
        | MirTerminator::Exit
        | MirTerminator::Unreachable => false,
    }
}

pub(super) fn op_uses_temp(op: &MirOp, temp: MirTempId) -> bool {
    match op {
        MirOp::Load { src, .. } => addr_uses_temp(src, temp),
        MirOp::Store { dst, src, .. } => {
            addr_uses_temp(dst, temp) || value_uses_specific_temp(src, temp)
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::StoreIndirect { src, .. }
        | MirOp::AdvanceAddress { index: src, .. } => value_uses_specific_temp(src, temp),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            value_uses_specific_temp(base, temp) || value_uses_specific_temp(index, temp)
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            value_uses_specific_temp(left, temp) || value_uses_specific_temp(right, temp)
        }
        MirOp::AddByteToWordMem { value, .. } | MirOp::SubByteFromWordMem { value, .. } => {
            value_uses_specific_temp(value, temp)
        }
        MirOp::Call { target, args, .. } => {
            call_target_uses_temp(target, temp)
                || args
                    .iter()
                    .any(|arg| value_uses_specific_temp(&arg.value, temp))
        }
        MirOp::LoadImm { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}

pub(super) fn op_uses_temp_more_than_once(op: &MirOp, temp: MirTempId) -> bool {
    let mut uses = 0usize;
    count_op_temp_uses(op, temp, &mut uses);
    uses > 1
}

fn count_op_temp_uses(op: &MirOp, temp: MirTempId, uses: &mut usize) {
    match op {
        MirOp::Load { src, .. } => count_addr_temp_uses(src, temp, uses),
        MirOp::Store { dst, src, .. } => {
            count_addr_temp_uses(dst, temp, uses);
            count_value_temp_uses(src, temp, uses);
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => count_value_temp_uses(src, temp, uses),
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            count_value_temp_uses(base, temp, uses);
            count_value_temp_uses(index, temp, uses);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            count_value_temp_uses(left, temp, uses);
            count_value_temp_uses(right, temp, uses);
        }
        MirOp::AddByteToWordMem { value, .. } | MirOp::SubByteFromWordMem { value, .. } => {
            count_value_temp_uses(value, temp, uses)
        }
        MirOp::Call { target, args, .. } => {
            count_call_target_temp_uses(target, temp, uses);
            for arg in args {
                count_value_temp_uses(&arg.value, temp, uses);
            }
        }
        MirOp::LoadImm { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn count_addr_temp_uses(addr: &MirAddr, temp: MirTempId, uses: &mut usize) {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            count_value_temp_uses(base, temp, uses);
            count_value_temp_uses(index, temp, uses);
        }
        MirAddr::PointerIndex { index, .. } => count_value_temp_uses(index, temp, uses),
        MirAddr::Deref { ptr, .. } => count_value_temp_uses(ptr, temp, uses),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::PointerCell { .. } => {}
    }
}

pub(super) fn count_call_target_temp_uses(
    target: &MirCallTarget,
    temp: MirTempId,
    uses: &mut usize,
) {
    if let MirCallTarget::Indirect { target, .. } = target {
        count_value_temp_uses(target, temp, uses);
    }
}

pub(super) fn count_value_temp_uses(value: &MirValue, temp: MirTempId, uses: &mut usize) {
    match value {
        MirValue::Def(MirDef::VTemp(id)) if *id == temp => *uses += 1,
        MirValue::Def(MirDef::VTempByte { id, .. }) if *id == temp => *uses += 1,
        MirValue::Word { lo, hi } => {
            count_value_temp_uses(lo, temp, uses);
            count_value_temp_uses(hi, temp, uses);
        }
        _ => {}
    }
}

fn addr_uses_temp(addr: &MirAddr, temp: MirTempId) -> bool {
    match addr {
        MirAddr::Direct(_) => false,
        MirAddr::ComputedIndex { base, index, .. } => {
            value_uses_specific_temp(base, temp) || value_uses_specific_temp(index, temp)
        }
        MirAddr::PointerIndex { index, .. } => value_uses_specific_temp(index, temp),
        MirAddr::Deref { ptr, .. } => value_uses_specific_temp(ptr, temp),
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => false,
        MirAddr::PointerCell { .. } => false,
    }
}

fn call_target_uses_temp(target: &MirCallTarget, temp: MirTempId) -> bool {
    match target {
        MirCallTarget::Routine(_)
        | MirCallTarget::Builtin { .. }
        | MirCallTarget::Runtime { .. } => false,
        MirCallTarget::Indirect { target, .. } => value_uses_specific_temp(target, temp),
    }
}

fn value_uses_specific_temp(value: &MirValue, temp: MirTempId) -> bool {
    match value {
        MirValue::Def(MirDef::VTemp(id)) => *id == temp,
        MirValue::Def(MirDef::VTempByte { id, .. }) => *id == temp,
        MirValue::Word { lo, hi } => {
            value_uses_specific_temp(lo, temp) || value_uses_specific_temp(hi, temp)
        }
        _ => false,
    }
}
