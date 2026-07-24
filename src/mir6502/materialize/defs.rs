use crate::mir6502::ir::{MirDef, MirOp, MirTempId};

pub(super) fn split_def_as_temp(def: &MirDef) -> Option<MirTempId> {
    match def {
        MirDef::VTemp(id) => Some(*id),
        _ => None,
    }
}

pub(super) fn op_def(op: &MirOp) -> Option<&MirDef> {
    match op {
        MirOp::LoadImm { dst, .. }
        | MirOp::Load { dst, .. }
        | MirOp::Move { dst, .. }
        | MirOp::LeaAddr { dst, .. }
        | MirOp::Extend { dst, .. }
        | MirOp::Truncate { dst, .. }
        | MirOp::Unary { dst, .. }
        | MirOp::Binary { dst, .. }
        | MirOp::LoadIndirect { dst, .. } => Some(dst),
        MirOp::Call { result, .. } => result.as_ref().map(|result| &result.dst),
        MirOp::Store { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::OffsetPointerByIndirectByte { .. }
        | MirOp::Compare { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => None,
    }
}
