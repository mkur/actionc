//! Original-MC68000 choices using resolved MIR widths and captured constants.
use super::*;

pub(super) fn constant(value: &Mir68kValue) -> Option<u32> {
    match value {
        Mir68kValue::U8(v) => Some(u32::from(*v)),
        Mir68kValue::U16(v) => Some(u32::from(*v)),
        Mir68kValue::U32(v) => Some(*v),
        _ => None,
    }
}
impl Builder<'_> {
    pub(super) fn add_displacement(&mut self, register: u8, value: u32) {
        if value == 0 {
            return;
        }
        if let Ok(offset) = i16::try_from(value as i32) {
            self.emit(Instruction::Lea {
                source: Ea::Displacement(register, offset),
                destination: register,
            });
        } else {
            self.emit(Instruction::AddAddress {
                source: Ea::Immediate(value),
                destination: register,
            });
        }
    }

    pub(super) fn constant_binary(
        &mut self,
        width: Width,
        operation: NirBinaryOp,
        right: &Mir68kValue,
    ) -> bool {
        let Some(value) = constant(right) else {
            return false;
        };
        if operation == NirBinaryOp::Mul {
            return self.constant_multiply(width, value);
        }
        if matches!(operation, NirBinaryOp::Lsh | NirBinaryOp::Rsh) {
            if value >= width.bytes() * 8 {
                self.mov(Width::Long, Ea::Immediate(0), Ea::D(0));
            } else {
                let mut count = value;
                while count != 0 {
                    let step = count.min(8);
                    self.emit(Instruction::LogicalShift {
                        width,
                        left: operation == NirBinaryOp::Lsh,
                        count: ShiftCount::Immediate(step as u8),
                        register: 0,
                    });
                    count -= step;
                }
            }
            return true;
        }
        let operation = match operation {
            NirBinaryOp::Add => Alu::Add,
            NirBinaryOp::Sub => Alu::Sub,
            NirBinaryOp::And => Alu::And,
            NirBinaryOp::Or => Alu::Or,
            NirBinaryOp::Xor => Alu::Xor,
            _ => return false,
        };
        if matches!(operation, Alu::Add | Alu::Sub) && (1..=8).contains(&value) {
            self.emit(Instruction::AddQuick {
                width,
                delta: if operation == Alu::Add {
                    value as i8
                } else {
                    -(value as i8)
                },
                register: 0,
            });
        } else {
            self.emit(Instruction::AluImmediate {
                operation,
                width,
                value,
                destination: 0,
            });
        }
        true
    }

    fn constant_multiply(&mut self, width: Width, value: u32) -> bool {
        let bits = width.bytes() * 8;
        let mask = u32::MAX >> (32 - bits);
        let value = value & mask;
        if value == 0 {
            self.mov(width, Ea::Immediate(0), Ea::D(0));
            return true;
        }
        // Search small shift/add identities, including negative factors, in
        // the resolved modular width. Dense constants retain general MUL.
        // Budget against even a fast MULU.W; the long fallback uses three MULs.
        let shift_cost = |n: u32| if n == 0 { 0 } else { 6 * n.div_ceil(8) + 2 * n };
        let alu_cost = if width == Width::Long { 8 } else { 4 };
        let mut best: Option<(u32, u32, u32, bool, bool)> = None;
        for (factor, negate) in [(value, false), (value.wrapping_neg() & mask, true)] {
            let low = factor.trailing_zeros();
            let odd = factor >> low;
            // factor = (2^high +/- 1) * 2^low; high=0 is a pure shift.
            let shape = if odd == 1 {
                Some((0, false))
            } else if (odd - 1).is_power_of_two() {
                Some(((odd - 1).trailing_zeros(), false))
            } else if let Some(next) = odd.checked_add(1).filter(|n| n.is_power_of_two()) {
                Some((next.trailing_zeros(), true))
            } else {
                None
            };
            let Some((high, subtract)) = shape else {
                continue;
            };
            let cost = shift_cost(low)
                + shift_cost(high)
                + if high != 0 { 4 + alu_cost } else { 0 }
                + if negate { alu_cost } else { 0 };
            let limit = if width == Width::Long { 100 } else { 34 };
            if cost < limit && best.is_none_or(|(old, ..)| cost < old) {
                best = Some((cost, low, high, subtract, negate));
            }
        }
        let Some((_, low, high, subtract, negate)) = best else {
            return false;
        };
        if high != 0 {
            self.mov(width, Ea::D(0), Ea::D(1));
        }
        for (stage, count) in [high, low].into_iter().enumerate() {
            let mut remaining = count;
            while remaining != 0 {
                let step = remaining.min(8);
                self.emit(Instruction::LogicalShift {
                    width,
                    left: true,
                    count: ShiftCount::Immediate(step as u8),
                    register: 0,
                });
                remaining -= step;
            }
            if stage == 0 && high != 0 {
                self.emit(Instruction::Alu {
                    operation: if subtract { Alu::Sub } else { Alu::Add },
                    width,
                    source: 1,
                    destination: 0,
                });
            }
        }
        if negate {
            self.emit(Instruction::Negate { width, register: 0 });
        }
        true
    }
}
