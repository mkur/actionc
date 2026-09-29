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
        if let Some(range) = self.producer(block, index) {
            let id = liveness::operation_output(op).ok_or("missing accumulator producer")?;
            if b.frame.temps.contains_key(&id) {
                return Err("accumulator producer unexpectedly has a memory home".into());
            }
            b.code.barrier();
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
                    let left = b
                        .word_operand(left)?
                        .ok_or("invalid accumulator left operand")?;
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
                    b.load_checked_word(left, None);
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
                _ => return Err("unsupported accumulator producer".into()),
            }
            return Ok(true);
        }
        let Some(range) = self.consumer(block, index) else {
            return Ok(false);
        };
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
}
