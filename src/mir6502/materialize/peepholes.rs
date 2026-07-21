use super::store_consumers::try_fold_direct_inc_dec_update;
use super::*;
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::ir::{MirRegisterSet, MirRoutine};
use crate::mir6502::rewrite::context::{MirExitStateChange, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::MirPostHomeRewritePlan;
use crate::mir6502::rewrite::posthome::structural_plan;

fn fold_direct_inc_dec_updates(ops: Vec<MirOp>, terminator: &MirTerminator) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some(consumed) = try_fold_direct_inc_dec_update(&ops, index, terminator, &mut out) {
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

pub(super) fn fold_structural_prefix(ops: Vec<MirOp>, terminator: &MirTerminator) -> Vec<MirOp> {
    fold_direct_inc_dec_updates(ops, terminator)
}

#[cfg(test)]
pub(super) fn fold_structural_peepholes(
    mut ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
    enable_direct_byte_word_update: bool,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    ops = forward_block_local_spill_accumulator(ops, terminator);
    ops = fold_structural_prefix(ops, terminator);
    ops = fold_staged_byte_word_updates(
        ops,
        routine_id,
        layout,
        enable_direct_byte_word_update,
        peephole_stats,
    );
    ops = fold_word_temp_producer_forwards(ops, routine_id, peephole_stats);
    ops = fold_staged_word_store_forwards(ops, routine_id, layout, terminator, peephole_stats);
    ops = fold_indirect_byte_const_stores(ops, routine_id, peephole_stats);
    ops = fold_indirect_y_const_stores(ops, routine_id, layout, peephole_stats);
    ops = fold_word_array_store_value_staging(ops, routine_id, layout, peephole_stats);
    ops = fold_indirect_byte_direct_stores(ops, routine_id, layout, peephole_stats);
    ops = fold_indirect_byte_const_compounds(ops, routine_id, peephole_stats);
    ops = fold_indirect_byte_direct_compounds(ops, routine_id, layout, peephole_stats);
    ops = fold_indirect_byte_compounds(ops, routine_id, peephole_stats);
    ops = fold_redundant_self_stores(ops, routine_id, layout, peephole_stats);
    ops = fold_adjacent_store_reloads(ops, routine_id, layout, terminator, peephole_stats);
    ops = fold_staged_binary_rhs(ops, routine_id, terminator, peephole_stats);
    ops = fold_staged_compare_rhs(ops, routine_id, terminator, peephole_stats);
    ops = fold_ssa_lite_byte_loads(ops, routine_id, layout, terminator, peephole_stats);
    ops = fold_dead_private_scratch_stores(ops, routine_id, terminator, peephole_stats);
    ops = fold_dead_reg_writes_before_overwrite(ops, routine_id, terminator, peephole_stats);
    ops = fold_dead_a_loads_before_flag_overwrite(ops, routine_id, terminator, peephole_stats);
    record_ssa_lite_block_facts(&ops, routine_id, layout, peephole_stats);
    ops
}

/// Legacy tail retained while Slice 8 migrates families in order. The staged
/// byte/word and word-forward families are intentionally absent: production
/// invokes their routine-level transactional discovery before this tail.
pub(super) fn fold_structural_before_cleanup_migrations(
    mut ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    ops = fold_redundant_self_stores(ops, routine_id, layout, peephole_stats);
    ops
}

pub(super) fn fold_structural_ssa_lite(
    mut ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    ops = fold_ssa_lite_byte_loads(ops, routine_id, layout, terminator, peephole_stats);
    ops
}

pub(super) fn fold_structural_machine_tail(
    mut ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    ops = fold_dead_reg_writes_before_overwrite(ops, routine_id, terminator, peephole_stats);
    ops = fold_dead_a_loads_before_flag_overwrite(ops, routine_id, terminator, peephole_stats);
    record_ssa_lite_block_facts(&ops, routine_id, layout, peephole_stats);
    ops
}

pub(in crate::mir6502) fn discover_rhs_and_adjacent_reloads(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if adjacent_store_reload_shape_at(&block.ops, index, layout).is_some()
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + 2,
                    vec![block.ops[index].clone()],
                    zn_exit_change(),
                    "adjacent-store-reload",
                    0,
                )
            {
                plans.push(plan);
            }
            if let Some((consumed, replacement)) = staged_binary_rhs_shape_at(&block.ops, index)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    "staged-binary-rhs",
                    1,
                )
            {
                plans.push(plan);
            }
            if let Some((consumed, replacement)) = staged_compare_rhs_shape_at(&block.ops, index)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    "staged-compare-rhs",
                    2,
                )
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_dead_private_scratch_stores(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if store_direct_private_scratch_byte(&block.ops[index]).is_some()
                && let Some(mut plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + 1,
                    Vec::new(),
                    MirExitStateChange::default(),
                    "dead-private-scratch-store",
                    0,
                )
            {
                plan.observations.push(("ssa-lite-dead-scratch-stores", 1));
                plans.push(plan);
            }
        }
    }
    plans
}

fn zn_exit_change() -> MirExitStateChange {
    MirExitStateChange {
        flags: MirFlagSet {
            z: true,
            n: true,
            ..MirFlagSet::default()
        },
        ..MirExitStateChange::default()
    }
}

pub(in crate::mir6502) fn discover_indirect_constant_stores(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some((consumed, replacement)) = indirect_byte_const_store_at(&block.ops, index)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    "indirect-byte-const-store",
                    0,
                )
            {
                plans.push(plan);
            }
            if let Some((consumed, replacement)) =
                indirect_y_const_store_at(&block.ops, index, routine.id, layout)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    "indirect-y-const-store",
                    1,
                )
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_indirect_stores_and_compounds(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            let candidates = [
                indirect_byte_direct_store_at(&block.ops, index, routine.id, layout)
                    .map(|candidate| (candidate, "indirect-byte-direct-store", 0)),
                indirect_byte_const_compound_at(&block.ops, index)
                    .map(|candidate| (candidate, "indirect-byte-const-compound", 1)),
                indirect_byte_direct_compound_at(&block.ops, index, routine.id, layout)
                    .map(|candidate| (candidate, "indirect-byte-direct-compound", 2)),
                indirect_byte_compound_at(&block.ops, index)
                    .map(|candidate| (candidate, "indirect-byte-compound", 3)),
            ];
            for ((consumed, replacement), stat, priority) in candidates.into_iter().flatten() {
                if let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    stat,
                    priority,
                ) {
                    plans.push(plan);
                }
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_word_array_value_staging(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some((consumed, replacement)) =
                word_array_store_value_staging_at(&block.ops, index, routine.id, layout)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    "word-array-store-value-staging",
                    0,
                )
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_staged_word_forwards(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
    enable_direct_byte_word_update: bool,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some((consumed, replacement)) = staged_byte_word_update_at(
                &block.ops,
                index,
                routine.id,
                layout,
                enable_direct_byte_word_update,
            ) && let Some(plan) = structural_plan(
                routine,
                context,
                block.id,
                index..index + consumed,
                replacement,
                clobbered_accumulator_exit(),
                "staged-byte-word-update",
                0,
            ) {
                plans.push(plan);
            }
            if let Some((consumed, replacement, stat)) =
                next_style_word_store_forward_at(&block.ops, index)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    clobbered_accumulator_exit(),
                    stat,
                    1,
                )
            {
                plans.push(plan);
            }
            if let Some((consumed, replacement, stat)) =
                key_style_updated_pointer_deref_forward_at(&block.ops, index)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    stat,
                    2,
                )
            {
                plans.push(plan);
            }
            if let Some((consumed, replacement)) =
                staged_word_store_forward_shape_at(&block.ops, index, routine.id, layout)
                && let Some(plan) = structural_plan(
                    routine,
                    context,
                    block.id,
                    index..index + consumed,
                    replacement,
                    MirExitStateChange::default(),
                    "staged-word-store-forward",
                    3,
                )
            {
                plans.push(plan);
            }
        }
    }
    plans
}

