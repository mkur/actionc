use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::counted_loops::{
    MirCountDirection, MirCountedLoop, MirCountedLoopShape, analyze_counted_loops,
};
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::analysis::machine_liveness::MirMachineLiveness;
use crate::mir6502::ir::{
    MirAddr, MirBlock, MirBlockId, MirCond, MirDef, MirEdge, MirFlagTest, MirMem, MirOp, MirReg,
    MirRoutine, MirTerminator, MirUpdateOp, MirValue, MirWidth,
};
use std::collections::{BTreeMap, BTreeSet};

use super::layout::MaterializeLayout;

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

/// Selects compact memory-resident latches from typed counted-loop facts after
/// physical update selection and block layout.
pub(super) fn select_counted_loop_latches(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
) -> usize {
    let mut selected = 0;
    loop {
        let Some(plan) = counted_loop_latch_plan(routine, layout) else {
            break;
        };
        let applied = match plan {
            CountedLoopLatchPlan::HeadTested(plan) => {
                apply_initialized_byte_countdown_plan(&mut routine.blocks, plan)
            }
            CountedLoopLatchPlan::RotatedHeadTested(plan) => {
                apply_rotated_head_tested_plan(&mut routine.blocks, plan)
            }
            CountedLoopLatchPlan::BottomGuarded(plan) => {
                apply_bottom_guarded_countdown_plan(&mut routine.blocks, plan)
            }
        };
        if !applied {
            break;
        }
        selected += 1;
    }
    selected
}

#[derive(Debug, Clone, Copy)]
enum CountedLoopLatchPlan {
    HeadTested(InitializedByteCountdownPlan),
    RotatedHeadTested(RotatedHeadTestedPlan),
    BottomGuarded(BottomGuardedByteCountdownPlan),
}

#[derive(Debug, Clone, Copy)]
struct InitializedByteCountdownPlan {
    preheader: MirBlockId,
    latch: MirBlockId,
    body: MirBlockId,
    exit: MirBlockId,
    header_index: usize,
    body_index: usize,
}

#[derive(Debug, Clone, Copy)]
struct RotatedHeadTestedPlan {
    preheader: MirBlockId,
    header: MirBlockId,
    body: MirBlockId,
    latch: MirBlockId,
}

#[derive(Debug, Clone, Copy)]
struct BottomGuardedByteCountdownPlan {
    preheader: MirBlockId,
    header: MirBlockId,
    body: MirBlockId,
    latch: MirBlockId,
    reload_accumulator: bool,
}

fn counted_loop_latch_plan(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
) -> Option<CountedLoopLatchPlan> {
    let cfg = MirCfg::from_routine(routine).ok()?;
    let liveness = MirMachineLiveness::analyze(routine, &cfg);
    for counted in analyze_counted_loops(routine) {
        if counted.width != MirWidth::Byte
            || counted.signed
            || counted.step != 1
            || counted.initial_guard_required
            || !counted_loop_mem_allows_rmw(&counted, layout)
            || !machine_state_dead_on_entry(&liveness, counted.exit)
        {
            continue;
        }
        match counted.shape {
            MirCountedLoopShape::HeadTested
                if counted.direction == MirCountDirection::Descending && counted.bound == 1 =>
            {
                if !machine_state_dead_on_entry(&liveness, counted.body) {
                    continue;
                }
                let header_index = routine
                    .blocks
                    .iter()
                    .position(|block| block.id == counted.header)?;
                let body_index = routine
                    .blocks
                    .iter()
                    .position(|block| block.id == counted.body)?;
                if header_index == 0
                    || routine.blocks[header_index - 1].id != counted.preheader
                    || body_index <= header_index + 1
                    || !routine.blocks[header_index + 1..body_index]
                        .iter()
                        .any(|block| block.id == counted.exit)
                {
                    continue;
                }
                let plan = InitializedByteCountdownPlan {
                    preheader: counted.preheader,
                    latch: counted.latch,
                    body: counted.body,
                    exit: counted.exit,
                    header_index,
                    body_index,
                };
                let mut candidate = routine.blocks.clone();
                if apply_initialized_byte_countdown_plan(&mut candidate, plan)
                    && estimated_routine_layout_bytes(&candidate)
                        < estimated_routine_layout_bytes(&routine.blocks)
                {
                    return Some(CountedLoopLatchPlan::HeadTested(plan));
                }
            }
            MirCountedLoopShape::HeadTested => {
                if !machine_state_dead_on_entry(&liveness, counted.body) {
                    continue;
                }
                let plan = RotatedHeadTestedPlan {
                    preheader: counted.preheader,
                    header: counted.header,
                    body: counted.body,
                    latch: counted.latch,
                };
                let mut candidate = routine.blocks.clone();
                if apply_rotated_head_tested_plan(&mut candidate, plan)
                    && estimated_routine_layout_bytes(&candidate)
                        < estimated_routine_layout_bytes(&routine.blocks)
                {
                    return Some(CountedLoopLatchPlan::RotatedHeadTested(plan));
                }
            }
            MirCountedLoopShape::BottomGuarded { guard } if counted.bound == 0 => {
                if !routine.blocks.iter().any(|block| block.id == guard)
                    || !machine_flags_dead_on_entry(&liveness, counted.body)
                {
                    continue;
                }
                let plan = BottomGuardedByteCountdownPlan {
                    preheader: counted.preheader,
                    header: counted.header,
                    body: counted.body,
                    latch: counted.latch,
                    reload_accumulator: machine_accumulator_live_on_entry(&liveness, counted.body),
                };
                let mut candidate = routine.blocks.clone();
                if apply_bottom_guarded_countdown_plan(&mut candidate, plan)
                    && estimated_routine_layout_bytes(&candidate)
                        < estimated_routine_layout_bytes(&routine.blocks)
                {
                    return Some(CountedLoopLatchPlan::BottomGuarded(plan));
                }
            }
            _ => {}
        }
    }
    None
}

