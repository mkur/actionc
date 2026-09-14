//! Block-local forwarding of private compiler temporary homes.
//! Source-visible memory is never cached. Calls and control transfers are barriers.
use super::machine::*;
use std::collections::BTreeSet;

const NZVC: u8 = 15;
const X: u8 = 16;
const ALL: u8 = NZVC | X;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Value {
    home: i16,
    width: Width,
}

pub(super) fn forward(blocks: &mut [MachineBlock], homes: &BTreeSet<i16>) {
    for block in blocks.iter_mut() {
        let live = flags_live_after(&block.instructions);
        let mut registers = [None; 8];
        let mut output = Vec::new();
        for (i, original) in block.instructions.iter().enumerate() {
            let mut instruction = original.clone();
            if let Instruction::Move {
                width,
                source: Ea::Displacement(6, home),
                destination: Ea::D(dst),
            } = instruction
                && homes.contains(&home)
                && let Some(src) = registers
                    .iter()
                    .position(|v| *v == Some(Value { home, width }))
            {
                if src == usize::from(dst) && live[i] & NZVC == 0 {
                    continue;
                }
                instruction = Instruction::Move {
                    width,
                    source: Ea::D(src as u8),
                    destination: Ea::D(dst),
                };
            }
            transfer(&instruction, homes, &mut registers);
            output.push(instruction);
        }
        block.instructions = output;
    }
    // After forwarding, some private homes have no readers anywhere in the
    // routine. Their stores can disappear if their CCR effects are also dead.
    // Keep frame reservations stable; source-visible storage never participates.
    let mut read = BTreeSet::new();
    let mut reference = |ea: Ea| {
        if let Ea::Displacement(6, offset) = ea {
            for &home in homes.range(..=offset).rev().take(1) {
                if i32::from(offset) < i32::from(home) + 4 {
                    read.insert(home);
                }
            }
        }
    };
    for instruction in blocks.iter().flat_map(|b| &b.instructions) {
        match instruction {
            Instruction::Move { source, .. } => reference(*source),
            Instruction::Lea { source, .. } | Instruction::AddAddress { source, .. } => {
                reference(*source)
            }
            Instruction::Jump(ea) | Instruction::Jsr(ea) => reference(*ea),
            _ => {}
        }
    }
    for block in blocks {
        let live = flags_live_after(&block.instructions);
        let mut i = 0;
        block.instructions.retain(|instruction| {
            let remove = matches!(instruction,
                Instruction::Move {source: Ea::D(_), destination: Ea::Displacement(6, home), ..}
                if homes.contains(home) && !read.contains(home) && live[i] & NZVC == 0);
            i += 1;
            !remove
        });
    }
}

fn transfer(instruction: &Instruction, homes: &BTreeSet<i16>, registers: &mut [Option<Value>; 8]) {
    match *instruction {
        Instruction::Move {
            width,
            source,
            destination,
        } => {
            let value = match source {
                Ea::D(r) => registers[r as usize].filter(|v| v.width == width),
                Ea::Displacement(6, home) if homes.contains(&home) => Some(Value { home, width }),
                _ => None,
            };
            match destination {
                Ea::D(r) => registers[r as usize] = value,
                Ea::A(6) => *registers = [None; 8],
                Ea::A(_) => {}
                Ea::Displacement(6, home) if homes.contains(&home) => {
                    for v in registers.iter_mut() {
                        if v.is_some_and(|v| v.home == home) {
                            *v = None;
                        }
                    }
                    if let Ea::D(r) = source {
                        registers[r as usize] = Some(Value { home, width });
                    }
                }
                _ => *registers = [None; 8],
            }
        }
        Instruction::AluImmediate {
            operation: Alu::Compare,
            ..
        }
        | Instruction::Alu {
            operation: Alu::Compare,
            ..
        }
        | Instruction::CompareImmediate { .. } => {}
        Instruction::AluImmediate { destination, .. }
        | Instruction::Alu { destination, .. }
        | Instruction::MultiplyUnsignedWord { destination, .. }
        | Instruction::AddExtend { destination, .. } => registers[destination as usize] = None,
        Instruction::MoveQuick { register, .. }
        | Instruction::Negate { register, .. }
        | Instruction::Extend { register, .. }
        | Instruction::LogicalShift { register, .. }
        | Instruction::SetCondition { register, .. }
        | Instruction::AddQuick { register, .. } => registers[register as usize] = None,
        Instruction::Lea { destination, .. } | Instruction::AddAddress { destination, .. } => {
            if destination == 6 {
                *registers = [None; 8];
            }
        }
        // Branch instructions may occur within a physical block (e.g. divide).
        // No cached value crosses either arm or a callable boundary.
        Instruction::Branch { .. }
        | Instruction::Jump(_)
        | Instruction::Jsr(_)
        | Instruction::Trap(_)
        | Instruction::Link { .. }
        | Instruction::Unlink(_)
        | Instruction::Rts => *registers = [None; 8],
    }
}

