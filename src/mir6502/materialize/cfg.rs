use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::ir::{MirBlock, MirBlockId, MirCond, MirOp, MirRoutine, MirTerminator};
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
    let original = routine.blocks.clone();
    let mut order = cfg.reverse_postorder().to_vec();
    order.extend(
        routine
            .blocks
            .iter()
            .map(|block| block.id)
            .filter(|id| !cfg.reachable().contains(id)),
    );
    let mut blocks = original
        .iter()
        .cloned()
        .into_iter()
        .map(|block| (block.id, block))
        .collect::<BTreeMap<_, _>>();
    let reordered = order
        .into_iter()
        .filter_map(|id| blocks.remove(&id))
        .collect::<Vec<_>>();
    debug_assert!(blocks.is_empty());
    let original_cost = estimated_layout_control_bytes(&original);
    let mut best = if estimated_layout_control_bytes(&reordered) < original_cost {
        reordered
    } else {
        original.clone()
    };
    refine_forward_branch_layout(&mut best);

    if best
        .iter()
        .map(|block| block.id)
        .eq(original.iter().map(|block| block.id))
    {
        return false;
    }
    routine.blocks = best;
    true
}

fn refine_forward_branch_layout(blocks: &mut Vec<MirBlock>) {
    // Keep a margin below the architectural +127-byte limit because the
    // operation cost table is intentionally conservative but not an emitter.
    const SAFE_FORWARD_SPAN: usize = 96;

    loop {
        let current_cost = estimated_layout_control_bytes(blocks);
        let mut best: Option<(usize, Vec<MirBlock>)> = None;

        for index in 0..blocks.len().saturating_sub(1) {
            let MirTerminator::Branch {
                cond,
                then_edge,
                else_edge,
            } = &blocks[index].terminator
            else {
                continue;
            };
            if !branch_has_native_condition(cond) {
                continue;
            }
            let next = blocks[index + 1].id;
            let alternate = if then_edge.target == next {
                else_edge.target
            } else if else_edge.target == next {
                then_edge.target
            } else {
                continue;
            };
            let Some(alternate_index) = blocks
                .iter()
                .position(|block| block.id == alternate)
                .filter(|alternate_index| *alternate_index > index + 1)
            else {
                continue;
            };
            if block_has_unmodeled_size(&blocks[alternate_index])
                || estimated_block_upper_bytes(&blocks[alternate_index]) > SAFE_FORWARD_SPAN
                || matches!(
                    blocks[alternate_index].terminator,
                    MirTerminator::Branch { .. }
                )
                || predecessor_uses_fallthrough(blocks, alternate_index)
            {
                continue;
            }

            let mut candidate = blocks.clone();
            let alternate_block = candidate.remove(alternate_index);
            candidate.insert(index + 1, alternate_block);
            let candidate_cost = estimated_layout_control_bytes(&candidate);
            if candidate_cost >= current_cost {
                continue;
            }
            let replace = best.as_ref().is_none_or(|(best_cost, best_blocks)| {
                candidate_cost < *best_cost
                    || candidate_cost == *best_cost
                        && block_id_order(&candidate) < block_id_order(best_blocks)
            });
            if replace {
                best = Some((candidate_cost, candidate));
            }
        }

        let Some((_, candidate)) = best else {
            break;
        };
        *blocks = candidate;
    }
}

