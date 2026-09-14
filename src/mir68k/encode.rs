//! Checked original-MC68000 encoding; addresses are supplied by the linker.
use super::machine::*;

pub fn encode(
    instruction: &Instruction,
    resolve: &impl Fn(Address) -> Result<u32, String>,
) -> Result<Vec<u8>, String> {
    let mut words = Vec::new();
    match *instruction {
        Instruction::Trap(vector) => {
            if vector > 15 {
                return Err("TRAP vector must be 0..15".into());
            }
            words.push(0x4e40 | u16::from(vector));
        }
        Instruction::AddExtend {
            width,
            source,
            destination,
        } => {
            check_register(source)?;
            check_register(destination)?;
            words.push(0xd100 | (destination as u16) << 9 | size_bits(width) << 6 | source as u16);
        }
        Instruction::AddQuick {
            width,
            delta,
            register,
        } => {
            check_register(register)?;
            if !matches!(delta, -8..=-1 | 1..=8) {
                return Err("ADDQ/SUBQ magnitude must be 1..8".into());
            }
            words.push(
                0x5000
                    | ((delta.unsigned_abs() as u16 & 7) << 9)
                    | if delta < 0 { 0x100 } else { 0 }
                    | size_bits(width) << 6
                    | register as u16,
            );
        }
        Instruction::MultiplyUnsignedWord {
            source,
            destination,
        } => {
            check_register(source)?;
            check_register(destination)?;
            words.push(0xc0c0 | (destination as u16) << 9 | source as u16);
        }
        Instruction::AddAddress {
            source,
            destination,
        } => {
            check_register(destination)?;
            let (ea, ext) = effective_address(source, Width::Long, resolve)?;
            words.push(0xd1c0 | (destination as u16) << 9 | ea);
            words.extend(ext);
        }
        Instruction::Alu {
            operation,
            width,
            source,
            destination,
        } => {
            check_register(source)?;
            check_register(destination)?;
            let (opcode, reg, ea) = match operation {
                Alu::Add => (0xd000, destination, source),
                Alu::Sub => (0x9000, destination, source),
                Alu::And => (0xc000, destination, source),
                Alu::Or => (0x8000, destination, source),
                Alu::Xor => (0xb100, source, destination),
                Alu::Compare => (0xb000, destination, source),
            };
            words.push(opcode | (reg as u16) << 9 | size_bits(width) << 6 | ea as u16);
        }
        Instruction::CompareImmediate {
            width,
            value,
            destination,
        } => {
            check_register(destination)?;
            words.push(0x0c00 | size_bits(width) << 6 | destination as u16);
            if width == Width::Long {
                long(&mut words, value);
            } else {
                words.push(value as u16 & if width == Width::Byte { 0xff } else { 0xffff });
            }
        }
        Instruction::Negate { width, register } => {
            check_register(register)?;
            words.push(0x4400 | size_bits(width) << 6 | register as u16);
        }
        Instruction::Extend { to, register } => {
            check_register(register)?;
            let op = match to {
                Width::Word => 0x4880,
                Width::Long => 0x48c0,
                Width::Byte => return Err("EXT requires word or long destination".into()),
            };
            words.push(op | register as u16);
        }
        Instruction::SetCondition {
            condition,
            register,
        } => {
            check_register(register)?;
            words.push(0x50c0 | (condition as u16) << 8 | register as u16);
        }
        Instruction::LogicalShift {
            width,
            left,
            count,
            register,
        } => {
            check_register(register)?;
            let (count, dynamic) = match count {
                ShiftCount::Register(r) => {
                    check_register(r)?;
                    (r, 0x20)
                }
                ShiftCount::Immediate(n) if (1..=8).contains(&n) => (n & 7, 0),
                _ => return Err("immediate logical shift count must be 1..8".into()),
            };
            words.push(
                0xe008
                    | (count as u16) << 9
                    | if left { 0x100 } else { 0 }
                    | size_bits(width) << 6
                    | dynamic
                    | register as u16,
            );
        }
        Instruction::Move {
            width,
            source,
            destination,
        } => {
            if matches!(destination, Ea::Immediate(_) | Ea::ImmediateAddress(_)) {
                return Err("MOVE destination is not alterable".into());
            }
            if width == Width::Byte
                && (matches!(source, Ea::A(_)) || matches!(destination, Ea::A(_)))
            {
                return Err("MC68000 has no byte address-register MOVE".into());
            }
            let (src, src_ext) = effective_address(source, width, resolve)?;
            let (dst, dst_ext) = effective_address(destination, width, resolve)?;
            let op = match width {
                Width::Byte => 0x1000,
                Width::Word => 0x3000,
                Width::Long => 0x2000,
            };
            words.push(op | ((dst & 7) << 9) | ((dst >> 3) << 6) | src);
            words.extend(src_ext);
            words.extend(dst_ext);
        }
        Instruction::Lea {
            source,
            destination,
        } => {
            check_register(destination)?;
            control_address(source)?;
            let (ea, ext) = effective_address(source, Width::Long, resolve)?;
            words.push(0x41c0 | ((destination as u16) << 9) | ea);
            words.extend(ext);
        }
        Instruction::Jump(ea) | Instruction::Jsr(ea) => {
            control_address(ea)?;
            let (bits, ext) = effective_address(ea, Width::Long, resolve)?;
            words.push(
                if matches!(instruction, Instruction::Jsr(_)) {
                    0x4e80
                } else {
                    0x4ec0
                } | bits,
            );
            words.extend(ext);
        }
        Instruction::Branch { condition, target } => {
            words.push(0x6006 | condition.inverse_bits() << 8);
            words.push(0x4ef9);
            long(&mut words, resolve(target)?);
        }
        Instruction::Link {
            register,
            displacement,
        } => {
            check_register(register)?;
            words.extend([0x4e50 | register as u16, displacement as u16]);
        }
        Instruction::Unlink(register) => {
            check_register(register)?;
            words.push(0x4e58 | register as u16);
        }
        Instruction::Rts => words.push(0x4e75),
    }
    Ok(words.into_iter().flat_map(u16::to_be_bytes).collect())
}

