use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::counted_loops::{
    MirCountDirection, MirCountedLoop, MirCountedLoopShape, analyze_counted_loops,
};
use crate::mir6502::analysis::effects::{MirFlagSet, classify_op, classify_terminator};
use crate::mir6502::analysis::machine_liveness::MirMachineLiveness;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{
    MirAddr, MirBlock, MirBlockId, MirByteIndexedSource, MirCallTarget, MirCond, MirDef, MirEdge,
    MirFixedZpSlot, MirFlag, MirFlagTest, MirMem, MirOp, MirPointerPair, MirReg, MirRoutine,
    MirTerminator, MirUpdateOp, MirValue, MirWidth,
};
use std::collections::{BTreeMap, BTreeSet};

use super::layout::MaterializeLayout;
use super::memory::op_may_write_mem;
use super::spills::visit_op_mems;

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
            CountedLoopLatchPlan::BottomFast(plan) => {
                apply_bottom_fast_countdown_plan(&mut routine.blocks, plan)
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

/// Carries a canonical byte induction value in X or Y across a loop backedge.
///
/// This is deliberately a late, target-owned home decision. It runs only
/// after ordinary homes and direct updates have been selected, and accepts a
/// loop only when the existing machine liveness, memory layout, and counted
/// loop facts prove that the carrier and delayed final writeback are safe.
pub(super) fn select_counted_loop_register_carriers(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
) -> usize {
    let mut selected = 0;
    loop {
        let Some(blocks) = counted_loop_register_carrier_candidate(routine, layout) else {
            break;
        };
        routine.blocks = blocks;
        selected += 1;
    }
    selected
}

fn counted_loop_register_carrier_candidate(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
) -> Option<Vec<MirBlock>> {
    let cfg = MirCfg::from_routine(routine).ok()?;
    let liveness = MirMachineLiveness::analyze(routine, &cfg);
    let original_cost = estimated_routine_layout_bytes(&routine.blocks);

    for counted in analyze_counted_loops(routine) {
        if let MirCountedLoopShape::FullRangeAscending { guard } = &counted.shape {
            if counted.width != MirWidth::Byte
                || counted.direction != MirCountDirection::Ascending
                || counted.signed
                || counted.step != 1
                || counted.bound != u16::from(u8::MAX)
                || counted.initial_guard_required
                || const_byte(&counted.initial_value) != Some(0)
                || !layout.mem_allows_direct_update(&counted.induction)
                || !machine_flags_dead_on_entry(&liveness, counted.body)
                || !machine_accumulator_dead_on_entry(&liveness, counted.exit)
                || !machine_flag_dead_on_entry(&liveness, counted.exit, MirFlag::C)
                || induction_address_is_taken(routine, layout, &counted.induction)
            {
                continue;
            }
            let Some(loop_nodes) = full_range_natural_loop(&cfg, &counted, *guard) else {
                continue;
            };
            let mut best: Option<(usize, Vec<MirBlock>)> = None;
            for carrier in [MirReg::Y, MirReg::X] {
                if !machine_register_unobserved_before_redefinition(
                    routine,
                    &cfg,
                    counted.exit,
                    carrier,
                ) {
                    continue;
                }
                let mut candidate = routine.blocks.clone();
                if !apply_full_range_register_carrier(
                    routine,
                    &cfg,
                    &mut candidate,
                    layout,
                    &liveness,
                    &counted,
                    &loop_nodes,
                    *guard,
                    carrier,
                ) {
                    continue;
                }
                let cost = estimated_routine_layout_bytes(&candidate);
                if cost >= original_cost {
                    continue;
                }
                // Y is the native byte-index carrier for both absolute and
                // indirect consumers. A small target preference also reflects
                // reload/store cleanup exposed only after carrier selection.
                let selection_cost = cost.saturating_add(usize::from(carrier == MirReg::X) * 2);
                if best
                    .as_ref()
                    .is_none_or(|(best_cost, _)| selection_cost < *best_cost)
                {
                    best = Some((selection_cost, candidate));
                }
            }
            if let Some((_, blocks)) = best {
                return Some(blocks);
            }
            continue;
        }
        if counted.shape != MirCountedLoopShape::HeadTested
            || counted.width != MirWidth::Byte
            || counted.signed
            || counted.step != 1
            || counted.initial_guard_required
            || !layout.mem_allows_direct_update(&counted.induction)
            || !machine_accumulator_dead_on_entry(&liveness, counted.body)
            || !machine_accumulator_dead_on_entry(&liveness, counted.exit)
            || induction_address_is_taken(routine, layout, &counted.induction)
        {
            continue;
        }
        let Some(loop_nodes) = canonical_natural_loop(&cfg, &counted) else {
            continue;
        };

        let mut best: Option<(usize, Vec<MirBlock>)> = None;
        for carrier in [MirReg::X, MirReg::Y] {
            if !machine_register_dead_on_entry(&liveness, counted.header, carrier) {
                continue;
            }
            let mut candidate = routine.blocks.clone();
            if !apply_counted_loop_register_carrier(
                routine,
                &mut candidate,
                layout,
                &liveness,
                &counted,
                &loop_nodes,
                carrier,
            ) {
                continue;
            }
            let cost = estimated_routine_layout_bytes(&candidate);
            if cost >= original_cost {
                continue;
            }
            if best.as_ref().is_none_or(|(best_cost, _)| cost < *best_cost) {
                best = Some((cost, candidate));
            }
        }
        if let Some((_, blocks)) = best {
            return Some(blocks);
        }
    }
    None
}

fn full_range_natural_loop(
    cfg: &MirCfg,
    counted: &MirCountedLoop,
    guard: MirBlockId,
) -> Option<BTreeSet<MirBlockId>> {
    let mut nodes = BTreeSet::from([counted.header, counted.latch]);
    let mut pending = vec![counted.latch];
    while let Some(block) = pending.pop() {
        for predecessor in cfg.predecessors(block) {
            if *predecessor != counted.header && nodes.insert(*predecessor) {
                pending.push(*predecessor);
            }
        }
    }
    if !nodes.contains(&counted.body)
        || !nodes.contains(&guard)
        || nodes.contains(&counted.preheader)
        || nodes.contains(&counted.exit)
    {
        return None;
    }
    if nodes.iter().any(|block| {
        cfg.predecessors(*block).iter().any(|predecessor| {
            if *block == counted.header {
                !nodes.contains(predecessor) && *predecessor != counted.preheader
            } else {
                !nodes.contains(predecessor)
            }
        })
    }) {
        return None;
    }
    for block in &nodes {
        for successor in cfg.successors(*block) {
            if !nodes.contains(successor) && *successor != counted.exit {
                return None;
            }
        }
    }
    Some(nodes)
}

fn canonical_natural_loop(cfg: &MirCfg, counted: &MirCountedLoop) -> Option<BTreeSet<MirBlockId>> {
    if cfg.predecessors(counted.exit) != &BTreeSet::from([counted.header]) {
        return None;
    }

    let mut nodes = BTreeSet::from([counted.header, counted.latch]);
    let mut pending = vec![counted.latch];
    while let Some(block) = pending.pop() {
        for predecessor in cfg.predecessors(block) {
            if *predecessor != counted.header && nodes.insert(*predecessor) {
                pending.push(*predecessor);
            }
        }
    }
    if !nodes.contains(&counted.body) || nodes.contains(&counted.preheader) {
        return None;
    }
    if nodes.iter().any(|block| {
        cfg.predecessors(*block)
            .iter()
            .any(|predecessor| *block != counted.header && !nodes.contains(predecessor))
    }) {
        return None;
    }
    for block in &nodes {
        for successor in cfg.successors(*block) {
            if !nodes.contains(successor)
                && !(*block == counted.header && *successor == counted.exit)
            {
                return None;
            }
        }
    }
    Some(nodes)
}

#[allow(clippy::too_many_arguments)]
fn apply_full_range_register_carrier(
    routine: &MirRoutine,
    cfg: &MirCfg,
    blocks: &mut Vec<MirBlock>,
    layout: &MaterializeLayout,
    liveness: &MirMachineLiveness,
    counted: &MirCountedLoop,
    loop_nodes: &BTreeSet<MirBlockId>,
    guard: MirBlockId,
    carrier: MirReg,
) -> bool {
    let Some(latch) = blocks.iter().find(|block| block.id == counted.latch) else {
        return false;
    };
    if !matches!(
        latch.ops.as_slice(),
        [MirOp::UpdateMem {
            op: MirUpdateOp::Inc,
            mem,
            width: MirWidth::Byte,
        }] if mem == &counted.induction
    ) || !matches!(
        &latch.terminator,
        MirTerminator::Jump(edge)
            if edge.target == counted.header && edge.args.is_empty()
    ) {
        return false;
    }

    let early_exit_sources = loop_nodes
        .iter()
        .copied()
        .filter(|block| *block != counted.header && *block != guard)
        .filter(|block| cfg_successor_is(blocks, *block, counted.exit))
        .collect::<Vec<_>>();
    let deferred_pair_restore = if carrier == MirReg::Y && early_exit_sources.is_empty() {
        let Some(guard_block) = blocks.iter().find(|block| block.id == guard) else {
            return false;
        };
        full_range_terminal_pair_restore(routine, guard_block, layout, &counted.induction)
    } else {
        None
    };
    if deferred_pair_restore.is_some()
        && [MirFlag::C, MirFlag::Z, MirFlag::N, MirFlag::V]
            .into_iter()
            .any(|flag| {
                !machine_flag_unobserved_before_redefinition(
                    routine,
                    cfg,
                    counted.exit,
                    guard,
                    flag,
                )
            })
    {
        return false;
    }
    let mut next_block = blocks
        .iter()
        .map(|block| block.id.0)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let normal_restore = if counted.final_value_observable || deferred_pair_restore.is_some() {
        let Some(id) = next_block.map(MirBlockId) else {
            return false;
        };
        next_block = id.0.checked_add(1);
        Some(id)
    } else {
        None
    };
    let early_restore = if counted.final_value_observable && !early_exit_sources.is_empty() {
        let Some(id) = next_block.map(MirBlockId) else {
            return false;
        };
        Some(id)
    } else {
        None
    };

    let Some(preheader) = blocks
        .iter_mut()
        .find(|block| block.id == counted.preheader)
    else {
        return false;
    };
    if !replace_induction_initialization(preheader, &counted.induction, carrier, 0) {
        return false;
    }
    preheader.terminator = MirTerminator::Jump(MirEdge::plain(counted.body));

    for block in blocks.iter_mut() {
        if !loop_nodes.contains(&block.id)
            || block.id == counted.header
            || block.id == counted.latch
        {
            continue;
        }
        if !block.params.is_empty()
            || !rewrite_full_range_loop_block(
                routine,
                block,
                layout,
                liveness,
                counted,
                carrier,
                block.id == counted.body,
                block.id == guard,
                deferred_pair_restore.is_some(),
            )
        {
            return false;
        }
    }

    let normal_target = normal_restore.unwrap_or(counted.exit);
    let Some(guard_block) = blocks.iter_mut().find(|block| block.id == guard) else {
        return false;
    };
    guard_block.ops.push(MirOp::UpdateReg {
        op: MirUpdateOp::Inc,
        reg: carrier,
    });
    guard_block.terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(MirFlagTest::ZClear),
        then_edge: MirEdge::plain(counted.body),
        else_edge: MirEdge::plain(normal_target),
    };

    if let Some(early_restore) = early_restore {
        for source in &early_exit_sources {
            let Some(block) = blocks.iter_mut().find(|block| block.id == *source) else {
                return false;
            };
            redirect_block_target(&mut block.terminator, counted.exit, early_restore);
        }
    }

    blocks.retain(|block| block.id != counted.header && block.id != counted.latch);
    let Some(exit_index) = blocks.iter().position(|block| block.id == counted.exit) else {
        return false;
    };
    let mut restore_blocks = Vec::new();
    if let Some(id) = normal_restore {
        let mut ops = Vec::new();
        if let Some(pair_restore) = &deferred_pair_restore {
            ops.extend(pair_restore.prefix.iter().cloned());
            ops.push(MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: u16::from(u8::MAX),
                width: MirWidth::Byte,
            });
            ops.push(pair_restore.advance.clone());
        }
        if counted.final_value_observable {
            ops.extend([
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: u16::from(u8::MAX),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(counted.induction.clone()),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ]);
        }
        restore_blocks.push(MirBlock {
            id,
            label: "full_range.restore_normal".to_string(),
            params: Vec::new(),
            ops,
            terminator: MirTerminator::Jump(MirEdge::plain(counted.exit)),
        });
    }
    if let Some(id) = early_restore {
        restore_blocks.push(MirBlock {
            id,
            label: "full_range.restore_exit".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Store {
                dst: MirAddr::Direct(counted.induction.clone()),
                src: MirValue::Def(MirDef::Reg(carrier)),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(counted.exit)),
        });
    }
    blocks.splice(exit_index..exit_index, restore_blocks);
    true
}