fn estimated_layout_control_bytes(blocks: &[MirBlock]) -> usize {
    let starts = estimated_block_starts(blocks);
    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let next = blocks.get(index + 1).map(|next| next.id);
            match &block.terminator {
                MirTerminator::Jump(edge) => {
                    if Some(edge.target) == next {
                        0
                    } else {
                        estimated_edge_transfer_bytes(blocks, edge.target)
                    }
                }
                MirTerminator::Branch {
                    cond,
                    then_edge,
                    else_edge,
                } => {
                    let branch_bytes = estimated_native_branch_bytes(cond);
                    let branch_position =
                        starts[index].saturating_add(estimated_block_body_bytes(block));
                    let then_fits = estimated_branch_target_fits(
                        blocks,
                        &starts,
                        index,
                        branch_position,
                        then_edge.target,
                        cond,
                    );
                    let else_fits = estimated_branch_target_fits(
                        blocks,
                        &starts,
                        index,
                        branch_position,
                        else_edge.target,
                        cond,
                    );
                    if next == Some(then_edge.target) {
                        branch_bytes
                            + usize::from(!else_fits)
                                * estimated_edge_transfer_bytes(blocks, else_edge.target)
                    } else if next == Some(else_edge.target) {
                        branch_bytes
                            + usize::from(!then_fits)
                                * estimated_edge_transfer_bytes(blocks, then_edge.target)
                    } else if then_fits {
                        branch_bytes + estimated_edge_transfer_bytes(blocks, else_edge.target)
                    } else if else_fits {
                        branch_bytes + estimated_edge_transfer_bytes(blocks, then_edge.target)
                    } else {
                        branch_bytes
                            + estimated_edge_transfer_bytes(blocks, then_edge.target)
                            + estimated_edge_transfer_bytes(blocks, else_edge.target)
                    }
                }
                MirTerminator::Return | MirTerminator::Exit => 1,
                MirTerminator::Unreachable => 0,
            }
        })
        .sum()
}

fn estimated_block_starts(blocks: &[MirBlock]) -> Vec<usize> {
    let mut cursor = 0usize;
    blocks
        .iter()
        .map(|block| {
            let start = cursor;
            cursor = cursor.saturating_add(estimated_block_upper_bytes(block));
            start
        })
        .collect()
}

fn estimated_block_upper_bytes(block: &MirBlock) -> usize {
    estimated_block_body_bytes(block)
        + match &block.terminator {
            MirTerminator::Jump(_) => 3,
            MirTerminator::Branch { cond, .. } => {
                estimated_native_branch_bytes(cond).saturating_add(6)
            }
            MirTerminator::Return | MirTerminator::Exit => 1,
            MirTerminator::Unreachable => 0,
        }
}

fn estimated_block_body_bytes(block: &MirBlock) -> usize {
    usize::from(crate::mir6502::rewrite::posthome::estimated_6502_cost(&block.ops).0)
}

fn estimated_branch_target_fits(
    blocks: &[MirBlock],
    starts: &[usize],
    source_index: usize,
    branch_position: usize,
    target: MirBlockId,
    cond: &MirCond,
) -> bool {
    if blocks.get(source_index + 1).map(|block| block.id) == Some(target) {
        return true;
    }
    let Some(target_index) = blocks.iter().position(|block| block.id == target) else {
        return false;
    };
    let target_position = starts[target_index];
    let first_fits = relative_branch_fits(branch_position, target_position);
    if matches!(cond, MirCond::AnyFlagTest(_)) {
        first_fits && relative_branch_fits(branch_position.saturating_add(2), target_position)
    } else {
        first_fits
    }
}

fn relative_branch_fits(branch_position: usize, target_position: usize) -> bool {
    let offset = target_position as isize - (branch_position as isize + 2);
    (-128..=127).contains(&offset)
}

fn estimated_native_branch_bytes(cond: &MirCond) -> usize {
    match cond {
        MirCond::AnyFlagTest(_) => 4,
        _ => 2,
    }
}

fn branch_has_native_condition(cond: &MirCond) -> bool {
    matches!(cond, MirCond::FlagTest(_) | MirCond::AnyFlagTest(_))
}

fn estimated_edge_transfer_bytes(blocks: &[MirBlock], target: MirBlockId) -> usize {
    if blocks.iter().any(|block| {
        block.id == target
            && block.params.is_empty()
            && block.ops.is_empty()
            && matches!(
                block.terminator,
                MirTerminator::Return | MirTerminator::Exit
            )
    }) {
        1
    } else {
        3
    }
}

fn predecessor_uses_fallthrough(blocks: &[MirBlock], target_index: usize) -> bool {
    let target = blocks[target_index].id;
    blocks
        .get(target_index.saturating_sub(1))
        .is_some_and(|predecessor| match &predecessor.terminator {
            MirTerminator::Jump(edge) => edge.target == target,
            MirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => then_edge.target == target || else_edge.target == target,
            MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => false,
        })
}