fn clobbered_accumulator_exit() -> MirExitStateChange {
    MirExitStateChange {
        registers: MirRegisterSet {
            a: true,
            ..MirRegisterSet::default()
        },
        flags: MirFlagSet::all(),
        ..MirExitStateChange::default()
    }
}

#[cfg(test)]
fn fold_staged_byte_word_updates(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    enable_direct_byte_word_update: bool,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) = staged_byte_word_update_at(
            &ops,
            index,
            routine_id,
            layout,
            enable_direct_byte_word_update,
        ) {
            peephole_stats.record(routine_id, "staged-byte-word-update");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
fn fold_staged_word_store_forwards(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) =
            staged_word_store_forward_at(&ops, index, routine_id, layout, terminator)
        {
            peephole_stats.record(routine_id, "staged-word-store-forward");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
fn fold_word_temp_producer_forwards(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement, stat)) = word_temp_producer_forward_at(&ops, index)
        {
            peephole_stats.record(routine_id, stat);
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
fn fold_indirect_byte_compounds(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) = indirect_byte_compound_at(&ops, index) {
            peephole_stats.record(routine_id, "indirect-byte-compound");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
pub(super) fn fold_indirect_byte_direct_compounds(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) =
            indirect_byte_direct_compound_at(&ops, index, routine_id, layout)
        {
            peephole_stats.record(routine_id, "indirect-byte-direct-compound");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
pub(super) fn fold_indirect_byte_const_compounds(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) = indirect_byte_const_compound_at(&ops, index) {
            peephole_stats.record(routine_id, "indirect-byte-const-compound");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
pub(super) fn fold_indirect_byte_const_stores(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) = indirect_byte_const_store_at(&ops, index) {
            peephole_stats.record(routine_id, "indirect-byte-const-store");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
pub(super) fn fold_indirect_y_const_stores(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) =
            indirect_y_const_store_at(&ops, index, routine_id, layout)
        {
            peephole_stats.record(routine_id, "indirect-y-const-store");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
pub(super) fn fold_word_array_store_value_staging(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) =
            word_array_store_value_staging_at(&ops, index, routine_id, layout)
        {
            peephole_stats.record(routine_id, "word-array-store-value-staging");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
fn fold_indirect_byte_direct_stores(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) =
            indirect_byte_direct_store_at(&ops, index, routine_id, layout)
        {
            peephole_stats.record(routine_id, "indirect-byte-direct-store");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

fn fold_redundant_self_stores(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if redundant_self_store_at(&ops, index, layout).is_some() {
            peephole_stats.record(routine_id, "redundant-self-store");
            out.push(ops[index].clone());
            index += 2;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
fn fold_adjacent_store_reloads(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if adjacent_store_reload_at(&ops, index, layout, terminator).is_some() {
            peephole_stats.record(routine_id, "adjacent-store-reload");
            out.push(ops[index].clone());
            index += 2;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
fn fold_staged_compare_rhs(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) = staged_compare_rhs_at(&ops, index, terminator) {
            peephole_stats.record(routine_id, "staged-compare-rhs");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
fn fold_staged_binary_rhs(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some((consumed, mut replacement)) = staged_binary_rhs_at(&ops, index, terminator) {
            peephole_stats.record(routine_id, "staged-binary-rhs");
            out.append(&mut replacement);
            index += consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
pub(super) fn fold_dead_private_scratch_stores(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if dead_private_scratch_store_at(&ops, index, terminator).is_some() {
            peephole_stats.record(routine_id, "dead-private-scratch-store");
            peephole_stats.record(routine_id, "ssa-lite-dead-scratch-stores");
            index += 1;
        } else {
            if let Some(stat) = private_scratch_store_retention_stat(&ops, index, terminator) {
                peephole_stats.record(routine_id, stat);
            }
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

fn fold_dead_a_loads_before_flag_overwrite(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::with_capacity(ops.len());
    let mut index = 0;
    while index < ops.len() {
        if dead_a_load_before_flag_overwrite_at(&ops, index, terminator).is_some() {
            peephole_stats.record(routine_id, "ssa-lite-dead-a-loads-removed");
            index += 1;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

pub(super) fn fold_dead_reg_writes_before_overwrite(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    terminator: &MirTerminator,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    let mut out = Vec::with_capacity(ops.len());
    let mut index = 0;
    while index < ops.len() {
        if dead_reg_write_before_overwrite_at(&ops, index, terminator).is_some() {
            peephole_stats.record(routine_id, "ssa-lite-dead-reg-writes");
            index += 1;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

#[cfg(test)]
fn word_temp_producer_forward_at(
    ops: &[MirOp],
    index: usize,
) -> Option<(usize, Vec<MirOp>, &'static str)> {
    next_style_word_store_forward_at(ops, index)
        .or_else(|| key_style_updated_pointer_deref_forward_at(ops, index))
}

fn key_style_updated_pointer_deref_forward_at(
    ops: &[MirOp],
    index: usize,
) -> Option<(usize, Vec<MirOp>, &'static str)> {
    let target_hi = store_x_direct_byte(ops.get(index)?)?;
    let target_lo = store_a_direct_byte(ops.get(index + 1)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 2)?)?
        != MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))
    {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 3)?)? != target_hi {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 4)?)?
        != MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))
    {
        return None;
    }
    let (first_consumer, first_offset) = load_indirect_a_byte(ops.get(index + 5)?)?;
    if first_consumer != fixed_pointer_consumer(POINTER_SCRATCH_LO) {
        return None;
    }
    let (byte_op, byte_const) = binary_a_const_update_ignore_carry(ops.get(index + 6)?)?;
    if byte_op != MirBinaryOp::Add {
        return None;
    }
    let value_slot = store_a_direct_byte(ops.get(index + 7)?)?;
    if !mem_is_private_scratch(&value_slot) {
        return None;
    }
    match ops.get(index + 8)? {
        MirOp::AddByteToWordMem { mem, value }
            if mem == &target_lo && value == &MirValue::PointerCell(value_slot.clone()) => {}
        _ => return None,
    }
    if load_a_direct_byte(ops.get(index + 9)?)? != target_lo {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 10)?)?
        != MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))
    {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 11)?)? != target_hi {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 12)?)?
        != MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))
    {
        return None;
    }
    let (second_consumer, second_offset) = load_indirect_a_byte(ops.get(index + 13)?)?;
    if second_consumer != fixed_pointer_consumer(POINTER_SCRATCH_LO)
        || second_offset != first_offset
    {
        return None;
    }
    let final_dst = store_a_direct_byte(ops.get(index + 14)?)?;

    Some((
        15,
        vec![
            ops[index].clone(),
            ops[index + 1].clone(),
            ops[index + 2].clone(),
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
            },
            MirOp::LoadIndirect {
                consumer: fixed_pointer_consumer(POINTER_SCRATCH_LO),
                dst: MirDef::Reg(MirReg::A),
                offset: first_offset,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(byte_const),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(value_slot.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(value_slot),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(target_lo),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(target_hi),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::LoadIndirect {
                consumer: fixed_pointer_consumer(POINTER_SCRATCH_LO),
                dst: MirDef::Reg(MirReg::A),
                offset: second_offset,
            },
            MirOp::Store {
                dst: MirAddr::Direct(final_dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ],
        "updated-pointer-deref-forward",
    ))
}

fn next_style_word_store_forward_at(
    ops: &[MirOp],
    index: usize,
) -> Option<(usize, Vec<MirOp>, &'static str)> {
    let param_hi = store_x_direct_byte(ops.get(index)?)?;
    let param_lo = store_a_direct_byte(ops.get(index + 1)?)?;
    if param_hi != offset_mem(&param_lo, 1) {
        return None;
    }
    let staged_lo = store_a_direct_byte(ops.get(index + 2)?)?;
    if !mem_is_private_scratch(&staged_lo) {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 3)?)? != param_hi {
        return None;
    }
    let staged_hi = store_a_direct_byte(ops.get(index + 4)?)?;
    if !mem_is_private_scratch(&staged_hi) || staged_hi == staged_lo {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 5)?)? != param_lo {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 6)?)?
        != MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))
    {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 7)?)? != param_hi {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 8)?)?
        != MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))
    {
        return None;
    }
    let (consumer, offset) = load_indirect_a_byte(ops.get(index + 9)?)?;
    if consumer != fixed_pointer_consumer(POINTER_SCRATCH_LO) {
        return None;
    }
    let value_slot = store_a_direct_byte(ops.get(index + 10)?)?;
    if !mem_is_private_scratch(&value_slot) {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 11)?)? != staged_lo {
        return None;
    }
    if binary_a_byte_update(ops.get(index + 12)?, &value_slot)? != MirBinaryOp::Add {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 13)?)? != staged_lo {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 14)?)? != staged_hi {
        return None;
    }
    if !binary_a_carry_zero_update(ops.get(index + 15)?, MirBinaryOp::Add) {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 16)?)? != staged_hi {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 17)?)? != staged_lo {
        return None;
    }
    let (const_op, const_value) = binary_a_const_update(ops.get(index + 18)?)?;
    if const_op != MirBinaryOp::Add {
        return None;
    }
    let target_lo = store_a_direct_byte(ops.get(index + 19)?)?;
    if load_a_direct_byte(ops.get(index + 20)?)? != staged_hi {
        return None;
    }
    if !binary_a_carry_zero_update(ops.get(index + 21)?, MirBinaryOp::Add) {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 22)?)? != offset_mem(&target_lo, 1) {
        return None;
    }

    Some((
        23,
        vec![
            ops[index].clone(),
            ops[index + 1].clone(),
            MirOp::Store {
                dst: MirAddr::Direct(target_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(offset_mem(&target_lo, 1)),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
            },
            MirOp::LoadIndirect {
                consumer: fixed_pointer_consumer(POINTER_SCRATCH_LO),
                dst: MirDef::Reg(MirReg::A),
                offset,
            },
            MirOp::Store {
                dst: MirAddr::Direct(value_slot.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::AddByteToWordMem {
                mem: target_lo.clone(),
                value: MirValue::PointerCell(value_slot),
            },
            MirOp::AddByteToWordMem {
                mem: target_lo,
                value: MirValue::ConstU8(const_value),
            },
        ],
        "binary-word-store-producer-forward",
    ))
}

#[cfg(test)]
fn staged_word_store_forward_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
) -> Option<(usize, Vec<MirOp>)> {
    let replacement = staged_word_store_forward_shape_at(ops, index, routine_id, layout)?;
    let (consumed, _) = &replacement;
    let staged_lo = store_a_direct_byte(ops.get(index + 2)?)?;
    let staged_hi = store_a_direct_byte(ops.get(index + 5)?)?;
    let after_forward = index + *consumed;
    if !private_scratch_store_removal_is_safe_after(ops, after_forward, terminator, &staged_lo)
        || !private_scratch_store_removal_is_safe_after(ops, after_forward, terminator, &staged_hi)
    {
        return None;
    }
    Some(replacement)
}

fn staged_word_store_forward_shape_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let source_lo = load_a_direct_byte(ops.get(index)?)?;
    let (op, _value) = binary_a_const_update(ops.get(index + 1)?)?;
    let staged_lo = store_a_direct_byte(ops.get(index + 2)?)?;
    let source_hi = load_a_direct_byte(ops.get(index + 3)?)?;
    if !binary_a_carry_zero_update(ops.get(index + 4)?, op) {
        return None;
    }
    let staged_hi = store_a_direct_byte(ops.get(index + 5)?)?;
    if staged_hi == staged_lo {
        return None;
    }
    if !mem_is_private_scratch(&staged_lo) || !mem_is_private_scratch(&staged_hi) {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 6)?)? != staged_lo {
        return None;
    }
    let target_lo = store_a_direct_byte(ops.get(index + 7)?)?;
    if load_a_direct_byte(ops.get(index + 8)?)? != staged_hi {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 9)?)? != offset_mem(&target_lo, 1) {
        return None;
    }
    if mem_may_overlap_word_target(routine_id, layout, &source_lo, &target_lo)
        || mem_may_overlap_word_target(routine_id, layout, &source_hi, &target_lo)
    {
        return None;
    }
    Some((
        10,
        vec![
            ops[index].clone(),
            ops[index + 1].clone(),
            MirOp::Store {
                dst: MirAddr::Direct(target_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            ops[index + 3].clone(),
            ops[index + 4].clone(),
            MirOp::Store {
                dst: MirAddr::Direct(offset_mem(&target_lo, 1)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ],
    ))
}

fn staged_byte_word_update_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    enable_direct_byte_word_update: bool,
) -> Option<(usize, Vec<MirOp>)> {
    if enable_direct_byte_word_update
        && let Some(replacement) = direct_byte_word_update_at(ops, index, routine_id, layout)
    {
        return Some(replacement);
    }
    if let Some(replacement) = forwarded_staged_byte_word_update_at(ops, index, routine_id, layout)
    {
        return Some(replacement);
    }

    let value_source = load_a_direct_byte(ops.get(index)?)?;
    let value_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&value_slot) {
        return None;
    }

    let target = load_a_direct_byte(ops.get(index + 2)?)?;
    let lo_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    let target_hi = load_a_direct_byte(ops.get(index + 4)?)?;
    if target_hi != offset_mem(&target, 1) {
        return None;
    }
    let hi_slot = store_a_direct_byte(ops.get(index + 5)?)?;
    if !mem_is_private_scratch(&lo_slot) || !mem_is_private_scratch(&hi_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 6)?)? != lo_slot {
        return None;
    }
    let op = binary_a_byte_update(ops.get(index + 7)?, &value_slot)?;
    let result_lo = store_a_direct_byte(ops.get(index + 8)?)?;
    if !mem_is_private_scratch(&result_lo) {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 9)?)? != hi_slot {
        return None;
    }
    if !binary_a_carry_zero_update(ops.get(index + 10)?, op) {
        return None;
    }
    let result_hi = store_a_direct_byte(ops.get(index + 11)?)?;
    if !mem_is_private_scratch(&result_hi) {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 12)?)? != result_lo {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 13)?)? != target {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 14)?)? != result_hi {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 15)?)? != offset_mem(&target, 1) {
        return None;
    }

    let update = match op {
        MirBinaryOp::Add => MirOp::AddByteToWordMem {
            mem: target,
            value: MirValue::PointerCell(value_source),
        },
        MirBinaryOp::Sub => MirOp::SubByteFromWordMem {
            mem: target,
            value: MirValue::PointerCell(value_source),
        },
        _ => return None,
    };
    Some((16, vec![update]))
}

