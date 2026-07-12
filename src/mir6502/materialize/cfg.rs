use crate::mir6502::ir::{MirBlockId, MirRoutine, MirTerminator};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn collapse_empty_jump_blocks(routine: &mut MirRoutine) {
    let Some(entry) = routine.blocks.first().map(|block| block.id) else {
        return;
    };
    let jump_blocks = routine
        .blocks
        .iter()
        .filter_map(|block| match block.terminator {
            MirTerminator::Jump(target)
                if block.id != entry && target != block.id && block.ops.is_empty() =>
            {
                Some((block.id, target))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    if jump_blocks.is_empty() {
        return;
    }

    for block in &mut routine.blocks {
        redirect_empty_jump_targets(&mut block.terminator, &jump_blocks);
    }
    routine
        .blocks
        .retain(|block| !jump_blocks.contains_key(&block.id));
}

fn redirect_empty_jump_targets(
    terminator: &mut MirTerminator,
    jump_blocks: &BTreeMap<MirBlockId, MirBlockId>,
) {
    match terminator {
        MirTerminator::Jump(target) => *target = resolved_empty_jump_target(*target, jump_blocks),
        MirTerminator::Branch {
            then_block,
            else_block,
            ..
        } => {
            *then_block = resolved_empty_jump_target(*then_block, jump_blocks);
            *else_block = resolved_empty_jump_target(*else_block, jump_blocks);
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
    }
}

fn resolved_empty_jump_target(
    mut target: MirBlockId,
    jump_blocks: &BTreeMap<MirBlockId, MirBlockId>,
) -> MirBlockId {
    let mut seen = BTreeSet::new();
    while seen.insert(target) {
        let Some(next) = jump_blocks.get(&target) else {
            break;
        };
        target = *next;
    }
    target
}
