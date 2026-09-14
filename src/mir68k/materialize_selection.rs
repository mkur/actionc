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
}
