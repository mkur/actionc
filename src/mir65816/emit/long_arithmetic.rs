//! Word arithmetic with an exact byte/word tail; no external accesses.
use super::*;

#[cfg(test)]
#[path = "long_arithmetic_tests.rs"]
mod tests;

impl Builder<'_> {
    pub(super) fn long_expression_return(
        &mut self,
        bytes: u8,
        operation: NirBinaryOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<(), String> {
        let left = self
            .long_arithmetic_operand(left, bytes)?
            .ok_or("invalid wide expression operand")?;
        let right = self
            .long_arithmetic_operand(right, bytes)?
            .ok_or("invalid wide expression operand")?;
        self.code.a16();
        self.edge_load(left.word(false));
        self.word_binary_rhs(operation, right.word(false), true);
        self.code.op(Implied::Tay); // Preserve low word without consuming carry.
        if bytes == 3 {
            self.code.a8();
            self.load_byte_operand(left.high_byte());
            self.arithmetic_high_byte(operation, right.high_byte());
            self.code.a16();
            self.code.word(WordOp::AndImm, 0xff);
        } else {
            self.edge_load(left.word(true));
            self.word_binary_rhs(operation, right.word(true), false);
        }
        self.code.op(Implied::Tax);
        self.code.op(Implied::Tya);
        Ok(())
    }
    fn long_arithmetic_operand(
        &self,
        value: &Mir65816Value,
        bytes: u8,
    ) -> Result<Option<LongOperand>, String> {
        // Numeric lanes zero-extend just as value_byte does. Signed widening
        // remains an explicit Cast; narrow captured temps are not widened here.
        match value {
            Mir65816Value::U8(value) => Ok(Some(LongOperand::Immediate(u32::from(*value)))),
            Mir65816Value::U16(value) => Ok(Some(LongOperand::Immediate(u32::from(*value)))),
            Mir65816Value::U24(value) => Ok(Some(LongOperand::Immediate(*value))),
            Mir65816Value::U32(value) => Ok(Some(LongOperand::Immediate(*value))),
            _ if bytes == 3 => Ok(self.pointer_operand(value)?.map(|value| match value {
                PointerOperand::Immediate(value) => LongOperand::Immediate(value),
                PointerOperand::Stack { low, bank } => LongOperand::Stack { low, high: bank },
            })),
            _ => self.long_operand(value),
        }
    }

    fn arithmetic_high_byte(&mut self, operation: NirBinaryOp, right: ByteOperand) {
        let (immediate, stack) = match operation {
            NirBinaryOp::Add => (ByteOp::AdcImm, ByteOp::AdcStack),
            NirBinaryOp::Sub => (ByteOp::SbcImm, ByteOp::SbcStack),
            NirBinaryOp::And => (ByteOp::AndImm, ByteOp::AndStack),
            NirBinaryOp::Or => (ByteOp::OraImm, ByteOp::OraStack),
            NirBinaryOp::Xor => (ByteOp::EorImm, ByteOp::EorStack),
            _ => unreachable!("checked native binary operation"),
        };
        // Carry/borrow belongs to the low-word operation; never initialize it here.
        match right {
            ByteOperand::Immediate(value) => self.code.byte(immediate, value),
            ByteOperand::Stack(offset) => self.code.byte(stack, offset),
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
        if !matches!(bytes, 3 | 4)
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
        // Validate every complete extent, including transient S movement, before
        // emitting any prefix. An unsupported operand must not hide a bad home.
        let destination = self.long_arithmetic_operand(
            &Mir65816Value::Temp(dest, ByteSize::new(bytes.into())),
            bytes,
        )?;
        let left = self.long_arithmetic_operand(left, bytes)?;
        let right = self.long_arithmetic_operand(right, bytes)?;
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
                && source.abs_diff(low) < bytes
            {
                return Ok(false);
            }
        }

        self.code.barrier();
        self.code.a16();
        for upper in [false, true] {
            if upper && bytes == 3 {
                // Exactly one high byte: never read/write a fourth byte or
                // let the hidden accumulator byte enter the result.
                self.code.a8();
                self.load_byte_operand(left.high_byte());
                self.arithmetic_high_byte(operation, right.high_byte());
            } else {
                self.edge_load(left.word(upper));
                self.word_binary_rhs(operation, right.word(upper), !upper);
            }
            // STA, SEP and the next LDA preserve low-word carry/borrow. A
            // contains only the high part, never a whole-temp identity.
            self.code
                .byte(ByteOp::StaStack, if upper { high } else { low });
        }
        Ok(true)
    }
}

impl LongOperand {
    fn high_byte(self) -> ByteOperand {
        match self {
            Self::Immediate(value) => ByteOperand::Immediate((value >> 16) as u8),
            Self::Stack { high, .. } => ByteOperand::Stack(high),
        }
    }
}
