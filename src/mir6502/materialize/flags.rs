use crate::mir6502::ir::{
    MirBinaryOp, MirCarryIn, MirCond, MirCondDest, MirDef, MirOp, MirTerminator,
};

pub(super) fn op_uses_previous_carry(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Binary {
            carry_in: Some(MirCarryIn::FromPrevious),
            ..
        }
    )
}

pub(super) fn op_overwrites_carry(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub,
            carry_in: None | Some(MirCarryIn::Clear | MirCarryIn::Set),
            ..
        } | MirOp::Compare {
            dst: MirCondDest::Flags,
            ..
        }
    )
}

pub(super) fn op_overwrites_overflow(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub,
            carry_in: None | Some(MirCarryIn::Clear | MirCarryIn::Set),
            ..
        }
    )
}

pub(super) fn op_clobbers_unknown_flag_or_a_effects(op: &MirOp) -> bool {
    matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. })
}

pub(super) fn op_has_opaque_flag_or_a_effects(op: &MirOp) -> bool {
    matches!(op, MirOp::Barrier { .. } | MirOp::MachineBlock { .. })
}

pub(super) fn terminator_consumes_flags(terminator: &MirTerminator) -> bool {
    matches!(
        terminator,
        MirTerminator::Branch {
            cond: MirCond::FlagTest(_) | MirCond::FusedCompare { .. },
            ..
        }
    )
}

pub(super) fn op_writes_flags(op: &MirOp) -> bool {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(_),
            ..
        }
        | MirOp::LoadImm {
            dst: MirDef::Reg(_),
            ..
        }
        | MirOp::Move {
            dst: MirDef::Reg(_),
            ..
        }
        | MirOp::Extend {
            dst: MirDef::Reg(_),
            ..
        }
        | MirOp::Truncate {
            dst: MirDef::Reg(_),
            ..
        }
        | MirOp::Unary {
            dst: MirDef::Reg(_),
            ..
        }
        | MirOp::Binary {
            dst: MirDef::Reg(_),
            ..
        }
        | MirOp::LoadIndirect {
            dst: MirDef::Reg(_),
            ..
        }
        | MirOp::Compare { .. }
        | MirOp::UpdateMem { .. } => true,
        MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::IndirectByteCompound { .. } => true,
        MirOp::Call { .. } => true,
        MirOp::RuntimeHelper { .. } => true,
        MirOp::Store { .. }
        | MirOp::Load { .. }
        | MirOp::Move { .. }
        | MirOp::Extend { .. }
        | MirOp::Truncate { .. }
        | MirOp::Unary { .. }
        | MirOp::Binary { .. }
        | MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}