fn cfg_successor_is(blocks: &[MirBlock], block: MirBlockId, target: MirBlockId) -> bool {
    blocks
        .iter()
        .find(|candidate| candidate.id == block)
        .is_some_and(|block| match &block.terminator {
            MirTerminator::Jump(edge) => edge.target == target,
            MirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => then_edge.target == target || else_edge.target == target,
            MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => false,
        })
}

#[derive(Debug, Clone)]
struct FullRangePairRestore {
    prefix: Vec<MirOp>,
    advance: MirOp,
}

fn full_range_terminal_pair_restore(
    routine: &MirRoutine,
    guard: &MirBlock,
    layout: &MaterializeLayout,
    induction: &MirMem,
) -> Option<FullRangePairRestore> {
    let store_index = guard.ops.len().checked_sub(3)?;
    let (stage_start, base, pair_lo) =
        staged_static_store_target(routine, &guard.ops, store_index, layout, induction)?;
    if !indexed_base_disjoint_from_fixed_pair(routine, layout, &base, pair_lo) {
        return None;
    }
    Some(FullRangePairRestore {
        prefix: guard.ops[stage_start..stage_start + 4].to_vec(),
        advance: guard.ops.get(stage_start + 5)?.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn rewrite_full_range_loop_block(
    routine: &MirRoutine,
    block: &mut MirBlock,
    layout: &MaterializeLayout,
    liveness: &MirMachineLiveness,
    counted: &MirCountedLoop,
    carrier: MirReg,
    body_entry: bool,
    guard: bool,
    defer_terminal_pair_restore: bool,
) -> bool {
    let original = block.ops.clone();
    let end = if guard {
        let Some(end) = original.len().checked_sub(2) else {
            return false;
        };
        end
    } else {
        original.len()
    };
    let mut replacement = Vec::with_capacity(original.len());
    if body_entry {
        replacement.push(MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::Def(MirDef::Reg(carrier)),
            width: MirWidth::Byte,
        });
    }
    let mut old_to_new = vec![None; original.len()];
    let mut a_holds_induction = body_entry;
    let mut y_holds_induction = carrier == MirReg::Y;
    let mut skip_next = false;
    let terminal_store = (guard && carrier == MirReg::Y)
        .then(|| {
            let store_index = end.checked_sub(1)?;
            let (stage_start, base, pair_lo) = staged_static_store_target(
                routine,
                &original,
                store_index,
                layout,
                &counted.induction,
            )?;
            indexed_base_disjoint_from_fixed_pair(routine, layout, &base, pair_lo).then_some((
                stage_start,
                store_index,
                base,
            ))
        })
        .flatten();

    for (op_index, op) in original[..end].iter().enumerate() {
        if let Some((stage_start, store_index, base)) = &terminal_store {
            if (*stage_start..*stage_start + 6).contains(&op_index) {
                continue;
            }
            if op_index == *store_index {
                old_to_new[op_index] = Some(replacement.len());
                replacement.push(MirOp::Store {
                    dst: MirAddr::AbsoluteIndexedY { base: base.clone() },
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                });
                if !defer_terminal_pair_restore {
                    replacement.extend_from_slice(&original[*stage_start..*stage_start + 4]);
                    replacement.push(MirOp::Move {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                        width: MirWidth::Byte,
                    });
                    replacement.push(original[*stage_start + 5].clone());
                    a_holds_induction = false;
                }
                y_holds_induction = true;
                continue;
            }
        }
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            op,
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            } if mem == &counted.induction
        ) && matches!(
            original.get(op_index + 1),
            Some(MirOp::Move {
                dst: MirDef::Reg(MirReg::Y),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            })
        ) {
            if !load_result_flags_are_dead(liveness, block.id, op_index + 1) {
                return false;
            }
            y_holds_induction = true;
            skip_next = true;
            continue;
        }
        if let MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } = op
            && mem == &counted.induction
        {
            if *reg == carrier {
                if !load_result_flags_are_dead(liveness, block.id, op_index) {
                    return false;
                }
                if carrier == MirReg::Y {
                    y_holds_induction = true;
                }
            } else {
                old_to_new[op_index] = Some(replacement.len());
                replacement.push(MirOp::Move {
                    dst: MirDef::Reg(*reg),
                    src: MirValue::Def(MirDef::Reg(carrier)),
                    width: MirWidth::Byte,
                });
                if *reg == MirReg::A {
                    a_holds_induction = true;
                }
                if *reg == MirReg::Y {
                    y_holds_induction = true;
                }
            }
            continue;
        }
        if let MirOp::Move {
            dst: MirDef::Reg(reg),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } = op
            && a_holds_induction
            && (*reg == carrier || carrier == MirReg::X && *reg == MirReg::Y)
        {
            if *reg == MirReg::Y {
                y_holds_induction = true;
            }
            continue;
        }
        if operation_references_induction(routine, op, layout, &counted.induction)
            || operation_has_unproven_indirect_alias(routine, op, layout, &counted.induction)
                && !staged_static_store_is_disjoint_from_induction(
                    routine,
                    &original,
                    op_index,
                    layout,
                    &counted.induction,
                )
            || matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::Barrier { .. }
                    | MirOp::MachineBlock { .. }
            )
        {
            return false;
        }

        let original_effects = classify_op(op);
        if register_written_or_clobbered(&original_effects.machine, carrier) {
            return false;
        }
        let mut rewritten = op.clone();
        if carrier == MirReg::X && y_holds_induction {
            replace_y_index_with_x(&mut rewritten);
            if register_read(&classify_op(&rewritten).machine.register_reads, MirReg::Y) {
                return false;
            }
        }
        let rewritten_effects = classify_op(&rewritten);
        if register_written_or_clobbered(&rewritten_effects.machine, carrier) {
            return false;
        }
        if register_written_or_clobbered(&rewritten_effects.machine, MirReg::A) {
            a_holds_induction = false;
        }
        if register_written_or_clobbered(&rewritten_effects.machine, MirReg::Y) {
            y_holds_induction = false;
        }
        old_to_new[op_index] = Some(replacement.len());
        replacement.push(rewritten);
    }

    if !guard {
        if terminator_conflicts_with_carrier(&block.terminator, carrier)
            || !remap_fused_producer(&mut block.terminator, block.id, &old_to_new)
        {
            return false;
        }
    }
    block.ops = replacement;
    true
}

