//! Emit a preallocated accumulator lifetime without creating a memory capture.
use super::*;

impl home_demand::Plan {
    pub(super) fn emit(
        &self,
        b: &mut Builder<'_>,
        block: BlockId,
        index: usize,
        op: &Mir65816Op,
    ) -> Result<bool, String> {
        if let Some((temp, _)) = self.native_producer(block, index) {
            let Mir65816Op::Call {
                target,
                args,
                result,
                plan,
                ..
            } = op
            else {
                return Err("missing native output producer".into());
            };
            if b.frame.temps.contains_key(&temp) {
                return Err("native output unexpectedly has a home".into());
            }
            let accumulator = self
                .consumer(block, index)
                .map(|_| match args.as_slice() {
                    [Mir65816Value::Temp(id, _)] => Ok(*id),
                    _ => Err("invalid native call accumulator input".to_string()),
                })
                .transpose()?;
            b.call_with_accumulator(
                target,
                args,
                *result,
                plan,
                CallResultUse::Native,
                accumulator,
            )?;
            return Ok(true);
        }
        if let Some((temp, range)) = self.native_consumer(block, index) {
            let Some(Mir65816AbiHome::NativeResult(lanes)) =
                home_demand::result_home(range.bytes.into())
            else {
                unreachable!()
            };
            if !b.code.consume_native(temp, lanes) {
                return Err("native consumer lost its output ownership".into());
            }
            if call_flow::zero_test(temp, range.bytes, op) {
                if range.bytes == 1 {
                    b.code.a8();
                    b.code.byte(ByteOp::CmpImm, 0);
                } else {
                    b.code.a16();
                    b.code.word(WordOp::CmpImm, 0);
                }
                // Prepared conditions consume only these fresh flags.
                return Ok(false);
            }
            if call_flow::private_store(b.routine, temp, range.bytes, op) {
                let Mir65816Op::Store { address, .. } = op else {
                    unreachable!()
                };
                // This unindexed object address requires no register preparation.
                // The semantic write belongs to Store, after native ownership ends.
                let memory = b.prepare_address(address)?;
                b.capture_call_result(memory, range.bytes)?;
                return Ok(true);
            }
            return Err("unsupported native output consumer".into());
        }
        if self.locals.emit(b, block, index, op)? {
            return Ok(true);
        }
        if let Some(range) = self.producer(block, index) {
            let id = liveness::operation_output(op).ok_or("missing accumulator producer")?;
            if b.frame.temps.contains_key(&id) {
                return Err("accumulator producer unexpectedly has a memory home".into());
            }
            b.code.barrier();
            if range.bytes == 3 && b.address_expression(op)? {
                return Ok(true);
            }
            match op {
                Mir65816Op::Load { address, .. } => {
                    let source = b.prepare_address(address)?;
                    b.check_transfer(source, source, range.bytes)?;
                    if range.bytes == 1 {
                        b.code.a8();
                    } else {
                        b.code.a16();
                    }
                    b.load_memory(source, 0)?;
                }
                Mir65816Op::Binary {
                    operation,
                    left,
                    right,
                    ..
                } => {
                    if range.bytes > 2 {
                        b.long_expression_return(range.bytes, *operation, left, right)?;
                        return Ok(true);
                    }
                    if range.bytes == 1 {
                        let left = if self.consumer(block, index).is_some() {
                            None
                        } else {
                            Some(
                                b.byte_operand(left)?
                                    .ok_or("invalid byte expression input")?,
                            )
                        };
                        let shift = home_demand::small_shift(*operation, right);
                        let right = if shift.is_some() {
                            None
                        } else {
                            Some(
                                b.byte_operand(right)?
                                    .ok_or("invalid byte expression operand")?,
                            )
                        };
                        b.code.a8();
                        if let Some(left) = left {
                            b.load_byte_operand(left);
                        }
                        if let Some(count) = shift {
                            for _ in 0..count {
                                b.code.op(if *operation == NirBinaryOp::Lsh {
                                    Implied::AslA
                                } else {
                                    Implied::LsrA
                                });
                            }
                        } else {
                            b.byte_expression_rhs(*operation, right.unwrap());
                        }
                        return Ok(true);
                    }
                    // A chain link consumes its predecessor's A16 and leaves
                    // its own result there. Neither value has a memory home.
                    let left = if let Some(input) = self.consumer(block, index) {
                        if input.bytes != range.bytes {
                            return Err("accumulator expression width mismatch".into());
                        }
                        None
                    } else {
                        Some(
                            b.word_operand(left)?
                                .ok_or("invalid accumulator left operand")?,
                        )
                    };
                    let shift = home_demand::small_shift(*operation, right);
                    let right = if shift.is_none() {
                        Some(
                            b.word_operand(right)?
                                .ok_or("invalid accumulator right operand")?,
                        )
                    } else {
                        None
                    };
                    b.code.a16();
                    if let Some(left) = left {
                        b.load_checked_word(left, None);
                    }
                    if let Some(count) = shift {
                        for _ in 0..count {
                            b.code.op(if *operation == NirBinaryOp::Lsh {
                                Implied::AslA
                            } else {
                                Implied::LsrA
                            });
                        }
                    } else {
                        b.word_binary_rhs(*operation, right.unwrap(), true);
                    }
                }
                Mir65816Op::Cast {
                    from, to, value, ..
                } => {
                    if from == to {
                        if self.consumer(block, index).is_none() && !b.wide_return(value)? {
                            return Err("unsupported wide expression return".into());
                        }
                    } else {
                        if self.consumer(block, index).is_none() {
                            if from.get() == 1 {
                                let source = b
                                    .byte_operand(value)?
                                    .ok_or("invalid byte widening input")?;
                                b.code.a8();
                                b.load_byte_operand(source);
                            } else {
                                let source = b
                                    .word_operand(value)?
                                    .ok_or("invalid word widening input")?;
                                b.code.a16();
                                b.load_checked_word(source, None);
                            }
                        }
                        b.code.a16();
                        if from.get() == 1 {
                            b.code.word(WordOp::AndImm, 0xff);
                        }
                        b.code.word(WordOp::LdxImm, 0);
                    }
                }
                _ => return Err("unsupported accumulator producer".into()),
            }
            return Ok(true);
        }
        let Some(range) = self.consumer(block, index) else {
            return Ok(false);
        };
        if let Mir65816Op::Call {
            target,
            args,
            result,
            plan,
            ..
        } = op
        {
            let Mir65816Value::Temp(id, _) = args[0] else {
                return Err("missing accumulator argument".into());
            };
            let source = b
                .routine
                .blocks
                .iter()
                .find(|candidate| candidate.id == block)
                .ok_or("missing call block")?;
            let result_use = if self.call_returns(b.routine, source, index) {
                CallResultUse::Native
            } else {
                CallResultUse::Capture
            };
            b.call_with_accumulator(target, args, *result, plan, result_use, Some(id))?;
            return Ok(true);
        }
        // Prepared conditions participate in the ordinary compare/branch
        // selector, which owns the Boolean capture or fused branch.
        if matches!(op, Mir65816Op::Compare { .. }) {
            return Ok(false);
        }
        if let Mir65816Op::Store { address, .. } = op {
            b.expression_store(address, range.bytes)?;
            return Ok(true);
        }
        let Mir65816Op::Cast { dest, to, .. } = op else {
            return Err("missing accumulator widening consumer".into());
        };
        let destination = b.temp(*dest)?;
        if destination.slot().width != to.get() as u8 {
            return Err("accumulator consumer has incomplete destination".into());
        }
        let Location::Stack(slot) = destination else {
            return Err("accumulator widening requires a private stack result".into());
        };
        b.displacement(slot.offset.into(), to.get() - 1)?;
        b.code.barrier();
        if range.bytes == 1 {
            // Stay in A8: write the original byte and explicit zero high
            // lanes. Hidden B is never copied into the widened result.
            b.code.a8();
            b.store_memory(destination.into(), 0)?;
            b.code.byte(ByteOp::LdaImm, 0);
            for byte in 1..to.get() {
                b.store_memory(destination.into(), byte)?;
            }
            return Ok(true);
        }
        b.code.a16();
        b.store_memory(destination.into(), 0)?;
        if to.get() > 2 {
            if to.get() == 3 {
                b.code.a8();
                b.code.byte(ByteOp::LdaImm, 0);
            } else {
                b.code.word(WordOp::LdaImm, 0);
            }
            b.store_memory(destination.into(), 2)?;
        }
        Ok(true)
    }

