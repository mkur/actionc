//! Two-word arithmetic over complete private values; no external accesses.
use super::*;

#[cfg(test)]
#[path = "long_arithmetic_tests.rs"]
mod tests;

impl Builder<'_> {
    fn long_arithmetic_operand(
        &self,
        value: &Mir65816Value,
    ) -> Result<Option<LongOperand>, String> {
        // Numeric lanes zero-extend just as value_byte does. Signed widening
        // remains an explicit Cast; narrow captured temps are not widened here.
        match value {
            Mir65816Value::U8(value) => Ok(Some(LongOperand::Immediate(u32::from(*value)))),
            Mir65816Value::U16(value) => Ok(Some(LongOperand::Immediate(u32::from(*value)))),
            Mir65816Value::U24(value) => Ok(Some(LongOperand::Immediate(*value))),
            _ => self.long_operand(value),
        }
    }

    pub(super) fn long_binary(
        &mut self,
        dest: TempId,
        bytes: u8,
        operation: NirBinaryOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<bool, String> {
        if bytes != 4 || !matches!(operation, NirBinaryOp::Add | NirBinaryOp::Sub) {
            return Ok(false);
        }
        // Validate every complete extent, including transient S movement, before
        // emitting any prefix. An unsupported operand must not hide a bad home.
        let destination = self.long_operand(&Mir65816Value::Temp(dest, ByteSize::new(4)))?;
        let left = self.long_arithmetic_operand(left)?;
        let right = self.long_arithmetic_operand(right)?;
        let (Some(LongOperand::Stack { low, high }), Some(left), Some(right)) =
            (destination, left, right)
        else {
            return Ok(false);
        };
        // The low result is stored before either high source is read. Complete
        // identity and disjoint homes are safe; partial overlaps retain fallback.
        for source in [left, right] {
            if let LongOperand::Stack { low: source, .. } = source
                && source != low
                && source.abs_diff(low) < 4
            {
                return Ok(false);
            }
        }

        self.code.barrier();
        self.code.a16();
        let subtract = operation == NirBinaryOp::Sub;
        for upper in [false, true] {
            self.edge_load(left.word(upper));
            if !upper {
                self.code
                    .op(if subtract { Implied::Sec } else { Implied::Clc });
            }
            match right.word(upper) {
                WordOperand::Immediate(value) => self.code.word(
                    if subtract {
                        WordOp::SbcImm
                    } else {
                        WordOp::AdcImm
                    },
                    value,
                ),
                WordOperand::Stack(offset) => self.code.byte(
                    if subtract {
                        ByteOp::SbcStack
                    } else {
                        ByteOp::AdcStack
                    },
                    offset,
                ),
                WordOperand::DirectPage(_) => unreachable!("long operands are stack or immediate"),
            }
            // STA and the next LDA preserve the low-word carry/borrow. The
            // final A contains only the high half, never a whole-temp identity.
            self.code
                .byte(ByteOp::StaStack, if upper { high } else { low });
        }
        Ok(true)
    }
}
