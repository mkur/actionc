//! Ordering over complete captured 32-bit values; external captures stay intact.
use super::*;

impl Builder<'_> {
    pub(super) fn long_order_condition(
        &self,
        dest: TempId,
        signed: bool,
        operation: NirCompareOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<Option<LongOrderCondition>, String> {
        let Some((destination, mut left, mut right)) =
            self.long_comparison_operands(dest, left, right)?
        else {
            return Ok(None);
        };
        let less = match operation {
            NirCompareOp::Lt => true,
            NirCompareOp::Ge => false,
            NirCompareOp::Gt | NirCompareOp::Le => {
                std::mem::swap(&mut left, &mut right);
                operation == NirCompareOp::Gt
            }
            _ => return Ok(None),
        };
        let predicate = match (signed, less) {
            (false, true) => Branch::CarryClear,
            (false, false) => Branch::CarrySet,
            (true, true) => Branch::Minus,
            (true, false) => Branch::Plus,
        };
        Ok(Some(LongOrderCondition {
            left,
            right,
            destination,
            predicate,
            signed,
        }))
    }

    pub(super) fn branch_on_long_order(
        &mut self,
        condition: &LongOrderCondition,
        yes: Label,
        dispatch: bool,
    ) {
        self.code.barrier();
        self.code.a16();
        let decide = self.code.label();
        if condition.signed {
            // A full low-to-high subtraction carries the low-word borrow into
            // the high result. Its N xor V is the signed 32-bit comparison.
            for high in [false, true] {
                self.edge_load(condition.left.word(high));
                if !high {
                    self.code.op(Implied::Sec);
                }
                match condition.right.word(high) {
                    WordOperand::Immediate(value) => self.code.word(WordOp::SbcImm, value),
                    WordOperand::Stack(offset) => self.code.byte(ByteOp::SbcStack, offset),
                    WordOperand::DirectPage(_) => {
                        unreachable!("long operands are stack or immediate")
                    }
                }
            }
            self.code.branch(Branch::OverflowClear, decide);
            self.code.word(WordOp::EorImm, 0x8000);
        } else {
            for high in [true, false] {
                self.edge_load(condition.left.word(high));
                match condition.right.word(high) {
                    WordOperand::Immediate(value) => self.code.word(WordOp::CmpImm, value),
                    WordOperand::Stack(offset) => self.code.byte(ByteOp::CmpStack, offset),
                    WordOperand::DirectPage(_) => {
                        unreachable!("long operands are stack or immediate")
                    }
                }
                if high {
                    // Unequal high words decide ordering; equal highs require the
                    // unsigned low-word comparison. Both paths produce the same C meaning.
                    self.code.branch(Branch::NotEqual, decide);
                }
            }
        }
        self.code.mark(decide);
        if dispatch {
            self.code.dispatch(condition.predicate, yes);
        } else {
            self.code.branch(condition.predicate, yes);
        }
    }

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
