//! Deliberately small test-only executor for emitted compiler regression fixtures.
//! Unsupported instructions fail the test; this is not a production emulator.
use crate::codegen::{AddressingMode, decode_6502_opcode};

/// Execute through the outermost RTS, preserving memory for guard assertions.
pub(crate) fn run_memory(memory: &mut [u8; 65536], entry: usize) -> Vec<u8> {
    run_memory_with_limit(memory, entry, 100_000)
}

pub(crate) fn run_memory_with_limit(
    memory: &mut [u8; 65536],
    entry: usize,
    step_limit: usize,
) -> Vec<u8> {
    let (mut a, mut x, mut y, mut sp) = (0u8, 0u8, 0u8, 0xffu8);
    let (mut c, mut z, mut n) = (false, false, false);
    let mut pc = entry;
    let mut depth = 0;
    let mut returns = Vec::new();
    for _ in 0..step_limit {
        let (name, mode, len) = decode_6502_opcode(memory[pc]).unwrap();
        let operand = memory[pc + 1];
        let word = u16::from_le_bytes([operand, memory[pc + 2]]);
        let addr = match mode {
            AddressingMode::Immediate => pc + 1,
            AddressingMode::ZeroPage => usize::from(operand),
            AddressingMode::ZeroPageX => usize::from(operand.wrapping_add(x)),
            AddressingMode::ZeroPageY => usize::from(operand.wrapping_add(y)),
            AddressingMode::Absolute => usize::from(word),
            AddressingMode::AbsoluteX => usize::from(word.wrapping_add(u16::from(x))),
            AddressingMode::AbsoluteY => usize::from(word.wrapping_add(u16::from(y))),
            AddressingMode::IndirectIndexedY => {
                let pointer = u16::from_le_bytes([
                    memory[usize::from(operand)],
                    memory[usize::from(operand.wrapping_add(1))],
                ]);
                usize::from(pointer.wrapping_add(u16::from(y)))
            }
            _ => 0,
        };
        pc += len;
        let value = if mode == AddressingMode::Accumulator {
            a
        } else {
            memory[addr]
        };
        let mut zn = None;
        match name {
            "LDA" => {
                a = value;
                zn = Some(a);
            }
            "LDX" => {
                x = value;
                zn = Some(x);
            }
            "LDY" => {
                y = value;
                zn = Some(y);
            }
            "STA" | "STX" | "STY" => {
                let v = match name {
                    "STA" => a,
                    "STX" => x,
                    _ => y,
                };
                memory[addr] = v;
                if addr == 0xa0 {
                    returns.push(v);
                }
            }
            "TAX" => {
                x = a;
                zn = Some(x);
            }
            "TAY" => {
                y = a;
                zn = Some(y);
            }
            "TXA" => {
                a = x;
                zn = Some(a);
            }
            "TYA" => {
                a = y;
                zn = Some(a);
            }
            "AND" => {
                a &= value;
                zn = Some(a);
            }
            "ORA" => {
                a |= value;
                zn = Some(a);
            }
            "EOR" => {
                a ^= value;
                zn = Some(a);
            }
            "CLC" => c = false,
            "SEC" => c = true,
            "ADC" | "SBC" => {
                let rhs = if name == "SBC" { !value } else { value };
                let sum = u16::from(a) + u16::from(rhs) + u16::from(c);
                c = sum > 255;
                a = sum as u8;
                zn = Some(a);
            }
            "CMP" | "CPX" | "CPY" => {
                let lhs = match name {
                    "CMP" => a,
                    "CPX" => x,
                    _ => y,
                };
                c = lhs >= value;
                zn = Some(lhs.wrapping_sub(value));
            }
            "ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC" => {
                let result = match name {
                    "ASL" | "ROL" => {
                        let old_c = c;
                        c = value & 0x80 != 0;
                        (value << 1) | u8::from(name == "ROL" && old_c)
                    }
                    "LSR" | "ROR" => {
                        let old_c = c;
                        c = value & 1 != 0;
                        (value >> 1) | if name == "ROR" && old_c { 0x80 } else { 0 }
                    }
                    "INC" => value.wrapping_add(1),
                    _ => value.wrapping_sub(1),
                };
                if mode == AddressingMode::Accumulator {
                    a = result;
                } else {
                    memory[addr] = result;
                }
                zn = Some(result);
            }
            "INX" => {
                x = x.wrapping_add(1);
                zn = Some(x);
            }
            "INY" => {
                y = y.wrapping_add(1);
                zn = Some(y);
            }
            "DEX" => {
                x = x.wrapping_sub(1);
                zn = Some(x);
            }
            "DEY" => {
                y = y.wrapping_sub(1);
                zn = Some(y);
            }
            "BCC" | "BCS" | "BEQ" | "BNE" | "BMI" | "BPL" => {
                let take = match name {
                    "BCC" => !c,
                    "BCS" => c,
                    "BEQ" => z,
                    "BNE" => !z,
                    "BMI" => n,
                    _ => !n,
                };
                if take {
                    pc = pc.checked_add_signed(isize::from(operand as i8)).unwrap();
                }
            }
            "JMP" if mode == AddressingMode::Absolute => pc = addr,
            "JSR" => {
                let ret = (pc - 1) as u16;
                memory[0x100 + usize::from(sp)] = (ret >> 8) as u8;
                sp = sp.wrapping_sub(1);
                memory[0x100 + usize::from(sp)] = ret as u8;
                sp = sp.wrapping_sub(1);
                depth += 1;
                pc = addr;
            }
            "RTS" => {
                if depth == 0 {
                    return returns;
                }
                sp = sp.wrapping_add(1);
                let lo = memory[0x100 + usize::from(sp)];
                sp = sp.wrapping_add(1);
                let hi = memory[0x100 + usize::from(sp)];
                pc = usize::from(u16::from_le_bytes([lo, hi])) + 1;
                depth -= 1;
            }
            "PHA" => {
                memory[0x100 + usize::from(sp)] = a;
                sp = sp.wrapping_sub(1);
            }
            "PHP" => {
                memory[0x100 + usize::from(sp)] =
                    0x30 | u8::from(c) | (u8::from(z) << 1) | (u8::from(n) << 7);
                sp = sp.wrapping_sub(1);
            }
            "PLP" => {
                sp = sp.wrapping_add(1);
                let status = memory[0x100 + usize::from(sp)];
                c = status & 1 != 0;
                z = status & 2 != 0;
                n = status & 0x80 != 0;
            }
            "PLA" => {
                sp = sp.wrapping_add(1);
                a = memory[0x100 + usize::from(sp)];
                zn = Some(a);
            }
            "NOP" => {}
            _ => panic!(
                "unsupported test CPU instruction {name} {mode:?} at {:04x}",
                pc - len
            ),
        }
        if let Some(value) = zn {
            z = value == 0;
            n = value & 0x80 != 0;
        }
    }
    panic!("test CPU did not terminate");
}
