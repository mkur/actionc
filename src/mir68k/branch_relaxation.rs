//! Monotone word-branch relaxation; distant targets retain absolute jumps.
use super::{encode, machine::*};
use std::collections::BTreeMap;

pub(super) fn relax(program: &mut MachineProgram) -> Result<(), String> {
    loop {
        let mut labels = BTreeMap::new();
        let mut cursor = 0u32;
        for block in &program.blocks {
            if labels.insert(block.id, cursor).is_some() {
                return Err("duplicate machine block ID".into());
            }
            for op in &block.instructions {
                cursor = cursor
                    .checked_add(encode::size(op)?)
                    .ok_or("code size overflow")?;
            }
        }
        let mut changed = false;
        cursor = 0;
        for block in &mut program.blocks {
            for instruction in &mut block.instructions {
                let size = encode::size(instruction)?;
                let candidate = match instruction {
                    Instruction::Branch { condition, target } => Some((Some(*condition), *target)),
                    Instruction::Jump(Ea::Absolute(target)) => Some((None, *target)),
                    _ => None,
                };
                if let Some((condition, target)) = candidate
                    && let Target::Block(id) = target.target
                    && target.addend == 0
                {
                    let address = *labels.get(&id).ok_or("missing branch target")?;
                    let displacement = i64::from(address) - (i64::from(cursor) + 2);
                    if i16::try_from(displacement).is_ok() {
                        *instruction = Instruction::BranchRelative { condition, target };
                        changed = true;
                    }
                }
                // This iteration uses one consistent layout, before shrinking.
                cursor += size;
            }
        }
        // Each change shortens a branch once. Distances between zero-addend
        // block labels can only shrink, so a chosen word branch stays in range.
        if !changed {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(id: u32) -> Address {
        Address::new(Target::Block(MachineBlockId(id)))
    }
    #[test]
    fn near_forward_and_backward_branches_shrink_but_far_targets_do_not() {
        let mut program = MachineProgram {
            blocks: vec![
                MachineBlock {
                    id: MachineBlockId(0),
                    instructions: vec![Instruction::Branch {
                        condition: Condition::Equal,
                        target: target(2),
                    }],
                },
                MachineBlock {
                    id: MachineBlockId(1),
                    instructions: vec![
                        Instruction::MoveQuick {
                            value: 0,
                            register: 0
                        };
                        16384
                    ],
                },
                MachineBlock {
                    id: MachineBlockId(2),
                    instructions: vec![
                        Instruction::Branch {
                            condition: Condition::NotEqual,
                            target: target(1),
                        },
                        Instruction::Jump(Ea::Absolute(target(3))),
                    ],
                },
                MachineBlock {
                    id: MachineBlockId(3),
                    instructions: vec![Instruction::Jump(Ea::Absolute(target(2)))],
                },
            ],
            ..Default::default()
        };
        relax(&mut program).unwrap();
        assert!(matches!(
            program.blocks[0].instructions[0],
            Instruction::Branch { .. }
        ));
        assert!(matches!(
            program.blocks[2].instructions[0],
            Instruction::Branch { .. }
        ));
        assert!(matches!(
            program.blocks[2].instructions[1],
            Instruction::BranchRelative {
                condition: None,
                ..
            }
        ));
        assert!(matches!(
            program.blocks[3].instructions[0],
            Instruction::BranchRelative {
                condition: None,
                ..
            }
        ));
        let once = program.clone();
        relax(&mut program).unwrap();
        assert_eq!(program, once);
    }
    #[test]
    fn shrinking_an_inner_jump_can_bring_an_outer_branch_into_range() {
        let mut program = MachineProgram {
            blocks: vec![
                MachineBlock {
                    id: MachineBlockId(0),
                    instructions: vec![
                        Instruction::Branch {
                            condition: Condition::Equal,
                            target: target(2),
                        },
                        Instruction::Jump(Ea::Absolute(target(2))),
                    ],
                },
                MachineBlock {
                    id: MachineBlockId(1),
                    instructions: vec![
                        Instruction::MoveQuick {
                            value: 0,
                            register: 0
                        };
                        16378
                    ],
                },
                MachineBlock {
                    id: MachineBlockId(2),
                    instructions: vec![Instruction::Rts],
                },
            ],
            ..Default::default()
        };
        // Initially the first displacement is 32768. The inner jump fits;
        // shrinking it makes the first branch eligible on the next iteration.
        relax(&mut program).unwrap();
        assert!(
            program.blocks[0]
                .instructions
                .iter()
                .all(|op| matches!(op, Instruction::BranchRelative { .. }))
        );
    }

    #[test]
    fn absolute_addresses_and_addends_keep_checked_absolute_encoding() {
        let input = vec![
            Instruction::Branch {
                condition: Condition::Equal,
                target: Address::absolute(0x10000),
            },
            Instruction::Jump(Ea::Absolute(Address {
                target: Target::Block(MachineBlockId(0)),
                addend: 2,
            })),
        ];
        let mut program = MachineProgram {
            blocks: vec![MachineBlock {
                id: MachineBlockId(0),
                instructions: input.clone(),
            }],
            ..Default::default()
        };
        relax(&mut program).unwrap();
        assert_eq!(program.blocks[0].instructions, input);
    }
}
