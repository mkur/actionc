use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::ir::MirOp;
#[cfg(test)]
use crate::mir6502::{analysis::effects::classify_terminator, ir::MirTerminator};

pub(super) fn op_uses_previous_carry(op: &MirOp) -> bool {
    classify_op(op).machine.uses_previous_carry
}

pub(super) fn op_overwrites_carry(op: &MirOp) -> bool {
    classify_op(op).machine.flag_writes.c
}

pub(super) fn op_overwrites_overflow(op: &MirOp) -> bool {
    classify_op(op).machine.flag_writes.v
}

pub(super) fn op_clobbers_unknown_flag_or_a_effects(op: &MirOp) -> bool {
    classify_op(op).machine.unknown_flag_or_a_effects
}

pub(super) fn op_has_opaque_flag_or_a_effects(op: &MirOp) -> bool {
    classify_op(op).machine.opaque_flag_or_a_effects
}

#[cfg(test)]
pub(super) fn terminator_consumes_flags(terminator: &MirTerminator) -> bool {
    classify_terminator(terminator).consumes_flags_compat
}

#[cfg(test)]
pub(super) fn op_writes_flags(op: &MirOp) -> bool {
    classify_op(op).machine.possible_flag_writes().any()
}