fn direct_byte_word_update_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let target = load_a_direct_byte(ops.get(index)?)?;
    let value_source = binary_a_direct_update_source(ops.get(index + 1)?)?;
    if mem_may_overlap_word_target(routine_id, layout, &value_source, &target) {
        return None;
    }
    let op = binary_a_byte_update(ops.get(index + 1)?, &value_source)?;
    if store_a_direct_byte(ops.get(index + 2)?)? != target {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 3)?)? != offset_mem(&target, 1) {
        return None;
    }
    if !binary_a_carry_zero_update(ops.get(index + 4)?, op) {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 5)?)? != offset_mem(&target, 1) {
        return None;
    }

    let update = match op {
        MirBinaryOp::Add => MirOp::AddByteToWordMem {
            mem: target,
            value: MirValue::PointerCell(value_source),
        },
        MirBinaryOp::Sub => MirOp::SubByteFromWordMem {
            mem: target,
            value: MirValue::PointerCell(value_source),
        },
        _ => return None,
    };
    Some((6, vec![update]))
}

fn forwarded_staged_byte_word_update_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let target = load_a_direct_byte(ops.get(index)?)?;
    let lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    let target_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if target_hi != offset_mem(&target, 1) {
        return None;
    }
    let hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&lo_slot) || !mem_is_private_scratch(&hi_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 4)?)? != lo_slot {
        return None;
    }
    let value_source = binary_a_direct_update_source(ops.get(index + 5)?)?;
    if mem_may_overlap_word_target(routine_id, layout, &value_source, &target) {
        return None;
    }
    let op = binary_a_byte_update(ops.get(index + 5)?, &value_source)?;
    let result_lo = store_a_direct_byte(ops.get(index + 6)?)?;
    if !mem_is_private_scratch(&result_lo) {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 7)?)? != hi_slot {
        return None;
    }
    if !binary_a_carry_zero_update(ops.get(index + 8)?, op) {
        return None;
    }
    let result_hi = store_a_direct_byte(ops.get(index + 9)?)?;
    if !mem_is_private_scratch(&result_hi) {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 10)?)? != result_lo {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 11)?)? != target {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 12)?)? != result_hi {
        return None;
    }
    if store_a_direct_byte(ops.get(index + 13)?)? != offset_mem(&target, 1) {
        return None;
    }

    let update = match op {
        MirBinaryOp::Add => MirOp::AddByteToWordMem {
            mem: target,
            value: MirValue::PointerCell(value_source),
        },
        MirBinaryOp::Sub => MirOp::SubByteFromWordMem {
            mem: target,
            value: MirValue::PointerCell(value_source),
        },
        _ => return None,
    };
    Some((14, vec![update]))
}