fn staged_static_store_is_disjoint_from_induction(
    routine: &MirRoutine,
    ops: &[MirOp],
    store_index: usize,
    layout: &MaterializeLayout,
    induction: &MirMem,
) -> bool {
    staged_static_store_target(routine, ops, store_index, layout, induction).is_some()
}

fn staged_static_store_target(
    routine: &MirRoutine,
    ops: &[MirOp],
    store_index: usize,
    layout: &MaterializeLayout,
    induction: &MirMem,
) -> Option<(usize, MirMem, MirFixedZpSlot)> {
    let Some(MirOp::StoreIndirect {
        consumer,
        offset: 0,
        ..
    }) = ops.get(store_index)
    else {
        return None;
    };
    let MirPointerPair::Fixed { lo: pair_lo } = consumer.pointer_pair() else {
        return None;
    };
    let pair_hi = MirFixedZpSlot(pair_lo.0.saturating_add(1));
    let pair_homes = [
        crate::mir6502::analysis::effects::MirHomeByte::FixedZeroPage(pair_lo),
        crate::mir6502::analysis::effects::MirHomeByte::FixedZeroPage(pair_hi),
    ];
    if [pair_lo, pair_hi]
        .into_iter()
        .any(|slot| same_physical_byte(routine, layout, &MirMem::FixedZeroPage(slot), induction))
    {
        return None;
    }

    for start in 0..store_index.saturating_sub(5) {
        let Some(base) = cfg_storage_address_byte_to_a(ops.get(start), 0) else {
            continue;
        };
        if cfg_store_a_to_fixed_zp(ops.get(start + 1)) != Some(pair_lo)
            || cfg_storage_address_byte_to_a(ops.get(start + 2), 1).as_ref() != Some(&base)
            || cfg_store_a_to_fixed_zp(ops.get(start + 3)) != Some(pair_hi)
            || !matches!(
                ops.get(start + 4),
                Some(MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                }) if same_physical_byte(routine, layout, mem, induction)
            )
            || !matches!(
                ops.get(start + 5),
                Some(MirOp::AdvanceAddress {
                    consumer: advance_consumer,
                    index: MirValue::Def(MirDef::Reg(MirReg::A)),
                    scale: 1,
                }) if advance_consumer.pointer_pair() == consumer.pointer_pair()
            )
            || !indexed_base_disjoint(routine, layout, &base, MirWidth::Byte, induction)
        {
            continue;
        }
        let pair_untouched = ops[start + 6..store_index].iter().all(|op| {
            let effects = classify_op(op);
            !pair_homes.iter().any(|home| {
                effects.homes.reads.contains(home)
                    || effects.homes.writes.contains(home)
                    || effects.addresses.pair_reads.contains(home)
                    || effects.addresses.pair_writes.contains(home)
            })
        });
        if pair_untouched {
            return Some((start, base, pair_lo));
        }
    }
    None
}

fn indexed_base_disjoint_from_fixed_pair(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
    base: &MirMem,
    pair_lo: MirFixedZpSlot,
) -> bool {
    let Some(start) = layout.mem_address(routine.id, base).map(u32::from) else {
        return false;
    };
    let last = start.saturating_add(u32::from(u8::MAX));
    [pair_lo.0, pair_lo.0.saturating_add(1)]
        .into_iter()
        .map(u32::from)
        .all(|address| address < start || address > last)
}

fn cfg_storage_address_byte_to_a(op: Option<&MirOp>, byte: u8) -> Option<MirMem> {
    let MirOp::Move {
        dst: MirDef::Reg(MirReg::A),
        src: MirValue::StorageAddrByte {
            mem,
            byte: source_byte,
        },
        width: MirWidth::Byte,
    } = op?
    else {
        return None;
    };
    (*source_byte == byte).then(|| mem.clone())
}

fn cfg_store_a_to_fixed_zp(op: Option<&MirOp>) -> Option<MirFixedZpSlot> {
    let MirOp::Store {
        dst: MirAddr::Direct(MirMem::FixedZeroPage(slot)),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    } = op?
    else {
        return None;
    };
    Some(*slot)
}

fn apply_counted_loop_register_carrier(
    routine: &MirRoutine,
    blocks: &mut [MirBlock],
    layout: &MaterializeLayout,
    liveness: &MirMachineLiveness,
    counted: &MirCountedLoop,
    loop_nodes: &BTreeSet<MirBlockId>,
    carrier: MirReg,
) -> bool {
    let Some(initial) = const_byte(&counted.initial_value) else {
        return false;
    };
    let Some(preheader) = blocks
        .iter_mut()
        .find(|block| block.id == counted.preheader)
    else {
        return false;
    };
    if !replace_induction_initialization(preheader, &counted.induction, carrier, initial) {
        return false;
    }

    for block in blocks.iter_mut() {
        if !loop_nodes.contains(&block.id) {
            continue;
        }
        if !block.params.is_empty() || terminator_conflicts_with_carrier(&block.terminator, carrier)
        {
            return false;
        }
        if block.id == counted.header {
            if !replace_induction_header(block, &counted.induction, carrier) {
                return false;
            }
            continue;
        }
        if !rewrite_induction_loop_block(routine, block, layout, liveness, counted, carrier) {
            return false;
        }
    }

    if counted.final_value_observable {
        let Some(exit) = blocks.iter_mut().find(|block| block.id == counted.exit) else {
            return false;
        };
        shift_fused_producer_after_insert(&mut exit.terminator, exit.id, 0, 1);
        exit.ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(counted.induction.clone()),
                src: MirValue::Def(MirDef::Reg(carrier)),
                width: MirWidth::Byte,
            },
        );
    }
    true
}

fn replace_induction_initialization(
    block: &mut MirBlock,
    induction: &MirMem,
    carrier: MirReg,
    initial: u8,
) -> bool {
    let Some(store_index) = block.ops.len().checked_sub(1) else {
        return false;
    };
    let MirOp::Store {
        dst: MirAddr::Direct(mem),
        src,
        width: MirWidth::Byte,
    } = &block.ops[store_index]
    else {
        return false;
    };
    if mem != induction {
        return false;
    }
    let first_removed = if src == &MirValue::ConstU8(initial) {
        store_index
    } else if src == &MirValue::Def(MirDef::Reg(MirReg::A)) && store_index > 0 {
        match &block.ops[store_index - 1] {
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(value),
                width: MirWidth::Byte,
            } if *value == initial => store_index - 1,
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value,
                width: MirWidth::Byte,
            } if *value == u16::from(initial) => store_index - 1,
            _ => return false,
        }
    } else {
        return false;
    };

    block.ops.truncate(first_removed);
    block.ops.push(MirOp::LoadImm {
        dst: MirDef::Reg(carrier),
        value: u16::from(initial),
        width: MirWidth::Byte,
    });
    true
}

fn replace_induction_header(block: &mut MirBlock, induction: &MirMem, carrier: MirReg) -> bool {
    let [
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            dst,
            op,
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right,
            width: MirWidth::Byte,
            signed,
        },
    ] = block.ops.as_slice()
    else {
        return false;
    };
    if mem != induction {
        return false;
    }
    block.ops = vec![MirOp::Compare {
        dst: dst.clone(),
        op: *op,
        left: MirValue::Def(MirDef::Reg(carrier)),
        right: right.clone(),
        width: MirWidth::Byte,
        signed: *signed,
    }];
    shift_fused_producer(&mut block.terminator, block.id, 1, 0);
    true
}