fn counted_loop_mem_allows_rmw(counted: &MirCountedLoop, layout: &MaterializeLayout) -> bool {
    layout.mem_allows_direct_update(&counted.induction)
}

fn apply_initialized_byte_countdown_plan(
    blocks: &mut Vec<MirBlock>,
    plan: InitializedByteCountdownPlan,
) -> bool {
    if plan.header_index >= blocks.len()
        || plan.body_index > blocks.len()
        || plan.header_index + 1 >= plan.body_index
    {
        return false;
    }
    let Some(preheader_index) = blocks.iter().position(|block| block.id == plan.preheader) else {
        return false;
    };
    let Some(latch_index) = blocks.iter().position(|block| block.id == plan.latch) else {
        return false;
    };
    blocks[preheader_index].terminator = MirTerminator::Jump(MirEdge::plain(plan.body));
    blocks[latch_index].terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(MirFlagTest::ZClear),
        then_edge: MirEdge::plain(plan.body),
        else_edge: MirEdge::plain(plan.exit),
    };

    let original = std::mem::take(blocks);
    let mut reordered = Vec::with_capacity(original.len() - 1);
    reordered.extend_from_slice(&original[..plan.header_index]);
    reordered.extend_from_slice(&original[plan.body_index..]);
    reordered.extend_from_slice(&original[plan.header_index + 1..plan.body_index]);
    *blocks = reordered;
    true
}

fn apply_bottom_guarded_countdown_plan(
    blocks: &mut Vec<MirBlock>,
    plan: BottomGuardedByteCountdownPlan,
) -> bool {
    let Some(preheader_index) = blocks.iter().position(|block| block.id == plan.preheader) else {
        return false;
    };
    let Some(header_index) = blocks.iter().position(|block| block.id == plan.header) else {
        return false;
    };
    let Some(latch_index) = blocks.iter().position(|block| block.id == plan.latch) else {
        return false;
    };
    let Some(induction) = bottom_guarded_latch_mem(&blocks[latch_index], plan.header) else {
        return false;
    };
    blocks[preheader_index].terminator = MirTerminator::Jump(MirEdge::plain(plan.body));
    blocks[latch_index].ops = vec![MirOp::UpdateMem {
        op: MirUpdateOp::Dec,
        mem: induction.clone(),
        width: MirWidth::Byte,
    }];
    if plan.reload_accumulator {
        blocks[latch_index].ops.push(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(induction),
            width: MirWidth::Byte,
        });
    }
    blocks[latch_index].terminator = MirTerminator::Jump(MirEdge::plain(plan.body));
    blocks.remove(header_index);
    true
}