fn indirect_byte_compound_at(ops: &[MirOp], index: usize) -> Option<(usize, Vec<MirOp>)> {
    let target_lo = load_a_direct_byte(ops.get(index)?)?;
    let target_lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&target_lo_slot) {
        return None;
    }
    let target_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    let target_hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&target_hi_slot) {
        return None;
    }

    let source_lo = load_a_direct_byte(ops.get(index + 4)?)?;
    let source_lo_fixed = fixed_store_a_byte(ops.get(index + 5)?)?;
    let source_hi = load_a_direct_byte(ops.get(index + 6)?)?;
    if source_hi != offset_mem(&source_lo, 1) {
        return None;
    }
    let source_hi_fixed = fixed_store_a_byte(ops.get(index + 7)?)?;
    if source_hi_fixed != source_lo_fixed.saturating_add(1) {
        return None;
    }
    let source_consumer = fixed_pointer_consumer(source_lo_fixed);
    let (loaded_source_consumer, offset) = load_indirect_a_byte(ops.get(index + 8)?)?;
    if loaded_source_consumer != source_consumer || offset > u8::MAX as u16 {
        return None;
    }
    let value_slot = store_a_direct_byte(ops.get(index + 9)?)?;
    if !mem_is_private_scratch(&value_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 10)?)? != target_lo_slot {
        return None;
    }
    let target_lo_fixed = fixed_store_a_byte(ops.get(index + 11)?)?;
    if load_a_direct_byte(ops.get(index + 12)?)? != target_hi_slot {
        return None;
    }
    let target_hi_fixed = fixed_store_a_byte(ops.get(index + 13)?)?;
    if target_hi_fixed != target_lo_fixed.saturating_add(1) {
        return None;
    }
    let target_consumer = fixed_pointer_consumer(target_lo_fixed);
    let (loaded_target_consumer, target_offset) = load_indirect_a_byte(ops.get(index + 14)?)?;
    if loaded_target_consumer != target_consumer || target_offset != offset {
        return None;
    }
    let op = binary_a_byte_update_ignore_carry(ops.get(index + 15)?, &value_slot)?;
    let (stored_target_consumer, store_offset) = store_indirect_a_byte(ops.get(index + 16)?)?;
    if stored_target_consumer != target_consumer || store_offset != offset {
        return None;
    }

    let replacement = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(target_lo),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(target_hi),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(source_lo),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                POINTER_INDEX_SCRATCH_LO,
            ))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(source_hi),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                POINTER_INDEX_SCRATCH_HI,
            ))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::IndirectByteCompound {
            op,
            target: fixed_pointer_consumer(POINTER_SCRATCH_LO),
            source: fixed_pointer_consumer(POINTER_INDEX_SCRATCH_LO),
            offset,
        },
    ];
    Some((17, replacement))
}

fn indirect_byte_direct_compound_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    if let Some(replacement) =
        indirect_byte_forwarded_direct_compound_at(ops, index, routine_id, layout)
    {
        return Some(replacement);
    }

    let target_lo = load_a_direct_byte(ops.get(index)?)?;
    let target_lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&target_lo_slot) {
        return None;
    }
    let target_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    let target_hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&target_hi_slot) {
        return None;
    }

    let value_source = load_a_direct_byte(ops.get(index + 4)?)?;
    if mem_may_overlap_fixed_pointer_scratch(routine_id, layout, &value_source) {
        return None;
    }
    let value_slot = store_a_direct_byte(ops.get(index + 5)?)?;
    if !mem_is_private_scratch(&value_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 6)?)? != target_lo_slot {
        return None;
    }
    let target_lo_fixed = fixed_store_a_byte(ops.get(index + 7)?)?;
    if load_a_direct_byte(ops.get(index + 8)?)? != target_hi_slot {
        return None;
    }
    let target_hi_fixed = fixed_store_a_byte(ops.get(index + 9)?)?;
    if target_hi_fixed != target_lo_fixed.saturating_add(1) {
        return None;
    }
    let target_consumer = fixed_pointer_consumer(target_lo_fixed);
    let (loaded_target_consumer, offset) = load_indirect_a_byte(ops.get(index + 10)?)?;
    if loaded_target_consumer != target_consumer || offset > u8::MAX as u16 {
        return None;
    }
    let op = binary_a_byte_update_ignore_carry(ops.get(index + 11)?, &value_slot)?;
    let (stored_target_consumer, store_offset) = store_indirect_a_byte(ops.get(index + 12)?)?;
    if stored_target_consumer != target_consumer || store_offset != offset {
        return None;
    }

    let replacement =
        direct_indirect_byte_compound_replacement(target_lo, target_hi, value_source, op, offset)?;
    Some((13, replacement))
}

fn indirect_byte_forwarded_direct_compound_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let target_lo = load_a_direct_byte(ops.get(index)?)?;
    let target_lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&target_lo_slot) {
        return None;
    }
    let target_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    let target_hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&target_hi_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 4)?)? != target_lo_slot {
        return None;
    }
    let target_lo_fixed = fixed_store_a_byte(ops.get(index + 5)?)?;
    if load_a_direct_byte(ops.get(index + 6)?)? != target_hi_slot {
        return None;
    }
    let target_hi_fixed = fixed_store_a_byte(ops.get(index + 7)?)?;
    if target_hi_fixed != target_lo_fixed.saturating_add(1) {
        return None;
    }
    let target_consumer = fixed_pointer_consumer(target_lo_fixed);
    let (loaded_target_consumer, offset) = load_indirect_a_byte(ops.get(index + 8)?)?;
    if loaded_target_consumer != target_consumer || offset > u8::MAX as u16 {
        return None;
    }

    let value_source = binary_a_direct_update_source(ops.get(index + 9)?)?;
    if mem_may_overlap_fixed_pointer_scratch(routine_id, layout, &value_source) {
        return None;
    }
    let op = binary_a_byte_update_ignore_carry(ops.get(index + 9)?, &value_source)?;
    let (stored_target_consumer, store_offset) = store_indirect_a_byte(ops.get(index + 10)?)?;
    if stored_target_consumer != target_consumer || store_offset != offset {
        return None;
    }

    Some((
        11,
        direct_indirect_byte_compound_replacement(target_lo, target_hi, value_source, op, offset)?,
    ))
}