fn flags_live_after(instructions: &[Instruction]) -> Vec<u8> {
    let mut live = ALL; // Successors may consume incoming CCR bits.
    let mut result = vec![0; instructions.len()];
    for (i, instruction) in instructions.iter().enumerate().rev() {
        result[i] = live;
        let (reads, writes) = match instruction {
            Instruction::Move {
                destination: Ea::A(_),
                ..
            }
            | Instruction::Lea { .. }
            | Instruction::AddAddress { .. }
            | Instruction::Jump(_)
            | Instruction::Link { .. }
            | Instruction::Unlink(_)
            | Instruction::Rts => (0, 0),
            Instruction::MoveQuick { .. }
            | Instruction::Move { .. }
            | Instruction::MultiplyUnsignedWord { .. }
            | Instruction::Extend { .. }
            | Instruction::CompareImmediate { .. } => (0, NZVC),
            Instruction::AluImmediate { operation, .. } | Instruction::Alu { operation, .. } => (
                0,
                if matches!(operation, Alu::Add | Alu::Sub) {
                    ALL
                } else {
                    NZVC
                },
            ),
            Instruction::Negate { .. } | Instruction::AddQuick { .. } => (0, ALL),
            Instruction::LogicalShift { count, .. } => (
                if matches!(count, ShiftCount::Register(_)) {
                    X
                } else {
                    0
                },
                ALL,
            ),
            Instruction::AddExtend { .. } => (X | 4, ALL), // ADDX also accumulates Z.
            Instruction::SetCondition { .. } | Instruction::Branch { .. } => (NZVC, 0),
            Instruction::Jsr(_) | Instruction::Trap(_) => (ALL, ALL),
        };
        live = (live & !writes) | reads;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mov(width: Width, source: Ea, destination: Ea) -> Instruction {
        Instruction::Move {
            width,
            source,
            destination,
        }
    }
    fn optimize(instructions: Vec<Instruction>) -> Vec<Instruction> {
        let mut blocks = vec![MachineBlock {
            id: MachineBlockId(0),
            instructions,
        }];
        forward(&mut blocks, &BTreeSet::from([-4]));
        blocks.pop().unwrap().instructions
    }
    #[test]
    fn removes_private_round_trip_when_flags_are_dead() {
        let compute = Instruction::AddQuick {
            width: Width::Long,
            delta: 1,
            register: 0,
        };
        let result = optimize(vec![
            mov(Width::Long, Ea::D(0), Ea::Displacement(6, -4)),
            mov(Width::Long, Ea::Displacement(6, -4), Ea::D(0)),
            compute.clone(),
        ]);
        assert_eq!(result, vec![compute]);
    }
    #[test]
    fn preserves_live_flags_and_big_endian_partial_reads() {
        let load = mov(Width::Byte, Ea::Displacement(6, -4), Ea::D(1));
        let input = vec![
            mov(Width::Long, Ea::D(0), Ea::Displacement(6, -4)),
            load.clone(),
        ];
        assert_eq!(optimize(input.clone()), input); // high byte of stored long != D0.low
        let result = optimize(vec![
            mov(Width::Long, Ea::D(0), Ea::Displacement(6, -4)),
            mov(Width::Long, Ea::Displacement(6, -4), Ea::D(1)),
            Instruction::SetCondition {
                condition: Condition::Equal,
                register: 2,
            },
        ]);
        assert_eq!(result[0], mov(Width::Long, Ea::D(0), Ea::D(1)));
    }
    #[test]
    fn calls_clobbers_and_source_memory_writes_end_forwarding() {
        for barrier in [
            Instruction::Jsr(Ea::Absolute(Address::absolute(0x10000))),
            Instruction::AddQuick {
                width: Width::Byte,
                delta: 1,
                register: 0,
            },
            mov(Width::Long, Ea::D(1), Ea::Indirect(0)),
            Instruction::Branch {
                condition: Condition::Equal,
                target: Address::absolute(0x10000),
            },
        ] {
            let load = mov(Width::Long, Ea::Displacement(6, -4), Ea::D(1));
            let result = optimize(vec![
                mov(Width::Long, Ea::D(0), Ea::Displacement(6, -4)),
                barrier,
                load.clone(),
            ]);
            assert_eq!(result.last(), Some(&load));
        }
    }
}