fn apply_rotated_head_tested_plan(blocks: &mut Vec<MirBlock>, plan: RotatedHeadTestedPlan) -> bool {
    let Some(preheader_index) = blocks.iter().position(|block| block.id == plan.preheader) else {
        return false;
    };
    let Some(header_index) = blocks.iter().position(|block| block.id == plan.header) else {
        return false;
    };
    if !blocks.iter().any(|block| block.id == plan.body)
        || !blocks.iter().any(|block| block.id == plan.latch)
    {
        return false;
    }
    blocks[preheader_index].terminator = MirTerminator::Jump(MirEdge::plain(plan.body));
    let header = blocks.remove(header_index);
    let Some(latch_index) = blocks.iter().position(|block| block.id == plan.latch) else {
        return false;
    };
    blocks.insert(latch_index.saturating_add(1), header);
    true
}

fn bottom_guarded_latch_mem(block: &MirBlock, header: MirBlockId) -> Option<MirMem> {
    let MirTerminator::Jump(edge) = &block.terminator else {
        return None;
    };
    if edge.target != header || !edge.args.is_empty() {
        return None;
    }
    match block.ops.as_slice() {
        [
            MirOp::UpdateMem {
                op: MirUpdateOp::Dec,
                mem,
                width: MirWidth::Byte,
            },
        ] => Some(mem.clone()),
        [
            MirOp::Binary {
                op: crate::mir6502::ir::MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0xff),
                width: MirWidth::Byte,
                carry_in: None | Some(crate::mir6502::ir::MirCarryIn::Clear),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ] => Some(mem.clone()),
        _ => None,
    }
}

fn machine_state_dead_on_entry(liveness: &MirMachineLiveness, block: MirBlockId) -> bool {
    !machine_accumulator_live_on_entry(liveness, block)
        && machine_flags_dead_on_entry(liveness, block)
}

fn machine_accumulator_live_on_entry(liveness: &MirMachineLiveness, block: MirBlockId) -> bool {
    liveness
        .live_in(block)
        .is_none_or(|live| live.register_live(MirReg::A))
}