fn direct_indirect_byte_compound_replacement(
    target_lo: MirMem,
    target_hi: MirMem,
    value_source: MirMem,
    op: MirBinaryOp,
    offset: u16,
) -> Option<Vec<MirOp>> {
    let target = fixed_pointer_consumer(POINTER_SCRATCH_LO);
    Some(vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(target_lo),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(target_hi),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::LoadIndirect {
            consumer: target,
            dst: MirDef::Reg(MirReg::A),
            offset,
        },
        MirOp::Binary {
            op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(value_source),
            width: MirWidth::Byte,
            carry_in: Some(match op {
                MirBinaryOp::Add => MirCarryIn::Clear,
                MirBinaryOp::Sub => MirCarryIn::Set,
                _ => return None,
            }),
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::StoreIndirect {
            consumer: target,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset,
        },
    ])
}

fn indirect_byte_const_compound_at(ops: &[MirOp], index: usize) -> Option<(usize, Vec<MirOp>)> {
    if let Some(replacement) = indirect_byte_delayed_const_compound_at(ops, index) {
        return Some(replacement);
    }

    let target_lo = load_a_direct_byte(ops.get(index)?)?;
    let target_lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&target_lo_slot) {
        return None;
    }
    let target_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    let target_hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&target_hi_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 4)?)? != target_lo_slot {
        return None;
    }
    let target_lo_fixed = fixed_store_a_byte(ops.get(index + 5)?)?;
    if load_a_direct_byte(ops.get(index + 6)?)? != target_hi_slot {
        return None;
    }
    let target_hi_fixed = fixed_store_a_byte(ops.get(index + 7)?)?;
    if target_hi_fixed != target_lo_fixed.saturating_add(1) {
        return None;
    }
    let target_consumer = fixed_pointer_consumer(target_lo_fixed);
    let (loaded_target_consumer, offset) = load_indirect_a_byte(ops.get(index + 8)?)?;
    if loaded_target_consumer != target_consumer || offset > u8::MAX as u16 {
        return None;
    }
    let (op, imm) = binary_a_const_update_ignore_carry(ops.get(index + 9)?)?;
    let (stored_target_consumer, store_offset) = store_indirect_a_byte(ops.get(index + 10)?)?;
    if stored_target_consumer != target_consumer || store_offset != offset {
        return None;
    }

    Some((
        11,
        indirect_byte_const_compound_replacement(target_lo, target_hi, op, imm, offset)?,
    ))
}

fn indirect_byte_delayed_const_compound_at(
    ops: &[MirOp],
    index: usize,
) -> Option<(usize, Vec<MirOp>)> {
    let target_lo = load_a_direct_byte(ops.get(index)?)?;
    let target_lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&target_lo_slot) {
        return None;
    }
    let target_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    let target_hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&target_hi_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 4)?)? != target_lo_slot {
        return None;
    }
    let target_lo_fixed = fixed_store_a_byte(ops.get(index + 5)?)?;
    if load_a_direct_byte(ops.get(index + 6)?)? != target_hi_slot {
        return None;
    }
    let target_hi_fixed = fixed_store_a_byte(ops.get(index + 7)?)?;
    if target_hi_fixed != target_lo_fixed.saturating_add(1) {
        return None;
    }
    let target_consumer = fixed_pointer_consumer(target_lo_fixed);
    let (loaded_target_consumer, offset) = load_indirect_a_byte(ops.get(index + 8)?)?;
    if loaded_target_consumer != target_consumer || offset > u8::MAX as u16 {
        return None;
    }
    let value_slot = store_a_direct_byte(ops.get(index + 9)?)?;
    if !mem_is_private_scratch(&value_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 10)?)? != target_lo_slot {
        return None;
    }
    if fixed_store_a_byte(ops.get(index + 11)?)? != target_lo_fixed {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 12)?)? != target_hi_slot {
        return None;
    }
    if fixed_store_a_byte(ops.get(index + 13)?)? != target_hi_fixed {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 14)?)? != value_slot {
        return None;
    }
    let (op, imm) = binary_a_const_update_ignore_carry(ops.get(index + 15)?)?;
    let (stored_target_consumer, store_offset) = store_indirect_a_byte(ops.get(index + 16)?)?;
    if stored_target_consumer != target_consumer || store_offset != offset {
        return None;
    }

    Some((
        17,
        indirect_byte_const_compound_replacement(target_lo, target_hi, op, imm, offset)?,
    ))
}

fn indirect_byte_const_compound_replacement(
    target_lo: MirMem,
    target_hi: MirMem,
    op: MirBinaryOp,
    imm: u8,
    offset: u16,
) -> Option<Vec<MirOp>> {
    let target = fixed_pointer_consumer(POINTER_SCRATCH_LO);
    Some(vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(target_lo),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(target_hi),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::LoadIndirect {
            consumer: target,
            dst: MirDef::Reg(MirReg::A),
            offset,
        },
        MirOp::Binary {
            op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(imm),
            width: MirWidth::Byte,
            carry_in: Some(match op {
                MirBinaryOp::Add => MirCarryIn::Clear,
                MirBinaryOp::Sub => MirCarryIn::Set,
                _ => return None,
            }),
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::StoreIndirect {
            consumer: target,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset,
        },
    ])
}

fn indirect_byte_const_store_at(ops: &[MirOp], index: usize) -> Option<(usize, Vec<MirOp>)> {
    let target_lo = load_a_direct_byte(ops.get(index)?)?;
    let target_lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&target_lo_slot) {
        return None;
    }
    let target_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    let target_hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&target_hi_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 4)?)? != target_lo_slot {
        return None;
    }
    let target_lo_fixed = fixed_store_a_byte(ops.get(index + 5)?)?;
    if load_a_direct_byte(ops.get(index + 6)?)? != target_hi_slot {
        return None;
    }
    let target_hi_fixed = fixed_store_a_byte(ops.get(index + 7)?)?;
    if target_hi_fixed != target_lo_fixed.saturating_add(1) {
        return None;
    }
    let const_load = load_a_const_byte(ops.get(index + 8)?)?;
    let target = fixed_pointer_consumer(target_lo_fixed);
    let (stored_target, offset) = store_indirect_a_byte(ops.get(index + 9)?)?;
    if stored_target != target || offset > u8::MAX as u16 {
        return None;
    }

    let mut replacement = stage_fixed_pointer_ops(target_lo, target_hi);
    replacement.push(const_load);
    replacement.push(MirOp::StoreIndirect {
        consumer: fixed_pointer_consumer(POINTER_SCRATCH_LO),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        offset,
    });
    Some((10, replacement))
}

