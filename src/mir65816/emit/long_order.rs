//! Ordering over complete captured 32-bit values; external captures stay intact.
use super::*;

impl Builder<'_> {
    fn long_comparison_operands(
        &self,
        dest: TempId,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<Option<(u8, LongOperand, LongOperand)>, String> {
        let home = self.temp(dest)?;
        if home.slot().width != 1 {
            return Err("comparison result temporary width mismatch".into());
        }
        let destination = match home {
            Location::Stack(slot) => Some(self.displacement(slot.offset.into(), 0)?),
            Location::DirectPage(_) => None,
        };
        // Preflight both complete inputs, even if one has an unsupported home.
        let left = self.long_operand(left)?;
        let right = self.long_operand(right)?;
        Ok(match (destination, left, right) {
            (Some(dest), Some(left), Some(right)) => Some((dest, left, right)),
            _ => None,
        })
    }

    pub(super) fn long_sign_condition(
        &self,
        dest: TempId,
        operation: NirCompareOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<Option<LongSignCondition>, String> {
        let (use_left, negative) = match (operation, left, right) {
            (NirCompareOp::Lt, _, Mir65816Value::U32(0)) => (true, true),
            (NirCompareOp::Gt, Mir65816Value::U32(0), _) => (false, true),
            (NirCompareOp::Ge, _, Mir65816Value::U32(0)) => (true, false),
            (NirCompareOp::Le, Mir65816Value::U32(0), _) => (false, false),
            _ => return Ok(None),
        };
        let Some((destination, a, b)) = self.long_comparison_operands(dest, left, right)? else {
            return Ok(None);
        };
        let source = match if use_left { a } else { b } {
            LongOperand::Immediate(value) => ByteOperand::Immediate((value >> 24) as u8),
            LongOperand::Stack { high, .. } => ByteOperand::Stack(high + 1),
        };
        Ok(Some(LongSignCondition {
            source,
            destination,
            negative,
        }))
    }

    pub(super) fn branch_on_long_sign(
        &mut self,
        condition: &LongSignCondition,
        yes: Label,
        dispatch: bool,
    ) {
        self.code.barrier();
        self.code.a8();
        self.load_byte_operand(condition.source);
        let predicate = if condition.negative {
            Branch::Minus
        } else {
            Branch::Plus
        };
        if dispatch {
            self.code.a16(); // REP preserves the sign of the captured top byte.
            self.code.dispatch(predicate, yes);
        } else {
            self.code.branch(predicate, yes);
        }
    }

    pub(super) fn materialize_long_sign(&mut self, condition: &LongSignCondition) {
        self.code.barrier();
        self.code.a8();
        self.load_byte_operand(condition.source);
        self.code.byte(ByteOp::CmpImm, 0x80);
        self.code.byte(ByteOp::LdaImm, 0); // Preserve sign-in-carry; hidden B is irrelevant.
        self.code.byte(ByteOp::AdcImm, 0);
        if !condition.negative {
            self.code.byte(ByteOp::EorImm, 1);
        }
        self.code.byte(ByteOp::StaStack, condition.destination);
    }
}
