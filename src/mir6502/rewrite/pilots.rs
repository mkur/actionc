#![allow(dead_code)] // Families become live incrementally during Slice 6.

use std::collections::BTreeSet;

use crate::mir6502::analysis::effects::{MirTempAccess, classify_op, classify_terminator};
use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::analysis::use_def::MirTempLane;
use crate::mir6502::ir::{MirDef, MirOp, MirRoutine, MirValue};
use crate::mir6502::rewrite::context::{MirProof, PreHomeRewriteContext};
use crate::mir6502::rewrite::plan::{
    MirChangeSet, MirEffectDelta, MirRemovedDefinition, MirRewritePlan,
};

pub(in crate::mir6502) fn discover_prehome_pilots(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = unused_lea_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
            if let Some(plan) = compare_operand_producer_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_compare_producers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = compare_operand_producer_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
            if let Some(plan) = addressed_byte_compare_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_compare_narrowing(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = compare_narrowing_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_inclusive_compare_reversals(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            let Some(candidate) =
                crate::mir6502::materialize::analyzed_inclusive_compare_reversal_candidate(
                    op, layout,
                )
            else {
                continue;
            };
            if !matches!(
                &block.terminator,
                crate::mir6502::ir::MirTerminator::Branch {
                    cond: crate::mir6502::ir::MirCond::BoolValue(
                        crate::mir6502::ir::MirValue::Def(crate::mir6502::ir::MirDef::VTemp(
                            condition
                        ))
                    ),
                    ..
                } if *condition == candidate.condition
            ) {
                continue;
            }
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: index..index + 1,
                replacement: vec![candidate.replacement],
                removed_defs: Vec::new(),
                exit_effect_delta: MirEffectDelta::Unchanged,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: "inclusive-compare-operand-reversal",
                observations: Vec::new(),
                family_priority: 22,
                estimated_byte_saving: 2,
                estimated_cycle_saving: 2,
            });
        }
    }
    plans
}

pub(in crate::mir6502) fn inclusive_compare_reversal_rank(
    routine: &MirRoutine,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|block| block.ops.iter())
        .filter(|op| {
            crate::mir6502::materialize::analyzed_inclusive_compare_reversal_candidate(op, layout)
                .is_some()
        })
        .count()
}

pub(in crate::mir6502) fn discover_dual_indirect_compares(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    discover_dual_indirect_compares_impl(routine, context, None)
}

pub(in crate::mir6502) fn discover_dual_indirect_compares_with_layout(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    discover_dual_indirect_compares_impl(routine, context, Some(layout))
}

fn discover_dual_indirect_compares_impl(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: Option<&crate::mir6502::materialize::MaterializeLayout>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        if let Some(candidate) =
            crate::mir6502::materialize::direct_indexed_byte_compare_candidate(block)
            && layout.is_some_and(|layout| {
                layout.mem_allows_pure_read_reordering(&candidate.required_upper_bound.mem)
            })
            && byte_index_upper_bound_is_proven(
                routine,
                context,
                block.id,
                candidate.start,
                &candidate.required_upper_bound,
                layout.expect("direct candidate requires a materialization layout"),
            )
        {
            let end = candidate.start + candidate.consumed;
            if let Some(definitions) = prove_removed_window_definitions(
                block.id,
                &block.ops,
                candidate.start,
                end,
                &candidate.replacement,
                context,
            ) {
                plans.push(MirRewritePlan {
                    generation: context.generation(),
                    block: block.id,
                    range: candidate.start..end,
                    replacement: candidate.replacement,
                    removed_defs: definitions
                        .into_iter()
                        .map(|definition| MirRemovedDefinition { definition })
                        .collect(),
                    exit_effect_delta: MirEffectDelta::MaterializedIndexConsumer,
                    change_set: MirChangeSet::prehome_operation_change(),
                    stat: "direct-indexed-byte-compare",
                    observations: Vec::new(),
                    family_priority: 35,
                    estimated_byte_saving: candidate.estimated_byte_saving,
                    estimated_cycle_saving: candidate.estimated_cycle_saving,
                });
            }
        }
        for index in 0..block.ops.len() {
            let Some(candidate) =
                crate::mir6502::materialize::dual_indirect_compare_candidate(block, index)
            else {
                continue;
            };
            let end = index + candidate.consumed;
            let window_end = context.point(MirSite::Op {
                block: block.id,
                op_index: end - 1,
            });
            if candidate.replacement.iter().any(|op| {
                let MirOp::MaterializeIndexedAddress { consumer, .. } = op else {
                    return false;
                };
                !context
                    .pointer_pair_dead_after(*consumer, window_end)
                    .is_proven()
            }) {
                continue;
            }
            let Some(definitions) = prove_removed_window_definitions(
                block.id,
                &block.ops,
                index,
                end,
                &candidate.replacement,
                context,
            ) else {
                continue;
            };
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: index..end,
                replacement: candidate.replacement,
                removed_defs: definitions
                    .into_iter()
                    .map(|definition| MirRemovedDefinition { definition })
                    .collect(),
                exit_effect_delta: MirEffectDelta::MaterializedPointerConsumer,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: candidate.stat,
                observations: Vec::new(),
                family_priority: 15,
                estimated_byte_saving: candidate.estimated_byte_saving,
                estimated_cycle_saving: candidate.estimated_cycle_saving,
            });
        }
    }
    plans
}

pub(in crate::mir6502) fn compare_narrowing_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .map(|block| {
            (0..block.ops.len())
                .filter(|index| {
                    crate::mir6502::materialize::analyzed_compare_narrowing_candidate(
                        &block.ops, *index,
                    )
                    .is_some()
                })
                .count()
        })
        .sum()
}

pub(in crate::mir6502) fn discover_byte_binary_compare_consumers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = byte_binary_compare_chain_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
                continue;
            }
            if let Some(plan) =
                byte_binary_compare_consumer_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn proven_byte_add_word_compare_branches(
    routine: &MirRoutine,
) -> Result<std::collections::BTreeSet<(crate::mir6502::ir::MirBlockId, usize)>, ()> {
    let snapshot =
        PreHomeAnalysisSnapshot::new(routine, MirRoutineGeneration::initial()).map_err(|_| ())?;
    let context = PreHomeRewriteContext::new(&snapshot);
    let mut sites = std::collections::BTreeSet::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            let Some(candidate) =
                crate::mir6502::materialize::analyzed_byte_add_word_compare_candidate(
                    &block.ops, index,
                )
            else {
                continue;
            };
            let end = index + candidate.consumed;
            if prove_removed_window_definitions(
                block.id,
                &block.ops,
                index,
                end,
                &candidate.proof_replacement(),
                &context,
            )
            .is_some()
            {
                sites.insert((block.id, index));
            }
        }
    }
    Ok(sites)
}

pub(in crate::mir6502) fn proven_word_arithmetic_compare_branches(
    routine: &MirRoutine,
) -> Result<std::collections::BTreeSet<(crate::mir6502::ir::MirBlockId, usize)>, ()> {
    let snapshot =
        PreHomeAnalysisSnapshot::new(routine, MirRoutineGeneration::initial()).map_err(|_| ())?;
    let context = PreHomeRewriteContext::new(&snapshot);
    let mut sites = std::collections::BTreeSet::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            let Some(candidate) =
                crate::mir6502::materialize::analyzed_word_arithmetic_compare_candidate(
                    &block.ops, index,
                )
            else {
                continue;
            };
            let end = index + candidate.consumed;
            let window_end = context.point(MirSite::Op {
                block: block.id,
                op_index: end - 1,
            });
            if candidate.clobbered_pointer_pairs.iter().any(|consumer| {
                !context
                    .pointer_pair_dead_after(*consumer, window_end)
                    .is_proven()
            }) {
                continue;
            }
            if prove_removed_window_definitions(
                block.id,
                &block.ops,
                index,
                end,
                &candidate.proof_replacement(),
                &context,
            )
            .is_some()
            {
                sites.insert((block.id, index));
            }
        }
    }
    Ok(sites)
}

pub(in crate::mir6502) fn proven_signed_word_zero_compare_branches(
    routine: &MirRoutine,
) -> Result<std::collections::BTreeSet<(crate::mir6502::ir::MirBlockId, usize)>, ()> {
    let snapshot =
        PreHomeAnalysisSnapshot::new(routine, MirRoutineGeneration::initial()).map_err(|_| ())?;
    let context = PreHomeRewriteContext::new(&snapshot);
    let mut sites = std::collections::BTreeSet::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            let Some(candidate) =
                crate::mir6502::materialize::analyzed_signed_word_zero_compare_candidate(
                    &block.ops, index,
                )
            else {
                continue;
            };
            let Some(condition_temp) = candidate.condition_temp() else {
                continue;
            };
            if !temp_is_used_only_by_block_terminator(routine, block.id, condition_temp) {
                continue;
            }
            let end = index + candidate.consumed;
            if candidate.consumed == 1
                || prove_removed_window_definitions(
                    block.id,
                    &block.ops,
                    index,
                    end,
                    &candidate.proof_replacement(),
                    &context,
                )
                .is_some()
            {
                sites.insert((block.id, index));
            }
        }
    }
    Ok(sites)
}

pub(in crate::mir6502) fn proven_direct_word_equality_compare_branches(
    routine: &MirRoutine,
) -> Result<std::collections::BTreeSet<(crate::mir6502::ir::MirBlockId, usize)>, ()> {
    let snapshot =
        PreHomeAnalysisSnapshot::new(routine, MirRoutineGeneration::initial()).map_err(|_| ())?;
    let context = PreHomeRewriteContext::new(&snapshot);
    let mut sites = std::collections::BTreeSet::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            let Some(candidate) =
                crate::mir6502::materialize::analyzed_direct_word_equality_compare_candidate(
                    &block.ops, index,
                )
            else {
                continue;
            };
            let Some(condition_temp) = candidate.condition_temp() else {
                continue;
            };
            if !temp_is_used_only_by_block_terminator(routine, block.id, condition_temp) {
                continue;
            }
            let end = index + candidate.consumed;
            if prove_removed_window_definitions(
                block.id,
                &block.ops,
                index,
                end,
                &candidate.proof_replacement(),
                &context,
            )
            .is_some()
            {
                sites.insert((block.id, index));
            }
        }
    }
    Ok(sites)
}

pub(in crate::mir6502) fn proven_direct_word_relational_compare_branches(
    routine: &MirRoutine,
) -> Result<std::collections::BTreeSet<(crate::mir6502::ir::MirBlockId, usize)>, ()> {
    let snapshot =
        PreHomeAnalysisSnapshot::new(routine, MirRoutineGeneration::initial()).map_err(|_| ())?;
    let context = PreHomeRewriteContext::new(&snapshot);
    let mut sites = std::collections::BTreeSet::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            let Some(candidate) =
                crate::mir6502::materialize::analyzed_direct_word_relational_compare_candidate(
                    &block.ops, index,
                )
            else {
                continue;
            };
            let Some(condition_temp) = candidate.condition_temp() else {
                continue;
            };
            if !temp_is_used_only_by_block_terminator(routine, block.id, condition_temp) {
                continue;
            }
            let end = index + candidate.consumed;
            if prove_removed_window_definitions(
                block.id,
                &block.ops,
                index,
                end,
                &candidate.proof_replacement(),
                &context,
            )
            .is_some()
            {
                sites.insert((block.id, index));
            }
        }
    }
    Ok(sites)
}

fn temp_is_used_only_by_block_terminator(
    routine: &MirRoutine,
    block: crate::mir6502::ir::MirBlockId,
    temp: crate::mir6502::ir::MirTempId,
) -> bool {
    let mut uses = 0usize;
    let mut expected_terminator_use = false;
    for candidate in &routine.blocks {
        uses += candidate
            .ops
            .iter()
            .map(|op| classify_op(op).temp_use_count(temp))
            .sum::<usize>();
        let terminator_uses = classify_terminator(&candidate.terminator)
            .logical
            .temp_uses
            .iter()
            .filter(|access| access.temp() == temp)
            .count();
        uses += terminator_uses;
        if candidate.id == block && terminator_uses == 1 {
            expected_terminator_use = true;
        }
    }
    expected_terminator_use && uses == 1
}