fn indirect_y_const_store_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let target_lo = load_a_direct_byte(ops.get(index)?)?;
    let target_lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&target_lo_slot) {
        return None;
    }
    let target_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    let target_hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&target_hi_slot) {
        return None;
    }
    let index_source = load_a_direct_byte(ops.get(index + 4)?)?;
    if mem_may_overlap_fixed_pointer_scratch(routine_id, layout, &index_source) {
        return None;
    }
    let index_slot = store_a_direct_byte(ops.get(index + 5)?)?;
    if !mem_is_private_scratch(&index_slot) {
        return None;
    }

    if load_a_direct_byte(ops.get(index + 6)?)? != target_lo_slot {
        return None;
    }
    let target_lo_fixed = fixed_store_a_byte(ops.get(index + 7)?)?;
    if load_a_direct_byte(ops.get(index + 8)?)? != target_hi_slot {
        return None;
    }
    let target_hi_fixed = fixed_store_a_byte(ops.get(index + 9)?)?;
    if target_hi_fixed != target_lo_fixed.saturating_add(1) {
        return None;
    }
    if load_reg_direct_byte(ops.get(index + 10)?, MirReg::Y)? != index_slot {
        return None;
    }
    let const_load = load_a_const_byte(ops.get(index + 11)?)?;
    if store_fixed_indirect_y_a_byte(ops.get(index + 12)?)? != target_lo_fixed {
        return None;
    }

    let mut replacement = stage_fixed_pointer_ops(target_lo, target_hi);
    replacement.push(MirOp::Load {
        dst: MirDef::Reg(MirReg::Y),
        src: MirAddr::Direct(index_source),
        width: MirWidth::Byte,
    });
    replacement.push(const_load);
    replacement.push(MirOp::Store {
        dst: MirAddr::FixedIndirectIndexedY {
            zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
        },
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    Some((13, replacement))
}

fn word_array_store_value_staging_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let value_lo = load_a_direct_byte(ops.get(index)?)?;
    if !mem_is_stable_delayed_indirect_store_source(routine_id, layout, &value_lo) {
        return None;
    }
    let value_lo_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if !mem_is_private_scratch(&value_lo_slot) {
        return None;
    }
    let value_hi = load_a_direct_byte(ops.get(index + 2)?)?;
    if value_hi != offset_mem(&value_lo, 1)
        || !mem_is_stable_delayed_indirect_store_source(routine_id, layout, &value_hi)
    {
        return None;
    }
    let value_hi_slot = store_a_direct_byte(ops.get(index + 3)?)?;
    if !mem_is_private_scratch(&value_hi_slot) {
        return None;
    }

    let staged_address = staged_word_array_store_address_at(ops, index + 4)?;
    let load_lo_index = index + 4 + staged_address.consumed;
    if load_a_direct_byte(ops.get(load_lo_index)?)? != value_lo_slot {
        return None;
    }
    let (lo_target, lo_offset) = store_indirect_a_byte(ops.get(load_lo_index + 1)?)?;
    if lo_target != staged_address.consumer || lo_offset != 0 {
        return None;
    }
    if load_a_direct_byte(ops.get(load_lo_index + 2)?)? != value_hi_slot {
        return None;
    }
    let (hi_target, hi_offset) = store_indirect_a_byte(ops.get(load_lo_index + 3)?)?;
    if hi_target != staged_address.consumer || hi_offset != 1 {
        return None;
    }

    let mut replacement = staged_address.ops;
    replacement.extend([
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(value_lo),
            width: MirWidth::Byte,
        },
        MirOp::StoreIndirect {
            consumer: staged_address.consumer,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset: 0,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(value_hi),
            width: MirWidth::Byte,
        },
        MirOp::StoreIndirect {
            consumer: staged_address.consumer,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset: 1,
        },
    ]);
    Some((4 + staged_address.consumed + 4, replacement))
}

fn indirect_byte_direct_store_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    if let Some((consumed, replacement)) =
        indirect_byte_direct_store_after_load_at(ops, index, routine_id, layout)
    {
        return Some((consumed, replacement));
    }
    indirect_byte_direct_store_after_store_at(ops, index, routine_id, layout)
}

fn indirect_byte_direct_store_after_load_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let value_source = load_a_direct_byte(ops.get(index)?)?;
    let (target_lo, target_hi, target, offset) =
        indirect_byte_direct_store_tail(ops, index + 1, routine_id, layout, &value_source)?;
    let mut replacement = stage_fixed_pointer_ops(target_lo, target_hi);
    replacement.push(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(value_source),
        width: MirWidth::Byte,
    });
    replacement.push(MirOp::StoreIndirect {
        consumer: target,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        offset,
    });
    Some((8, replacement))
}

fn indirect_byte_direct_store_after_store_at(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, Vec<MirOp>)> {
    let value_source = store_a_direct_byte(ops.get(index)?)?;
    let (target_lo, target_hi, target, offset) =
        indirect_byte_direct_store_tail(ops, index + 1, routine_id, layout, &value_source)?;
    let mut replacement = vec![ops[index].clone()];
    replacement.extend(stage_fixed_pointer_ops(target_lo, target_hi));
    replacement.push(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(value_source),
        width: MirWidth::Byte,
    });
    replacement.push(MirOp::StoreIndirect {
        consumer: target,
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        offset,
    });
    Some((8, replacement))
}

fn indirect_byte_direct_store_tail(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    value_source: &MirMem,
) -> Option<(MirMem, MirMem, MirAddressConsumer, u16)> {
    if !mem_is_stable_delayed_indirect_store_source(routine_id, layout, value_source) {
        return None;
    }
    let value_slot = store_a_direct_byte(ops.get(index)?)?;
    if !mem_is_private_scratch(&value_slot) {
        return None;
    }
    let target_lo = load_a_direct_byte(ops.get(index + 1)?)?;
    let target_lo_fixed = fixed_store_a_byte(ops.get(index + 2)?)?;
    let target_hi = load_a_direct_byte(ops.get(index + 3)?)?;
    if target_hi != offset_mem(&target_lo, 1) {
        return None;
    }
    let target_hi_fixed = fixed_store_a_byte(ops.get(index + 4)?)?;
    if target_hi_fixed != target_lo_fixed.saturating_add(1) {
        return None;
    }
    if load_a_direct_byte(ops.get(index + 5)?)? != value_slot {
        return None;
    }
    let target = fixed_pointer_consumer(target_lo_fixed);
    let (stored_target, offset) = store_indirect_a_byte(ops.get(index + 6)?)?;
    if stored_target != target || offset > u8::MAX as u16 {
        return None;
    }
    Some((
        target_lo,
        target_hi,
        fixed_pointer_consumer(POINTER_SCRATCH_LO),
        offset,
    ))
}

fn stage_fixed_pointer_ops(target_lo: MirMem, target_hi: MirMem) -> Vec<MirOp> {
    vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(target_lo),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(target_hi),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ]
}

fn load_a_const_byte(op: &MirOp) -> Option<MirOp> {
    match op {
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::ConstU8(_),
            width: MirWidth::Byte,
        }
        | MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            width: MirWidth::Byte,
            ..
        } => Some(op.clone()),
        _ => None,
    }
}

fn load_reg_direct_byte(op: &MirOp, reg: MirReg) -> Option<MirMem> {
    let MirOp::Load {
        dst: MirDef::Reg(dst),
        src: MirAddr::Direct(mem),
        width: MirWidth::Byte,
    } = op
    else {
        return None;
    };
    (*dst == reg).then(|| mem.clone())
}

fn store_fixed_indirect_y_a_byte(op: &MirOp) -> Option<u8> {
    let MirOp::Store {
        dst: MirAddr::FixedIndirectIndexedY { zp },
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    } = op
    else {
        return None;
    };
    Some(zp.0)
}

struct StagedWordArrayStoreAddress {
    consumer: MirAddressConsumer,
    consumed: usize,
    ops: Vec<MirOp>,
}

fn staged_word_array_store_address_at(
    ops: &[MirOp],
    index: usize,
) -> Option<StagedWordArrayStoreAddress> {
    if let Some(consumer) = materialize_indexed_address_consumer(ops.get(index)?) {
        return Some(StagedWordArrayStoreAddress {
            consumer,
            consumed: 1,
            ops: vec![ops[index].clone()],
        });
    }

    if pure_byte_reg_write(ops.get(index)?)? != MirReg::A {
        return None;
    }
    let consumer = materialize_indexed_address_consumer(ops.get(index + 1)?)?;
    Some(StagedWordArrayStoreAddress {
        consumer,
        consumed: 2,
        ops: vec![ops[index].clone(), ops[index + 1].clone()],
    })
}

fn materialize_indexed_address_consumer(op: &MirOp) -> Option<MirAddressConsumer> {
    let MirOp::MaterializeIndexedAddress { consumer, .. } = op else {
        return None;
    };
    Some(*consumer)
}

