//! Exact BYTE arithmetic on captured values; no external reads or scratch staging.
use super::*;

#[cfg(test)]
#[path = "byte_arithmetic_tests.rs"]
mod tests;

impl Builder<'_> {
    fn arithmetic_byte_operand(
        &self,
        value: &Mir65816Value,
    ) -> Result<Option<ByteOperand>, String> {
        match value {
            Mir65816Value::U8(_) | Mir65816Value::Param(_) => {}
            Mir65816Value::Temp(id, _) => {
                // BYTE arithmetic does not own a scalar/pointer capture binding.
                // Never read an omitted capture's former allocated home.
                if self.borrowed.contains_key(id) || self.scalar_borrowed.contains_key(id) {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        }
        let operand = self.byte_operand(value)?;
        if matches!(operand, Some(ByteOperand::Stack(at)) if u32::from(at) <= self.code.delta()) {
            return Err("BYTE arithmetic requires a nonzero private stack home".into());
        }
        Ok(operand)
    }

    pub(super) fn direct_byte_binary(
        &mut self,
        dest: TempId,
        bytes: u8,
        operation: NirBinaryOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<bool, String> {
        if bytes != 1
            || !matches!(
                operation,
                NirBinaryOp::Add
                    | NirBinaryOp::Sub
                    | NirBinaryOp::And
                    | NirBinaryOp::Or
                    | NirBinaryOp::Xor
            )
        {
            return Ok(false);
        }
        let Mir65816Value::U8(right) = right else {
            return Ok(false);
        };
        let destination = self.temp(dest)?;
        if destination.slot().width != 1 {
            return Err("BYTE arithmetic result width mismatch".into());
        }
        let Location::Stack(destination) = destination else {
            return Ok(false);
        };
        if destination.offset == 0 {
            return Err("BYTE arithmetic requires a nonzero result home".into());
        }
        let destination = self.displacement(destination.offset.into(), 0)?;
        let Some(left) = self.arithmetic_byte_operand(left)? else {
            return Ok(false);
        };
        // Both reads precede the exact-byte store, including identical homes.
        // Earlier capture, expression, fusion and DP/X owners are unchanged.
        self.code.barrier();
        self.code.a8();
        self.load_byte_operand(left);
        self.byte_expression_rhs(operation, ByteOperand::Immediate(*right));
        self.code.byte(ByteOp::StaStack, destination);
        Ok(true)
    }
}
