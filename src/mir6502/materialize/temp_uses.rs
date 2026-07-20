use crate::mir6502::analysis::effects::{
    classify_op, classify_terminator, classify_value,
    count_call_target_temp_uses as classified_call_target_temp_uses,
};
use crate::mir6502::ir::{MirCallTarget, MirOp, MirTempId, MirTerminator, MirValue};

pub(super) fn value_uses_temp(value: &MirValue) -> bool {
    !classify_value(value).logical.temp_uses.is_empty()
}

pub(super) fn terminator_uses_temp(terminator: &MirTerminator, temp: MirTempId) -> bool {
    classify_terminator(terminator)
        .logical
        .temp_uses
        .iter()
        .any(|access| access.temp() == temp)
}

pub(super) fn op_uses_temp(op: &MirOp, temp: MirTempId) -> bool {
    classify_op(op).uses_temp(temp)
}

pub(super) fn op_uses_temp_more_than_once(op: &MirOp, temp: MirTempId) -> bool {
    classify_op(op).temp_use_count(temp) > 1
}

pub(super) fn count_call_target_temp_uses(
    target: &MirCallTarget,
    temp: MirTempId,
    uses: &mut usize,
) {
    *uses += classified_call_target_temp_uses(target, temp);
}

pub(super) fn count_value_temp_uses(value: &MirValue, temp: MirTempId, uses: &mut usize) {
    *uses += classify_value(value)
        .logical
        .temp_uses
        .iter()
        .filter(|access| access.temp() == temp)
        .count();
}