pub(in crate::mir6502) fn byte_binary_compare_consumer_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .map(|block| {
            (0..block.ops.len())
                .filter(|index| {
                    crate::mir6502::materialize::analyzed_byte_binary_compare_chain_candidate(
                        &block.ops, *index,
                    )
                    .is_some()
                        || crate::mir6502::materialize::analyzed_byte_binary_compare_candidate(
                            &block.ops, *index,
                        )
                        .is_some()
                })
                .count()
        })
        .sum()
}

pub(in crate::mir6502) fn discover_call_arg_producers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = call_arg_producer_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_return_slot_call_arg_forwards(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) =
                return_slot_call_arg_forward_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn return_slot_call_arg_forward_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .map(|block| {
            (0..block.ops.len())
                .filter(|index| {
                    crate::mir6502::materialize::analyzed_return_slot_call_arg_candidate(
                        &block.ops, *index,
                    )
                    .is_some_and(|candidate| !candidate.blocked_home_overlap)
                })
                .count()
        })
        .sum()
}

pub(in crate::mir6502) fn discover_call_arg_exprs(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    config: &crate::mir6502::passes::Mir6502Config,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) =
                call_arg_expr_plan(block.id, &block.ops, index, context, config, layout)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn call_arg_expr_rank(
    routine: &MirRoutine,
    config: &crate::mir6502::passes::Mir6502Config,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> usize {
    routine
        .blocks
        .iter()
        .map(|block| {
            (0..block.ops.len())
                .filter(|index| {
                    crate::mir6502::materialize::analyzed_call_arg_expr_candidate(
                        &block.ops, *index, config, layout,
                    )
                    .is_some()
                })
                .count()
        })
        .sum()
}

pub(in crate::mir6502) fn discover_call_result_store_consumers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) =
                loaded_arg_call_result_store_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
            if let Some(plan) = call_result_store_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_stored_call_result_aliases(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            let Some(candidate) =
                crate::mir6502::materialize::analyzed_stored_call_result_alias_candidate(
                    &block.ops, index,
                )
            else {
                continue;
            };
            let end = index + candidate.consumed;
            if prove_consumed_temp_definition(
                block.id,
                candidate.result_temp,
                index,
                index + 1,
                end - 1,
                context,
            )
            .is_none()
            {
                continue;
            }
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: index..end,
                replacement: candidate.replacement,
                removed_defs: Vec::new(),
                exit_effect_delta: MirEffectDelta::MaterializedStoreConsumer,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: "stored-call-result-alias-forward",
                observations: Vec::new(),
                family_priority: 95,
                estimated_byte_saving: 1,
                estimated_cycle_saving: 1,
            });
        }
    }
    plans
}

pub(in crate::mir6502) fn stored_call_result_alias_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|block| {
            (0..block.ops.len()).filter(|index| {
                crate::mir6502::materialize::analyzed_stored_call_result_alias_candidate(
                    &block.ops, *index,
                )
                .is_some()
            })
        })
        .count()
}

pub(in crate::mir6502) fn call_result_store_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .map(|block| {
            (0..block.ops.len())
                .map(|index| {
                    usize::from(
                        crate::mir6502::materialize::analyzed_call_result_store_candidate(
                            &block.ops, index,
                        )
                        .is_some(),
                    ) + usize::from(
                        crate::mir6502::materialize::analyzed_loaded_arg_call_result_store_candidate(
                            &block.ops, index,
                        )
                        .is_some(),
                    )
                })
                .sum::<usize>()
        })
        .sum()
}