fn block_has_unmodeled_size(block: &MirBlock) -> bool {
    block
        .ops
        .iter()
        .any(|op| matches!(op, MirOp::Barrier { .. } | MirOp::MachineBlock { .. }))
}

fn block_id_order(blocks: &[MirBlock]) -> Vec<MirBlockId> {
    blocks.iter().map(|block| block.id).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{MirAddr, MirDef, MirEdge, MirFlagTest, MirMem, MirReg, MirWidth};

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn absolute_loads(count: usize) -> Vec<MirOp> {
        (0..count)
            .map(|offset| MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::Absolute(0x4000u16.saturating_add(offset as u16))),
                width: MirWidth::Byte,
            })
            .collect()
    }

    fn branch(then_target: u32, else_target: u32) -> MirTerminator {
        MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet),
            then_edge: MirEdge::plain(MirBlockId(then_target)),
            else_edge: MirEdge::plain(MirBlockId(else_target)),
        }
    }

    #[test]
    fn reach_aware_layout_moves_a_small_far_loop_block_beside_its_branch() {
        let mut blocks = vec![
            block(0, Vec::new(), branch(1, 3)),
            block(1, absolute_loads(50), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
            block(
                3,
                absolute_loads(2),
                MirTerminator::Jump(MirEdge::plain(MirBlockId(0))),
            ),
        ];

        refine_forward_branch_layout(&mut blocks);

        assert_eq!(
            block_id_order(&blocks),
            vec![MirBlockId(0), MirBlockId(3), MirBlockId(1), MirBlockId(2)]
        );
        assert_eq!(estimated_layout_control_bytes(&blocks), 7);
    }

    #[test]
    fn reach_aware_layout_keeps_an_already_near_alternate_target_stable() {
        let mut blocks = vec![
            block(0, Vec::new(), branch(1, 3)),
            block(1, absolute_loads(1), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
            block(
                3,
                absolute_loads(2),
                MirTerminator::Jump(MirEdge::plain(MirBlockId(0))),
            ),
        ];
        let original = block_id_order(&blocks);

        refine_forward_branch_layout(&mut blocks);

        assert_eq!(block_id_order(&blocks), original);
    }

    #[test]
    fn reach_aware_layout_can_make_a_far_pure_return_the_fallthrough() {
        let mut blocks = vec![
            block(0, Vec::new(), branch(1, 3)),
            block(1, absolute_loads(50), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
            block(3, Vec::new(), MirTerminator::Return),
        ];

        refine_forward_branch_layout(&mut blocks);

        assert_eq!(
            block_id_order(&blocks),
            vec![MirBlockId(0), MirBlockId(3), MirBlockId(1), MirBlockId(2)]
        );
    }

    #[test]
    fn reach_aware_layout_does_not_detach_compare_helper_chains() {
        let mut blocks = vec![
            block(0, Vec::new(), branch(1, 3)),
            block(1, absolute_loads(50), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
            block(3, Vec::new(), branch(1, 2)),
        ];
        let original = block_id_order(&blocks);

        refine_forward_branch_layout(&mut blocks);

        assert_eq!(block_id_order(&blocks), original);
    }

    #[test]
    fn reach_aware_layout_rejects_blocks_with_unmodeled_machine_size() {
        let mut blocks = vec![
            block(0, Vec::new(), branch(1, 3)),
            block(1, absolute_loads(50), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
            block(
                3,
                vec![MirOp::Barrier {
                    effects: Default::default(),
                }],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(0))),
            ),
        ];
        let original = block_id_order(&blocks);

        refine_forward_branch_layout(&mut blocks);

        assert_eq!(block_id_order(&blocks), original);
    }

    #[test]
    fn reach_aware_layout_tie_breaks_equal_candidates_by_stable_block_id() {
        let mut blocks = vec![
            block(0, Vec::new(), branch(1, 4)),
            block(1, absolute_loads(50), MirTerminator::Return),
            block(2, Vec::new(), branch(3, 5)),
            block(3, absolute_loads(50), MirTerminator::Return),
            block(4, Vec::new(), MirTerminator::Return),
            block(5, Vec::new(), MirTerminator::Return),
        ];

        refine_forward_branch_layout(&mut blocks);

        assert_eq!(blocks[1].id, MirBlockId(4));
    }
}