    pub(super) fn prepare_return(&self, b: &mut Builder<'_>, block: &Mir65816Block) -> bool {
        let Some(range) = self.consumer(block.id, block.ops.len()) else {
            return false;
        };
        b.code.a16();
        if range.bytes == 1 {
            b.code.word(WordOp::AndImm, 0xff);
        }
        true
    }

    pub(super) fn comparisons(
        &self,
        b: &Builder<'_>,
        block: &Mir65816Block,
    ) -> Result<BTreeMap<usize, Condition>, String> {
        let mut conditions = BTreeMap::new();
        for (index, op) in block.ops.iter().enumerate() {
            let native = self.native_consumer(block.id, index).is_some();
            let Some(input) = self
                .native_consumer(block.id, index)
                .map(|(_, r)| r)
                .or_else(|| self.consumer(block.id, index))
            else {
                continue;
            };
            let Mir65816Op::Compare {
                dest,
                operation,
                right,
                ..
            } = op
            else {
                continue;
            };
            let Location::Stack(home) = b.temp(*dest)? else {
                return Err("comparison result needs a private byte home".into());
            };
            if home.width != 1 {
                return Err("invalid comparison result width".into());
            }
            let destination = b.displacement(home.offset.into(), 0)?;
            let predicate = match operation {
                NirCompareOp::Eq => Branch::Equal,
                NirCompareOp::Ne => Branch::NotEqual,
                NirCompareOp::Lt => Branch::CarryClear,
                NirCompareOp::Ge => Branch::CarrySet,
                _ => return Err("unsupported accumulator comparison".into()),
            };
            let condition = if input.bytes == 1 {
                Condition::Byte(ByteCondition {
                    left: ByteOperand::Immediate(0),
                    left_in_a: true,
                    right: if native {
                        ByteOperand::Immediate(0)
                    } else {
                        b.byte_operand(right)?
                            .ok_or("invalid byte comparison operand")?
                    },
                    destination,
                    predicate,
                })
            } else {
                Condition::Word(WordCondition {
                    kind: WordComparison::UnsignedOrEquality,
                    left: WordOperand::Immediate(0),
                    left_temp: None,
                    left_in_a: true,
                    right: if native {
                        WordOperand::Immediate(0)
                    } else {
                        b.word_operand(right)?
                            .ok_or("invalid word comparison operand")?
                    },
                    destination,
                    predicate,
                })
            };
            conditions.insert(index, condition);
        }
        Ok(conditions)
    }
}

impl Builder<'_> {
    pub(super) fn byte_expression_rhs(&mut self, operation: NirBinaryOp, right: ByteOperand) {
        let (immediate, stack) = match operation {
            NirBinaryOp::Add => {
                self.code.op(Implied::Clc);
                (ByteOp::AdcImm, ByteOp::AdcStack)
            }
            NirBinaryOp::Sub => {
                self.code.op(Implied::Sec);
                (ByteOp::SbcImm, ByteOp::SbcStack)
            }
            NirBinaryOp::And => (ByteOp::AndImm, ByteOp::AndStack),
            NirBinaryOp::Or => (ByteOp::OraImm, ByteOp::OraStack),
            NirBinaryOp::Xor => (ByteOp::EorImm, ByteOp::EorStack),
            _ => unreachable!("checked byte expression"),
        };
        match right {
            ByteOperand::Immediate(n) => self.code.byte(immediate, n),
            ByteOperand::Stack(offset) => self.code.byte(stack, offset),
        }
    }
}