fn rewrite_induction_loop_block(
    routine: &MirRoutine,
    block: &mut MirBlock,
    layout: &MaterializeLayout,
    liveness: &MirMachineLiveness,
    counted: &MirCountedLoop,
    carrier: MirReg,
) -> bool {
    let original = block.ops.clone();
    let mut replacement = Vec::with_capacity(original.len());
    let mut old_to_new = vec![None; original.len()];
    let mut y_holds_induction = false;

    for (op_index, op) in original.iter().enumerate() {
        if let MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } = op
            && mem == &counted.induction
        {
            match (*reg, carrier) {
                (MirReg::A, _) => {
                    old_to_new[op_index] = Some(replacement.len());
                    replacement.push(MirOp::Move {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirValue::Def(MirDef::Reg(carrier)),
                        width: MirWidth::Byte,
                    });
                }
                (MirReg::Y, MirReg::X) | (MirReg::Y, MirReg::Y) => {
                    if !load_result_flags_are_dead(liveness, block.id, op_index) {
                        return false;
                    }
                    y_holds_induction = true;
                }
                (reg, selected) if reg == selected => {
                    if !load_result_flags_are_dead(liveness, block.id, op_index) {
                        return false;
                    }
                }
                _ => return false,
            }
            continue;
        }
        if let MirOp::UpdateMem {
            op: update,
            mem,
            width: MirWidth::Byte,
        } = op
            && block.id == counted.latch
            && op_index + 1 == original.len()
            && mem == &counted.induction
        {
            old_to_new[op_index] = Some(replacement.len());
            replacement.push(MirOp::UpdateReg {
                op: *update,
                reg: carrier,
            });
            continue;
        }
        if operation_references_induction(routine, op, layout, &counted.induction)
            || operation_has_unproven_indirect_alias(routine, op, layout, &counted.induction)
            || matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::Barrier { .. }
                    | MirOp::MachineBlock { .. }
            )
        {
            return false;
        }

        let original_effects = classify_op(op);
        if register_written_or_clobbered(&original_effects.machine, carrier)
            || (register_read(&original_effects.machine.register_reads, carrier)
                && !(carrier == MirReg::Y && y_holds_induction))
        {
            return false;
        }

        let mut rewritten = op.clone();
        if carrier == MirReg::X && y_holds_induction {
            replace_y_index_with_x(&mut rewritten);
            if register_read(&classify_op(&rewritten).machine.register_reads, MirReg::Y) {
                return false;
            }
        }
        let rewritten_effects = classify_op(&rewritten);
        if register_written_or_clobbered(&rewritten_effects.machine, carrier) {
            return false;
        }
        if register_written_or_clobbered(&original_effects.machine, MirReg::Y) {
            y_holds_induction = false;
        }
        old_to_new[op_index] = Some(replacement.len());
        replacement.push(rewritten);
    }

    if block.id == counted.latch
        && !replacement
            .last()
            .is_some_and(|op| matches!(op, MirOp::UpdateReg { reg, .. } if *reg == carrier))
    {
        return false;
    }
    if carrier == MirReg::X
        && y_holds_induction
        && (register_read(
            &classify_terminator(&block.terminator)
                .machine
                .register_reads,
            MirReg::Y,
        ) || liveness
            .live_out(block.id)
            .is_none_or(|live| live.register_live(MirReg::Y)))
    {
        return false;
    }
    if !remap_fused_producer(&mut block.terminator, block.id, &old_to_new) {
        return false;
    }
    block.ops = replacement;
    true
}

fn operation_references_induction(
    routine: &MirRoutine,
    op: &MirOp,
    layout: &MaterializeLayout,
    induction: &MirMem,
) -> bool {
    let mut aliases = false;
    visit_op_mems(op, &mut |mem| {
        aliases |= same_physical_byte(routine, layout, mem, induction);
    });
    aliases
}

fn operation_has_unproven_indirect_alias(
    routine: &MirRoutine,
    op: &MirOp,
    layout: &MaterializeLayout,
    induction: &MirMem,
) -> bool {
    let effects = classify_op(op);
    if !effects.memory.indirect_reads && !effects.memory.indirect_writes {
        return false;
    }
    !match op {
        MirOp::Load { src, width, .. }
        | MirOp::Store {
            dst: src, width, ..
        } => indexed_addr_disjoint(routine, layout, src, *width, induction),
        MirOp::CompareDirectIndexedBytes { left, right, .. } => {
            indexed_base_disjoint(routine, layout, left, MirWidth::Byte, induction)
                && indexed_base_disjoint(routine, layout, right, MirWidth::Byte, induction)
        }
        MirOp::UpdateIndexedMem { base, .. } => {
            indexed_base_disjoint(routine, layout, base, MirWidth::Byte, induction)
        }
        _ => false,
    }
}

fn indexed_addr_disjoint(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
    addr: &MirAddr,
    width: MirWidth,
    induction: &MirMem,
) -> bool {
    match addr {
        MirAddr::AbsoluteIndexedX { base } | MirAddr::AbsoluteIndexedY { base } => {
            indexed_base_disjoint(routine, layout, base, width, induction)
        }
        _ => false,
    }
}

fn indexed_base_disjoint(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
    base: &MirMem,
    width: MirWidth,
    induction: &MirMem,
) -> bool {
    let Some(base) = layout.mem_address(routine.id, base) else {
        return false;
    };
    let Some(induction) = layout.mem_address(routine.id, induction) else {
        return false;
    };
    let last = u32::from(base)
        .saturating_add(255)
        .saturating_add(u32::from(width_bytes(width)).saturating_sub(1));
    if last > u32::from(u16::MAX) {
        return false;
    }
    let induction = u32::from(induction);
    induction < u32::from(base) || induction > last
}

fn same_physical_byte(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
    left: &MirMem,
    right: &MirMem,
) -> bool {
    left == right
        || layout
            .mem_address(routine.id, left)
            .zip(layout.mem_address(routine.id, right))
            .is_some_and(|(left, right)| left == right)
}

fn induction_address_is_taken(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
    induction: &MirMem,
) -> bool {
    routine.blocks.iter().any(|block| {
        block
            .ops
            .iter()
            .any(|op| operation_takes_induction_address(routine, layout, op, induction))
    })
}