fn machine_flags_dead_on_entry(liveness: &MirMachineLiveness, block: MirBlockId) -> bool {
    liveness.live_in(block).is_some_and(|live| {
        !live.flags_live(MirFlagSet {
            c: true,
            z: true,
            n: true,
            v: true,
        })
    })
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

fn estimated_routine_layout_bytes(blocks: &[MirBlock]) -> usize {
    blocks
        .iter()
        .map(estimated_block_body_bytes)
        .sum::<usize>()
        .saturating_add(estimated_layout_control_bytes(blocks))
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
    use crate::mir6502::ir::{
        MirAddr, MirCompareOp, MirCondDest, MirDef, MirEdge, MirFlagTest, MirMem, MirProgram,
        MirReg, MirWidth,
    };

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn layout_for(routine: &MirRoutine) -> MaterializeLayout {
        MaterializeLayout::new(
            &MirProgram {
                statics: Vec::new(),
                globals: Vec::new(),
                routines: vec![routine.clone()],
                machine_blocks: Vec::new(),
                runtime_helpers: Vec::new(),
            },
            0x3000,
        )
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

    #[test]
    fn initialized_byte_countdown_rotates_to_the_selected_dec_backedge() {
        let counter = MirMem::Local {
            id: crate::nir::LocalId(0),
            offset: 0,
        };
        let mut routine = MirRoutine {
            id: crate::mir6502::ir::RoutineId(0),
            name: "countdown".to_string(),
            abi: crate::mir6502::ir::MirRoutineAbi::Action,
            frame: Default::default(),
            temps: Vec::new(),
            blocks: vec![
                block(
                    0,
                    vec![
                        MirOp::Move {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirValue::ConstU8(3),
                            width: MirWidth::Byte,
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(counter.clone()),
                            src: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                        },
                    ],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                ),
                block(
                    1,
                    vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(counter.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Compare {
                            dst: MirCondDest::Flags,
                            op: MirCompareOp::Ge,
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::ConstU8(1),
                            width: MirWidth::Byte,
                            signed: false,
                        },
                    ],
                    MirTerminator::Branch {
                        cond: MirCond::FlagTest(MirFlagTest::CSet),
                        then_edge: MirEdge::plain(MirBlockId(3)),
                        else_edge: MirEdge::plain(MirBlockId(2)),
                    },
                ),
                block(2, Vec::new(), MirTerminator::Return),
                block(
                    3,
                    Vec::new(),
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(4))),
                ),
                block(
                    4,
                    vec![MirOp::UpdateMem {
                        op: MirUpdateOp::Dec,
                        mem: counter,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                ),
            ],
            effects: Default::default(),
        };

        let mut a_live_routine = routine.clone();
        a_live_routine.blocks[3].ops.push(MirOp::Move {
            dst: MirDef::Reg(MirReg::X),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        });
        let a_live_layout = layout_for(&a_live_routine);
        assert_eq!(
            select_counted_loop_latches(&mut a_live_routine, &a_live_layout),
            0
        );

        let mut carry_live_routine = routine.clone();
        carry_live_routine.blocks[3].ops.push(MirOp::Binary {
            op: crate::mir6502::ir::MirBinaryOp::Add,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::ConstU8(0),
            right: MirValue::ConstU8(0),
            width: MirWidth::Byte,
            carry_in: Some(crate::mir6502::ir::MirCarryIn::FromPrevious),
            carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
        });
        let carry_live_layout = layout_for(&carry_live_routine);
        assert_eq!(
            select_counted_loop_latches(&mut carry_live_routine, &carry_live_layout),
            0
        );

        let layout = layout_for(&routine);
        assert_eq!(select_counted_loop_latches(&mut routine, &layout), 1);
        assert_eq!(
            block_id_order(&routine.blocks),
            vec![MirBlockId(0), MirBlockId(3), MirBlockId(4), MirBlockId(2)]
        );
        assert!(matches!(
            routine.blocks[0].terminator,
            MirTerminator::Jump(ref edge) if edge.target == MirBlockId(3)
        ));
        assert!(matches!(
            routine.blocks[2].terminator,
            MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::ZClear),
                ref then_edge,
                ref else_edge,
            } if then_edge.target == MirBlockId(3) && else_edge.target == MirBlockId(2)
        ));
    }

    fn bottom_guarded_countdown(initial: MirValue, guard_reloads_counter: bool) -> MirRoutine {
        let counter = MirMem::Local {
            id: crate::nir::LocalId(0),
            offset: 0,
        };
        let body = if guard_reloads_counter {
            MirBlockId(2)
        } else {
            MirBlockId(3)
        };
        let mut blocks = vec![
            block(
                0,
                vec![MirOp::Store {
                    dst: MirAddr::Direct(counter.clone()),
                    src: initial,
                    width: MirWidth::Byte,
                }],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(
                1,
                vec![
                    MirOp::Load {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirAddr::Direct(counter.clone()),
                        width: MirWidth::Byte,
                    },
                    MirOp::Compare {
                        dst: MirCondDest::Flags,
                        op: MirCompareOp::Ge,
                        left: MirValue::Def(MirDef::Reg(MirReg::A)),
                        right: MirValue::ConstU8(0),
                        width: MirWidth::Byte,
                        signed: false,
                    },
                ],
                MirTerminator::Branch {
                    cond: MirCond::FlagTest(MirFlagTest::CSet),
                    then_edge: MirEdge::plain(body),
                    else_edge: MirEdge::plain(MirBlockId(5)),
                },
            ),
        ];
        if guard_reloads_counter {
            blocks.push(block(
                2,
                Vec::new(),
                MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            ));
        }
        let mut guard_ops = Vec::new();
        if guard_reloads_counter {
            guard_ops.push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(counter.clone()),
                width: MirWidth::Byte,
            });
        }
        guard_ops.push(MirOp::Compare {
            dst: MirCondDest::Flags,
            op: MirCompareOp::Lt,
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            signed: false,
        });
        blocks.extend([
            block(
                3,
                guard_ops,
                MirTerminator::Branch {
                    cond: MirCond::FlagTest(MirFlagTest::CClear),
                    then_edge: MirEdge::plain(MirBlockId(5)),
                    else_edge: MirEdge::plain(MirBlockId(4)),
                },
            ),
            block(
                4,
                vec![
                    MirOp::Binary {
                        op: crate::mir6502::ir::MirBinaryOp::Add,
                        dst: MirDef::Reg(MirReg::A),
                        left: MirValue::Def(MirDef::Reg(MirReg::A)),
                        right: MirValue::ConstU8(0xff),
                        width: MirWidth::Byte,
                        carry_in: Some(crate::mir6502::ir::MirCarryIn::Clear),
                        carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                    },
                    MirOp::Store {
                        dst: MirAddr::Direct(counter),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                ],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(5, Vec::new(), MirTerminator::Return),
        ]);
        MirRoutine {
            id: crate::mir6502::ir::RoutineId(0),
            name: "bottom_countdown".to_string(),
            abi: crate::mir6502::ir::MirRoutineAbi::Action,
            frame: Default::default(),
            temps: Vec::new(),
            blocks,
            effects: Default::default(),
        }
    }

    #[test]
    fn bottom_guarded_countdowns_select_dec_for_byte_boundaries() {
        for initial in [0, 1, 127, 128, 255] {
            let mut routine = bottom_guarded_countdown(MirValue::ConstU8(initial), true);
            let layout = layout_for(&routine);

            assert_eq!(select_counted_loop_latches(&mut routine, &layout), 1);
            assert!(!routine.blocks.iter().any(|block| block.id == MirBlockId(1)));
            assert!(matches!(
                routine
                    .blocks
                    .iter()
                    .find(|block| block.id == MirBlockId(4))
                    .map(|block| block.ops.as_slice()),
                Some([MirOp::UpdateMem {
                    op: MirUpdateOp::Dec,
                    width: MirWidth::Byte,
                    ..
                }])
            ));
        }
    }

    #[test]
    fn bottom_guarded_countdown_reloads_a_only_when_the_body_needs_it() {
        let mut routine = bottom_guarded_countdown(MirValue::ConstU8(9), false);
        let layout = layout_for(&routine);

        assert_eq!(select_counted_loop_latches(&mut routine, &layout), 1);
        let latch = routine
            .blocks
            .iter()
            .find(|block| block.id == MirBlockId(4))
            .expect("countdown latch");
        assert!(matches!(
            latch.ops.as_slice(),
            [MirOp::UpdateMem { .. }, MirOp::Load { .. }]
        ));
    }

    #[test]
    fn bottom_guarded_countdown_keeps_a_dynamic_entry_guard() {
        let mut routine = bottom_guarded_countdown(MirValue::Def(MirDef::Reg(MirReg::X)), true);
        let layout = layout_for(&routine);

        assert_eq!(select_counted_loop_latches(&mut routine, &layout), 0);
        assert!(routine.blocks.iter().any(|block| block.id == MirBlockId(1)));
    }

    fn ascending_head_tested_loop(initial: u8, exclusive_bound: u8) -> MirRoutine {
        let counter = MirMem::Local {
            id: crate::nir::LocalId(0),
            offset: 0,
        };
        MirRoutine {
            id: crate::mir6502::ir::RoutineId(0),
            name: "ascending_counted_loop".to_string(),
            abi: crate::mir6502::ir::MirRoutineAbi::Action,
            frame: Default::default(),
            temps: Vec::new(),
            blocks: vec![
                block(
                    0,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(counter.clone()),
                        src: MirValue::ConstU8(initial),
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                ),
                block(
                    1,
                    vec![
                        MirOp::Load {
                            dst: MirDef::Reg(MirReg::A),
                            src: MirAddr::Direct(counter.clone()),
                            width: MirWidth::Byte,
                        },
                        MirOp::Compare {
                            dst: MirCondDest::Flags,
                            op: MirCompareOp::Lt,
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::ConstU8(exclusive_bound),
                            width: MirWidth::Byte,
                            signed: false,
                        },
                    ],
                    MirTerminator::Branch {
                        cond: MirCond::FlagTest(MirFlagTest::CClear),
                        then_edge: MirEdge::plain(MirBlockId(2)),
                        else_edge: MirEdge::plain(MirBlockId(3)),
                    },
                ),
                block(
                    2,
                    vec![
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 7,
                            width: MirWidth::Byte,
                        },
                        MirOp::UpdateMem {
                            op: MirUpdateOp::Inc,
                            mem: counter,
                            width: MirWidth::Byte,
                        },
                    ],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                ),
                block(3, Vec::new(), MirTerminator::Return),
            ],
            effects: Default::default(),
        }
    }

    #[test]
    fn ascending_counted_loops_rotate_inc_latches_after_proven_entry() {
        for (initial, bound) in [(0, 1), (0, 127), (127, 128), (254, 255)] {
            let mut routine = ascending_head_tested_loop(initial, bound);
            let layout = layout_for(&routine);

            assert_eq!(select_counted_loop_latches(&mut routine, &layout), 1);
            assert_eq!(
                block_id_order(&routine.blocks),
                vec![MirBlockId(0), MirBlockId(2), MirBlockId(1), MirBlockId(3)]
            );
            assert!(matches!(
                routine.blocks[0].terminator,
                MirTerminator::Jump(ref edge) if edge.target == MirBlockId(2)
            ));
        }
    }
}
