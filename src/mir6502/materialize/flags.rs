use crate::mir6502::analysis::effects::{classify_op, classify_terminator};
use crate::mir6502::ir::{MirOp, MirTerminator};

pub(super) fn op_uses_previous_carry(op: &MirOp) -> bool {
    classify_op(op).machine.uses_previous_carry
}

pub(super) fn op_overwrites_carry(op: &MirOp) -> bool {
    classify_op(op).machine.definitely_overwrites_carry
}

pub(super) fn op_overwrites_overflow(op: &MirOp) -> bool {
    classify_op(op).machine.definitely_overwrites_overflow
}

pub(super) fn op_clobbers_unknown_flag_or_a_effects(op: &MirOp) -> bool {
    classify_op(op).machine.unknown_flag_or_a_effects
}

pub(super) fn op_has_opaque_flag_or_a_effects(op: &MirOp) -> bool {
    classify_op(op).machine.opaque_flag_or_a_effects
}

pub(super) fn terminator_consumes_flags(terminator: &MirTerminator) -> bool {
    classify_terminator(terminator).consumes_flags_compat
}

pub(super) fn op_writes_flags(op: &MirOp) -> bool {
    classify_op(op).machine.writes_any_flags_compat
}
