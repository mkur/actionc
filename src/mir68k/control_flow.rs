//! Fallthrough selection after edge transfers have been materialized.
use super::machine::*;

pub(super) fn fallthrough(blocks: &mut [MachineBlock]) {
    for index in 0..blocks.len().saturating_sub(1) {
        let next = Address::new(Target::Block(blocks[index + 1].id));
        let ops = &mut blocks[index].instructions;
        if matches!(ops.last(), Some(Instruction::Jump(Ea::Absolute(target))) if *target == next) {
            ops.pop();
        } else if let [
            ..,
            Instruction::Branch { condition, target },
            Instruction::Jump(Ea::Absolute(other)),
        ] = ops.as_slice()
            && *target == next
        {
            let branch = Instruction::Branch {
                condition: condition.inverse(),
                target: *other,
            };
            ops.pop();
            *ops.last_mut().unwrap() = branch;
        }
        // Both outcomes may now fall through. CMP still executes, preserving
        // evaluation; no flags are part of the typed-MIR edge contract.
        if matches!(ops.last(), Some(Instruction::Branch { target, .. }) if *target == next) {
            ops.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_edges_fall_through_without_losing_labels_or_addends() {
        let next = Address::new(Target::Block(MachineBlockId(1)));
        let other = Address::new(Target::Block(MachineBlockId(0)));
        let mut blocks = vec![
            MachineBlock {
                id: MachineBlockId(0),
                instructions: vec![
                    Instruction::Branch {
                        condition: Condition::Less,
                        target: next,
                    },
                    Instruction::Jump(Ea::Absolute(other)),
                ],
            },
            MachineBlock {
                id: MachineBlockId(1),
                instructions: vec![Instruction::Rts],
            },
        ];
        fallthrough(&mut blocks);
        assert_eq!(
            blocks[0].instructions,
            [Instruction::Branch {
                condition: Condition::GreaterOrEqual,
                target: other
            }]
        );
        let once = blocks.clone();
        fallthrough(&mut blocks);
        assert_eq!(blocks, once);
        blocks[0].instructions = vec![Instruction::Jump(Ea::Absolute(Address {
            addend: 2,
            ..next
        }))];
        fallthrough(&mut blocks);
        assert_eq!(blocks[0].instructions.len(), 1);
        blocks[0].instructions = vec![Instruction::Jump(Ea::Absolute(next))];
        fallthrough(&mut blocks);
        assert!(blocks[0].instructions.is_empty());
        assert_eq!(blocks[0].id, MachineBlockId(0));
    }
}