fn mem_may_overlap_fixed_pointer_scratch(
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    mem: &MirMem,
) -> bool {
    match mem {
        MirMem::FixedZeroPage(slot) => (POINTER_SCRATCH_LO..=POINTER_SCRATCH_HI).contains(&slot.0),
        MirMem::ZeroPage(_) => true,
        _ => layout.mem_address(routine_id, mem).is_some_and(|address| {
            (u16::from(POINTER_SCRATCH_LO)..=u16::from(POINTER_SCRATCH_HI)).contains(&address)
        }),
    }
}

fn mem_may_overlap_word_target(
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    mem: &MirMem,
    target_lo: &MirMem,
) -> bool {
    let target_hi = offset_mem(target_lo, 1);
    if mem == target_lo || mem == &target_hi {
        return true;
    }
    let Some(source_address) = layout.mem_address(routine_id, mem) else {
        return false;
    };
    [target_lo, &target_hi].into_iter().any(|target| {
        layout
            .mem_address(routine_id, target)
            .is_some_and(|target_address| target_address == source_address)
    })
}

fn mem_is_stable_delayed_indirect_store_source(
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    mem: &MirMem,
) -> bool {
    if mem_may_overlap_fixed_pointer_scratch(routine_id, layout, mem) {
        return false;
    }
    match mem {
        MirMem::Absolute(address) => *address < 0x0100,
        MirMem::FixedZeroPage(_) => true,
        MirMem::Global { .. }
        | MirMem::Static { .. }
        | MirMem::Local { .. }
        | MirMem::Param { .. }
        | MirMem::Spill { .. } => true,
        MirMem::ZeroPage(_) => false,
    }
}

fn redundant_self_store_at(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
) -> Option<usize> {
    let loaded = load_a_direct_byte(ops.get(index)?)?;
    let stored = store_a_direct_byte(ops.get(index + 1)?)?;
    if loaded == stored && mem_allows_idempotent_store_removal(layout, &loaded) {
        Some(2)
    } else {
        None
    }
}

fn mem_allows_idempotent_store_removal(layout: &MaterializeLayout, mem: &MirMem) -> bool {
    match mem {
        MirMem::Global { id, .. } => layout.global_allows_idempotent_store_removal(*id),
        MirMem::Static { .. }
        | MirMem::Local { .. }
        | MirMem::Param { .. }
        | MirMem::Spill { .. }
        | MirMem::ZeroPage(_)
        | MirMem::FixedZeroPage(_) => true,
        MirMem::Absolute(_) => false,
    }
}

#[cfg(test)]
fn adjacent_store_reload_at(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    terminator: &MirTerminator,
) -> Option<usize> {
    adjacent_store_reload_shape_at(ops, index, layout)?;
    if can_remove_spill_reload_at(ops, index + 1, terminator)
        || can_remove_spill_reload_before_later_a_use(ops, index + 1, terminator)
    {
        Some(2)
    } else {
        None
    }
}

fn adjacent_store_reload_shape_at(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
) -> Option<usize> {
    let stored = store_a_direct_byte(ops.get(index)?)?;
    let loaded = load_a_direct_byte(ops.get(index + 1)?)?;
    if stored != loaded || !mem_allows_idempotent_store_removal(layout, &stored) {
        return None;
    }
    Some(2)
}

#[cfg(test)]
pub(super) fn staged_compare_rhs_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<(usize, Vec<MirOp>)> {
    let replacement = staged_compare_rhs_shape_at(ops, index)?;
    let rhs_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    private_scratch_store_removal_is_safe_after(ops, index + 4, terminator, &rhs_slot)
        .then_some(replacement)
}

fn staged_compare_rhs_shape_at(ops: &[MirOp], index: usize) -> Option<(usize, Vec<MirOp>)> {
    let rhs_source = load_a_direct_byte(ops.get(index)?)?;
    let rhs_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if rhs_source == rhs_slot
        || !mem_is_private_scratch(&rhs_slot)
        || !mem_is_stable_delayed_compare_source(&rhs_source)
    {
        return None;
    }

    let left_source = load_a_direct_byte(ops.get(index + 2)?)?;
    if left_source == rhs_slot {
        return None;
    }
    let MirOp::Compare {
        dst,
        op,
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::PointerCell(compare_rhs),
        width: MirWidth::Byte,
        signed,
    } = ops.get(index + 3)?
    else {
        return None;
    };
    if *compare_rhs != rhs_slot {
        return None;
    }

    Some((
        4,
        vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(left_source),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: dst.clone(),
                op: *op,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(rhs_source),
                width: MirWidth::Byte,
                signed: *signed,
            },
        ],
    ))
}

#[cfg(test)]
fn staged_binary_rhs_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<(usize, Vec<MirOp>)> {
    let replacement = staged_binary_rhs_shape_at(ops, index)?;
    let rhs_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    private_scratch_store_removal_is_safe_after(ops, index + 4, terminator, &rhs_slot)
        .then_some(replacement)
}

fn staged_binary_rhs_shape_at(ops: &[MirOp], index: usize) -> Option<(usize, Vec<MirOp>)> {
    let rhs_source = load_a_direct_byte(ops.get(index)?)?;
    let rhs_slot = store_a_direct_byte(ops.get(index + 1)?)?;
    if rhs_source == rhs_slot
        || !mem_is_private_scratch(&rhs_slot)
        || !mem_is_stable_delayed_compare_source(&rhs_source)
    {
        return None;
    }

    let left_source = load_a_direct_byte(ops.get(index + 2)?)?;
    if left_source == rhs_slot {
        return None;
    }
    let MirOp::Binary {
        op,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::PointerCell(binary_rhs),
        width: MirWidth::Byte,
        carry_in,
        carry_out,
    } = ops.get(index + 3)?
    else {
        return None;
    };
    if *binary_rhs != rhs_slot {
        return None;
    }

    Some((
        4,
        vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(left_source),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: *op,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(rhs_source),
                width: MirWidth::Byte,
                carry_in: *carry_in,
                carry_out: *carry_out,
            },
        ],
    ))
}

#[cfg(test)]
pub(super) fn dead_private_scratch_store_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<usize> {
    let stored = store_direct_private_scratch_byte(ops.get(index)?)?;
    if private_scratch_store_removal_is_safe_after(ops, index + 1, terminator, &stored) {
        Some(1)
    } else {
        None
    }
}

fn store_direct_private_scratch_byte(op: &MirOp) -> Option<MirMem> {
    let MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(_)),
        width: MirWidth::Byte,
        ..
    } = op
    else {
        return None;
    };
    mem_is_private_scratch(mem).then(|| mem.clone())
}

#[cfg(test)]
fn private_scratch_store_retention_stat(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<&'static str> {
    let stored = store_direct_private_scratch_byte(ops.get(index)?)?;
    if !mem_is_private_scratch(&stored) {
        return None;
    }
    if mem_is_read_after(ops, index + 1, terminator, &stored) {
        return Some("ssa-lite-store-retained-unhandled-read");
    }
    if terminator_has_successors(terminator) {
        return Some("ssa-lite-store-retained-live-out");
    }
    Some("ssa-lite-store-retained-unknown")
}

#[cfg(test)]
pub(super) fn private_scratch_store_removal_is_safe_after(
    ops: &[MirOp],
    start: usize,
    terminator: &MirTerminator,
    mem: &MirMem,
) -> bool {
    if !mem_is_private_scratch(mem) {
        return false;
    }
    if mem_is_overwritten_before_read_or_transfer(ops, start, mem) {
        return true;
    }
    !terminator_has_successors(terminator) && !mem_is_read_after(ops, start, terminator, mem)
}

#[cfg(test)]
fn mem_is_overwritten_before_read_or_transfer(ops: &[MirOp], start: usize, mem: &MirMem) -> bool {
    for op in ops.iter().skip(start) {
        if op_reads_mem(op, mem) {
            return false;
        }
        if op_definitely_writes_mem(op, mem) {
            return true;
        }
        if op_may_have_unknown_memory_effects(op) || op_may_write_mem(op, mem) {
            return false;
        }
    }
    false
}

