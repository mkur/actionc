use crate::mir6502::analysis::effects::classify_op;
#[cfg(test)]
use crate::mir6502::analysis::effects::classify_value;
#[cfg(test)]
use crate::mir6502::ir::MirValue;
use crate::mir6502::ir::{MirOp, MirReg};

pub(super) fn op_reads_reg(op: &MirOp, reg: MirReg) -> bool {
    // Runtime-helper ABI homes were not part of this legacy query. The central
    // effects model records them for routine-wide machine liveness.
    !matches!(op, MirOp::RuntimeHelper { .. }) && classify_op(op).reads_reg(reg)
}

pub(super) fn op_writes_reg(op: &MirOp, reg: MirReg) -> bool {
    // Keep the same compatibility boundary as `op_reads_reg` until its callers
    // migrate to the typed routine-level query.
    !matches!(op, MirOp::RuntimeHelper { .. }) && classify_op(op).writes_reg(reg)
}

#[cfg(test)]
pub(super) fn value_reads_reg(value: &MirValue, reg: MirReg) -> bool {
    classify_value(value).reads_reg(reg)
}