pub fn size(instruction: &Instruction) -> Result<u32, String> {
    Ok(encode(instruction, &|_| Ok(0))?.len() as u32)
}

fn check_register(register: u8) -> Result<(), String> {
    if register < 8 {
        Ok(())
    } else {
        Err(format!("invalid MC68000 register {register}"))
    }
}
fn control_address(ea: Ea) -> Result<(), String> {
    if matches!(ea, Ea::Indirect(_) | Ea::Displacement(..) | Ea::Absolute(_)) {
        Ok(())
    } else {
        Err("instruction requires a control addressing mode".into())
    }
}
fn long(words: &mut Vec<u16>, value: u32) {
    words.extend([(value >> 16) as u16, value as u16]);
}

fn effective_address(
    ea: Ea,
    width: Width,
    resolve: &impl Fn(Address) -> Result<u32, String>,
) -> Result<(u16, Vec<u16>), String> {
    let mut ext = Vec::new();
    let bits = match ea {
        Ea::D(r) | Ea::A(r) | Ea::Indirect(r) | Ea::Displacement(r, _) => {
            check_register(r)?;
            let mode = match ea {
                Ea::D(_) => 0,
                Ea::A(_) => 1,
                Ea::Indirect(_) => 2,
                Ea::Displacement(_, d) => {
                    ext.push(d as u16);
                    5
                }
                _ => unreachable!(),
            };
            (mode << 3) | r as u16
        }
        Ea::Absolute(a) => {
            long(&mut ext, resolve(a)?);
            0x39
        }
        Ea::Immediate(value) => {
            match width {
                Width::Byte => ext.push((value & 0xff) as u16),
                Width::Word => ext.push(value as u16),
                Width::Long => long(&mut ext, value),
            }
            0x3c
        }
        Ea::ImmediateAddress(a) => {
            if width != Width::Long {
                return Err("native address immediate must be four bytes".into());
            }
            long(&mut ext, resolve(a)?);
            0x3c
        }
    };
    Ok((bits, ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(i: Instruction) -> Vec<u8> {
        encode(&i, &|a| match a.target {
            Target::Absolute(v) => Ok(v),
            _ => Err("unresolved".into()),
        })
        .unwrap()
    }
    #[test]
    fn encodings_match_independent_harness_programs() {
        for (width, value, expected) in [
            (Width::Byte, 0x81, vec![0x13fc, 0x0081, 0x0002, 0]),
            (Width::Word, 0x8765, vec![0x33fc, 0x8765, 0x0002, 0]),
            (
                Width::Long,
                0x123489ab,
                vec![0x23fc, 0x1234, 0x89ab, 0x0002, 0],
            ),
        ] {
            assert_eq!(
                bytes(Instruction::Move {
                    width,
                    source: Ea::Immediate(value),
                    destination: Ea::Absolute(Address::absolute(0x20000))
                }),
                expected
                    .into_iter()
                    .flat_map(u16::to_be_bytes)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(
            bytes(Instruction::Link {
                register: 6,
                displacement: -8
            }),
            [0x4e, 0x56, 0xff, 0xf8]
        );
        assert_eq!(bytes(Instruction::Unlink(6)), [0x4e, 0x5e]);
        assert_eq!(bytes(Instruction::Rts), [0x4e, 0x75]);
        assert_eq!(bytes(Instruction::Trap(14)), [0x4e, 0x4e]);
        assert_eq!(
            bytes(Instruction::AddExtend {
                width: Width::Long,
                source: 1,
                destination: 1
            }),
            [0xd3, 0x81]
        );
        assert_eq!(
            bytes(Instruction::AddQuick {
                width: Width::Long,
                delta: 1,
                register: 0
            }),
            [0x52, 0x80]
        );
        assert_eq!(
            bytes(Instruction::AddQuick {
                width: Width::Long,
                delta: -1,
                register: 3
            }),
            [0x53, 0x83]
        );
        // M68000 PRM, MULU.W instruction format (4-139).
        assert_eq!(
            bytes(Instruction::MultiplyUnsignedWord {
                source: 1,
                destination: 0
            }),
            [0xc0, 0xc1]
        );
        assert_eq!(
            bytes(Instruction::Jsr(Ea::Absolute(Address::absolute(0x1001a)))),
            [0x4e, 0xb9, 0, 1, 0, 0x1a]
        );
        assert_eq!(
            bytes(Instruction::Move {
                width: Width::Long,
                source: Ea::Displacement(6, -4),
                destination: Ea::D(0)
            }),
            [0x20, 0x2e, 0xff, 0xfc]
        );
        assert_eq!(
            bytes(Instruction::Branch {
                condition: Condition::Equal,
                target: Address::absolute(0x10000)
            }),
            [0x66, 6, 0x4e, 0xf9, 0, 1, 0, 0]
        );
    }
    #[test]
    fn illegal_operands_are_diagnostics() {
        for instruction in [
            Instruction::Move {
                width: Width::Byte,
                source: Ea::D(0),
                destination: Ea::A(0),
            },
            Instruction::Move {
                width: Width::Long,
                source: Ea::D(0),
                destination: Ea::Immediate(1),
            },
            Instruction::Move {
                width: Width::Word,
                source: Ea::ImmediateAddress(Address::absolute(0)),
                destination: Ea::D(0),
            },
            Instruction::Jsr(Ea::D(0)),
            Instruction::Unlink(8),
            Instruction::Trap(16),
            Instruction::AddExtend {
                width: Width::Long,
                source: 8,
                destination: 0,
            },
            Instruction::AddQuick {
                width: Width::Long,
                delta: 0,
                register: 0,
            },
            Instruction::AddQuick {
                width: Width::Long,
                delta: 9,
                register: 0,
            },
            Instruction::MultiplyUnsignedWord {
                source: 8,
                destination: 0,
            },
        ] {
            assert!(size(&instruction).is_err());
        }
    }
}

fn size_bits(width: Width) -> u16 {
    match width {
        Width::Byte => 0,
        Width::Word => 1,
        Width::Long => 2,
    }
}