pub(in crate::mir6502) fn discover_store_consumers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    config: &crate::mir6502::passes::Mir6502Config,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, candidate) in crate::mir6502::materialize::analyzed_store_consumer_candidates(
            routine.id, block, config, layout,
        ) {
            let plan = match candidate.stat {
                "indexed-byte-inc-dec-update" => {
                    store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "address-store-consumer" => {
                    address_store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "cast-store-consumer" => {
                    cast_store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "word-carry-chain-store-consumer" => {
                    store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "word-arithmetic-indirect-store-consumer" => {
                    store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "absolute-word-sub-indirect-store-consumer" => {
                    store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "word-arithmetic-dual-indirect-store-consumer" => {
                    store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "word-arithmetic-pointer-store-consumer" => {
                    store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "word-arithmetic-result-consumer" => {
                    store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "byte-mul-add-sub-word-store-consumer" => {
                    byte_mul_add_sub_word_store_consumer_plan(
                        block.id, &block.ops, index, candidate, context,
                    )
                }
                "byte-mul-word-store-consumer" => byte_mul_word_store_consumer_plan(
                    block.id, &block.ops, index, candidate, context,
                ),
                "word-helper-store-consumer" => {
                    word_store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "word-store-consumer" => {
                    word_store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "direct-copy-store-consumer" => {
                    direct_copy_store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "abstract-byte-inc-dec-store-consumer" => {
                    byte_store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "byte-store-consumer" => {
                    byte_store_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                "store-expr-consumer" => {
                    store_expr_consumer_plan(block.id, &block.ops, index, candidate, context)
                }
                _ => None,
            };
            if let Some(plan) = plan {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_word_carry_chain_store_consumers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    config: &crate::mir6502::passes::Mir6502Config,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, candidate) in
            crate::mir6502::materialize::analyzed_word_carry_chain_store_candidates(
                block, config, layout,
            )
        {
            if let Some(plan) = store_consumer_plan(block.id, &block.ops, index, candidate, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_widened_byte_shift_store_consumers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, candidate) in
            crate::mir6502::materialize::analyzed_widened_byte_shift_store_candidates(
                routine.id, block, layout,
            )
        {
            if let Some(plan) = store_consumer_plan(block.id, &block.ops, index, candidate, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn store_consumer_rank(routine: &MirRoutine) -> usize {
    logical_definition_lane_count(routine)
}

pub(in crate::mir6502) fn discover_direct_pointer_temp_rematerializations(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            let Some(candidate) =
                crate::mir6502::materialize::analyzed_direct_pointer_temp_rematerialization_candidate(
                    &block.ops,
                    index,
                )
            else {
                continue;
            };
            if let Some(plan) = direct_pointer_temp_rematerialization_plan(
                block.id, &block.ops, index, candidate, context,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_pointer_rewrites(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = discover_direct_pointer_temp_rematerializations(routine, context);
    plans.extend(discover_pointer_temp_derefs(routine, context, layout));
    plans
}

pub(in crate::mir6502) fn discover_pointer_temp_derefs(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, candidate) in
            crate::mir6502::materialize::analyzed_pointer_temp_deref_candidates(
                block, routine.id, layout,
            )
        {
            if let Some(plan) =
                pointer_temp_deref_plan(block.id, &block.ops, index, candidate, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn pointer_rewrite_rank(routine: &MirRoutine) -> usize {
    logical_definition_lane_count(routine)
}

pub(in crate::mir6502) fn discover_index_rewrites(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, candidate) in crate::mir6502::materialize::analyzed_index_rewrite_candidates(
            routine.id, block, layout,
        ) {
            if candidate
                .required_upper_bound
                .as_ref()
                .is_some_and(|bound| {
                    !byte_index_upper_bound_is_proven(
                        routine,
                        context,
                        block.id,
                        candidate.start,
                        bound,
                        layout,
                    )
                })
            {
                continue;
            }
            let plan = match candidate.stat {
                "delayed-byte-index-consumer" => {
                    delayed_byte_index_plan(block.id, &block.ops, index, candidate, context)
                }
                "indexed-byte-copy" | "adjacent-static-indexed-byte-copy" => {
                    indexed_byte_copy_plan(block.id, &block.ops, index, candidate, context)
                }
                "indexed-word-copy" => {
                    indexed_word_copy_plan(block.id, &block.ops, index, candidate, context)
                }
                "indexed-to-indirect-word-copy" => {
                    indexed_word_copy_plan(block.id, &block.ops, index, candidate, context)
                }
                "dynamic-inline-byte-index" => {
                    dynamic_inline_byte_index_plan(block.id, &block.ops, index, candidate, context)
                }
                "prepare-dynamic-byte-index" => {
                    dynamic_byte_index_plan(block.id, &block.ops, index, candidate, context)
                }
                "prepare-dynamic-word-index" => {
                    dynamic_word_index_plan(block.id, &block.ops, index, candidate, context)
                }
                _ => None,
            };
            if let Some(plan) = plan {
                plans.push(plan);
            }
        }
    }
    plans
}

fn byte_index_upper_bound_is_proven(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    target: crate::mir6502::ir::MirBlockId,
    target_start: usize,
    requirement: &crate::mir6502::materialize::ByteIndexUpperBound,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> bool {
    if !layout.mem_allows_pure_read_reordering(&requirement.mem) {
        return false;
    }
    if layout.mem_address(routine.id, &requirement.mem).is_none() {
        return false;
    }
    routine.blocks.iter().any(|guard| {
        context.block_dominates(guard.id, target)
            && guard_proves_byte_index_upper_bound(
                routine,
                context,
                guard,
                target,
                target_start,
                requirement,
                layout,
            )
    })
}

fn guard_proves_byte_index_upper_bound(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    guard: &crate::mir6502::ir::MirBlock,
    target: crate::mir6502::ir::MirBlockId,
    target_start: usize,
    requirement: &crate::mir6502::materialize::ByteIndexUpperBound,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> bool {
    let crate::mir6502::ir::MirTerminator::Branch {
        cond,
        then_edge,
        else_edge,
    } = &guard.terminator
    else {
        return false;
    };
    let then_path = then_edge.target == target || context.block_dominates(then_edge.target, target);
    let else_path = else_edge.target == target || context.block_dominates(else_edge.target, target);
    let selected_then = match (then_path, else_path) {
        (true, false) => true,
        (false, true) => false,
        _ => return false,
    };
    let selected_successor = if selected_then {
        then_edge.target
    } else {
        else_edge.target
    };

    let (compare_index, op, left, right, condition_is_true) = match cond {
        crate::mir6502::ir::MirCond::BoolValue(MirValue::Def(
            MirDef::VTemp(condition)
            | MirDef::VTempByte {
                id: condition,
                byte: 0,
            },
        )) => {
            let Some((index, op, left, right)) =
                guard.ops.iter().enumerate().find_map(|(index, operation)| {
                    let MirOp::Compare {
                        dst: crate::mir6502::ir::MirCondDest::Temp(dst),
                        op,
                        left,
                        right,
                        width: crate::mir6502::ir::MirWidth::Byte,
                        signed: false,
                    } = operation
                    else {
                        return None;
                    };
                    (*dst == *condition).then_some((index, *op, left, right))
                })
            else {
                return false;
            };
            (index, op, left, right, selected_then)
        }
        crate::mir6502::ir::MirCond::FusedCompare {
            producer,
            flag_test,
        } if producer.block == guard.id => {
            let Some(producer_op) = guard.ops.get(producer.op_index) else {
                return false;
            };
            let MirOp::Compare {
                op,
                left,
                right,
                width: crate::mir6502::ir::MirWidth::Byte,
                signed: false,
                ..
            } = producer_op
            else {
                return false;
            };
            let relation_when_flag_is_true =
                byte_compare_flag_test(*op).is_some_and(|expected| expected == *flag_test);
            if !relation_when_flag_is_true {
                return false;
            }
            (producer.op_index, *op, left, right, selected_then)
        }
        crate::mir6502::ir::MirCond::FlagTest(flag_test) => {
            let Some((index, op, left, right)) =
                guard
                    .ops
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(index, operation)| {
                        let MirOp::Compare {
                            dst: crate::mir6502::ir::MirCondDest::Flags,
                            op,
                            left,
                            right,
                            width: crate::mir6502::ir::MirWidth::Byte,
                            signed: false,
                        } = operation
                        else {
                            return None;
                        };
                        (byte_compare_flag_test(*op) == Some(flag_test.clone()))
                            .then_some((index, *op, left, right))
                    })
            else {
                return false;
            };
            (index, op, left, right, selected_then)
        }
        _ => return false,
    };
    // A plain flag-test branch refers to the machine flags at block exit. This
    // also keeps all three accepted condition shapes on one conservative rule.
    if compare_index + 1 != guard.ops.len() {
        return false;
    }

    let (op, constant, load_index) =
        if let Some((load_index, mem)) = byte_compare_loaded_mem(&guard.ops, compare_index, left) {
            if mem != requirement.mem {
                return false;
            }
            let Some(constant) = byte_compare_constant(right) else {
                return false;
            };
            (op, constant, load_index)
        } else if let Some((load_index, mem)) =
            byte_compare_loaded_mem(&guard.ops, compare_index, right)
        {
            if mem != requirement.mem {
                return false;
            }
            let Some(constant) = byte_compare_constant(left) else {
                return false;
            };
            (reverse_compare_operands_for_bound(op), constant, load_index)
        } else {
            return false;
        };
    let Some(proven_max) = comparison_upper_bound(op, constant, condition_is_true) else {
        return false;
    };
    if proven_max > requirement.max
        || guard.ops[load_index + 1..]
            .iter()
            .any(|op| op_may_write_byte_index_mem(op, routine.id, &requirement.mem, layout))
    {
        return false;
    }

    byte_index_mem_is_stable_to_target(
        routine,
        context,
        guard.id,
        selected_successor,
        target,
        target_start,
        &requirement.mem,
        layout,
    )
}

fn op_may_write_byte_index_mem(
    op: &MirOp,
    routine: crate::mir6502::ir::RoutineId,
    mem: &crate::mir6502::ir::MirMem,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> bool {
    if crate::mir6502::materialize::op_may_write_mem(op, mem) {
        return true;
    }
    let Some(index_address) = layout.mem_address(routine, mem).map(u32::from) else {
        return true;
    };
    classify_op(op).memory.direct_writes.iter().any(|write| {
        let Some(start) = layout.mem_address(routine, &write.base).map(u32::from) else {
            return true;
        };
        index_address >= start && index_address < start + u32::from(write.bytes)
    })
}

fn byte_compare_flag_test(
    op: crate::mir6502::ir::MirCompareOp,
) -> Option<crate::mir6502::ir::MirFlagTest> {
    use crate::mir6502::ir::{MirCompareOp, MirFlagTest};
    match op {
        MirCompareOp::Eq => Some(MirFlagTest::ZSet),
        MirCompareOp::Ne => Some(MirFlagTest::ZClear),
        MirCompareOp::Lt => Some(MirFlagTest::CClear),
        MirCompareOp::Ge => Some(MirFlagTest::CSet),
        MirCompareOp::Le | MirCompareOp::Gt => None,
    }
}

fn byte_compare_loaded_mem(
    ops: &[MirOp],
    before: usize,
    value: &crate::mir6502::ir::MirValue,
) -> Option<(usize, crate::mir6502::ir::MirMem)> {
    if let MirValue::PointerCell(mem) = value {
        return Some((before, mem.clone()));
    }
    let temp = match value {
        MirValue::Def(MirDef::VTemp(temp) | MirDef::VTempByte { id: temp, byte: 0 }) => *temp,
        _ => return None,
    };
    for (index, op) in ops[..before].iter().enumerate().rev() {
        match op {
            MirOp::Load {
                dst: MirDef::VTemp(dst) | MirDef::VTempByte { id: dst, byte: 0 },
                src: crate::mir6502::ir::MirAddr::Direct(mem),
                width: crate::mir6502::ir::MirWidth::Byte,
            } if *dst == temp => return Some((index, mem.clone())),
            _ if classify_op(op)
                .logical
                .temp_defs
                .iter()
                .any(|definition| definition.temp() == temp) =>
            {
                return None;
            }
            _ => {}
        }
    }
    None
}

fn byte_compare_constant(value: &crate::mir6502::ir::MirValue) -> Option<u8> {
    match value {
        MirValue::ConstU8(value) => Some(*value),
        MirValue::ConstU16(value) => u8::try_from(*value).ok(),
        _ => None,
    }
}

fn reverse_compare_operands_for_bound(
    op: crate::mir6502::ir::MirCompareOp,
) -> crate::mir6502::ir::MirCompareOp {
    use crate::mir6502::ir::MirCompareOp;
    match op {
        MirCompareOp::Eq => MirCompareOp::Eq,
        MirCompareOp::Ne => MirCompareOp::Ne,
        MirCompareOp::Lt => MirCompareOp::Gt,
        MirCompareOp::Le => MirCompareOp::Ge,
        MirCompareOp::Gt => MirCompareOp::Lt,
        MirCompareOp::Ge => MirCompareOp::Le,
    }
}

fn comparison_upper_bound(
    op: crate::mir6502::ir::MirCompareOp,
    constant: u8,
    condition_is_true: bool,
) -> Option<u8> {
    use crate::mir6502::ir::MirCompareOp;
    match (op, condition_is_true) {
        (MirCompareOp::Lt, true) | (MirCompareOp::Ge, false) => constant.checked_sub(1),
        (MirCompareOp::Le, true) | (MirCompareOp::Gt, false) => Some(constant),
        (MirCompareOp::Eq, true) | (MirCompareOp::Ne, false) => Some(constant),
        _ => None,
    }
}

fn byte_index_mem_is_stable_to_target(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    guard: crate::mir6502::ir::MirBlockId,
    selected_successor: crate::mir6502::ir::MirBlockId,
    target: crate::mir6502::ir::MirBlockId,
    target_start: usize,
    mem: &crate::mir6502::ir::MirMem,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> bool {
    let cfg = context.cfg();
    let mut can_reach_target_without_guard = BTreeSet::from([target]);
    let mut work = vec![target];
    while let Some(block) = work.pop() {
        for predecessor in cfg.predecessors(block) {
            if *predecessor != guard && can_reach_target_without_guard.insert(*predecessor) {
                work.push(*predecessor);
            }
        }
    }
    if !can_reach_target_without_guard.contains(&selected_successor) {
        return false;
    }

    let mut visited = BTreeSet::new();
    let mut work = vec![selected_successor];
    while let Some(block_id) = work.pop() {
        if block_id == guard
            || !can_reach_target_without_guard.contains(&block_id)
            || !visited.insert(block_id)
        {
            continue;
        }
        let Some(block_index) = cfg.block_index(block_id) else {
            return false;
        };
        let block = &routine.blocks[block_index];
        let ops = if block_id == target {
            let Some(prefix) = block.ops.get(..target_start) else {
                return false;
            };
            prefix
        } else {
            &block.ops
        };
        if ops
            .iter()
            .any(|op| op_may_write_byte_index_mem(op, routine.id, mem, layout))
        {
            return false;
        }
        if block_id != target {
            work.extend(cfg.successors(block_id).iter().copied());
        }
    }
    visited.contains(&target)
}

pub(in crate::mir6502) fn discover_indexed_to_indirect_word_copies(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, candidate) in
            crate::mir6502::materialize::analyzed_indexed_to_indirect_word_copy_candidates(
                block, layout,
            )
        {
            if let Some(plan) =
                indexed_word_copy_plan(block.id, &block.ops, index, candidate, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_private_indirect_word_copies(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, candidate) in
            crate::mir6502::materialize::analyzed_private_indirect_word_copy_candidates(
                block, layout,
            )
        {
            if let Some(plan) =
                indexed_word_copy_plan(block.id, &block.ops, index, candidate, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_indirect_to_indexed_word_copies(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, candidate) in
            crate::mir6502::materialize::analyzed_indirect_to_indexed_word_copy_candidates(
                block, layout,
            )
        {
            if let Some(plan) =
                indexed_word_copy_plan(block.id, &block.ops, index, candidate, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn index_rewrite_rank(routine: &MirRoutine) -> usize {
    logical_definition_lane_count(routine)
}

type IndexRewriteCandidate = crate::mir6502::materialize::IndexRewriteCandidate;

fn delayed_byte_index_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: IndexRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    index_rewrite_plan(block, ops, index, candidate, context)
}

fn indexed_byte_copy_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: IndexRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    index_rewrite_plan(block, ops, index, candidate, context)
}

fn indexed_word_copy_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: IndexRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    index_rewrite_plan(block, ops, index, candidate, context)
}

fn dynamic_inline_byte_index_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: IndexRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    index_rewrite_plan(block, ops, index, candidate, context)
}

fn dynamic_byte_index_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: IndexRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    index_rewrite_plan(block, ops, index, candidate, context)
}

fn dynamic_word_index_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: IndexRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    index_rewrite_plan(block, ops, index, candidate, context)
}

fn index_rewrite_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: IndexRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let end = index.checked_add(candidate.consumed)?;
    if end > ops.len() {
        return None;
    }
    let definitions =
        prove_removed_window_definitions(block, ops, index, end, &candidate.replacement, context)?;
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..end,
        replacement: candidate.replacement,
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::MaterializedIndexConsumer,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: candidate.stat,
        observations: candidate.observations,
        family_priority: candidate.family_priority,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn logical_definition_lane_count(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .map(|op| {
            classify_op(op)
                .logical
                .temp_defs
                .into_iter()
                .map(|access| match access {
                    MirTempAccess::Exact { .. } => 1,
                    MirTempAccess::Full(_) => 2,
                })
                .sum::<usize>()
        })
        .sum()
}

fn direct_pointer_temp_rematerialization_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: crate::mir6502::materialize::PointerRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    pointer_rewrite_plan(
        block,
        ops,
        index,
        candidate,
        context,
        MirEffectDelta::Unchanged,
        "direct-pointer-temp-rematerialization",
        10,
    )
}

fn pointer_temp_deref_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: crate::mir6502::materialize::PointerRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    pointer_rewrite_plan(
        block,
        ops,
        index,
        candidate,
        context,
        MirEffectDelta::MaterializedPointerConsumer,
        "pointer-temp-deref",
        20,
    )
}

#[allow(clippy::too_many_arguments)]
fn pointer_rewrite_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: crate::mir6502::materialize::PointerRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
    exit_effect_delta: MirEffectDelta,
    stat: &'static str,
    family_priority: u16,
) -> Option<MirRewritePlan> {
    let end = index.checked_add(candidate.consumed)?;
    if end > ops.len() {
        return None;
    }
    let definitions =
        prove_removed_window_definitions(block, ops, index, end, &candidate.replacement, context)?;
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..end,
        replacement: candidate.replacement,
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta,
        change_set: MirChangeSet::prehome_operation_change(),
        stat,
        observations: Vec::new(),
        family_priority,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

type StoreConsumerCandidate = crate::mir6502::materialize::StoreConsumerRewriteCandidate;

fn address_store_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: StoreConsumerCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    store_consumer_plan(block, ops, index, candidate, context)
}

fn cast_store_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: StoreConsumerCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    store_consumer_plan(block, ops, index, candidate, context)
}

fn byte_mul_add_sub_word_store_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: StoreConsumerCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    store_consumer_plan(block, ops, index, candidate, context)
}

fn byte_mul_word_store_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: StoreConsumerCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    store_consumer_plan(block, ops, index, candidate, context)
}

fn word_store_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: StoreConsumerCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    store_consumer_plan(block, ops, index, candidate, context)
}

fn direct_copy_store_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: StoreConsumerCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    store_consumer_plan(block, ops, index, candidate, context)
}

fn byte_store_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: StoreConsumerCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    store_consumer_plan(block, ops, index, candidate, context)
}

fn store_expr_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: StoreConsumerCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    store_consumer_plan(block, ops, index, candidate, context)
}

fn store_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    candidate: crate::mir6502::materialize::StoreConsumerRewriteCandidate,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let end = index.checked_add(candidate.consumed)?;
    if end > ops.len() {
        return None;
    }
    let (estimated_byte_saving, estimated_cycle_saving) =
        if candidate.stat == "word-arithmetic-dual-indirect-store-consumer" {
            let original = super::posthome::estimated_6502_cost(&ops[index..end]);
            let replacement = super::posthome::estimated_6502_cost(&candidate.replacement);
            let byte_saving = original.0.saturating_sub(replacement.0);
            if byte_saving == 0 {
                return None;
            }
            (byte_saving, original.1.saturating_sub(replacement.1))
        } else {
            (1, 1)
        };
    let definitions =
        prove_removed_window_definitions(block, ops, index, end, &candidate.replacement, context)?;
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..end,
        replacement: candidate.replacement,
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: if candidate.stat == "word-helper-store-consumer" {
            MirEffectDelta::MaterializedRuntimeHelperStoreConsumer
        } else {
            MirEffectDelta::MaterializedStoreConsumer
        },
        change_set: MirChangeSet::prehome_operation_change(),
        stat: candidate.stat,
        observations: Vec::new(),
        family_priority: candidate.family_priority,
        estimated_byte_saving,
        estimated_cycle_saving,
    })
}

pub(in crate::mir6502) fn prove_removed_window_definitions(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    start: usize,
    end: usize,
    replacement: &[MirOp],
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<Vec<crate::mir6502::analysis::use_def::MirDefSite>> {
    let replacement_lanes = replacement
        .iter()
        .flat_map(|op| classify_op(op).logical.temp_defs)
        .flat_map(|access| match access {
            MirTempAccess::Exact { temp, byte } => vec![MirTempLane { temp, byte }],
            MirTempAccess::Full(temp) => {
                vec![MirTempLane { temp, byte: 0 }, MirTempLane { temp, byte: 1 }]
            }
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut definitions = Vec::new();
    for (index, op) in ops.iter().enumerate().take(end).skip(start) {
        let site = MirSite::Op {
            block,
            op_index: index,
        };
        for access in classify_op(op).logical.temp_defs {
            let temp = match access {
                MirTempAccess::Full(temp) | MirTempAccess::Exact { temp, .. } => temp,
            };
            definitions.extend(
                context
                    .definitions_at(temp, site)
                    .into_iter()
                    .filter(|definition| !replacement_lanes.contains(&definition.lane)),
            );
        }
    }
    definitions.sort_unstable();
    definitions.dedup();
    if definitions.is_empty() {
        return None;
    }
    let removed_lanes = definitions
        .iter()
        .map(|definition| definition.lane)
        .collect::<std::collections::BTreeSet<_>>();
    if replacement.iter().any(|op| {
        classify_op(op)
            .logical
            .temp_uses
            .into_iter()
            .flat_map(|access| match access {
                MirTempAccess::Exact { temp, byte } => vec![MirTempLane { temp, byte }],
                MirTempAccess::Full(temp) => {
                    vec![MirTempLane { temp, byte: 0 }, MirTempLane { temp, byte: 1 }]
                }
            })
            .any(|lane| removed_lanes.contains(&lane))
    }) {
        return None;
    }

    let window_end = context.point(MirSite::Op {
        block,
        op_index: end - 1,
    });
    for definition in &definitions {
        let mut reached_use = false;
        for use_index in start..end {
            for usage in context.uses_at(
                definition.lane.temp,
                MirSite::Op {
                    block,
                    op_index: use_index,
                },
            ) {
                if !usage.requirement.requires(definition.lane)
                    || !context
                        .definition_reaches_use(*definition, usage)
                        .is_proven()
                {
                    continue;
                }
                if !matches!(
                    context.unique_reaching_definition(usage, definition.lane),
                    MirProof::Proven(reaching) if reaching == *definition
                ) {
                    return None;
                }
                reached_use = true;
            }
        }
        if !reached_use
            || !context
                .temp_definition_dead_after(*definition, window_end)
                .is_proven()
        {
            return None;
        }
    }
    Some(definitions)
}

fn call_result_store_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate = crate::mir6502::materialize::analyzed_call_result_store_candidate(ops, index)?;
    let definitions = prove_consumed_temp_definition(
        block,
        candidate.result_temp,
        index,
        index + 1,
        index + 1,
        context,
    )?;
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 2,
        replacement: candidate.replacement.into_iter().collect(),
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::ForwardedCallResultStore {
            base: candidate.return_slot,
            width: candidate.result_width,
            selected_arg_register: None,
        },
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "analyzed-call-result-store-consumer",
        observations: Vec::new(),
        family_priority: 90,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn loaded_arg_call_result_store_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_loaded_arg_call_result_store_candidate(ops, index)?;
    let mut definitions = prove_consumed_temp_definition(
        block,
        candidate.arg_temp,
        index,
        index + 1,
        index + 1,
        context,
    )?;
    definitions.extend(prove_consumed_temp_definition(
        block,
        candidate.result_temp,
        index + 1,
        index + 2,
        index + 2,
        context,
    )?);
    definitions.sort_unstable();
    definitions.dedup();
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 3,
        replacement: candidate.replacement.into_iter().collect(),
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::ForwardedCallResultStore {
            base: candidate.return_slot,
            width: candidate.result_width,
            selected_arg_register: Some(crate::mir6502::ir::MirReg::A),
        },
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "analyzed-call-result-loaded-arg-store-consumer",
        observations: Vec::new(),
        family_priority: 80,
        estimated_byte_saving: 2,
        estimated_cycle_saving: 2,
    })
}

fn prove_consumed_temp_definition(
    block: crate::mir6502::ir::MirBlockId,
    temp: crate::mir6502::ir::MirTempId,
    definition_index: usize,
    consumer_index: usize,
    window_end_index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<Vec<crate::mir6502::analysis::use_def::MirDefSite>> {
    let definition_site = MirSite::Op {
        block,
        op_index: definition_index,
    };
    let consumer_site = MirSite::Op {
        block,
        op_index: consumer_index,
    };
    let definitions = context.definitions_at(temp, definition_site);
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let uses = context
            .uses_at(temp, consumer_site)
            .into_iter()
            .filter(|usage| usage.requirement.requires(definition.lane))
            .collect::<Vec<_>>();
        if uses.is_empty()
            || uses.iter().any(|usage| {
                !matches!(
                    context.unique_reaching_definition(*usage, definition.lane),
                    MirProof::Proven(reaching) if reaching == *definition
                )
            })
            || !context
                .temp_definition_dead_after(
                    *definition,
                    context.point(MirSite::Op {
                        block,
                        op_index: window_end_index,
                    }),
                )
                .is_proven()
        {
            return None;
        }
    }
    Some(definitions)
}

pub(in crate::mir6502) fn discover_unused_lea_addrs(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = unused_lea_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
        }
    }
    plans
}

fn unused_lea_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let MirOp::LeaAddr {
        dst: MirDef::VTemp(temp),
        ..
    } = ops.get(index)?
    else {
        return None;
    };
    let site = MirSite::Op {
        block,
        op_index: index,
    };
    let definitions = context.definitions_at(*temp, site);
    if definitions.is_empty()
        || definitions.iter().any(|definition| {
            !context
                .temp_definition_dead_after(*definition, context.point(site))
                .is_proven()
        })
    {
        return None;
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 1,
        replacement: Vec::new(),
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "analyzed-unused-lea-addr",
        observations: Vec::new(),
        family_priority: 10,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn compare_operand_producer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_compare_operand_rewrite_candidate(ops, index)?;
    let compare_index = index + candidate.consumed - 1;
    let compare_site = MirSite::Op {
        block,
        op_index: compare_index,
    };
    let mut definitions = Vec::new();
    for producer_index in index..compare_index {
        let site = MirSite::Op {
            block,
            op_index: producer_index,
        };
        for access in classify_op(&ops[producer_index]).logical.temp_defs {
            let temp = match access {
                MirTempAccess::Full(temp) | MirTempAccess::Exact { temp, .. } => temp,
            };
            definitions.extend(context.definitions_at(temp, site));
        }
    }
    definitions.sort_unstable();
    definitions.dedup();
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let temp = definition.lane.temp;
        for usage_index in index + 1..=compare_index {
            for usage in context.uses_at(
                temp,
                MirSite::Op {
                    block,
                    op_index: usage_index,
                },
            ) {
                if usage.requirement.requires(definition.lane)
                    && !matches!(
                        context.unique_reaching_definition(usage, definition.lane),
                        MirProof::Proven(reaching) if reaching == *definition
                    )
                {
                    return None;
                }
            }
        }
        if !context
            .temp_definition_dead_after(*definition, context.point(compare_site))
            .is_proven()
        {
            return None;
        }
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + candidate.consumed,
        replacement: vec![candidate.replacement],
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "compare-operand-consumer-prebranch",
        observations: Vec::new(),
        family_priority: 20,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn addressed_byte_compare_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate = crate::mir6502::materialize::addressed_byte_compare_candidate(ops, index)?;
    let end = index + candidate.consumed;
    let definitions =
        prove_removed_window_definitions(block, ops, index, end, &candidate.replacement, context)?;
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..end,
        replacement: candidate.replacement,
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::SelectedResultRegister(crate::mir6502::ir::MirReg::A),
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "addressed-byte-compare-consumer",
        observations: Vec::new(),
        family_priority: 25,
        estimated_byte_saving: 6,
        estimated_cycle_saving: 6,
    })
}

fn compare_narrowing_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate = crate::mir6502::materialize::analyzed_compare_narrowing_candidate(ops, index)?;
    let producer_site = MirSite::Op {
        block,
        op_index: index,
    };
    let compare_site = MirSite::Op {
        block,
        op_index: index + 1,
    };
    let definition = context
        .definitions_at(candidate.temp, producer_site)
        .into_iter()
        .find(|definition| definition.lane.byte == 1)?;
    let high_uses = context
        .uses_at(candidate.temp, compare_site)
        .into_iter()
        .filter(|usage| usage.requirement.requires(definition.lane))
        .collect::<Vec<_>>();
    if high_uses.is_empty()
        || high_uses.iter().any(|usage| {
            !matches!(
                context.unique_reaching_definition(*usage, definition.lane),
                MirProof::Proven(reaching) if reaching == definition
            )
        })
        || !context
            .temp_definition_dead_after(definition, context.point(compare_site))
            .is_proven()
    {
        return None;
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 2,
        replacement: candidate.replacement.into_iter().collect(),
        removed_defs: vec![MirRemovedDefinition { definition }],
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "byte-derived-word-bitwise-zero-compare-narrowed",
        observations: Vec::new(),
        family_priority: 30,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn byte_binary_compare_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_byte_binary_compare_candidate(ops, index)?;
    let producer_site = MirSite::Op {
        block,
        op_index: index,
    };
    let compare_site = MirSite::Op {
        block,
        op_index: index + 1,
    };
    let definitions = context.definitions_at(candidate.temp, producer_site);
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let uses = context
            .uses_at(candidate.temp, compare_site)
            .into_iter()
            .filter(|usage| usage.requirement.requires(definition.lane))
            .collect::<Vec<_>>();
        if uses.is_empty()
            || uses.iter().any(|usage| {
                !matches!(
                    context.unique_reaching_definition(*usage, definition.lane),
                    MirProof::Proven(reaching) if reaching == *definition
                )
            })
            || !context
                .temp_definition_dead_after(*definition, context.point(compare_site))
                .is_proven()
        {
            return None;
        }
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 2,
        replacement: candidate.replacement.into_iter().collect(),
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::SelectedResultRegister(crate::mir6502::ir::MirReg::A),
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "byte-binary-compare-consumer",
        observations: Vec::new(),
        family_priority: 40,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn byte_binary_compare_chain_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_byte_binary_compare_chain_candidate(ops, index)?;
    let end = index + candidate.consumed;
    let definitions =
        prove_removed_window_definitions(block, ops, index, end, &candidate.replacement, context)?;
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..end,
        replacement: candidate.replacement,
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::SelectedResultRegister(crate::mir6502::ir::MirReg::A),
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "byte-binary-compare-producer-chain",
        observations: Vec::new(),
        family_priority: 35,
        estimated_byte_saving: 4,
        estimated_cycle_saving: 6,
    })
}

fn call_arg_producer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate = crate::mir6502::materialize::analyzed_call_arg_producer_candidate(ops, index)?;
    let call_index = index + candidate.consumed - 1;
    let call_site = MirSite::Op {
        block,
        op_index: call_index,
    };
    let mut definitions = Vec::new();
    for producer_index in index..call_index {
        let site = MirSite::Op {
            block,
            op_index: producer_index,
        };
        for access in classify_op(&ops[producer_index]).logical.temp_defs {
            let temp = match access {
                MirTempAccess::Full(temp) | MirTempAccess::Exact { temp, .. } => temp,
            };
            if candidate.temps.contains(&temp) {
                definitions.extend(context.definitions_at(temp, site));
            }
        }
    }
    definitions.sort_unstable();
    definitions.dedup();
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let temp = definition.lane.temp;
        for usage_index in index + 1..=call_index {
            for usage in context.uses_at(
                temp,
                MirSite::Op {
                    block,
                    op_index: usage_index,
                },
            ) {
                if usage.requirement.requires(definition.lane)
                    && !matches!(
                        context.unique_reaching_definition(usage, definition.lane),
                        MirProof::Proven(reaching) if reaching == *definition
                    )
                {
                    return None;
                }
            }
        }
        if !context
            .temp_definition_dead_after(*definition, context.point(call_site))
            .is_proven()
        {
            return None;
        }
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + candidate.consumed,
        replacement: vec![candidate.replacement],
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "call-arg-producer",
        observations: Vec::new(),
        family_priority: 50,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn return_slot_call_arg_forward_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_return_slot_call_arg_candidate(ops, index)?;
    if candidate.blocked_home_overlap {
        return None;
    }
    let producer_site = MirSite::Op {
        block,
        op_index: index,
    };
    let consumer_site = MirSite::Op {
        block,
        op_index: index + 1,
    };
    let definitions = context.definitions_at(candidate.temp, producer_site);
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let uses = context
            .uses_at(candidate.temp, consumer_site)
            .into_iter()
            .filter(|usage| usage.requirement.requires(definition.lane))
            .collect::<Vec<_>>();
        if uses.is_empty()
            || uses.iter().any(|usage| {
                !matches!(
                    context.unique_reaching_definition(*usage, definition.lane),
                    MirProof::Proven(reaching) if reaching == *definition
                )
            })
            || !context
                .temp_definition_dead_after(*definition, context.point(consumer_site))
                .is_proven()
        {
            return None;
        }
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 2,
        replacement: candidate.replacement.into_iter().collect(),
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::ForwardedReturnSlot {
            base: candidate.return_slot,
            width: candidate.result_width,
        },
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "return-slot-call-arg-forwards",
        observations: Vec::new(),
        family_priority: 60,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn call_arg_expr_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
    config: &crate::mir6502::passes::Mir6502Config,
    layout: &crate::mir6502::materialize::MaterializeLayout,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_call_arg_expr_candidate(ops, index, config, layout)?;
    let call_index = index + candidate.consumed - 1;
    let call_site = MirSite::Op {
        block,
        op_index: call_index,
    };
    let mut definitions = Vec::new();
    for producer_index in index..call_index {
        let site = MirSite::Op {
            block,
            op_index: producer_index,
        };
        for access in classify_op(&ops[producer_index]).logical.temp_defs {
            let temp = match access {
                MirTempAccess::Full(temp) | MirTempAccess::Exact { temp, .. } => temp,
            };
            if candidate.temps.contains(&temp) {
                definitions.extend(context.definitions_at(temp, site));
            }
        }
    }
    definitions.sort_unstable();
    definitions.dedup();
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let temp = definition.lane.temp;
        for usage_index in index + 1..=call_index {
            for usage in context.uses_at(
                temp,
                MirSite::Op {
                    block,
                    op_index: usage_index,
                },
            ) {
                if usage.requirement.requires(definition.lane)
                    && !matches!(
                        context.unique_reaching_definition(usage, definition.lane),
                        MirProof::Proven(reaching) if reaching == *definition
                    )
                {
                    return None;
                }
            }
        }
        if !context
            .temp_definition_dead_after(*definition, context.point(call_site))
            .is_proven()
        {
            return None;
        }
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + candidate.consumed,
        replacement: candidate.replacement,
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::MaterializedCallArguments,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "call-arg-expr-consumer",
        observations: [
            (
                "indexed-word-load-ax-call-arg",
                candidate.indexed_word_loads,
            ),
            (
                "indexed-word-arithmetic-ax-call-arg",
                candidate.indexed_word_arithmetic,
            ),
            (
                "word-arithmetic-direct-action-call-arg",
                candidate.direct_word_arithmetic,
            ),
            (
                "indexed-byte-direct-fixed-action-call-arg",
                candidate.direct_indexed_byte_fixed_args,
            ),
            (
                "direct-binary-rhs-call-arg-candidates",
                candidate.direct_binary_rhs_candidates,
            ),
            (
                "direct-binary-rhs-call-arg",
                candidate.direct_binary_rhs_selected,
            ),
            (
                "direct-binary-rhs-call-arg-blocked-overlap",
                candidate.direct_binary_rhs_blocked_overlap,
            ),
            (
                "direct-binary-rhs-call-arg-blocked-nonordinary",
                candidate.direct_binary_rhs_blocked_nonordinary,
            ),
            (
                "pure-ax-byte-call-schedule",
                candidate.pure_ax_byte_schedule,
            ),
            (
                "paired-word-shift-call-args",
                candidate.paired_word_shift_args,
            ),
        ]
        .into_iter()
        .filter(|(_, count)| *count != 0)
        .collect(),
        family_priority: 70,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    use crate::mir6502::ir::{
        MirAddr, MirArgHome, MirBinaryOp, MirBlock, MirCallAbi, MirCallArg, MirCallResult,
        MirCallTarget, MirCarryIn, MirCarryOut, MirCompareOp, MirCond, MirCondDest, MirEdge,
        MirEdgeArg, MirEffects, MirFlagTest, MirFrame, MirGlobal, MirGlobalBacking, MirMem,
        MirProgram, MirRegisterSet, MirResultHome, MirRoutineAbi, MirTemp, MirTempId,
        MirTerminator, MirValue, MirWidth, RoutineId,
    };
    use crate::mir6502::rewrite::driver::MirPreHomeRewriteDriver;

    fn routine(blocks: Vec<MirBlock>) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "pilot".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: crate::mir6502::ir::MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn lea(temp: u32) -> MirOp {
        MirOp::LeaAddr {
            dst: MirDef::VTemp(MirTempId(temp)),
            target: MirMem::Absolute(0x4000),
            width: MirWidth::Word,
        }
    }

    #[test]
    fn unused_lea_folds_but_terminator_and_successor_uses_block_it() {
        let mut local = routine(vec![block(0, vec![lea(1)], MirTerminator::Return)]);
        let mut driver = MirPreHomeRewriteDriver::default();
        let result = driver
            .run_fixed_point(&mut local, discover_prehome_pilots)
            .unwrap();
        assert!(local.blocks[0].ops.is_empty());
        assert_eq!(result.applied, 1);
        assert!(result.converged);
        let second = driver
            .run_fixed_point(&mut local, discover_prehome_pilots)
            .unwrap();
        assert_eq!((second.applied, second.rounds), (0, 1));

        let mut terminator_use = routine(vec![
            block(
                0,
                vec![lea(1)],
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Word,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        let blocked = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut terminator_use, discover_prehome_pilots)
            .unwrap();
        assert_eq!(
            (blocked.applied, terminator_use.blocks[0].ops.len()),
            (0, 1)
        );

        let mut successor_use = routine(vec![
            block(
                0,
                vec![lea(1)],
                MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
            ),
            block(
                1,
                vec![MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    width: MirWidth::Word,
                }],
                MirTerminator::Return,
            ),
        ]);
        let blocked = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut successor_use, discover_prehome_pilots)
            .unwrap();
        assert_eq!((blocked.applied, successor_use.blocks[0].ops.len()), (0, 1));
    }

    #[test]
    fn compare_operand_producer_folds_with_definition_identity_proof() {
        let mut routine = routine(vec![block(
            0,
            vec![
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(1)),
                    value: 7,
                    width: MirWidth::Byte,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU8(9),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ],
            MirTerminator::Return,
        )]);
        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut routine, discover_prehome_pilots)
            .unwrap();
        assert_eq!(result.applied, 1);
        assert!(matches!(
            &routine.blocks[0].ops[..],
            [MirOp::Compare {
                left: MirValue::ConstU8(7),
                ..
            }]
        ));
    }

    #[test]
    fn inclusive_compare_reversal_runs_through_prehome_driver() {
        let condition = MirTempId(0);
        let compare = MirOp::Compare {
            dst: MirCondDest::Temp(condition),
            op: MirCompareOp::Le,
            left: MirValue::PointerCell(MirMem::Absolute(0x00E0)),
            right: MirValue::PointerCell(MirMem::Absolute(0x00E1)),
            width: MirWidth::Byte,
            signed: false,
        };
        let mut routine = routine(vec![
            block(
                0,
                vec![compare],
                MirTerminator::Branch {
                    cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(condition))),
                    then_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(1)),
                    else_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(2)),
                },
            ),
            block(1, Vec::new(), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
        ]);
        routine.temps = vec![MirTemp { id: condition }];
        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![routine.clone()],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);
        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                &mut routine,
                |routine, context| discover_inclusive_compare_reversals(routine, context, &layout),
                |routine| inclusive_compare_reversal_rank(routine, &layout),
            )
            .unwrap();

        assert_eq!(result.applied, 1);
        assert_eq!(
            result
                .applied_by_stat
                .get("inclusive-compare-operand-reversal"),
            Some(&1)
        );
        assert!(matches!(
            routine.blocks[0].ops.as_slice(),
            [MirOp::Compare {
                op: MirCompareOp::Ge,
                left: MirValue::PointerCell(MirMem::Absolute(0x00E1)),
                right: MirValue::PointerCell(MirMem::Absolute(0x00E0)),
                ..
            }]
        ));
    }

    #[test]
    fn compare_operand_producer_subsumes_two_loaded_byte_consumer() {
        let mut routine = routine(vec![block(
            0,
            vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                    width: MirWidth::Byte,
                },
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(2)),
                    src: MirAddr::Direct(MirMem::Absolute(0x4001)),
                    width: MirWidth::Byte,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ],
            MirTerminator::Return,
        )]);
        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut routine, discover_compare_producers)
            .unwrap();
        assert_eq!(result.applied, 1);
        assert!(matches!(
            &routine.blocks[0].ops[..],
            [MirOp::Compare {
                left: MirValue::PointerCell(MirMem::Absolute(0x4000)),
                right: MirValue::PointerCell(MirMem::Absolute(0x4001)),
                ..
            }]
        ));
    }

    #[test]
    fn compare_operand_producer_preserves_later_terminator_and_successor_uses() {
        fn word_compare_ops() -> Vec<MirOp> {
            vec![
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(1)),
                    value: 7,
                    width: MirWidth::Word,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU16(9),
                    width: MirWidth::Word,
                    signed: false,
                },
            ]
        }

        fn assert_blocked(mut candidate: MirRoutine) {
            let original_ops = candidate.blocks[0].ops.clone();
            let result = MirPreHomeRewriteDriver::default()
                .run_fixed_point(&mut candidate, discover_compare_producers)
                .unwrap();
            assert_eq!(result.applied, 0);
            assert_eq!(candidate.blocks[0].ops, original_ops);
        }

        let mut local_ops = word_compare_ops();
        local_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 1,
            }),
            width: MirWidth::Byte,
        });
        assert_blocked(routine(vec![block(0, local_ops, MirTerminator::Return)]));

        assert_blocked(routine(vec![
            block(
                0,
                word_compare_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Word,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]));

        for (src, width) in [
            (
                MirValue::Def(MirDef::VTempByte {
                    id: MirTempId(1),
                    byte: 1,
                }),
                MirWidth::Byte,
            ),
            (MirValue::Def(MirDef::VTemp(MirTempId(1))), MirWidth::Word),
        ] {
            assert_blocked(routine(vec![
                block(
                    0,
                    word_compare_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width,
                    }],
                    MirTerminator::Return,
                ),
            ]));
        }
    }

    #[test]
    fn compare_narrowing_uses_lane_aware_routine_deadness() {
        fn narrowing_ops() -> Vec<MirOp> {
            vec![
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(2)),
                    value: 7,
                    width: MirWidth::Byte,
                },
                MirOp::Binary {
                    op: crate::mir6502::ir::MirBinaryOp::And,
                    dst: MirDef::VTemp(MirTempId(1)),
                    left: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                    right: MirValue::ConstU16(3),
                    width: MirWidth::Word,
                    carry_in: None,
                    carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU16(0),
                    width: MirWidth::Word,
                    signed: false,
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    discover_compare_narrowing,
                    compare_narrowing_rank,
                )
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(0, narrowing_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[1..=2],
            [
                MirOp::Binary {
                    width: MirWidth::Byte,
                    ..
                },
                MirOp::Compare {
                    width: MirWidth::Byte,
                    ..
                }
            ]
        ));

        let mut local_high_use = narrowing_ops();
        local_high_use.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 1,
            }),
            width: MirWidth::Byte,
        });
        let mut local_high_use = routine(vec![block(0, local_high_use, MirTerminator::Return)]);
        assert_eq!(run(&mut local_high_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                narrowing_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Word,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for (src, width) in [
            (
                MirValue::Def(MirDef::VTempByte {
                    id: MirTempId(1),
                    byte: 1,
                }),
                MirWidth::Byte,
            ),
            (MirValue::Def(MirDef::VTemp(MirTempId(1))), MirWidth::Word),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    narrowing_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }
    }

    #[test]
    fn byte_binary_compare_selection_uses_routine_deadness() {
        fn compare_ops() -> Vec<MirOp> {
            vec![
                MirOp::Binary {
                    op: crate::mir6502::ir::MirBinaryOp::Xor,
                    dst: MirDef::VTemp(MirTempId(1)),
                    left: MirValue::ConstU8(0xaa),
                    right: MirValue::ConstU8(0x55),
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU8(0),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    discover_byte_binary_compare_consumers,
                    byte_binary_compare_consumer_rank,
                )
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(0, compare_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::Binary {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    ..
                },
                MirOp::Compare {
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    ..
                }
            ]
        ));

        let mut local_use_ops = compare_ops();
        local_use_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut local_use = routine(vec![block(0, local_use_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut local_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                compare_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for src in [
            MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 0,
            }),
            MirValue::Def(MirDef::VTemp(MirTempId(1))),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    compare_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }
    }

    #[test]
    fn byte_binary_compare_selection_folds_loaded_rhs_chain_before_branch_expansion() {
        let compare_mem = MirMem::Global {
            id: crate::nir::SymbolId(1),
            offset: 0,
        };
        let binary_mem = MirMem::Global {
            id: crate::nir::SymbolId(2),
            offset: 0,
        };
        let mut candidate = routine(vec![
            block(
                0,
                vec![
                    MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(1)),
                        src: MirAddr::Direct(compare_mem.clone()),
                        width: MirWidth::Byte,
                    },
                    MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(2)),
                        src: MirAddr::Direct(binary_mem.clone()),
                        width: MirWidth::Byte,
                    },
                    MirOp::Binary {
                        op: crate::mir6502::ir::MirBinaryOp::Sub,
                        dst: MirDef::VTemp(MirTempId(3)),
                        left: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                        right: MirValue::ConstU8(1),
                        width: MirWidth::Byte,
                        carry_in: None,
                        carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                    },
                    MirOp::Compare {
                        dst: MirCondDest::Temp(MirTempId(4)),
                        op: MirCompareOp::Le,
                        left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        right: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                        width: MirWidth::Byte,
                        signed: false,
                    },
                ],
                MirTerminator::Branch {
                    cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(4)))),
                    then_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(1)),
                    else_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(2)),
                },
            ),
            block(1, Vec::new(), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
        ]);

        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                &mut candidate,
                discover_byte_binary_compare_consumers,
                byte_binary_compare_consumer_rank,
            )
            .unwrap();

        assert_eq!(result.applied, 1);
        assert!(matches!(
            &candidate.blocks[0].ops[..],
            [
                MirOp::Binary {
                    op: crate::mir6502::ir::MirBinaryOp::Sub,
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    left: MirValue::PointerCell(mem),
                    right: MirValue::ConstU8(1),
                    carry_in: Some(crate::mir6502::ir::MirCarryIn::Set),
                    ..
                },
                MirOp::Compare {
                    dst: MirCondDest::Temp(MirTempId(4)),
                    op: MirCompareOp::Ge,
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    right: MirValue::PointerCell(compare),
                    ..
                }
            ] if mem == &binary_mem && compare == &compare_mem
        ));
    }

    #[test]
    fn call_arg_producer_uses_routine_definition_identity_and_deadness() {
        fn call_ops() -> Vec<MirOp> {
            vec![
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(1)),
                    value: 7,
                    width: MirWidth::Byte,
                },
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: MirCallAbi {
                        params: vec![MirArgHome::Reg(crate::mir6502::ir::MirReg::A)],
                        result: None,
                        clobbers: MirRegisterSet::default(),
                        preserves: MirRegisterSet::default(),
                    },
                    args: vec![MirCallArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                        home: MirArgHome::Reg(crate::mir6502::ir::MirReg::A),
                    }],
                    result: None,
                    effects: MirEffects::default(),
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point(candidate, discover_call_arg_producers)
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(0, call_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [MirOp::Call {
                args,
                ..
            }] if matches!(args[0].value, MirValue::ConstU8(7))
        ));

        let mut local_use_ops = call_ops();
        local_use_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut local_use = routine(vec![block(0, local_use_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut local_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                call_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for src in [
            MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 0,
            }),
            MirValue::Def(MirDef::VTemp(MirTempId(1))),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    call_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }
    }

    #[test]
    fn return_slot_call_arg_forward_uses_routine_deadness() {
        fn call_abi(result: Option<MirResultHome>, params: Vec<MirArgHome>) -> MirCallAbi {
            MirCallAbi {
                params,
                result,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            }
        }

        fn call_ops() -> Vec<MirOp> {
            vec![
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: call_abi(Some(MirResultHome::ReturnSlot { offset: 0 }), Vec::new()),
                    args: Vec::new(),
                    result: Some(MirCallResult {
                        dst: MirDef::VTemp(MirTempId(1)),
                        width: MirWidth::Byte,
                        home: MirResultHome::ReturnSlot { offset: 0 },
                    }),
                    effects: MirEffects::default(),
                },
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(2)),
                    abi: call_abi(None, vec![MirArgHome::Reg(crate::mir6502::ir::MirReg::A)]),
                    args: vec![MirCallArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                        home: MirArgHome::Reg(crate::mir6502::ir::MirReg::A),
                    }],
                    result: None,
                    effects: MirEffects::default(),
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    discover_return_slot_call_arg_forwards,
                    return_slot_call_arg_forward_rank,
                )
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(0, call_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::Call { result: None, .. },
                MirOp::Call { args, .. }
            ] if matches!(args[0].value, MirValue::PointerCell(_))
        ));

        let mut later_ops = call_ops();
        later_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut later_use = routine(vec![block(0, later_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut later_use), 0);

        let mut successor_use = routine(vec![
            block(
                0,
                call_ops(),
                MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
            ),
            block(
                1,
                vec![MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            ),
        ]);
        assert_eq!(run(&mut successor_use), 0);
    }

    #[test]
    fn call_result_store_selection_uses_lane_aware_routine_deadness() {
        fn result_store_ops(width: MirWidth) -> Vec<MirOp> {
            let result = MirDef::VTemp(MirTempId(10));
            vec![
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: MirCallAbi {
                        params: Vec::new(),
                        result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                        clobbers: MirRegisterSet::default(),
                        preserves: MirRegisterSet::default(),
                    },
                    args: Vec::new(),
                    result: Some(MirCallResult {
                        dst: result.clone(),
                        width,
                        home: MirResultHome::ReturnSlot { offset: 0 },
                    }),
                    effects: MirEffects::default(),
                },
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                    src: MirValue::Def(result),
                    width,
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    discover_call_result_store_consumers,
                    call_result_store_rank,
                )
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(
            0,
            result_store_ops(MirWidth::Byte),
            MirTerminator::Return,
        )]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::Call { result: None, .. },
                MirOp::Store {
                    src: MirValue::PointerCell(MirMem::FixedZeroPage(_)),
                    ..
                }
            ]
        ));

        let mut terminator_use = routine(vec![
            block(
                0,
                result_store_ops(MirWidth::Byte),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(10))),
                        width: MirWidth::Byte,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for (src, width) in [
            (
                MirValue::Def(MirDef::VTempByte {
                    id: MirTempId(10),
                    byte: 1,
                }),
                MirWidth::Byte,
            ),
            (MirValue::Def(MirDef::VTemp(MirTempId(10))), MirWidth::Word),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    result_store_ops(MirWidth::Word),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5100)),
                        src,
                        width,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }
    }

    #[test]
    fn loaded_arg_call_result_store_selection_proves_both_definitions_dead() {
        fn loaded_result_store_ops() -> Vec<MirOp> {
            let arg = MirDef::VTemp(MirTempId(20));
            let result = MirDef::VTemp(MirTempId(21));
            vec![
                MirOp::Load {
                    dst: arg.clone(),
                    src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                    width: MirWidth::Byte,
                },
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: MirCallAbi {
                        params: vec![MirArgHome::Reg(crate::mir6502::ir::MirReg::A)],
                        result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                        clobbers: MirRegisterSet::default(),
                        preserves: MirRegisterSet::default(),
                    },
                    args: vec![MirCallArg {
                        value: MirValue::Def(arg),
                        width: MirWidth::Byte,
                        home: MirArgHome::Reg(crate::mir6502::ir::MirReg::A),
                    }],
                    result: Some(MirCallResult {
                        dst: result.clone(),
                        width: MirWidth::Byte,
                        home: MirResultHome::ReturnSlot { offset: 0 },
                    }),
                    effects: MirEffects::default(),
                },
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                    src: MirValue::Def(result),
                    width: MirWidth::Byte,
                },
            ]
        }

        fn discover_loaded(
            routine: &MirRoutine,
            context: &PreHomeRewriteContext<'_, '_>,
        ) -> Vec<MirRewritePlan> {
            routine
                .blocks
                .iter()
                .flat_map(|block| {
                    (0..block.ops.len()).filter_map(|index| {
                        loaded_arg_call_result_store_plan(block.id, &block.ops, index, context)
                    })
                })
                .collect()
        }

        fn discover_loaded_rank(routine: &MirRoutine) -> usize {
            routine
                .blocks
                .iter()
                .map(|block| {
                    (0..block.ops.len())
                        .filter(|index| {
                            crate::mir6502::materialize::analyzed_loaded_arg_call_result_store_candidate(
                                &block.ops,
                                *index,
                            )
                            .is_some()
                        })
                        .count()
                })
                .sum()
        }

        let run = |candidate: &mut MirRoutine| {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(candidate, discover_loaded, |routine| {
                    discover_loaded_rank(routine)
                })
                .unwrap()
                .applied
        };

        let mut local = routine(vec![block(
            0,
            loaded_result_store_ops(),
            MirTerminator::Return,
        )]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::Load {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    ..
                },
                MirOp::Call { result: None, .. },
                MirOp::Store {
                    src: MirValue::PointerCell(MirMem::FixedZeroPage(_)),
                    ..
                }
            ]
        ));

        for temp in [MirTempId(20), MirTempId(21)] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    loaded_result_store_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5100)),
                        src: MirValue::Def(MirDef::VTemp(temp)),
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }
    }

    #[test]
    fn call_arg_expression_selection_uses_routine_deadness() {
        fn call_ops() -> Vec<MirOp> {
            vec![
                MirOp::Binary {
                    op: crate::mir6502::ir::MirBinaryOp::Add,
                    dst: MirDef::VTemp(MirTempId(1)),
                    left: MirValue::ConstU8(1),
                    right: MirValue::ConstU8(2),
                    width: MirWidth::Byte,
                    carry_in: Some(crate::mir6502::ir::MirCarryIn::Clear),
                    carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                },
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: MirCallAbi {
                        params: vec![MirArgHome::Reg(crate::mir6502::ir::MirReg::A)],
                        result: None,
                        clobbers: MirRegisterSet::default(),
                        preserves: MirRegisterSet::default(),
                    },
                    args: vec![MirCallArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                        home: MirArgHome::Reg(crate::mir6502::ir::MirReg::A),
                    }],
                    result: None,
                    effects: MirEffects::default(),
                },
            ]
        }

        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);
        let config = crate::mir6502::passes::Mir6502Config::default();
        let run = |candidate: &mut MirRoutine| {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    |routine, context| discover_call_arg_exprs(routine, context, &config, &layout),
                    |routine| call_arg_expr_rank(routine, &config, &layout),
                )
                .unwrap()
                .applied
        };

        let mut local = routine(vec![block(0, call_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::Move {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    ..
                },
                MirOp::Binary {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    ..
                },
                MirOp::Call { .. }
            ]
        ));

        let mut later_ops = call_ops();
        later_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut later_use = routine(vec![block(0, later_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut later_use), 0);
    }

    #[test]
    fn store_consumers_use_routine_definition_identity_and_deadness() {
        fn copy_ops() -> Vec<MirOp> {
            vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x4100)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    width: MirWidth::Byte,
                },
            ]
        }

        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);
        let config = crate::mir6502::passes::Mir6502Config::default();
        let run = |candidate: &mut MirRoutine| {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    |routine, context| discover_store_consumers(routine, context, &config, &layout),
                    store_consumer_rank,
                )
                .unwrap()
                .applied
        };

        let mut local = routine(vec![block(0, copy_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::Move {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    src: MirValue::PointerCell(MirMem::Absolute(0x4000)),
                    ..
                },
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x4100)),
                    src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    ..
                }
            ]
        ));

        let mut later_ops = copy_ops();
        later_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x4200)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut later_use = routine(vec![block(0, later_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut later_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                copy_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for src in [
            MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 0,
            }),
            MirValue::Def(MirDef::VTemp(MirTempId(1))),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    copy_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x4200)),
                        src,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }

        let mut byte_result_use = routine(vec![block(
            0,
            vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(2)),
                    src: MirAddr::Direct(MirMem::Absolute(0x4300)),
                    width: MirWidth::Byte,
                },
                MirOp::Binary {
                    op: crate::mir6502::ir::MirBinaryOp::Sub,
                    dst: MirDef::VTemp(MirTempId(3)),
                    left: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                    right: MirValue::ConstU8(1),
                    width: MirWidth::Byte,
                    carry_in: Some(crate::mir6502::ir::MirCarryIn::Set),
                    carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x4300)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x4301)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                    width: MirWidth::Byte,
                },
            ],
            MirTerminator::Return,
        )]);
        assert_eq!(run(&mut byte_result_use), 0);

        let mut delayed_index = routine(vec![block(
            0,
            vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(5)),
                    src: MirAddr::Direct(MirMem::Global {
                        id: crate::nir::SymbolId(5),
                        offset: 0,
                    }),
                    width: MirWidth::Byte,
                },
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(6)),
                    src: MirAddr::Direct(MirMem::Absolute(0x4400)),
                    width: MirWidth::Byte,
                },
                MirOp::Binary {
                    op: crate::mir6502::ir::MirBinaryOp::Sub,
                    dst: MirDef::VTemp(MirTempId(7)),
                    left: MirValue::Def(MirDef::VTemp(MirTempId(6))),
                    right: MirValue::ConstU8(1),
                    width: MirWidth::Byte,
                    carry_in: Some(crate::mir6502::ir::MirCarryIn::Set),
                    carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                },
                MirOp::Store {
                    dst: MirAddr::ComputedIndex {
                        base: MirValue::GlobalAddr(crate::nir::SymbolId(10)),
                        index: MirValue::Def(MirDef::VTemp(MirTempId(5))),
                        elem_size: 1,
                        offset: 0,
                    },
                    src: MirValue::Def(MirDef::VTemp(MirTempId(7))),
                    width: MirWidth::Byte,
                },
            ],
            MirTerminator::Return,
        )]);
        assert_eq!(run(&mut delayed_index), 1);
        assert!(!delayed_index.blocks[0].ops.iter().any(|op| {
            matches!(
                op,
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(5)),
                    ..
                } | MirOp::Binary {
                    dst: MirDef::VTemp(MirTempId(7)),
                    ..
                }
            )
        }));
    }

    #[test]
    fn pointer_rewrites_use_definition_identity_and_routine_deadness() {
        fn pointer_ops() -> Vec<MirOp> {
            vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                    width: MirWidth::Word,
                },
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(2)),
                    src: MirAddr::Deref {
                        ptr: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        offset: 3,
                    },
                    width: MirWidth::Byte,
                },
            ]
        }

        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);
        let run = |candidate: &mut MirRoutine| {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    |routine, context| discover_pointer_rewrites(routine, context, &layout),
                    pointer_rewrite_rank,
                )
                .unwrap()
                .applied
        };

        let mut local = routine(vec![block(0, pointer_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [MirOp::Load {
                src: MirAddr::Deref {
                    ptr: MirValue::Word { lo, hi },
                    offset: 3,
                },
                ..
            }] if matches!(lo.as_ref(), MirValue::PointerCell(MirMem::Absolute(0x4000)))
                && matches!(hi.as_ref(), MirValue::PointerCell(MirMem::Absolute(0x4001)))
        ));

        let mut selected = routine(vec![block(0, pointer_ops(), MirTerminator::Return)]);
        let selected_count = MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                &mut selected,
                |routine, context| discover_pointer_temp_derefs(routine, context, &layout),
                pointer_rewrite_rank,
            )
            .unwrap()
            .applied;
        assert_eq!(selected_count, 1);
        assert!(matches!(
            &selected.blocks[0].ops[..],
            [
                MirOp::MaterializeAddress { .. },
                MirOp::LoadIndirect { offset: 3, .. }
            ]
        ));

        let mut later_ops = pointer_ops();
        later_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 0,
            }),
            width: MirWidth::Byte,
        });
        let mut later_use = routine(vec![block(0, later_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut later_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                pointer_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Word,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for (src, width) in [
            (
                MirValue::Def(MirDef::VTempByte {
                    id: MirTempId(1),
                    byte: 1,
                }),
                MirWidth::Byte,
            ),
            (MirValue::Def(MirDef::VTemp(MirTempId(1))), MirWidth::Word),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    pointer_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }

        let mut redefined_ops = pointer_ops();
        redefined_ops.insert(
            1,
            MirOp::LoadImm {
                dst: MirDef::VTemp(MirTempId(1)),
                value: 0x1234,
                width: MirWidth::Word,
            },
        );
        let mut redefined = routine(vec![block(0, redefined_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut redefined), 0);

        let mut clobbered_ops = pointer_ops();
        clobbered_ops.insert(
            1,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
                src: MirValue::ConstU16(0),
                width: MirWidth::Word,
            },
        );
        let mut clobbered = routine(vec![block(0, clobbered_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut clobbered), 0);
    }

    #[test]
    fn index_rewrites_use_transactional_definition_and_cfg_proofs() {
        fn indexed_byte_copy_ops() -> Vec<MirOp> {
            vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirAddr::ComputedIndex {
                        base: MirValue::GlobalAddr(crate::nir::SymbolId(1)),
                        index: MirValue::ConstU8(2),
                        elem_size: 1,
                        offset: 0,
                    },
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::ComputedIndex {
                        base: MirValue::GlobalAddr(crate::nir::SymbolId(2)),
                        index: MirValue::ConstU8(3),
                        elem_size: 1,
                        offset: 0,
                    },
                    src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    width: MirWidth::Byte,
                },
            ]
        }

        fn delayed_index_ops() -> Vec<MirOp> {
            vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(2)),
                    src: MirAddr::Direct(MirMem::Global {
                        id: crate::nir::SymbolId(3),
                        offset: 0,
                    }),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::ComputedIndex {
                        base: MirValue::GlobalAddr(crate::nir::SymbolId(4)),
                        index: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                        elem_size: 1,
                        offset: 0,
                    },
                    src: MirValue::ConstU8(7),
                    width: MirWidth::Byte,
                },
            ]
        }

        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);
        let run = |candidate: &mut MirRoutine| {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    |routine, context| discover_index_rewrites(routine, context, &layout),
                    index_rewrite_rank,
                )
                .unwrap()
                .applied
        };

        let mut local = routine(vec![block(
            0,
            indexed_byte_copy_ops(),
            MirTerminator::Return,
        )]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::MaterializeIndexedAddress { .. },
                MirOp::MaterializeIndexedAddress { .. },
                MirOp::LoadIndirect { .. },
                MirOp::StoreIndirect { .. }
            ]
        ));

        let mut later_ops = indexed_byte_copy_ops();
        later_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut later_use = routine(vec![block(0, later_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut later_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                indexed_byte_copy_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for src in [
            MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 0,
            }),
            MirValue::Def(MirDef::VTemp(MirTempId(1))),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    indexed_byte_copy_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5001)),
                        src,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }

        let mut delayed = routine(vec![block(0, delayed_index_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut delayed), 1);
        assert!(!delayed.blocks[0].ops.iter().any(|op| {
            classify_op(op).uses_temp(MirTempId(2))
                || classify_op(op)
                    .logical
                    .temp_defs
                    .iter()
                    .any(|access| access.temp() == MirTempId(2))
        }));

        let mut delayed_successor = routine(vec![
            block(
                0,
                delayed_index_ops(),
                MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
            ),
            block(
                1,
                vec![MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x5002)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            ),
        ]);
        assert_eq!(run(&mut delayed_successor), 0);

        let mut clobbered_ops = delayed_index_ops();
        clobbered_ops.insert(
            1,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Global {
                    id: crate::nir::SymbolId(3),
                    offset: 0,
                }),
                src: MirValue::ConstU8(9),
                width: MirWidth::Byte,
            },
        );
        let mut clobbered = routine(vec![block(0, clobbered_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut clobbered), 0);
    }

    #[test]
    fn index_rewrite_effect_contract_covers_all_selection_families() {
        let computed = |symbol, index, elem_size| MirAddr::ComputedIndex {
            base: MirValue::GlobalAddr(crate::nir::SymbolId(symbol)),
            index,
            elem_size,
            offset: 0,
        };
        let pointer = |symbol, index, elem_size| MirAddr::PointerIndex {
            ptr: MirMem::Global {
                id: crate::nir::SymbolId(symbol),
                offset: 0,
            },
            index,
            elem_size,
            offset: 0,
        };
        let index_load = |temp, symbol| MirOp::Load {
            dst: MirDef::VTemp(MirTempId(temp)),
            src: MirAddr::Direct(MirMem::Global {
                id: crate::nir::SymbolId(symbol),
                offset: 0,
            }),
            width: MirWidth::Byte,
        };

        let cases = vec![
            (
                "indexed-word-copy",
                vec![
                    MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(1)),
                        src: computed(1, MirValue::ConstU8(2), 2),
                        width: MirWidth::Word,
                    },
                    MirOp::Store {
                        dst: computed(2, MirValue::ConstU8(3), 2),
                        src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Word,
                    },
                ],
            ),
            (
                "indexed-word-copy",
                vec![
                    MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(10)),
                        src: MirAddr::Direct(MirMem::Global {
                            id: crate::nir::SymbolId(9),
                            offset: 0,
                        }),
                        width: MirWidth::Word,
                    },
                    index_load(11, 10),
                    MirOp::Load {
                        dst: MirDef::VTemp(MirTempId(12)),
                        src: MirAddr::ComputedIndex {
                            base: MirValue::Def(MirDef::VTemp(MirTempId(10))),
                            index: MirValue::Def(MirDef::VTemp(MirTempId(11))),
                            elem_size: 2,
                            offset: 0,
                        },
                        width: MirWidth::Word,
                    },
                    MirOp::Store {
                        dst: MirAddr::ComputedIndex {
                            base: MirValue::Def(MirDef::VTemp(MirTempId(10))),
                            index: MirValue::Def(MirDef::VTemp(MirTempId(11))),
                            elem_size: 2,
                            offset: 1,
                        },
                        src: MirValue::Def(MirDef::VTemp(MirTempId(12))),
                        width: MirWidth::Word,
                    },
                ],
            ),
            (
                "dynamic-inline-byte-index",
                vec![
                    MirOp::LeaAddr {
                        dst: MirDef::VTemp(MirTempId(2)),
                        target: MirMem::Global {
                            id: crate::nir::SymbolId(3),
                            offset: 0,
                        },
                        width: MirWidth::Word,
                    },
                    index_load(3, 4),
                    MirOp::Store {
                        dst: MirAddr::ComputedIndex {
                            base: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                            index: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                            elem_size: 1,
                            offset: 0,
                        },
                        src: MirValue::ConstU8(5),
                        width: MirWidth::Byte,
                    },
                ],
            ),
            (
                "prepare-dynamic-byte-index",
                vec![
                    index_load(4, 5),
                    MirOp::Store {
                        dst: pointer(6, MirValue::Def(MirDef::VTemp(MirTempId(4))), 1),
                        src: MirValue::ConstU8(6),
                        width: MirWidth::Byte,
                    },
                ],
            ),
            (
                "prepare-dynamic-word-index",
                vec![
                    index_load(5, 7),
                    MirOp::Store {
                        dst: pointer(8, MirValue::Def(MirDef::VTemp(MirTempId(5))), 2),
                        src: MirValue::ConstU16(0x1234),
                        width: MirWidth::Word,
                    },
                ],
            ),
        ];

        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);
        for (stat, ops) in cases {
            let mut candidate = routine(vec![block(0, ops, MirTerminator::Return)]);
            let result = MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    &mut candidate,
                    |routine, context| discover_index_rewrites(routine, context, &layout),
                    index_rewrite_rank,
                )
                .unwrap_or_else(|error| panic!("{stat}: {error:?}"));
            assert_eq!(result.applied_by_stat.get(stat), Some(&1), "{stat}");
        }
    }

    fn adjacent_static_copy_ops(
        array: crate::nir::SymbolId,
        index: crate::nir::SymbolId,
    ) -> Vec<MirOp> {
        vec![
            MirOp::LeaAddr {
                dst: MirDef::VTemp(MirTempId(1)),
                target: MirMem::Global {
                    id: array,
                    offset: 0,
                },
                width: MirWidth::Word,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(2)),
                src: MirAddr::Direct(MirMem::Global {
                    id: index,
                    offset: 0,
                }),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::VTemp(MirTempId(3)),
                left: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(4)),
                src: MirAddr::Direct(MirMem::Global {
                    id: index,
                    offset: 0,
                }),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(5)),
                src: MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    index: MirValue::Def(MirDef::VTemp(MirTempId(4))),
                    elem_size: 1,
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    index: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(MirDef::VTemp(MirTempId(5))),
                width: MirWidth::Byte,
            },
        ]
    }

    fn adjacent_static_compare_ops(
        array: crate::nir::SymbolId,
        index: crate::nir::SymbolId,
    ) -> Vec<MirOp> {
        vec![
            MirOp::LeaAddr {
                dst: MirDef::VTemp(MirTempId(1)),
                target: MirMem::Global {
                    id: array,
                    offset: 0,
                },
                width: MirWidth::Word,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(2)),
                src: MirAddr::Direct(MirMem::Global {
                    id: index,
                    offset: 0,
                }),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::VTemp(MirTempId(3)),
                left: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(4)),
                src: MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    index: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                    elem_size: 1,
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
            MirOp::LeaAddr {
                dst: MirDef::VTemp(MirTempId(5)),
                target: MirMem::Global {
                    id: array,
                    offset: 0,
                },
                width: MirWidth::Word,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(6)),
                src: MirAddr::Direct(MirMem::Global {
                    id: index,
                    offset: 0,
                }),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(7)),
                src: MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(MirTempId(5))),
                    index: MirValue::Def(MirDef::VTemp(MirTempId(6))),
                    elem_size: 1,
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Temp(MirTempId(8)),
                op: MirCompareOp::Lt,
                left: MirValue::Def(MirDef::VTemp(MirTempId(4))),
                right: MirValue::Def(MirDef::VTemp(MirTempId(7))),
                width: MirWidth::Byte,
                signed: false,
            },
        ]
    }

    fn empty_layout_program() -> MirProgram {
        MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: Vec::new(),
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        }
    }

    fn direct_index_layout_program(
        array: crate::nir::SymbolId,
        index: crate::nir::SymbolId,
    ) -> MirProgram {
        MirProgram {
            globals: vec![
                MirGlobal {
                    id: array,
                    name: "array".to_string(),
                    kind: "array".to_string(),
                    width: None,
                    storage_size: 256,
                    backing: MirGlobalBacking::Ordinary { offset: 0 },
                    init: None,
                },
                MirGlobal {
                    id: index,
                    name: "index".to_string(),
                    kind: "byte".to_string(),
                    width: Some(MirWidth::Byte),
                    storage_size: 1,
                    backing: MirGlobalBacking::Ordinary { offset: 256 },
                    init: None,
                },
            ],
            ..empty_layout_program()
        }
    }

    #[test]
    fn adjacent_static_indexed_copy_uses_shared_y_when_guard_proves_no_wrap() {
        let array = crate::nir::SymbolId(10);
        let index = crate::nir::SymbolId(11);
        let guard = block(
            0,
            vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Lt,
                left: MirValue::PointerCell(MirMem::Global {
                    id: index,
                    offset: 0,
                }),
                right: MirValue::ConstU8(0xFE),
                width: MirWidth::Byte,
                signed: false,
            }],
            MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::CClear),
                then_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(1)),
                else_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(2)),
            },
        );
        let mut candidate = routine(vec![
            guard,
            block(
                1,
                adjacent_static_copy_ops(array, index),
                MirTerminator::Return,
            ),
            block(2, Vec::new(), MirTerminator::Return),
        ]);
        let program = direct_index_layout_program(array, index);
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);

        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                &mut candidate,
                |routine, context| discover_index_rewrites(routine, context, &layout),
                index_rewrite_rank,
            )
            .expect("adjacent direct copy selection succeeds");

        assert_eq!(
            result.applied_by_stat["adjacent-static-indexed-byte-copy"],
            1
        );
        assert!(matches!(
            candidate.blocks[1].ops.as_slice(),
            [
                MirOp::Load {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::Y),
                    ..
                },
                MirOp::Load {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    src: MirAddr::AbsoluteIndexedY {
                        base: MirMem::Global { offset: 0, .. }
                    },
                    ..
                },
                MirOp::Store {
                    dst: MirAddr::AbsoluteIndexedY {
                        base: MirMem::Global { offset: 1, .. }
                    },
                    ..
                }
            ]
        ));
    }

    #[test]
    fn adjacent_static_indexed_copy_keeps_wrapping_fallback_without_bound() {
        let array = crate::nir::SymbolId(10);
        let index = crate::nir::SymbolId(11);
        let mut candidate = routine(vec![block(
            0,
            adjacent_static_copy_ops(array, index),
            MirTerminator::Return,
        )]);
        let program = direct_index_layout_program(array, index);
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);

        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                &mut candidate,
                |routine, context| discover_index_rewrites(routine, context, &layout),
                index_rewrite_rank,
            )
            .expect("wrapping fallback selection succeeds");

        assert_eq!(result.applied_by_stat["indexed-byte-copy"], 1);
        assert!(
            !result
                .applied_by_stat
                .contains_key("adjacent-static-indexed-byte-copy")
        );
        assert!(
            candidate.blocks[0]
                .ops
                .iter()
                .any(|op| matches!(op, MirOp::LoadIndirect { .. }))
        );
    }

    #[test]
    fn adjacent_static_indexed_compare_uses_one_y_when_guard_proves_no_wrap() {
        let array = crate::nir::SymbolId(20);
        let index = crate::nir::SymbolId(21);
        let guard = block(
            0,
            vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Lt,
                left: MirValue::PointerCell(MirMem::Global {
                    id: index,
                    offset: 0,
                }),
                right: MirValue::ConstU8(0xFF),
                width: MirWidth::Byte,
                signed: false,
            }],
            MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::CClear),
                then_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(1)),
                else_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(3)),
            },
        );
        let mut candidate = routine(vec![
            guard,
            block(
                1,
                adjacent_static_compare_ops(array, index),
                MirTerminator::Branch {
                    cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(8)))),
                    then_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(2)),
                    else_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(3)),
                },
            ),
            block(2, Vec::new(), MirTerminator::Return),
            block(3, Vec::new(), MirTerminator::Return),
        ]);
        let program = direct_index_layout_program(array, index);
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);

        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut candidate, |routine, context| {
                discover_dual_indirect_compares_with_layout(routine, context, &layout)
            })
            .expect("adjacent direct compare selection succeeds");

        assert_eq!(result.applied_by_stat["direct-indexed-byte-compare"], 1);
        assert!(matches!(
            candidate.blocks[1].ops.as_slice(),
            [
                MirOp::Load {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::Y),
                    ..
                },
                MirOp::CompareDirectIndexedBytes {
                    left: MirMem::Global { offset: 1, .. },
                    right: MirMem::Global { offset: 0, .. },
                    ..
                }
            ]
        ));
    }

    #[test]
    fn adjacent_static_indexed_compare_is_not_direct_without_no_wrap_proof() {
        let array = crate::nir::SymbolId(20);
        let index = crate::nir::SymbolId(21);
        let mut candidate = routine(vec![
            block(
                0,
                adjacent_static_compare_ops(array, index),
                MirTerminator::Branch {
                    cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(8)))),
                    then_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(1)),
                    else_edge: MirEdge::plain(crate::mir6502::ir::MirBlockId(2)),
                },
            ),
            block(1, Vec::new(), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
        ]);
        let program = direct_index_layout_program(array, index);
        let layout = crate::mir6502::materialize::MaterializeLayout::new(&program, 0x3000);

        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut candidate, |routine, context| {
                discover_dual_indirect_compares_with_layout(routine, context, &layout)
            })
            .expect("wrapping compare fallback selection succeeds");

        assert!(
            !result
                .applied_by_stat
                .contains_key("direct-indexed-byte-compare")
        );
        assert!(
            !candidate.blocks[0]
                .ops
                .iter()
                .any(|op| matches!(op, MirOp::CompareDirectIndexedBytes { .. }))
        );
    }

    #[test]
    fn later_generation_rejects_a_stale_plan() {
        let mut routine = routine(vec![block(0, vec![lea(1)], MirTerminator::Return)]);
        let snapshot = crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot::new(
            &routine,
            MirRoutineGeneration::initial(),
        )
        .unwrap();
        let plans = discover_prehome_pilots(&routine, &PreHomeRewriteContext::new(&snapshot));
        drop(snapshot);
        let mut driver = MirPreHomeRewriteDriver::default();
        driver.apply_batch(&mut routine, plans.clone()).unwrap();
        assert!(matches!(
            driver.apply_batch(&mut routine, plans),
            Err(crate::mir6502::rewrite::driver::MirRewriteError::StalePlan { .. })
        ));
    }
}