fn operation_takes_induction_address(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
    op: &MirOp,
    induction: &MirMem,
) -> bool {
    let value = |value| value_takes_induction_address(routine, layout, value, induction);
    let addr = |addr| addr_takes_induction_address(routine, layout, addr, induction);
    match op {
        MirOp::Load { src, .. } => addr(src),
        MirOp::Store { dst, src, .. } => addr(dst) || value(src),
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => value(src),
        MirOp::LeaAddr { target, .. } => same_physical_byte(routine, layout, target, induction),
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            value(left) || value(right)
        }
        MirOp::AddByteToWordMem { value: rhs, .. }
        | MirOp::SubByteFromWordMem { value: rhs, .. } => value(rhs),
        MirOp::PackedRealCopy {
            source,
            destination,
            ..
        } => addr(source) || addr(destination),
        MirOp::Call { target, args, .. } => {
            let target_takes_address = match target {
                MirCallTarget::Indirect { target, .. } => value(target),
                _ => false,
            };
            target_takes_address || args.iter().any(|arg| value(&arg.value))
        }
        MirOp::MaterializeIndexedAddress { base, index, .. } => value(base) || value(index),
        MirOp::LoadImm { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateReg { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::BinaryDirectIndexedByte { .. }
        | MirOp::OffsetPointerByIndirectByte { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyDirectWordToIndirect { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::AbsoluteWordSubToIndirect { .. }
        | MirOp::CompareDirectIndexedBytes { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::CompareIndirectWords { .. }
        | MirOp::PackedRealCompare { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}

fn addr_takes_induction_address(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
    addr: &MirAddr,
    induction: &MirMem,
) -> bool {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            value_takes_induction_address(routine, layout, base, induction)
                || value_takes_induction_address(routine, layout, index, induction)
        }
        MirAddr::PointerIndex { index, .. } => {
            value_takes_induction_address(routine, layout, index, induction)
        }
        MirAddr::Deref { ptr, .. } => {
            value_takes_induction_address(routine, layout, ptr, induction)
        }
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::PointerCell { .. } => false,
    }
}

fn value_takes_induction_address(
    routine: &MirRoutine,
    layout: &MaterializeLayout,
    value: &MirValue,
    induction: &MirMem,
) -> bool {
    match value {
        MirValue::StorageAddrByte { mem, .. } => {
            same_physical_byte(routine, layout, mem, induction)
        }
        MirValue::GlobalAddr(id) => same_physical_byte(
            routine,
            layout,
            &MirMem::Global { id: *id, offset: 0 },
            induction,
        ),
        MirValue::StaticAddr(id) => same_physical_byte(
            routine,
            layout,
            &MirMem::Static { id: *id, offset: 0 },
            induction,
        ),
        MirValue::Word { lo, hi } => {
            value_takes_induction_address(routine, layout, lo, induction)
                || value_takes_induction_address(routine, layout, hi, induction)
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::PointerCell(_) => false,
    }
}

fn replace_y_index_with_x(op: &mut MirOp) {
    fn addr(addr: &mut MirAddr) {
        if let MirAddr::AbsoluteIndexedY { base } = addr {
            *addr = MirAddr::AbsoluteIndexedX { base: base.clone() };
        }
    }
    match op {
        MirOp::Load { src, .. } => addr(src),
        MirOp::Store { dst, .. } => addr(dst),
        MirOp::CompareDirectIndexedBytes { index, .. } if *index == MirReg::Y => {
            *index = MirReg::X;
        }
        MirOp::BinaryDirectIndexedByte {
            source: MirByteIndexedSource::Absolute { index, .. },
            ..
        } if *index == MirReg::Y => {
            *index = MirReg::X;
        }
        _ => {}
    }
}

fn terminator_conflicts_with_carrier(terminator: &MirTerminator, carrier: MirReg) -> bool {
    let effects = classify_terminator(terminator);
    register_read(&effects.machine.register_reads, carrier)
        || register_written_or_clobbered(&effects.machine, carrier)
}

fn register_read(registers: &crate::mir6502::ir::MirRegisterSet, reg: MirReg) -> bool {
    match reg {
        MirReg::A => registers.a,
        MirReg::X => registers.x,
        MirReg::Y => registers.y,
    }
}

fn register_written_or_clobbered(
    effects: &crate::mir6502::analysis::effects::MirMachineEffects,
    reg: MirReg,
) -> bool {
    register_read(&effects.register_writes, reg)
        || register_read(&effects.register_clobbers, reg)
        || register_read(&effects.conservative_register_clobbers, reg)
}

fn load_result_flags_are_dead(
    liveness: &MirMachineLiveness,
    block: MirBlockId,
    op_index: usize,
) -> bool {
    liveness
        .flags_dead_after(
            MirFlagSet {
                z: true,
                n: true,
                ..MirFlagSet::default()
            },
            MirSite::Op { block, op_index },
        )
        .unwrap_or(false)
}

fn remap_fused_producer(
    terminator: &mut MirTerminator,
    block: MirBlockId,
    old_to_new: &[Option<usize>],
) -> bool {
    if let MirTerminator::Branch {
        cond: MirCond::FusedCompare { producer, .. },
        ..
    } = terminator
        && producer.block == block
    {
        let Some(Some(new_index)) = old_to_new.get(producer.op_index) else {
            return false;
        };
        producer.op_index = *new_index;
    }
    true
}

fn shift_fused_producer(
    terminator: &mut MirTerminator,
    block: MirBlockId,
    old_index: usize,
    new_index: usize,
) {
    if let MirTerminator::Branch {
        cond: MirCond::FusedCompare { producer, .. },
        ..
    } = terminator
        && producer.block == block
        && producer.op_index == old_index
    {
        producer.op_index = new_index;
    }
}

fn shift_fused_producer_after_insert(
    terminator: &mut MirTerminator,
    block: MirBlockId,
    inserted_at: usize,
    amount: usize,
) {
    if let MirTerminator::Branch {
        cond: MirCond::FusedCompare { producer, .. },
        ..
    } = terminator
        && producer.block == block
        && producer.op_index >= inserted_at
    {
        producer.op_index = producer.op_index.saturating_add(amount);
    }
}

fn const_byte(value: &MirValue) -> Option<u8> {
    match value {
        MirValue::ConstU8(value) => Some(*value),
        MirValue::ConstU16(value) => u8::try_from(*value).ok(),
        _ => None,
    }
}

fn width_bytes(width: MirWidth) -> u8 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

#[derive(Debug, Clone, Copy)]
enum CountedLoopLatchPlan {
    HeadTested(InitializedByteCountdownPlan),
    RotatedHeadTested(RotatedHeadTestedPlan),
    BottomFast(BottomFastByteCountdownPlan),
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
    remove_entry_header: bool,
    reload_accumulator: bool,
}

#[derive(Debug, Clone, Copy)]
struct BottomFastByteCountdownPlan {
    preheader: MirBlockId,
    header: MirBlockId,
    body: MirBlockId,
    guard: MirBlockId,
    latch: MirBlockId,
    restore: MirBlockId,
    split_guard_prefix: Option<usize>,
    restore_accumulator: bool,
    remove_entry_header: bool,
    reload_accumulator: bool,
}

#[derive(Debug, Clone, Copy)]
enum BottomFastBodyShape {
    Existing,
    SplitGuard { prefix_len: usize },
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
            || !counted_loop_mem_allows_rmw(&counted, layout)
        {
            continue;
        }
        match counted.shape {
            MirCountedLoopShape::HeadTested
                if counted.direction == MirCountDirection::Descending && counted.bound == 1 =>
            {
                if counted.initial_guard_required
                    || !machine_state_dead_on_entry(&liveness, counted.body)
                    || !machine_state_dead_on_entry(&liveness, counted.exit)
                {
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
                if counted.initial_guard_required
                    || !machine_state_dead_on_entry(&liveness, counted.body)
                    || !machine_state_dead_on_entry(&liveness, counted.exit)
                {
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
            MirCountedLoopShape::FullRangeAscending { .. } => {}
            MirCountedLoopShape::BottomGuarded { guard } => {
                if !routine.blocks.iter().any(|block| block.id == guard)
                    || counted.bound > 0 && !machine_flags_dead_on_entry(&liveness, counted.body)
                    || !machine_flag_unobserved_before_redefinition(
                        routine,
                        &cfg,
                        counted.body,
                        counted.latch,
                        MirFlag::V,
                    )
                {
                    continue;
                }
                let remove_entry_header = !counted.initial_guard_required
                    && machine_flags_dead_on_entry(&liveness, counted.body);
                let guarded_reload_accumulator =
                    machine_accumulator_live_on_entry(&liveness, counted.body);
                if counted.bound == 0
                    && let Some(body_shape) =
                        bottom_fast_countdown_shape(routine, &cfg, &counted, guard)
                    && let Some((body, restore, split_guard_prefix)) =
                        bottom_fast_block_ids(routine, &counted, body_shape)
                {
                    let reload_accumulator = match body_shape {
                        BottomFastBodyShape::Existing => guarded_reload_accumulator,
                        BottomFastBodyShape::SplitGuard { prefix_len } => routine
                            .blocks
                            .iter()
                            .find(|block| block.id == guard)
                            .is_none_or(|block| accumulator_live_in_ops(&block.ops[..prefix_len])),
                    };
                    let Some(guard_block) = routine.blocks.iter().find(|block| block.id == guard)
                    else {
                        continue;
                    };
                    let plan = BottomFastByteCountdownPlan {
                        preheader: counted.preheader,
                        header: counted.header,
                        body,
                        guard,
                        latch: counted.latch,
                        restore,
                        split_guard_prefix,
                        restore_accumulator: !bottom_guard_reloads_counter(
                            guard_block,
                            &counted.induction,
                        ),
                        remove_entry_header,
                        reload_accumulator,
                    };
                    let mut candidate = routine.blocks.clone();
                    const MAX_FAST_COUNTDOWN_SIZE_GROWTH: usize = 6;
                    let applied = apply_bottom_fast_countdown_plan(&mut candidate, plan);
                    let candidate_cost = estimated_routine_layout_bytes(&candidate);
                    let original_cost = estimated_routine_layout_bytes(&routine.blocks);
                    if applied
                        && candidate_cost
                            <= original_cost.saturating_add(MAX_FAST_COUNTDOWN_SIZE_GROWTH)
                    {
                        return Some(CountedLoopLatchPlan::BottomFast(plan));
                    }
                }
                let plan = BottomGuardedByteCountdownPlan {
                    preheader: counted.preheader,
                    header: counted.header,
                    body: counted.body,
                    latch: counted.latch,
                    remove_entry_header,
                    reload_accumulator: guarded_reload_accumulator,
                };
                let mut candidate = routine.blocks.clone();
                if apply_bottom_guarded_countdown_plan(&mut candidate, plan)
                    && estimated_routine_layout_bytes(&candidate)
                        < estimated_routine_layout_bytes(&routine.blocks)
                {
                    return Some(CountedLoopLatchPlan::BottomGuarded(plan));
                }
            }
        }
    }
    None
}

fn counted_loop_mem_allows_rmw(counted: &MirCountedLoop, layout: &MaterializeLayout) -> bool {
    layout.mem_allows_direct_update(&counted.induction)
}

fn bottom_fast_countdown_shape(
    routine: &MirRoutine,
    cfg: &MirCfg,
    counted: &MirCountedLoop,
    guard: MirBlockId,
) -> Option<BottomFastBodyShape> {
    let initial = match counted.initial_value {
        MirValue::ConstU8(value) => value,
        MirValue::ConstU16(value) if value <= u16::from(u8::MAX) => value as u8,
        _ => return None,
    };
    // Zero- and one-start loops do not execute enough continued latches to
    // repay the terminal restoration path selected below.
    if !(2..=i8::MAX as u8).contains(&initial) || counted.body == counted.latch {
        return None;
    }
    if counted.body == guard {
        if cfg.predecessors(guard) != &BTreeSet::from([counted.header]) {
            return None;
        }
        let block = routine.blocks.iter().find(|block| block.id == guard)?;
        let prefix_len = bottom_guard_prefix_len(block, &counted.induction, counted.bound)?;
        if block.ops[..prefix_len]
            .iter()
            .any(|op| op_may_write_mem(op, &counted.induction))
        {
            return None;
        }
        return Some(BottomFastBodyShape::SplitGuard { prefix_len });
    }
    let guard_block = routine.blocks.iter().find(|block| block.id == guard)?;
    if bottom_guard_prefix_len(guard_block, &counted.induction, counted.bound)? != 0 {
        return None;
    }
    let Some(body_blocks) = bottom_guarded_body_blocks(routine, cfg, counted, guard) else {
        return None;
    };
    body_blocks
        .iter()
        .all(|block_id| {
            routine
                .blocks
                .iter()
                .find(|block| block.id == *block_id)
                .is_some_and(|block| {
                    block
                        .ops
                        .iter()
                        .all(|op| !op_may_write_mem(op, &counted.induction))
                })
        })
        .then_some(BottomFastBodyShape::Existing)
}

fn bottom_fast_block_ids(
    routine: &MirRoutine,
    counted: &MirCountedLoop,
    shape: BottomFastBodyShape,
) -> Option<(MirBlockId, MirBlockId, Option<usize>)> {
    let first = routine
        .blocks
        .iter()
        .map(|block| block.id.0)
        .max()
        .unwrap_or(0)
        .checked_add(1)?;
    match shape {
        BottomFastBodyShape::Existing => Some((counted.body, MirBlockId(first), None)),
        BottomFastBodyShape::SplitGuard { prefix_len } => Some((
            MirBlockId(first),
            MirBlockId(first.checked_add(1)?),
            Some(prefix_len),
        )),
    }
}

fn accumulator_live_in_ops(ops: &[MirOp]) -> bool {
    for op in ops {
        let effects = classify_op(op);
        if register_read(&effects.machine.register_reads, MirReg::A)
            || matches!(op, MirOp::MachineBlock { .. }) && effects.machine.opaque_flag_or_a_effects
        {
            return true;
        }
        if register_written_or_clobbered(&effects.machine, MirReg::A) {
            return false;
        }
    }
    false
}

fn bottom_guard_prefix_len(block: &MirBlock, induction: &MirMem, bound: u16) -> Option<usize> {
    let compare_index = block.ops.len().checked_sub(1)?;
    let guard_limit = u8::try_from(bound.checked_add(1)?).ok()?;
    let MirOp::Compare {
        op: crate::mir6502::ir::MirCompareOp::Lt,
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::ConstU8(limit),
        width: MirWidth::Byte,
        signed: false,
        ..
    } = &block.ops[compare_index]
    else {
        return None;
    };
    if *limit != guard_limit {
        return None;
    }
    match compare_index
        .checked_sub(1)
        .and_then(|index| block.ops.get(index))
    {
        Some(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }) if mem == induction => compare_index.checked_sub(1),
        _ => Some(compare_index),
    }
}

fn bottom_guard_reloads_counter(block: &MirBlock, induction: &MirMem) -> bool {
    block
        .ops
        .len()
        .checked_sub(2)
        .and_then(|index| block.ops.get(index))
        .is_some_and(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if mem == induction
            )
        })
}

fn bottom_guarded_body_blocks(
    routine: &MirRoutine,
    cfg: &MirCfg,
    counted: &MirCountedLoop,
    guard: MirBlockId,
) -> Option<BTreeSet<MirBlockId>> {
    let boundaries = BTreeSet::from([counted.header, guard, counted.latch, counted.exit]);
    let mut blocks = BTreeSet::new();
    let mut pending = vec![counted.body];
    while let Some(block) = pending.pop() {
        if boundaries.contains(&block) || !blocks.insert(block) {
            continue;
        }
        if !routine.blocks.iter().any(|candidate| candidate.id == block) {
            return None;
        }
        pending.extend(cfg.successors(block).iter().copied());
    }
    if blocks.is_empty()
        || cfg
            .predecessors(guard)
            .iter()
            .any(|predecessor| !blocks.contains(predecessor))
    {
        return None;
    }
    for block in &blocks {
        if cfg.predecessors(*block).iter().any(|predecessor| {
            !blocks.contains(predecessor)
                && !(*block == counted.body && *predecessor == counted.header)
        }) {
            return None;
        }
        if cfg.successors(*block).iter().any(|successor| {
            !blocks.contains(successor) && *successor != guard && *successor != counted.exit
        }) {
            return None;
        }
    }
    Some(blocks)
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
    if plan.remove_entry_header {
        blocks[preheader_index].terminator = MirTerminator::Jump(MirEdge::plain(plan.body));
    }
    blocks[latch_index].ops = vec![MirOp::UpdateMem {
        op: MirUpdateOp::Dec,
        mem: induction.clone(),
        width: MirWidth::Byte,
    }];
    if plan.reload_accumulator {
        blocks[latch_index].ops.push(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(induction.clone()),
            width: MirWidth::Byte,
        });
    }
    blocks[latch_index].terminator = MirTerminator::Jump(MirEdge::plain(plan.body));
    if plan.remove_entry_header {
        blocks.remove(header_index);
    }
    true
}

fn apply_bottom_fast_countdown_plan(
    blocks: &mut Vec<MirBlock>,
    plan: BottomFastByteCountdownPlan,
) -> bool {
    let Some(preheader_index) = blocks.iter().position(|block| block.id == plan.preheader) else {
        return false;
    };
    let Some(header_index) = blocks.iter().position(|block| block.id == plan.header) else {
        return false;
    };
    if blocks.iter().any(|block| block.id == plan.restore) {
        return false;
    }
    let Some(latch_index) = blocks.iter().position(|block| block.id == plan.latch) else {
        return false;
    };
    let Some(induction) = bottom_guarded_latch_mem(&blocks[latch_index], plan.header) else {
        return false;
    };
    if plan.remove_entry_header {
        blocks[preheader_index].terminator = MirTerminator::Jump(MirEdge::plain(plan.body));
    }
    let split_body = if let Some(prefix_len) = plan.split_guard_prefix {
        let Some(guard_index) = blocks.iter().position(|block| block.id == plan.guard) else {
            return false;
        };
        if prefix_len > blocks[guard_index].ops.len() {
            return false;
        }
        let guard_ops = blocks[guard_index].ops.split_off(prefix_len);
        let body_ops = std::mem::replace(&mut blocks[guard_index].ops, guard_ops);
        redirect_block_target(&mut blocks[header_index].terminator, plan.guard, plan.body);
        Some(MirBlock {
            id: plan.body,
            label: "countdown.body".to_string(),
            params: Vec::new(),
            ops: body_ops,
            terminator: MirTerminator::Jump(MirEdge::plain(plan.latch)),
        })
    } else {
        for block in blocks.iter_mut() {
            redirect_block_target(&mut block.terminator, plan.guard, plan.latch);
        }
        None
    };
    blocks[latch_index].ops = vec![MirOp::UpdateMem {
        op: MirUpdateOp::Dec,
        mem: induction.clone(),
        width: MirWidth::Byte,
    }];
    if plan.reload_accumulator {
        blocks[latch_index].ops.push(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(induction.clone()),
            width: MirWidth::Byte,
        });
    }
    blocks[latch_index].terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(MirFlagTest::NClear),
        then_edge: MirEdge::plain(plan.body),
        else_edge: MirEdge::plain(plan.restore),
    };
    let mut restore_index = latch_index + 1;
    if let Some(body) = split_body {
        blocks.insert(latch_index, body);
        restore_index += 1;
    }
    let restore_ops = if plan.restore_accumulator {
        vec![
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: 0,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(induction),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ]
    } else {
        vec![MirOp::UpdateMem {
            op: MirUpdateOp::Inc,
            mem: induction,
            width: MirWidth::Byte,
        }]
    };
    blocks.insert(
        restore_index,
        MirBlock {
            id: plan.restore,
            label: "countdown.restore".to_string(),
            params: Vec::new(),
            ops: restore_ops,
            terminator: MirTerminator::Jump(MirEdge::plain(plan.guard)),
        },
    );
    if plan.remove_entry_header {
        let Some(index) = blocks.iter().position(|block| block.id == plan.header) else {
            return false;
        };
        blocks.remove(index);
    }
    true
}

fn redirect_block_target(
    terminator: &mut MirTerminator,
    original: MirBlockId,
    replacement: MirBlockId,
) {
    match terminator {
        MirTerminator::Jump(edge) => {
            if edge.target == original {
                edge.target = replacement;
            }
        }
        MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            if then_edge.target == original {
                then_edge.target = replacement;
            }
            if else_edge.target == original {
                else_edge.target = replacement;
            }
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
    }
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
        [
            MirOp::Binary {
                op: crate::mir6502::ir::MirBinaryOp::Sub,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: None | Some(crate::mir6502::ir::MirCarryIn::Set),
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

/// Proves that a changed machine flag cannot be observed after a selected
/// latch before an ordinary MIR operation definitely replaces it. Compiler
/// barriers emit no machine instruction, so their deliberately opaque memory
/// effects do not make an incoming flag value observable here.
fn machine_flag_unobserved_before_redefinition(
    routine: &MirRoutine,
    cfg: &MirCfg,
    start: MirBlockId,
    redefining_latch: MirBlockId,
    flag: MirFlag,
) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(block_id) = pending.pop() {
        if block_id == redefining_latch || !visited.insert(block_id) {
            continue;
        }
        let Some(block) = routine.blocks.iter().find(|block| block.id == block_id) else {
            return false;
        };
        let mut redefined = false;
        for op in &block.ops {
            if matches!(op, MirOp::Barrier { .. }) {
                continue;
            }
            let effects = classify_op(op);
            if effects.machine.flag_reads.contains(flag)
                || matches!(op, MirOp::MachineBlock { .. })
                    && effects.machine.opaque_flag_or_a_effects
            {
                return false;
            }
            if effects.machine.flag_writes.contains(flag)
                || effects.machine.flag_clobbers.contains(flag)
            {
                redefined = true;
                break;
            }
        }
        if redefined {
            continue;
        }
        let terminator = classify_terminator(&block.terminator);
        if terminator.machine.flag_reads.contains(flag) {
            return false;
        }
        pending.extend(cfg.successors(block_id).iter().copied());
    }
    true
}

fn machine_register_unobserved_before_redefinition(
    routine: &MirRoutine,
    cfg: &MirCfg,
    start: MirBlockId,
    register: MirReg,
) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(block_id) = pending.pop() {
        if !visited.insert(block_id) {
            continue;
        }
        let Some(block) = routine.blocks.iter().find(|block| block.id == block_id) else {
            return false;
        };
        let mut redefined = false;
        for op in &block.ops {
            if matches!(op, MirOp::Barrier { .. }) {
                continue;
            }
            let effects = classify_op(op);
            if register_read(&effects.machine.register_reads, register)
                || matches!(op, MirOp::MachineBlock { .. })
                    && register_read(&effects.machine.conservative_register_clobbers, register)
            {
                return false;
            }
            if register_read(&effects.machine.register_writes, register)
                || register_read(&effects.machine.register_clobbers, register)
            {
                redefined = true;
                break;
            }
        }
        if redefined {
            continue;
        }
        let terminator = classify_terminator(&block.terminator);
        if register_read(&terminator.machine.register_reads, register) {
            return false;
        }
        pending.extend(cfg.successors(block_id).iter().copied());
    }
    true
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

fn machine_accumulator_dead_on_entry(liveness: &MirMachineLiveness, block: MirBlockId) -> bool {
    !machine_accumulator_live_on_entry(liveness, block)
}

fn machine_register_dead_on_entry(
    liveness: &MirMachineLiveness,
    block: MirBlockId,
    reg: MirReg,
) -> bool {
    liveness
        .live_in(block)
        .is_some_and(|live| !live.register_live(reg))
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

fn machine_flag_dead_on_entry(
    liveness: &MirMachineLiveness,
    block: MirBlockId,
    flag: MirFlag,
) -> bool {
    liveness
        .live_in(block)
        .is_some_and(|live| !live.flag_live(flag))
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
        bottom_guarded_countdown_to(initial, 0, guard_reloads_counter)
    }

    fn bottom_guarded_countdown_to(
        initial: MirValue,
        bound: u8,
        guard_reloads_counter: bool,
    ) -> MirRoutine {
        assert!(bound < u8::MAX);
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
                        right: MirValue::ConstU8(bound),
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
            right: MirValue::ConstU8(bound + 1),
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
            let latch = routine
                .blocks
                .iter()
                .find(|block| block.id == MirBlockId(4))
                .expect("countdown latch");
            assert!(matches!(
                latch.ops.as_slice(),
                [MirOp::UpdateMem {
                    op: MirUpdateOp::Dec,
                    width: MirWidth::Byte,
                    ..
                }]
            ));
            if (2..=127).contains(&initial) {
                assert!(routine.blocks.iter().any(|block| block.id == MirBlockId(3)));
                assert!(matches!(
                    latch.terminator,
                    MirTerminator::Branch {
                        cond: MirCond::FlagTest(MirFlagTest::NClear),
                        ..
                    }
                ));
                assert!(routine.blocks.iter().any(|block| {
                    matches!(
                        block.ops.as_slice(),
                        [MirOp::UpdateMem {
                            op: MirUpdateOp::Inc,
                            width: MirWidth::Byte,
                            ..
                        }]
                    ) && matches!(
                        block.terminator,
                        MirTerminator::Jump(ref edge) if edge.target == MirBlockId(3)
                    )
                }));
            } else {
                assert!(routine.blocks.iter().any(|block| block.id == MirBlockId(3)));
                assert!(matches!(latch.terminator, MirTerminator::Jump(_)));
            }
        }
    }

    #[test]
    fn bottom_guarded_countdowns_select_dec_for_constant_lower_bounds() {
        for bound in [1u8, 3, 127, 254] {
            let mut routine = bottom_guarded_countdown_to(
                MirValue::ConstU8(bound.saturating_add(1)),
                bound,
                true,
            );
            let layout = layout_for(&routine);

            assert_eq!(select_counted_loop_latches(&mut routine, &layout), 1);
            assert!(!routine.blocks.iter().any(|block| block.id == MirBlockId(1)));
            let guard = routine
                .blocks
                .iter()
                .find(|block| block.id == MirBlockId(3))
                .expect("lower-bound guard");
            assert!(matches!(
                guard.ops.as_slice(),
                [
                    MirOp::Load { .. },
                    MirOp::Compare {
                        op: MirCompareOp::Lt,
                        right: MirValue::ConstU8(limit),
                        ..
                    }
                ] if *limit == bound + 1
            ));
            let latch = routine
                .blocks
                .iter()
                .find(|block| block.id == MirBlockId(4))
                .expect("countdown latch");
            assert!(matches!(
                latch.ops.as_slice(),
                [MirOp::UpdateMem {
                    op: MirUpdateOp::Dec,
                    width: MirWidth::Byte,
                    ..
                }]
            ));
            assert!(matches!(
                latch.terminator,
                MirTerminator::Jump(ref edge) if edge.target == MirBlockId(2)
            ));
        }

        let mut dynamic =
            bottom_guarded_countdown_to(MirValue::Def(MirDef::Reg(MirReg::X)), 3, true);
        let layout = layout_for(&dynamic);
        assert_eq!(select_counted_loop_latches(&mut dynamic, &layout), 1);
        assert!(dynamic.blocks.iter().any(|block| block.id == MirBlockId(1)));
        assert!(matches!(
            dynamic
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

    #[test]
    fn bottom_guarded_countdown_reloads_a_only_when_the_body_needs_it() {
        let mut routine = bottom_guarded_countdown(MirValue::ConstU8(1), false);
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

        assert_eq!(select_counted_loop_latches(&mut routine, &layout), 1);
        assert!(routine.blocks.iter().any(|block| block.id == MirBlockId(1)));
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

    #[test]
    fn fast_bottom_countdown_restores_observable_final_value_and_rejects_mutation() {
        let counter = MirMem::Local {
            id: crate::nir::LocalId(0),
            offset: 0,
        };

        let mut observable = bottom_guarded_countdown(MirValue::ConstU8(9), true);
        observable
            .blocks
            .iter_mut()
            .find(|block| block.id == MirBlockId(5))
            .expect("countdown exit")
            .ops
            .push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(counter.clone()),
                width: MirWidth::Byte,
            });
        let layout = layout_for(&observable);
        assert_eq!(select_counted_loop_latches(&mut observable, &layout), 1);
        let latch = observable
            .blocks
            .iter()
            .find(|block| block.id == MirBlockId(4))
            .expect("countdown latch");
        assert!(matches!(
            latch.terminator,
            MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::NClear),
                ..
            }
        ));
        assert!(observable.blocks.iter().any(|block| {
            matches!(
                block.ops.as_slice(),
                [MirOp::UpdateMem {
                    op: MirUpdateOp::Inc,
                    ..
                }]
            )
        }));

        let mut mutated = bottom_guarded_countdown(MirValue::ConstU8(9), true);
        mutated
            .blocks
            .iter_mut()
            .find(|block| block.id == MirBlockId(2))
            .expect("countdown body")
            .ops
            .push(MirOp::Store {
                dst: MirAddr::Direct(counter),
                src: MirValue::ConstU8(0),
                width: MirWidth::Byte,
            });
        let layout = layout_for(&mutated);
        assert_eq!(select_counted_loop_latches(&mut mutated, &layout), 1);
        assert!(mutated.blocks.iter().any(|block| block.id == MirBlockId(3)));
    }

    #[test]
    fn fast_bottom_countdown_rejects_an_unproven_aliasing_barrier() {
        let mut routine = bottom_guarded_countdown(MirValue::ConstU8(9), true);
        routine
            .blocks
            .iter_mut()
            .find(|block| block.id == MirBlockId(2))
            .expect("countdown body")
            .ops
            .push(MirOp::Barrier {
                effects: crate::mir6502::ir::MirEffects {
                    opaque: true,
                    ..Default::default()
                },
            });
        let layout = layout_for(&routine);

        assert!(select_counted_loop_latches(&mut routine, &layout) <= 1);
        assert!(routine.blocks.iter().any(|block| block.id == MirBlockId(3)));
        assert!(routine.blocks.iter().all(|block| {
            !matches!(
                block.terminator,
                MirTerminator::Branch {
                    cond: MirCond::FlagTest(MirFlagTest::NClear),
                    ..
                }
            )
        }));
    }

    fn ascending_head_tested_loop(initial: u8, exclusive_bound: u8) -> MirRoutine {
        let counter = MirMem::Absolute(0x0080);
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

    fn full_range_ascending_loop(body_prefix: Vec<MirOp>, exit_ops: Vec<MirOp>) -> MirRoutine {
        let counter = MirMem::Absolute(0x0080);
        let mut body_ops = body_prefix;
        body_ops.extend([
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(counter.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Ge,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(u8::MAX),
                width: MirWidth::Byte,
                signed: false,
            },
        ]);
        MirRoutine {
            id: crate::mir6502::ir::RoutineId(0),
            name: "full_range_ascending_loop".to_string(),
            abi: crate::mir6502::ir::MirRoutineAbi::Action,
            frame: Default::default(),
            temps: Vec::new(),
            blocks: vec![
                block(
                    0,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(counter.clone()),
                        src: MirValue::ConstU8(0),
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
                            op: MirCompareOp::Le,
                            left: MirValue::Def(MirDef::Reg(MirReg::A)),
                            right: MirValue::ConstU8(u8::MAX),
                            width: MirWidth::Byte,
                            signed: false,
                        },
                    ],
                    MirTerminator::Branch {
                        cond: MirCond::AnyFlagTest([MirFlagTest::CClear, MirFlagTest::ZSet]),
                        then_edge: MirEdge::plain(MirBlockId(2)),
                        else_edge: MirEdge::plain(MirBlockId(4)),
                    },
                ),
                block(
                    2,
                    body_ops,
                    MirTerminator::Branch {
                        cond: MirCond::FlagTest(MirFlagTest::CSet),
                        then_edge: MirEdge::plain(MirBlockId(4)),
                        else_edge: MirEdge::plain(MirBlockId(3)),
                    },
                ),
                block(
                    3,
                    vec![MirOp::UpdateMem {
                        op: MirUpdateOp::Inc,
                        mem: counter,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                ),
                block(4, exit_ops, MirTerminator::Return),
            ],
            effects: Default::default(),
        }
    }

    fn indexed_full_range_body_prefix() -> Vec<MirOp> {
        vec![
            MirOp::Move {
                dst: MirDef::Reg(MirReg::Y),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::AbsoluteIndexedY {
                    base: MirMem::Absolute(0x4000),
                },
                width: MirWidth::Byte,
            },
        ]
    }

    #[test]
    fn full_range_byte_loop_carrier_selects_y_and_wraps_with_bne() {
        let mut routine = full_range_ascending_loop(indexed_full_range_body_prefix(), Vec::new());
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            1
        );
        assert!(!routine.blocks.iter().any(|block| block.id == MirBlockId(1)));
        assert!(!routine.blocks.iter().any(|block| block.id == MirBlockId(3)));
        assert!(matches!(
            routine.blocks[0].ops.last(),
            Some(MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::Y),
                value: 0,
                width: MirWidth::Byte,
            })
        ));
        let body = routine
            .blocks
            .iter()
            .find(|block| block.id == MirBlockId(2))
            .expect("full-range body");
        assert!(body.ops.iter().all(|op| {
            !matches!(
                op,
                MirOp::Load {
                    src: MirAddr::Direct(MirMem::Absolute(0x0080)),
                    ..
                } | MirOp::UpdateMem {
                    mem: MirMem::Absolute(0x0080),
                    ..
                }
            )
        }));
        assert!(matches!(
            body.ops.last(),
            Some(MirOp::UpdateReg {
                op: MirUpdateOp::Inc,
                reg: MirReg::Y,
            })
        ));
        assert!(matches!(
            body.terminator,
            MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::ZClear),
                ref then_edge,
                ref else_edge,
            } if then_edge.target == MirBlockId(2) && else_edge.target == MirBlockId(4)
        ));
    }

    #[test]
    fn full_range_byte_loop_restores_an_observable_final_255() {
        let counter = MirMem::Absolute(0x0080);
        let mut routine = full_range_ascending_loop(
            indexed_full_range_body_prefix(),
            vec![MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(counter.clone()),
                width: MirWidth::Byte,
            }],
        );
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            1
        );
        assert!(routine.blocks.iter().any(|block| {
            matches!(
                block.ops.as_slice(),
                [
                    MirOp::LoadImm {
                        dst: MirDef::Reg(MirReg::A),
                        value: 255,
                        ..
                    },
                    MirOp::Store {
                        dst: MirAddr::Direct(mem),
                        ..
                    }
                ] if mem == &counter
            )
        }));
    }

    #[test]
    fn full_range_byte_loop_restores_current_value_on_early_exit() {
        let counter = MirMem::Absolute(0x0080);
        let mut routine = full_range_ascending_loop(
            indexed_full_range_body_prefix(),
            vec![MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(counter.clone()),
                width: MirWidth::Byte,
            }],
        );
        let body = routine
            .blocks
            .iter_mut()
            .find(|block| block.id == MirBlockId(2))
            .expect("full-range body");
        let guard_ops = body.ops.split_off(body.ops.len() - 2);
        body.ops.extend([
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: 200,
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(200),
                width: MirWidth::Byte,
                signed: false,
            },
        ]);
        body.terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet),
            then_edge: MirEdge::plain(MirBlockId(4)),
            else_edge: MirEdge::plain(MirBlockId(5)),
        };
        routine.blocks.push(block(
            5,
            guard_ops,
            MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::CSet),
                then_edge: MirEdge::plain(MirBlockId(4)),
                else_edge: MirEdge::plain(MirBlockId(3)),
            },
        ));
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            1
        );
        assert!(routine.blocks.iter().any(|block| {
            block.label == "full_range.restore_normal"
                && matches!(
                    block.ops.as_slice(),
                    [
                        MirOp::LoadImm {
                            dst: MirDef::Reg(MirReg::A),
                            value: 255,
                            ..
                        },
                        MirOp::Store {
                            dst: MirAddr::Direct(mem),
                            ..
                        }
                    ] if mem == &counter
                )
        }));
        assert!(routine.blocks.iter().any(|block| {
            block.label == "full_range.restore_exit"
                && matches!(
                    block.ops.as_slice(),
                    [MirOp::Store {
                        dst: MirAddr::Direct(mem),
                        src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                        ..
                    }] if mem == &counter
                )
        }));
    }

    #[test]
    fn full_range_byte_loop_rejects_an_effect_barrier() {
        let mut prefix = indexed_full_range_body_prefix();
        prefix.push(MirOp::Barrier {
            effects: Default::default(),
        });
        let mut routine = full_range_ascending_loop(prefix, Vec::new());
        let original = routine.blocks.clone();
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            0
        );
        assert_eq!(routine.blocks, original);
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

    #[test]
    fn counted_loop_carrier_rewrites_induction_loads_and_direct_indexes() {
        let counter = MirMem::Absolute(0x0080);
        let mut routine = ascending_head_tested_loop(0, 8);
        routine.blocks[2].ops = vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(counter.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::AbsoluteIndexedY {
                    base: MirMem::Absolute(0x4000),
                },
                width: MirWidth::Byte,
            },
            MirOp::UpdateMem {
                op: MirUpdateOp::Inc,
                mem: counter,
                width: MirWidth::Byte,
            },
        ];
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            1
        );
        assert!(matches!(
            routine.blocks[0].ops.as_slice(),
            [MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::X),
                value: 0,
                width: MirWidth::Byte,
            }]
        ));
        assert!(matches!(
            routine.blocks[1].ops.as_slice(),
            [MirOp::Compare {
                left: MirValue::Def(MirDef::Reg(MirReg::X)),
                ..
            }]
        ));
        assert!(matches!(
            routine.blocks[2].ops.as_slice(),
            [
                MirOp::Load {
                    src: MirAddr::AbsoluteIndexedX { .. },
                    ..
                },
                MirOp::UpdateReg {
                    op: MirUpdateOp::Inc,
                    reg: MirReg::X,
                }
            ]
        ));
    }

    #[test]
    fn counted_loop_carrier_writes_back_an_observable_final_value() {
        let counter = MirMem::Absolute(0x0080);
        let mut routine = ascending_head_tested_loop(0, 8);
        routine.blocks[3].ops.push(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(counter.clone()),
            width: MirWidth::Byte,
        });
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            1
        );
        assert!(matches!(
            routine.blocks[3].ops.first(),
            Some(MirOp::Store {
                dst: MirAddr::Direct(mem),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
            }) if mem == &counter
        ));
    }

    #[test]
    fn counted_loop_carrier_rejects_effect_barriers() {
        let mut routine = ascending_head_tested_loop(0, 8);
        routine.blocks[2].ops.insert(
            0,
            MirOp::Barrier {
                effects: Default::default(),
            },
        );
        let original = routine.blocks.clone();
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            0
        );
        assert_eq!(routine.blocks, original);
    }

    #[test]
    fn counted_loop_carrier_rejects_an_address_taken_induction_home() {
        let counter = MirMem::Absolute(0x0080);
        let mut routine = ascending_head_tested_loop(0, 8);
        routine.blocks[3].ops.push(MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::StorageAddrByte {
                mem: counter,
                byte: 0,
            },
            width: MirWidth::Byte,
        });
        let original = routine.blocks.clone();
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            0
        );
        assert_eq!(routine.blocks, original);
    }

    #[test]
    fn counted_loop_carrier_rejects_a_wrapping_index_alias() {
        let counter = MirMem::Absolute(0x0080);
        let mut routine = ascending_head_tested_loop(0, 8);
        routine.blocks[2].ops = vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(counter.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::AbsoluteIndexedY {
                    base: MirMem::Absolute(0xff80),
                },
                width: MirWidth::Byte,
            },
            MirOp::UpdateMem {
                op: MirUpdateOp::Inc,
                mem: counter,
                width: MirWidth::Byte,
            },
        ];
        let original = routine.blocks.clone();
        let layout = layout_for(&routine);

        assert_eq!(
            select_counted_loop_register_carriers(&mut routine, &layout),
            0
        );
        assert_eq!(routine.blocks, original);
    }
}