#[cfg(test)]
fn terminator_has_successors(terminator: &MirTerminator) -> bool {
    matches!(
        terminator,
        MirTerminator::Jump(_) | MirTerminator::Branch { .. }
    )
}

fn dead_a_load_before_flag_overwrite_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<usize> {
    let next = ops.get(index + 1)?;
    let same_value_index_load = match ops.get(index)? {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(source),
            width: MirWidth::Byte,
        } => matches!(
            next,
            MirOp::Load {
                dst: MirDef::Reg(MirReg::X | MirReg::Y),
                src: MirAddr::Direct(next_source),
                width: MirWidth::Byte,
            } if next_source == source
        ),
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value,
            width: MirWidth::Byte,
        } => matches!(
            next,
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::X | MirReg::Y),
                value: next_value,
                width: MirWidth::Byte,
            } if next_value == value
        ),
        _ => false,
    };

    if same_value_index_load
        && !op_reads_reg(next, MirReg::A)
        && !op_uses_previous_carry(next)
        && op_writes_flags(next)
        && tail_does_not_read_reg(ops, index + 1, terminator, MirReg::A)
    {
        Some(1)
    } else {
        None
    }
}

fn dead_reg_write_before_overwrite_at(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
) -> Option<usize> {
    let current = ops.get(index)?;
    let next = ops.get(index + 1)?;
    let reg = pure_byte_reg_write(current)?;
    if !op_writes_reg(next, reg)
        || op_reads_reg(next, reg)
        || op_uses_previous_carry(next)
        || !op_writes_flags(next)
    {
        return None;
    }
    if tail_does_not_read_reg(ops, index + 1, terminator, reg) {
        Some(1)
    } else {
        None
    }
}

fn pure_byte_reg_write(op: &MirOp) -> Option<MirReg> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } if pure_direct_load_mem(mem) => Some(*reg),
        MirOp::LoadImm {
            dst: MirDef::Reg(reg),
            width: MirWidth::Byte,
            ..
        } => Some(*reg),
        MirOp::Move {
            dst: MirDef::Reg(reg),
            src,
            width: MirWidth::Byte,
        } if pure_move_value(src) => Some(*reg),
        _ => None,
    }
}

fn pure_direct_load_mem(mem: &MirMem) -> bool {
    !matches!(mem, MirMem::Absolute(_))
}

fn pure_move_value(value: &MirValue) -> bool {
    match value {
        MirValue::PointerCell(mem) => pure_direct_load_mem(mem),
        MirValue::Word { lo, hi } => pure_move_value(lo) && pure_move_value(hi),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => true,
    }
}

fn tail_does_not_read_reg(
    ops: &[MirOp],
    start: usize,
    terminator: &MirTerminator,
    reg: MirReg,
) -> bool {
    for op in ops.iter().skip(start) {
        if op_reads_reg(op, reg) {
            return false;
        }
        if op_may_clobber_reg(op, reg) {
            return true;
        }
    }
    !terminator_reads_reg(terminator, reg)
}

fn terminator_reads_reg(terminator: &MirTerminator, reg: MirReg) -> bool {
    match terminator {
        MirTerminator::Branch {
            cond: MirCond::BoolValue(value),
            ..
        } => value_reads_reg(value, reg),
        MirTerminator::Jump(_)
        | MirTerminator::Branch { .. }
        | MirTerminator::Return
        | MirTerminator::Exit
        | MirTerminator::Unreachable => false,
    }
}

fn mem_is_stable_delayed_compare_source(mem: &MirMem) -> bool {
    matches!(
        mem,
        MirMem::Global { .. }
            | MirMem::Static { .. }
            | MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
            | MirMem::FixedZeroPage(_)
    )
}

fn load_a_direct_byte(op: &MirOp) -> Option<MirMem> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some(mem.clone()),
        _ => None,
    }
}

fn store_a_direct_byte(op: &MirOp) -> Option<MirMem> {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } => Some(mem.clone()),
        _ => None,
    }
}

fn store_x_direct_byte(op: &MirOp) -> Option<MirMem> {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::Def(MirDef::Reg(MirReg::X)),
            width: MirWidth::Byte,
        } => Some(mem.clone()),
        _ => None,
    }
}

fn fixed_store_a_byte(op: &MirOp) -> Option<u8> {
    match store_a_direct_byte(op)? {
        MirMem::FixedZeroPage(slot) => Some(slot.0),
        _ => None,
    }
}

pub(super) fn fixed_pointer_consumer(lo: u8) -> MirAddressConsumer {
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(lo),
    })
}

fn load_indirect_a_byte(op: &MirOp) -> Option<(MirAddressConsumer, u16)> {
    match op {
        MirOp::LoadIndirect {
            consumer,
            dst: MirDef::Reg(MirReg::A),
            offset,
        } => Some((*consumer, *offset)),
        _ => None,
    }
}

fn store_indirect_a_byte(op: &MirOp) -> Option<(MirAddressConsumer, u16)> {
    match op {
        MirOp::StoreIndirect {
            consumer,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset,
        } => Some((*consumer, *offset)),
        _ => None,
    }
}

fn binary_a_byte_update(op: &MirOp, rhs: &MirMem) -> Option<MirBinaryOp> {
    match op {
        MirOp::Binary {
            op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
            carry_in,
            carry_out: MirCarryOut::Produce,
        } if mem == rhs
            && ((*op == MirBinaryOp::Add && *carry_in == Some(MirCarryIn::Clear))
                || (*op == MirBinaryOp::Sub && *carry_in == Some(MirCarryIn::Set))) =>
        {
            Some(*op)
        }
        _ => None,
    }
}

fn binary_a_byte_update_ignore_carry(op: &MirOp, rhs: &MirMem) -> Option<MirBinaryOp> {
    match op {
        MirOp::Binary {
            op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
            carry_in,
            carry_out: MirCarryOut::Ignore,
        } if mem == rhs
            && ((*op == MirBinaryOp::Add && *carry_in == Some(MirCarryIn::Clear))
                || (*op == MirBinaryOp::Sub && *carry_in == Some(MirCarryIn::Set))) =>
        {
            Some(*op)
        }
        _ => None,
    }
}

fn binary_a_direct_update_source(op: &MirOp) -> Option<MirMem> {
    match op {
        MirOp::Binary {
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
            ..
        } => Some(mem.clone()),
        _ => None,
    }
}

fn binary_a_const_update_ignore_carry(op: &MirOp) -> Option<(MirBinaryOp, u8)> {
    match op {
        MirOp::Binary {
            op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(value),
            width: MirWidth::Byte,
            carry_in,
            carry_out: MirCarryOut::Ignore,
        } if (*op == MirBinaryOp::Add && *carry_in == Some(MirCarryIn::Clear))
            || (*op == MirBinaryOp::Sub && *carry_in == Some(MirCarryIn::Set)) =>
        {
            Some((*op, *value))
        }
        _ => None,
    }
}

fn binary_a_const_update(op: &MirOp) -> Option<(MirBinaryOp, u8)> {
    match op {
        MirOp::Binary {
            op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(value),
            width: MirWidth::Byte,
            carry_in,
            carry_out: MirCarryOut::Produce,
        } if (*op == MirBinaryOp::Add && *carry_in == Some(MirCarryIn::Clear))
            || (*op == MirBinaryOp::Sub && *carry_in == Some(MirCarryIn::Set)) =>
        {
            Some((*op, *value))
        }
        _ => None,
    }
}

fn binary_a_carry_zero_update(op: &MirOp, binary_op: MirBinaryOp) -> bool {
    matches!(
        op,
        MirOp::Binary {
            op,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(0),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::FromPrevious),
            carry_out: MirCarryOut::Ignore,
        } if *op == binary_op
    )
}

pub(super) fn mem_is_private_scratch(mem: &MirMem) -> bool {
    matches!(mem, MirMem::Spill { .. } | MirMem::ZeroPage(_))
}
