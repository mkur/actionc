use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::ir::{MirBlock, MirBlockId, MirCond, MirRoutine, MirTerminator};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn collapse_empty_jump_blocks(routine: &mut MirRoutine) {
    let Some(entry) = routine.blocks.first().map(|block| block.id) else {
        return;
    };
    let jump_blocks = routine
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            MirTerminator::Jump(edge)
                if block.id != entry
                    && edge.target != block.id
                    && edge.args.is_empty()
                    && block.params.is_empty()
                    && block.ops.is_empty() =>
            {
                Some((block.id, edge.target))
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

/// Places reachable blocks in CFG reverse postorder while preserving the
/// routine entry and the original relative order of unreachable blocks.
///
/// Compare expansion deliberately appends helper blocks so stable block IDs
/// are easy to construct.  Leaving that construction order intact, however,
/// strands short comparison chains after routine exits. Reverse postorder puts
/// those blocks back beside their incoming edges and exposes fall-throughs to
/// the emitter without changing CFG identity.
pub(super) fn layout_blocks_in_reverse_postorder(routine: &mut MirRoutine) -> bool {
    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return false;
    };
    let mut order = cfg.reverse_postorder().to_vec();
    order.extend(
        routine
            .blocks
            .iter()
            .map(|block| block.id)
            .filter(|id| !cfg.reachable().contains(id)),
    );
    if order
        .iter()
        .copied()
        .eq(routine.blocks.iter().map(|block| block.id))
    {
        return false;
    }
    let original_cost = estimated_fallthrough_control_bytes(&routine.blocks);

    let mut blocks = std::mem::take(&mut routine.blocks)
        .into_iter()
        .map(|block| (block.id, block))
        .collect::<BTreeMap<_, _>>();
    let reordered = order
        .into_iter()
        .filter_map(|id| blocks.remove(&id))
        .collect::<Vec<_>>();
    debug_assert!(blocks.is_empty());
    if estimated_fallthrough_control_bytes(&reordered) >= original_cost {
        routine.blocks = reordered;
        routine.blocks.sort_by_key(|block| {
            cfg.block_index(block.id)
                .expect("layout candidate came from the routine CFG")
        });
        return false;
    }
    routine.blocks = reordered;
    true
}

fn estimated_fallthrough_control_bytes(blocks: &[MirBlock]) -> usize {
    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let next = blocks.get(index + 1).map(|next| next.id);
            match &block.terminator {
                MirTerminator::Jump(edge) => usize::from(Some(edge.target) != next) * 3,
                MirTerminator::Branch {
                    cond,
                    then_edge,
                    else_edge,
                } => {
                    let has_fallthrough =
                        next == Some(then_edge.target) || next == Some(else_edge.target);
                    match cond {
                        MirCond::AnyFlagTest(_) => 4 + usize::from(!has_fallthrough) * 3,
                        _ => 2 + usize::from(!has_fallthrough) * 3,
                    }
                }
                MirTerminator::Return | MirTerminator::Exit => 1,
                MirTerminator::Unreachable => 0,
            }
        })
        .sum()
}

fn redirect_empty_jump_targets(
    terminator: &mut MirTerminator,
    jump_blocks: &BTreeMap<MirBlockId, MirBlockId>,
) {
    match terminator {
        MirTerminator::Jump(edge) => {
            edge.target = resolved_empty_jump_target(edge.target, jump_blocks)
        }
        MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            then_edge.target = resolved_empty_jump_target(then_edge.target, jump_blocks);
            else_edge.target = resolved_empty_jump_target(else_edge.target, jump_blocks);
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
