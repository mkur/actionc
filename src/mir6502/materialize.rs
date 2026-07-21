use super::analysis::cfg::{MirCfg, MirCfgError};
use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirAddr, MirAddressConsumer, MirArgHome, MirBinaryOp, MirBlockId, MirCallAbi, MirCallArg,
    MirCallTarget, MirCarryIn, MirCarryOut, MirCompareOp, MirCond, MirCondDest, MirDef, MirEffects,
    MirFixedZpSlot, MirFlagTest, MirMem, MirMemoryEffect, MirOp, MirOpRef, MirPointerPair,
    MirProgram, MirReg, MirResultHome, MirRuntimeHelper, MirSpillId, MirTemp, MirTempId,
    MirTerminator, MirUnaryOp, MirUpdateOp, MirValue, MirWidth, RoutineId,
};
use super::passes::Mir6502Config;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
use super::ir::MirZpSlot;

mod abi;
mod block_args;
mod call_result;
mod calls;
mod cfg;
mod compare_branch;
mod dead_spills;
mod defs;
mod flags;
mod home_census;
mod indexes;
mod layout;
mod lea;
mod memory;
mod peepholes;
mod pointers;
mod regs;
mod runtime;
mod spills;
mod ssa_lite;
mod stats;
mod store_consumers;
mod temp_liveness;
mod temp_rewrite;
mod temp_uses;
mod temp_widths;
mod temps;
mod values;
mod word_values;
mod zp;

use super::rewrite::driver::{
    MirPostHomeRewriteDriver, MirPreHomeRewriteDriver, MirRewriteRunResult,
};
use super::rewrite::pilots::{
    byte_binary_compare_consumer_rank, compare_narrowing_rank,
    discover_byte_binary_compare_consumers, discover_compare_narrowing, discover_compare_producers,
    discover_index_rewrites, discover_pointer_rewrites, discover_unused_lea_addrs,
};
use abi::{prepend_action_abi_param_prologue, width_bytes};
use block_args::lower_block_arguments;
use calls::{
    CallArgExprRewriteCandidate, CallArgProducerRewriteCandidate, CallResultStoreRewriteCandidate,
    LoadedArgCallResultStoreRewriteCandidate, call_arg_expr_rewrite_candidate,
    call_arg_producer_rewrite_candidate, call_result_store_rewrite_candidate,
    forward_param_register_homes, loaded_arg_call_result_store_rewrite_candidate, materialize_call,
    try_materialize_forwarded_call_result_store,
    try_materialize_loaded_arg_forwarded_call_result_store,
};
use calls::{ReturnSlotCallArgForwardCandidate, return_slot_call_arg_forward_candidate};
#[cfg(test)]
use calls::{fold_call_arg_producers, forward_return_slot_call_result_args};
#[cfg(test)]
use calls::{
    try_fuse_call_result_store_consumer, try_fuse_loaded_arg_call_result_store_consumer,
    try_materialize_call_arg_expr_producers,
};
use cfg::collapse_empty_jump_blocks;
#[cfg(test)]
use compare_branch::fold_compare_operand_producers_before_branches;
use compare_branch::{
    ByteBinaryCompareRewriteCandidate, CompareNarrowingCandidate, CompareOperandRewriteCandidate,
    byte_binary_compare_rewrite_candidate, byte_bitwise_zero_compare_narrowing_candidate,
    compare_branch_plan, compare_operand_rewrite_candidate, expand_compare_branch_consumers,
};
#[cfg(test)]
use compare_branch::{
    byte_binary_compare_consumer_observation, try_fuse_byte_binary_compare_consumer,
    try_fuse_byte_compare_consumer, try_fuse_compare_operand_producers,
};
use dead_spills::remove_dead_spill_stores;
use defs::{op_def, split_def_as_temp};
use flags::{
    op_clobbers_unknown_flag_or_a_effects, op_has_opaque_flag_or_a_effects, op_overwrites_carry,
    op_overwrites_overflow, op_uses_previous_carry, op_writes_flags, terminator_consumes_flags,
};
use home_census::{
    HomeFateTracker, apply_register_home_plan, record_final_home_allocations,
    record_home_demand_census,
};
#[cfg(test)]
use indexes::{
    DelayedByteIndexExpr, materialize_computed_index_read, materialize_computed_index_write,
    materialize_delayed_byte_indexed_read, materialize_delayed_byte_indexed_write,
    materialize_dynamic_byte_index_read, materialize_dynamic_byte_index_write,
};
use indexes::{
    collect_delayed_byte_index_plan, indexed_addr_parts,
    indexed_word_copy_rematerialized_producer_ops, materialize_base_address,
    materialize_index_to_y, materialize_indexed_read_to_def, materialize_indexed_write_from_value,
    storage_address_value, try_fuse_dynamic_inline_byte_index, try_fuse_indexed_byte_copy,
    try_fuse_indexed_word_copy, try_prepare_dynamic_byte_index, try_prepare_dynamic_word_index,
};
pub(super) use layout::MaterializeLayout;
use lea::{lower_address_to_def, lower_lea_addrs_with_final_layout};
use memory::{
    mem_is_read_after, op_definitely_writes_mem, op_may_have_unknown_memory_effects,
    op_may_write_mem, op_reads_mem,
};
#[cfg(test)]
use peepholes::{
    dead_private_scratch_store_at, fixed_pointer_consumer, fold_dead_private_scratch_stores,
    fold_dead_reg_writes_before_overwrite, fold_indirect_byte_const_compounds,
    fold_indirect_byte_const_stores, fold_indirect_byte_direct_compounds,
    fold_indirect_y_const_stores, fold_word_array_store_value_staging, staged_compare_rhs_at,
};
use peepholes::{fold_structural_cleanup_tail, fold_structural_prefix};
use pointers::{
    is_zero_word_value, materialize_pointer_deref_address, materialize_pointer_deref_read,
    materialize_pointer_deref_read_byte, materialize_pointer_deref_write,
    materialize_pointer_deref_write_byte, pointer_value_from_mem,
    select_direct_pointer_temp_rematerialization, select_pointer_temp_deref,
    word_value_splits_to_constants,
};
#[cfg(test)]
use pointers::{rematerialize_direct_pointer_temp_derefs, try_fuse_pointer_temp_deref};
use regs::{op_reads_reg, op_writes_reg, value_reads_reg};
use runtime::{
    ensure_helper_decl, helper_for_binary, materialize_runtime_helper_binary,
    runtime_helper_result_width,
};
pub(super) use runtime::{helper_abi, helper_effects};
#[cfg(test)]
use spills::can_remove_spill_store_reload_pair_at;
#[cfg(test)]
pub(super) use spills::spill_accounting_for_routine;
use spills::{
    can_remove_spill_reload_at, can_remove_spill_reload_before_later_a_use,
    color_basic_block_spills, color_routine_spills, fold_indirect_load_spill_consumers,
    forward_block_local_spill_accumulator, lower_block_local_byte_spills_to_zero_page,
    op_may_clobber_reg, prune_unused_spills,
};
#[cfg(test)]
use ssa_lite::scan_ssa_lite_v2_observability;
#[cfg(test)]
use ssa_lite::{
    MirCopyPropByteValue, SsaLiteValueKey, classify_mir_copy_prop_byte_value,
    fold_mir_copy_prop_const_uses, fold_mir_copy_prop_const_uses_with_terminator,
    scan_ssa_lite_block_env, temp_byte_binary_candidate_reason_for_test,
};
use ssa_lite::{
    fold_mir_copy_prop_const_uses_with_terminator_and_live_out, fold_ssa_lite_byte_loads,
    fold_ssa_lite_single_predecessor_loads, record_ssa_lite_block_facts,
    record_ssa_lite_v2_observability,
};
use stats::{MirPeepholeStats, maybe_report_peepholes};
use store_consumers::{
    materialize_value_to_mem, select_byte_mul_add_sub_word_store_consumer,
    select_byte_store_consumer, select_direct_copy_store_consumer, select_store_expr_producers,
    select_word_store_consumer, try_fuse_byte_mul_word_store_consumer,
    try_fuse_cast_store_consumer,
};
#[cfg(test)]
use store_consumers::{
    try_fuse_byte_mul_add_sub_word_store_consumer, try_fuse_byte_store_consumer,
    try_fuse_direct_copy_store_consumer, try_fuse_word_store_consumer,
    try_materialize_store_expr_producers,
};
use temp_liveness::{MirTempLiveSet, analyze_temp_liveness, record_temp_liveness_observability};
use temp_rewrite::{replace_temp_addr, replace_temp_value};
use temp_uses::{
    count_call_target_temp_uses, count_value_temp_uses, op_uses_temp, op_uses_temp_more_than_once,
    terminator_uses_temp, value_uses_temp,
};
use temp_widths::collect_temp_widths;
use temps::{
    cleanup_pre_materialization_temp_artifacts,
    cleanup_pre_materialization_temp_artifacts_with_liveness, def_is_used_after,
    materialize_fused_compare_dest, materialize_temp_ops, materialize_terminator, store_a_to_spill,
    temp_is_used_after,
};
use values::{
    offset_mem, return_slot_mem, split_address, split_def, split_value, split_value_as_word,
    split_value_with_storage_widths, split_value_with_temp_widths,
};
use word_values::forward_unique_word_load_address_consumers;
use zp::{
    allocate_zero_page_slots, find_zp_range, mark_zp_range, reserve_pointer_scratch_slots,
    reserve_used_fixed_zero_page_slots, source_zero_page_slots,
};

const POINTER_SCRATCH_LO: u8 = 0xAC;
const POINTER_SCRATCH_HI: u8 = 0xAD;
const POINTER_INDEX_SCRATCH_LO: u8 = 0xAE;
const POINTER_INDEX_SCRATCH_HI: u8 = 0xAF;
const INDIRECT_CALL_TARGET_LO: u8 = 0xE4;
const INDIRECT_CALL_TARGET_HI: u8 = 0xE5;
const DEST_POINTER_SCRATCH_LO: u8 = 0xAA;

const DEFAULT_POINTER_PAIR: MirAddressConsumer =
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(POINTER_SCRATCH_LO),
    });
const INDEX_POINTER_PAIR: MirAddressConsumer =
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(POINTER_INDEX_SCRATCH_LO),
    });
const DEST_POINTER_PAIR: MirAddressConsumer =
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(DEST_POINTER_SCRATCH_LO),
    });

pub(in crate::mir6502) fn analyzed_compare_operand_rewrite_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<CompareOperandRewriteCandidate> {
    compare_operand_rewrite_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_compare_narrowing_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<CompareNarrowingCandidate> {
    byte_bitwise_zero_compare_narrowing_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_byte_binary_compare_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<ByteBinaryCompareRewriteCandidate> {
    byte_binary_compare_rewrite_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_call_arg_producer_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<CallArgProducerRewriteCandidate> {
    call_arg_producer_rewrite_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_return_slot_call_arg_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<ReturnSlotCallArgForwardCandidate> {
    return_slot_call_arg_forward_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_call_arg_expr_candidate(
    ops: &[MirOp],
    index: usize,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
) -> Option<CallArgExprRewriteCandidate> {
    call_arg_expr_rewrite_candidate(ops, index, config, layout)
}

pub(in crate::mir6502) fn analyzed_call_result_store_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<CallResultStoreRewriteCandidate> {
    call_result_store_rewrite_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_loaded_arg_call_result_store_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<LoadedArgCallResultStoreRewriteCandidate> {
    loaded_arg_call_result_store_rewrite_candidate(ops, index)
}

#[derive(Debug, Clone)]
pub(in crate::mir6502) struct StoreConsumerRewriteCandidate {
    pub start: usize,
    pub consumed: usize,
    pub replacement: Vec<MirOp>,
    pub stat: &'static str,
    pub family_priority: u16,
}

#[derive(Debug, Clone)]
pub(in crate::mir6502) struct PointerRewriteCandidate {
    pub consumed: usize,
    pub replacement: Vec<MirOp>,
}

#[derive(Debug, Clone)]
pub(in crate::mir6502) struct IndexRewriteCandidate {
    pub start: usize,
    pub consumed: usize,
    pub replacement: Vec<MirOp>,
    pub stat: &'static str,
    pub observations: Vec<(&'static str, usize)>,
    pub family_priority: u16,
}

pub(in crate::mir6502) fn analyzed_direct_pointer_temp_rematerialization_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<PointerRewriteCandidate> {
    select_direct_pointer_temp_rematerialization(ops, index, false)
}

pub(in crate::mir6502) fn analyzed_pointer_temp_deref_candidates(
    block: &super::ir::MirBlock,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Vec<(usize, PointerRewriteCandidate)> {
    let temp_widths = collect_temp_widths(&block.ops);
    (0..block.ops.len())
        .filter_map(|index| {
            select_pointer_temp_deref(&block.ops, index, routine_id, layout, &temp_widths, false)
                .map(|candidate| (index, candidate))
        })
        .collect()
}

pub(in crate::mir6502) fn analyzed_store_consumer_candidates(
    routine_id: RoutineId,
    block: &super::ir::MirBlock,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
) -> Vec<(usize, StoreConsumerRewriteCandidate)> {
    let ops = &block.ops;
    let temp_widths = collect_temp_widths(ops);
    let delayed_byte_indexes = collect_delayed_byte_index_plan(ops);
    (0..ops.len())
        .filter_map(|index| {
            analyzed_store_consumer_candidate_at(
                routine_id,
                block.id,
                ops,
                index,
                &block.terminator,
                config,
                layout,
                &temp_widths,
                &delayed_byte_indexes,
            )
            .map(|candidate| (candidate.start, candidate))
        })
        .collect()
}

pub(in crate::mir6502) fn analyzed_index_rewrite_candidates(
    routine_id: RoutineId,
    block: &super::ir::MirBlock,
    layout: &MaterializeLayout,
) -> Vec<(usize, IndexRewriteCandidate)> {
    let ops = &block.ops;
    let delayed_byte_indexes = collect_delayed_byte_index_plan(ops);
    (0..ops.len())
        .filter_map(|index| {
            analyzed_index_rewrite_candidate_at(
                routine_id,
                ops,
                index,
                layout,
                &delayed_byte_indexes,
            )
            .map(|candidate| (candidate.start, candidate))
        })
        .collect()
}

fn analyzed_index_rewrite_candidate_at(
    routine_id: RoutineId,
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    delayed_byte_indexes: &indexes::DelayedByteIndexPlan,
) -> Option<IndexRewriteCandidate> {
    let selected =
        |consumed: usize, replacement: Vec<MirOp>, stat: &'static str, family_priority: u16| {
            (consumed > 0).then_some(IndexRewriteCandidate {
                start: index,
                consumed,
                replacement,
                stat,
                observations: Vec::new(),
                family_priority,
            })
        };

    let mut replacement = Vec::new();
    let consumed =
        try_fuse_indexed_byte_copy(ops, index, layout, delayed_byte_indexes, &mut replacement);
    if consumed > 0 {
        return Some(expand_delayed_index_rewrite_window(
            ops,
            index,
            IndexRewriteCandidate {
                start: index,
                consumed,
                replacement,
                stat: "indexed-byte-copy",
                observations: Vec::new(),
                family_priority: 100,
            },
            delayed_byte_indexes,
        ));
    }

    let mut replacement = Vec::new();
    let consumed = try_fuse_indexed_word_copy(ops, index, layout, &mut replacement);
    if let Some(candidate) = selected(consumed, replacement, "indexed-word-copy", 110) {
        return Some(expand_index_rewrite_window_with_producers(
            ops,
            index,
            candidate,
            indexed_word_copy_rematerialized_producer_ops(ops, index),
        ));
    }

    let mut replacement = Vec::new();
    let consumed = try_fuse_dynamic_inline_byte_index(ops, index, &mut replacement);
    if let Some(candidate) = selected(consumed, replacement, "dynamic-inline-byte-index", 120) {
        return Some(candidate);
    }

    let mut replacement = Vec::new();
    let consumed = try_prepare_dynamic_byte_index(ops, index, layout, &mut replacement);
    if let Some(candidate) = selected(consumed, replacement, "prepare-dynamic-byte-index", 130) {
        return Some(candidate);
    }

    let mut replacement = Vec::new();
    let consumed = try_prepare_dynamic_word_index(ops, index, routine_id, layout, &mut replacement);
    if let Some(candidate) = selected(consumed, replacement, "prepare-dynamic-word-index", 140) {
        return Some(candidate);
    }

    delayed_byte_index_rewrite_candidate_at(ops, index, layout, delayed_byte_indexes)
}

fn delayed_byte_index_rewrite_candidate_at(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    delayed_byte_indexes: &indexes::DelayedByteIndexPlan,
) -> Option<IndexRewriteCandidate> {
    let mut replacement = Vec::new();
    let used_delayed_index = match ops.get(index)? {
        MirOp::Load {
            dst,
            src: src @ (MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. }),
            width,
        } => materialize_indexed_read_to_def(
            dst.clone(),
            indexed_addr_parts(src)?,
            *width,
            layout,
            Some(delayed_byte_indexes),
            &mut replacement,
        ),
        MirOp::Store {
            dst: dst @ (MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. }),
            src,
            width,
        } => materialize_indexed_write_from_value(
            indexed_addr_parts(dst)?,
            src.clone(),
            *width,
            layout,
            Some(delayed_byte_indexes),
            &mut replacement,
        ),
        _ => false,
    };
    used_delayed_index.then(|| {
        expand_delayed_index_rewrite_window(
            ops,
            index,
            IndexRewriteCandidate {
                start: index,
                consumed: 1,
                replacement,
                stat: "delayed-byte-index-consumer",
                observations: Vec::new(),
                family_priority: 150,
            },
            delayed_byte_indexes,
        )
    })
}

fn expand_delayed_index_rewrite_window(
    ops: &[MirOp],
    index: usize,
    candidate: IndexRewriteCandidate,
    delayed_byte_indexes: &indexes::DelayedByteIndexPlan,
) -> IndexRewriteCandidate {
    let producer_ops =
        delayed_producer_ops_for_window(ops, index, candidate.consumed, delayed_byte_indexes);
    let producer_count = producer_ops.len();
    let mut candidate =
        expand_index_rewrite_window_with_producers(ops, index, candidate, producer_ops);
    if producer_count != 0 {
        candidate
            .observations
            .push(("delayed-byte-index-producer", producer_count));
    }
    candidate
}

fn expand_index_rewrite_window_with_producers(
    ops: &[MirOp],
    index: usize,
    mut candidate: IndexRewriteCandidate,
    producer_ops: BTreeSet<usize>,
) -> IndexRewriteCandidate {
    let Some(start) = producer_ops.iter().copied().min() else {
        return candidate;
    };
    if start >= index {
        return candidate;
    }
    let mut replacement = ops[start..index]
        .iter()
        .enumerate()
        .filter(|(offset, _)| !producer_ops.contains(&(start + offset)))
        .map(|(_, op)| op.clone())
        .collect::<Vec<_>>();
    replacement.extend(candidate.replacement);
    candidate.start = start;
    candidate.consumed = index + candidate.consumed - start;
    candidate.replacement = replacement;
    candidate
}

#[allow(clippy::too_many_arguments)]
fn analyzed_store_consumer_candidate_at(
    routine_id: RoutineId,
    block_id: MirBlockId,
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &indexes::DelayedByteIndexPlan,
) -> Option<StoreConsumerRewriteCandidate> {
    let mut replacement = Vec::new();
    let consumed =
        try_fuse_address_store_consumer(ops, index, routine_id, layout, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "address-store-consumer",
            family_priority: 100,
        });
    }

    let consumed = try_fuse_cast_store_consumer(ops, index, layout, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "cast-store-consumer",
            family_priority: 110,
        });
    }

    let mut selected_helpers = Vec::new();
    let consumed = select_byte_mul_add_sub_word_store_consumer(
        ops,
        index,
        config,
        layout,
        temp_widths,
        &mut selected_helpers,
        &mut replacement,
    );
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "byte-mul-add-sub-word-store-consumer",
            family_priority: 120,
        });
    }

    let consumed = try_fuse_byte_mul_word_store_consumer(
        ops,
        index,
        config,
        layout,
        temp_widths,
        &mut selected_helpers,
        &mut replacement,
    );
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "byte-mul-word-store-consumer",
            family_priority: 130,
        });
    }

    let consumed = select_word_store_consumer(ops, index, config, layout, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "word-store-consumer",
            family_priority: 140,
        });
    }

    let consumed = select_direct_copy_store_consumer(ops, index, layout, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "direct-copy-store-consumer",
            family_priority: 150,
        });
    }

    let mut selected_stats = MirPeepholeStats::default();
    let consumed = select_byte_store_consumer(
        ops,
        index,
        terminator,
        routine_id,
        block_id,
        layout,
        temp_widths,
        delayed_byte_indexes,
        &mut selected_stats,
        &mut replacement,
    );
    if consumed > 0 {
        let (start, consumed, replacement) = expand_delayed_store_consumer_window(
            ops,
            index,
            consumed,
            replacement,
            delayed_byte_indexes,
        );
        return Some(StoreConsumerRewriteCandidate {
            start,
            consumed,
            replacement,
            stat: "byte-store-consumer",
            family_priority: if start < index { 90 } else { 160 },
        });
    }

    let consumed =
        select_store_expr_producers(ops, index, terminator, config, layout, &mut replacement);
    (consumed > 0).then_some(StoreConsumerRewriteCandidate {
        start: index,
        consumed,
        replacement,
        stat: "store-expr-consumer",
        family_priority: 170,
    })
}

fn expand_delayed_store_consumer_window(
    ops: &[MirOp],
    index: usize,
    consumed: usize,
    replacement: Vec<MirOp>,
    delayed_byte_indexes: &indexes::DelayedByteIndexPlan,
) -> (usize, usize, Vec<MirOp>) {
    let producer_ops = delayed_producer_ops_for_window(ops, index, consumed, delayed_byte_indexes);
    let Some(start) = producer_ops.iter().copied().min() else {
        return (index, consumed, replacement);
    };
    if start >= index {
        return (index, consumed, replacement);
    }
    let mut expanded = ops[start..index]
        .iter()
        .enumerate()
        .filter(|(offset, _)| !producer_ops.contains(&(start + offset)))
        .map(|(_, op)| op.clone())
        .collect::<Vec<_>>();
    expanded.extend(replacement);
    (start, index + consumed - start, expanded)
}

fn delayed_producer_ops_for_window(
    ops: &[MirOp],
    index: usize,
    consumed: usize,
    delayed_byte_indexes: &indexes::DelayedByteIndexPlan,
) -> BTreeSet<usize> {
    ops[index..index + consumed]
        .iter()
        .filter_map(|op| match op {
            MirOp::Load { src, .. } | MirOp::Store { dst: src, .. } => indexed_addr_parts(src),
            _ => None,
        })
        .filter_map(|parts| delayed_byte_indexes.producer_ops_for_value(&parts.index))
        .flatten()
        .copied()
        .collect()
}

pub(super) fn materialize_program(
    mut program: MirProgram,
    config: &Mir6502Config,
    object_origin: u16,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let mut helpers = Vec::new();
    let mut peephole_stats = MirPeepholeStats::default();
    let mut home_fates = BTreeMap::<RoutineId, HomeFateTracker>::new();
    reserve_pointer_scratch_slots(&mut program);
    allocate_zero_page_slots(&mut program);
    {
        let (routines, machine_blocks) = (&mut program.routines, &mut program.machine_blocks);
        for routine in routines {
            prepend_action_abi_param_prologue(routine, machine_blocks, &mut helpers);
        }
    }
    let layout = MaterializeLayout::new(&program, object_origin);
    for routine in &mut program.routines {
        cleanup_pre_materialization_temp_artifacts(routine, &layout);
        lower_block_arguments(routine).map_err(|diagnostic| vec![diagnostic])?;
        run_analyzed_compare_producer_rewrites(routine, &mut peephole_stats)?;
        run_analyzed_compare_narrowing(routine, &mut peephole_stats)?;
        expand_compare_branch_consumers(&mut routine.blocks, &layout, config);
        verify_cfg_after_transform(routine, "compare/branch expansion")?;
        collapse_empty_jump_blocks(routine);
        verify_cfg_after_transform(routine, "empty-jump collapse")?;
        run_analyzed_byte_binary_compare_consumers(routine, &mut peephole_stats)?;
        run_analyzed_pointer_rewrites(routine, &layout, &mut peephole_stats)?;
        run_analyzed_call_arg_producers(routine, &mut peephole_stats)?;
        run_analyzed_return_slot_call_arg_forwards(routine, &mut peephole_stats)?;
        for block in &mut routine.blocks {
            block.ops = forward_param_register_homes(std::mem::take(&mut block.ops));
            block.ops = normalize_byte_add_sub_carry(std::mem::take(&mut block.ops));
        }
        run_analyzed_call_arg_exprs(routine, config, &layout, &mut helpers, &mut peephole_stats)?;
        run_analyzed_call_result_store_consumers(routine, &mut peephole_stats)?;
        run_analyzed_store_consumers(routine, config, &layout, &mut helpers, &mut peephole_stats)?;
        run_analyzed_unused_lea_addrs(routine, &mut peephole_stats)?;
        let word_load_address_forwards =
            forward_unique_word_load_address_consumers(routine, &layout);
        peephole_stats.record_many(
            routine.id,
            "word-load-address-consumer-forwards",
            word_load_address_forwards,
        );
        run_analyzed_index_rewrites(routine, &layout, &mut peephole_stats)?;
        for block in &mut routine.blocks {
            block.ops = materialize_ops(
                routine.id,
                block.id,
                block.ops.clone(),
                &block.terminator,
                config,
                &layout,
                &mut helpers,
                &mut peephole_stats,
            );
            block.ops = normalize_synthetic_byte_storage_high_ops(
                std::mem::take(&mut block.ops),
                routine.id,
                &layout,
            );
        }
        materialize_word_compare_temp_ops(routine, &layout);
        run_pre_home_cleanup_fixed_point(routine, &layout, &mut peephole_stats);
        let home_liveness = analyze_temp_liveness(routine);
        let home_plan = record_home_demand_census(routine, &home_liveness, &mut peephole_stats);
        home_fates.insert(routine.id, HomeFateTracker::from_plan(&home_plan));
        apply_register_home_plan(routine, &home_plan, &mut peephole_stats);
        for (block_index, block) in routine.blocks.iter_mut().enumerate() {
            let live_out = home_liveness
                .live_out(block_index)
                .expect("block liveness exists");
            block.ops =
                materialize_temp_ops(std::mem::take(&mut block.ops), &mut routine.frame.spills);
            block.ops = normalize_synthetic_byte_storage_high_ops(
                std::mem::take(&mut block.ops),
                routine.id,
                &layout,
            );
            let ops = std::mem::take(&mut block.ops);
            block.ops = fold_indirect_load_spill_consumers(ops, live_out);
            let ops = std::mem::take(&mut block.ops);
            block.ops = fold_structural_prefix(ops, &block.terminator);
        }
        run_analyzed_staged_word_forwards(
            routine,
            &layout,
            config.enable_direct_byte_word_update,
            &mut peephole_stats,
        )?;
        run_analyzed_indirect_constant_stores(routine, &layout, &mut peephole_stats)?;
        run_analyzed_word_array_value_staging(routine, &layout, &mut peephole_stats)?;
        run_analyzed_indirect_stores_and_compounds(routine, &layout, &mut peephole_stats)?;
        for block in &mut routine.blocks {
            let ops = std::mem::take(&mut block.ops);
            block.ops = fold_structural_cleanup_tail(
                ops,
                routine.id,
                &layout,
                &block.terminator,
                &mut peephole_stats,
            );
        }
        run_analyzed_indexed_base_pointer_staging(routine, &mut peephole_stats)?;
        for block in &mut routine.blocks {
            block.terminator =
                materialize_terminator(block.id, &block.terminator, &block.ops, config);
            materialize_fused_compare_dest(block.id, &block.terminator, &mut block.ops);
        }
        fold_ssa_lite_single_predecessor_loads(routine, &layout, &mut peephole_stats);
        remove_dead_spill_stores(routine);
        let remap = color_basic_block_spills(routine);
        if let Some(tracker) = home_fates.get_mut(&routine.id) {
            tracker.apply_spill_remap(&remap);
        }
        let routine_remap = color_routine_spills(routine);
        peephole_stats.record_many(
            routine.id,
            "routine-spill-color-remaps",
            routine_remap.len(),
        );
        if let Some(tracker) = home_fates.get_mut(&routine.id) {
            tracker.apply_spill_remap(&routine_remap);
        }
        prune_unused_spills(routine);
        reserve_used_fixed_zero_page_slots(routine);
    }
    for helper in helpers {
        ensure_helper_decl(&mut program, helper);
    }
    let layout = MaterializeLayout::new(&program, object_origin);
    for routine in &mut program.routines {
        for block in &mut routine.blocks {
            block.ops = lower_lea_addrs_with_final_layout(routine.id, block.ops.clone(), &layout);
            block.ops = normalize_synthetic_byte_storage_high_ops(
                std::mem::take(&mut block.ops),
                routine.id,
                &layout,
            );
            let ops = std::mem::take(&mut block.ops);
            block.ops = fold_structural_prefix(ops, &block.terminator);
        }
        run_analyzed_staged_word_forwards(
            routine,
            &layout,
            config.enable_direct_byte_word_update,
            &mut peephole_stats,
        )?;
        run_analyzed_indirect_constant_stores(routine, &layout, &mut peephole_stats)?;
        run_analyzed_word_array_value_staging(routine, &layout, &mut peephole_stats)?;
        run_analyzed_indirect_stores_and_compounds(routine, &layout, &mut peephole_stats)?;
        for block in &mut routine.blocks {
            let ops = std::mem::take(&mut block.ops);
            block.ops = fold_structural_cleanup_tail(
                ops,
                routine.id,
                &layout,
                &block.terminator,
                &mut peephole_stats,
            );
        }
        run_analyzed_indexed_base_pointer_staging(routine, &mut peephole_stats)?;
        fold_ssa_lite_single_predecessor_loads(routine, &layout, &mut peephole_stats);
        remove_dead_spill_stores(routine);
        let remap = color_basic_block_spills(routine);
        if let Some(tracker) = home_fates.get_mut(&routine.id) {
            tracker.apply_spill_remap(&remap);
        }
        let routine_remap = color_routine_spills(routine);
        peephole_stats.record_many(
            routine.id,
            "routine-spill-color-remaps",
            routine_remap.len(),
        );
        if let Some(tracker) = home_fates.get_mut(&routine.id) {
            tracker.apply_spill_remap(&routine_remap);
        }
        prune_unused_spills(routine);
        reserve_used_fixed_zero_page_slots(routine);
    }
    let zero_page_remaps = lower_block_local_byte_spills_to_zero_page(&mut program);
    for (routine, remap) in zero_page_remaps {
        if let Some(tracker) = home_fates.get_mut(&routine) {
            tracker.apply_zero_page_remap(&remap);
        }
    }
    allocate_zero_page_slots(&mut program);
    materialize_remaining_pointer_cell_values(&mut program);
    record_final_home_allocations(&program, &mut peephole_stats);
    for routine in &program.routines {
        if let Some(tracker) = home_fates.get(&routine.id) {
            tracker.record_final_fates(routine, &mut peephole_stats);
        }
    }
    record_unspecified_add_sub_carry_observability(&program, &mut peephole_stats);
    maybe_report_peepholes(&program, &peephole_stats, config);
    Ok(program)
}

fn run_analyzed_staged_word_forwards(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    enable_direct_byte_word_update: bool,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            peepholes::discover_staged_word_forwards(
                routine,
                context,
                layout,
                enable_direct_byte_word_update,
            )
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home staged word forwarding failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_word_array_value_staging(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            peepholes::discover_word_array_value_staging(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home word-array staging failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_indirect_constant_stores(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            peepholes::discover_indirect_constant_stores(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home indirect constant-store rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_indirect_stores_and_compounds(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            peepholes::discover_indirect_stores_and_compounds(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home indirect structural rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_indexed_base_pointer_staging(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, indexes::discover_indexed_base_pointer_staging)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home indexed base-pointer staging failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_compare_producer_rewrites(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, discover_compare_producers)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("pre-branch compare rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_compare_narrowing(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(routine, discover_compare_narrowing, compare_narrowing_rank)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("pre-branch compare narrowing failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_byte_binary_compare_consumers(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            discover_byte_binary_compare_consumers,
            byte_binary_compare_consumer_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("byte binary compare selection failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_pointer_rewrites(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| discover_pointer_rewrites(routine, context, layout),
            super::rewrite::pilots::pointer_rewrite_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("pointer rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_index_rewrites(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| discover_index_rewrites(routine, context, layout),
            super::rewrite::pilots::index_rewrite_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("index selection failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_call_arg_producers(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, super::rewrite::pilots::discover_call_arg_producers)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("call argument producer rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_return_slot_call_arg_forwards(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            super::rewrite::pilots::discover_return_slot_call_arg_forwards,
            super::rewrite::pilots::return_slot_call_arg_forward_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("return-slot call argument forwarding failed: {error:?}"),
            )]
        })?;
    let candidates = result.candidates;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    peephole_stats.record_many(
        routine.id,
        "return-slot-call-arg-forward-candidates",
        candidates,
    );
    Ok(())
}

fn run_analyzed_call_arg_exprs(
    routine: &mut super::ir::MirRoutine,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| {
                super::rewrite::pilots::discover_call_arg_exprs(routine, context, config, layout)
            },
            |routine| super::rewrite::pilots::call_arg_expr_rank(routine, config, layout),
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("call argument expression selection failed: {error:?}"),
            )]
        })?;
    for op in routine.blocks.iter().flat_map(|block| &block.ops) {
        if let MirOp::RuntimeHelper { helper, .. } = op {
            helpers.push(helper.clone());
        }
    }
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_unused_lea_addrs(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, discover_unused_lea_addrs)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("unused address rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_call_result_store_consumers(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            super::rewrite::pilots::discover_call_result_store_consumers,
            super::rewrite::pilots::call_result_store_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("call-result store rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_store_consumers(
    routine: &mut super::ir::MirRoutine,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| {
                super::rewrite::pilots::discover_store_consumers(routine, context, config, layout)
            },
            super::rewrite::pilots::store_consumer_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("store-consumer selection failed: {error:?}"),
            )]
        })?;
    for op in routine.blocks.iter().flat_map(|block| &block.ops) {
        if let MirOp::RuntimeHelper { helper, .. } = op {
            helpers.push(helper.clone());
        }
    }
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn record_prehome_rewrite_result(
    routine_id: RoutineId,
    result: MirRewriteRunResult,
    peephole_stats: &mut MirPeepholeStats,
) {
    for (stat, count) in result.applied_by_stat {
        peephole_stats.record_many(routine_id, stat, count);
    }
    peephole_stats.record_many(
        routine_id,
        "prehome-rewrite-analysis-builds",
        result.analysis_builds,
    );
    peephole_stats.record_many(routine_id, "prehome-rewrite-rounds", result.rounds);
    peephole_stats.record_many(routine_id, "prehome-rewrite-candidates", result.candidates);
    peephole_stats.record_many(routine_id, "prehome-rewrite-applied", result.applied);
    peephole_stats.record_many(
        routine_id,
        "prehome-rewrite-overlap-rejections",
        result.overlap_rejections,
    );
}

fn verify_cfg_after_transform(
    routine: &super::ir::MirRoutine,
    transform: &str,
) -> Result<(), Vec<MirDiagnostic>> {
    MirCfg::from_routine(routine).map(|_| ()).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| cfg_diagnostic(routine, transform, error))
            .collect()
    })
}

fn cfg_diagnostic(
    routine: &super::ir::MirRoutine,
    transform: &str,
    error: MirCfgError,
) -> MirDiagnostic {
    let message = format!("{transform} produced invalid CFG: {}", error.message);
    if let Some(block) = routine.blocks.iter().find(|block| block.id == error.block) {
        MirDiagnostic::block(&routine.name, &block.label, message)
    } else {
        MirDiagnostic::routine(&routine.name, message)
    }
}

const PRE_HOME_CLEANUP_MAX_ROUNDS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreHomeCleanupResult {
    rounds: usize,
    change_rounds: usize,
    changed_blocks: usize,
    removed_ops: usize,
    converged: bool,
}

fn run_pre_home_cleanup_fixed_point(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> PreHomeCleanupResult {
    let original_temps = routine.temps.clone();
    let initial_liveness = analyze_temp_liveness(routine);
    record_temp_liveness_observability(routine.id, &initial_liveness, peephole_stats);
    for block in &routine.blocks {
        record_ssa_lite_v2_observability(&block.ops, routine.id, layout, peephole_stats);
    }

    let mut result = PreHomeCleanupResult {
        rounds: 0,
        change_rounds: 0,
        changed_blocks: 0,
        removed_ops: 0,
        converged: false,
    };

    for round in 0..PRE_HOME_CLEANUP_MAX_ROUNDS {
        result.rounds += 1;
        let before = routine
            .blocks
            .iter()
            .map(|block| block.ops.clone())
            .collect::<Vec<_>>();
        let before_op_count = before.iter().map(Vec::len).sum::<usize>();
        let liveness = analyze_temp_liveness(routine);

        for (block_index, block) in routine.blocks.iter_mut().enumerate() {
            let ops = std::mem::take(&mut block.ops);
            let live_out = liveness
                .live_out(block_index)
                .expect("block liveness exists");
            block.ops = if round == 0 {
                fold_mir_copy_prop_const_uses_with_terminator_and_live_out(
                    ops,
                    &block.terminator,
                    live_out,
                    block.id,
                    routine.id,
                    layout,
                    peephole_stats,
                )
            } else {
                // Candidate/blocker observability is recorded from the first round.
                // Later rounds contribute only structural fixed-point counters.
                let mut scratch_stats = MirPeepholeStats::default();
                fold_mir_copy_prop_const_uses_with_terminator_and_live_out(
                    ops,
                    &block.terminator,
                    live_out,
                    block.id,
                    routine.id,
                    layout,
                    &mut scratch_stats,
                )
            };
        }

        let cleanup_liveness = analyze_temp_liveness(routine);
        cleanup_pre_materialization_temp_artifacts_with_liveness(
            routine,
            layout,
            &cleanup_liveness,
        );

        assert_eq!(
            routine.temps, original_temps,
            "pre-home cleanup must not create or remove temp IDs"
        );
        let after_op_count = routine
            .blocks
            .iter()
            .map(|block| block.ops.len())
            .sum::<usize>();
        assert!(
            after_op_count <= before_op_count,
            "pre-home cleanup must not add operations"
        );
        let changed_blocks = routine
            .blocks
            .iter()
            .zip(&before)
            .filter(|(block, before_ops)| block.ops != **before_ops)
            .count();
        if changed_blocks == 0 {
            result.converged = true;
            break;
        }

        let removed_ops = before_op_count - after_op_count;
        result.change_rounds += 1;
        result.changed_blocks += changed_blocks;
        result.removed_ops += removed_ops;
        peephole_stats.record_many_dynamic(
            routine.id,
            format!("pre-home-fixed-point-round-{}-changed-blocks", round + 1),
            changed_blocks,
        );
        peephole_stats.record_many_dynamic(
            routine.id,
            format!("pre-home-fixed-point-round-{}-removed-ops", round + 1),
            removed_ops,
        );
    }

    peephole_stats.record_many(routine.id, "pre-home-fixed-point-rounds", result.rounds);
    peephole_stats.record_many(
        routine.id,
        "pre-home-fixed-point-change-rounds",
        result.change_rounds,
    );
    peephole_stats.record_many(
        routine.id,
        "pre-home-fixed-point-changed-blocks",
        result.changed_blocks,
    );
    peephole_stats.record_many(
        routine.id,
        "pre-home-fixed-point-removed-ops",
        result.removed_ops,
    );
    if result.converged {
        peephole_stats.record(routine.id, "pre-home-fixed-point-converged");
    } else {
        peephole_stats.record(routine.id, "pre-home-fixed-point-limit-hit");
    }
    result
}

fn materialize_remaining_pointer_cell_values(program: &mut MirProgram) {
    for routine in &mut program.routines {
        for block in &mut routine.blocks {
            block.ops = materialize_remaining_pointer_cell_ops(std::mem::take(&mut block.ops));
        }
    }
}

fn materialize_word_compare_temp_ops(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
) {
    let mut temps = FreshTemps::new(&routine.temps);
    for block in &mut routine.blocks {
        block.ops = materialize_word_compare_temp_block(
            std::mem::take(&mut block.ops),
            layout,
            &mut temps,
            &mut routine.temps,
        );
    }
}

struct FreshTemps {
    next: u32,
}

impl FreshTemps {
    fn new(temps: &[MirTemp]) -> Self {
        Self {
            next: temps
                .iter()
                .map(|temp| temp.id.0)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        }
    }

    fn fresh(&mut self, temps: &mut Vec<MirTemp>) -> MirTempId {
        let id = MirTempId(self.next);
        self.next = self.next.saturating_add(1);
        temps.push(MirTemp { id });
        id
    }
}

fn materialize_word_compare_temp_block(
    ops: Vec<MirOp>,
    layout: &MaterializeLayout,
    fresh: &mut FreshTemps,
    temps: &mut Vec<MirTemp>,
) -> Vec<MirOp> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            MirOp::Compare {
                dst: MirCondDest::Temp(dst),
                op,
                left,
                right,
                width: MirWidth::Word,
                signed,
            } if !signed || matches!(op, MirCompareOp::Eq | MirCompareOp::Ne) => {
                let (left_lo, left_hi) = split_value_as_word(left, layout);
                let (right_lo, right_hi) = split_value_as_word(right, layout);
                materialize_word_compare_temp(
                    &mut out, fresh, temps, dst, op, left_lo, left_hi, right_lo, right_hi,
                );
            }
            other => out.push(other),
        }
    }
    out
}

fn materialize_word_compare_temp(
    out: &mut Vec<MirOp>,
    fresh: &mut FreshTemps,
    temps: &mut Vec<MirTemp>,
    dst: MirTempId,
    op: MirCompareOp,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
) {
    match op {
        MirCompareOp::Eq | MirCompareOp::Ne => {
            let lo = fresh.fresh(temps);
            push_byte_compare_temp(out, lo, op, left_lo, right_lo);
            push_byte_compare_temp(out, dst, op, left_hi, right_hi);
            push_bool_binary(
                out,
                match op {
                    MirCompareOp::Eq => MirBinaryOp::And,
                    MirCompareOp::Ne => MirBinaryOp::Or,
                    _ => unreachable!(),
                },
                dst,
                dst,
                lo,
            );
        }
        MirCompareOp::Lt | MirCompareOp::Le => {
            materialize_word_rel_compare_temp(
                out, fresh, temps, dst, op, left_lo, left_hi, right_lo, right_hi,
            );
        }
        MirCompareOp::Gt => {
            materialize_word_rel_compare_temp(
                out,
                fresh,
                temps,
                dst,
                MirCompareOp::Lt,
                right_lo,
                right_hi,
                left_lo,
                left_hi,
            );
        }
        MirCompareOp::Ge => {
            materialize_word_rel_compare_temp(
                out,
                fresh,
                temps,
                dst,
                MirCompareOp::Le,
                right_lo,
                right_hi,
                left_lo,
                left_hi,
            );
        }
    }
}

fn materialize_word_rel_compare_temp(
    out: &mut Vec<MirOp>,
    fresh: &mut FreshTemps,
    temps: &mut Vec<MirTemp>,
    dst: MirTempId,
    op: MirCompareOp,
    left_lo: MirValue,
    left_hi: MirValue,
    right_lo: MirValue,
    right_hi: MirValue,
) {
    debug_assert!(matches!(op, MirCompareOp::Lt | MirCompareOp::Le));
    let hi_eq = fresh.fresh(temps);
    let lo_rel = fresh.fresh(temps);
    push_byte_compare_temp(
        out,
        dst,
        MirCompareOp::Lt,
        left_hi.clone(),
        right_hi.clone(),
    );
    push_byte_compare_temp(out, hi_eq, MirCompareOp::Eq, left_hi, right_hi);
    push_byte_compare_temp(out, lo_rel, op, left_lo, right_lo);
    push_bool_binary(out, MirBinaryOp::And, hi_eq, hi_eq, lo_rel);
    push_bool_binary(out, MirBinaryOp::Or, dst, dst, hi_eq);
}

fn push_byte_compare_temp(
    out: &mut Vec<MirOp>,
    dst: MirTempId,
    op: MirCompareOp,
    left: MirValue,
    right: MirValue,
) {
    out.push(MirOp::Compare {
        dst: MirCondDest::Temp(dst),
        op,
        left,
        right,
        width: MirWidth::Byte,
        signed: false,
    });
}

fn push_bool_binary(
    out: &mut Vec<MirOp>,
    op: MirBinaryOp,
    dst: MirTempId,
    left: MirTempId,
    right: MirTempId,
) {
    out.push(MirOp::Binary {
        op,
        dst: MirDef::VTemp(dst),
        left: MirValue::Def(MirDef::VTemp(left)),
        right: MirValue::Def(MirDef::VTemp(right)),
        width: MirWidth::Byte,
        carry_in: None,
        carry_out: MirCarryOut::Ignore,
    });
}

fn materialize_remaining_pointer_cell_ops(ops: Vec<MirOp>) -> Vec<MirOp> {
    let mut out = Vec::new();
    for op in ops {
        match op {
            MirOp::Move {
                dst,
                src: MirValue::PointerCell(mem),
                width: MirWidth::Byte,
            } => materialize_remaining_pointer_cell_byte_to_def(
                MirValue::PointerCell(mem),
                dst,
                &mut out,
            ),
            MirOp::Move {
                dst,
                src,
                width: MirWidth::Byte,
            } if value_contains_pointer_cell(&src) => {
                let (lo, _) = split_remaining_word_value(src);
                materialize_remaining_pointer_cell_byte_to_def(lo, dst, &mut out);
            }
            MirOp::Move {
                dst,
                src,
                width: MirWidth::Word,
            } if value_contains_pointer_cell(&src) => {
                if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                    let (lo, hi) = split_remaining_word_value(src);
                    materialize_remaining_pointer_cell_byte_to_def(lo, lo_dst, &mut out);
                    materialize_remaining_pointer_cell_byte_to_def(hi, hi_dst, &mut out);
                } else {
                    out.push(MirOp::Move {
                        dst,
                        src,
                        width: MirWidth::Word,
                    });
                }
            }
            MirOp::Store {
                dst,
                src: MirValue::PointerCell(mem),
                width: MirWidth::Byte,
            } => materialize_remaining_pointer_cell_byte_to_addr(
                MirValue::PointerCell(mem),
                dst,
                &mut out,
            ),
            MirOp::Store {
                dst,
                src,
                width: MirWidth::Byte,
            } if value_contains_pointer_cell(&src) => {
                let (lo, _) = split_remaining_word_value(src);
                materialize_remaining_pointer_cell_byte_to_addr(lo, dst, &mut out);
            }
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                src,
                width: MirWidth::Word,
            } if value_contains_pointer_cell(&src) => {
                let (lo, hi) = split_remaining_word_value(src);
                materialize_remaining_pointer_cell_byte_to_addr(
                    lo,
                    MirAddr::Direct(dst.clone()),
                    &mut out,
                );
                materialize_remaining_pointer_cell_byte_to_addr(
                    hi,
                    MirAddr::Direct(offset_mem(&dst, 1)),
                    &mut out,
                );
            }
            MirOp::Call {
                target,
                abi,
                args,
                result,
                effects,
            } => {
                let mut materialized_args = Vec::new();
                for mut arg in args {
                    if let (MirValue::PointerCell(mem), MirArgHome::Reg(reg), MirWidth::Byte) =
                        (&arg.value, &arg.home, arg.width)
                    {
                        out.push(MirOp::Load {
                            dst: MirDef::Reg(*reg),
                            src: MirAddr::Direct(mem.clone()),
                            width: MirWidth::Byte,
                        });
                        arg.value = MirValue::Def(MirDef::Reg(*reg));
                    }
                    materialized_args.push(arg);
                }
                out.push(MirOp::Call {
                    target,
                    abi,
                    args: materialized_args,
                    result,
                    effects,
                });
            }
            other => out.push(other),
        }
    }
    out
}

fn materialize_remaining_pointer_cell_byte_to_def(
    value: MirValue,
    dst: MirDef,
    out: &mut Vec<MirOp>,
) {
    match value {
        MirValue::PointerCell(mem) => out.push(MirOp::Load {
            dst,
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }),
        value => out.push(MirOp::Move {
            dst,
            src: value,
            width: MirWidth::Byte,
        }),
    }
}

fn materialize_remaining_pointer_cell_byte_to_addr(
    value: MirValue,
    dst: MirAddr,
    out: &mut Vec<MirOp>,
) {
    let src = match value {
        MirValue::PointerCell(mem) => {
            out.push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            });
            MirValue::Def(MirDef::Reg(MirReg::A))
        }
        value => value,
    };
    out.push(MirOp::Store {
        dst,
        src,
        width: MirWidth::Byte,
    });
}

fn split_remaining_word_value(value: MirValue) -> (MirValue, MirValue) {
    match value {
        MirValue::Word { lo, hi } => (*lo, *hi),
        MirValue::PointerCell(mem) => {
            let hi = offset_mem(&mem, 1);
            (MirValue::PointerCell(mem), MirValue::PointerCell(hi))
        }
        MirValue::ConstU16(value) => (
            MirValue::ConstU8((value & 0x00FF) as u8),
            MirValue::ConstU8((value >> 8) as u8),
        ),
        value => (value, MirValue::ConstU8(0)),
    }
}

fn value_contains_pointer_cell(value: &MirValue) -> bool {
    match value {
        MirValue::PointerCell(_) => true,
        MirValue::Word { lo, hi } => {
            value_contains_pointer_cell(lo) || value_contains_pointer_cell(hi)
        }
        _ => false,
    }
}

fn record_unspecified_add_sub_carry_observability(
    program: &MirProgram,
    peephole_stats: &mut MirPeepholeStats,
) {
    for routine in &program.routines {
        for block in &routine.blocks {
            for op in &block.ops {
                let MirOp::Binary {
                    op,
                    width,
                    carry_in: None,
                    ..
                } = op
                else {
                    continue;
                };
                if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) {
                    continue;
                }
                peephole_stats.record(routine.id, "mir6502-carry-none-addsub");
                match width {
                    MirWidth::Byte => {
                        peephole_stats.record(routine.id, "mir6502-carry-none-addsub-byte");
                    }
                    MirWidth::Word => {
                        peephole_stats.record(routine.id, "mir6502-carry-none-addsub-word");
                    }
                }
                match op {
                    MirBinaryOp::Add => {
                        peephole_stats.record(routine.id, "mir6502-carry-none-add");
                    }
                    MirBinaryOp::Sub => {
                        peephole_stats.record(routine.id, "mir6502-carry-none-sub");
                    }
                    _ => {}
                }
            }
        }
    }
}

fn materialize_ops(
    routine_id: RoutineId,
    _block_id: MirBlockId,
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    #[cfg(test)]
    let ops = rematerialize_direct_pointer_temp_derefs(ops);
    #[cfg(test)]
    let ops = fold_call_arg_producers(ops);
    #[cfg(test)]
    let (ops, call_result_forwards) = forward_return_slot_call_result_args(ops, terminator);
    #[cfg(test)]
    peephole_stats.record_many(
        routine_id,
        "return-slot-call-arg-forward-candidates",
        call_result_forwards.candidates,
    );
    #[cfg(test)]
    peephole_stats.record_many(
        routine_id,
        "return-slot-call-arg-forwards",
        call_result_forwards.forwarded,
    );
    #[cfg(test)]
    peephole_stats.record_many(
        routine_id,
        "return-slot-call-arg-forward-blocked-home-overlap",
        call_result_forwards.blocked_home_overlap,
    );
    #[cfg(test)]
    let ops = forward_param_register_homes(ops);
    #[cfg(test)]
    let ops = normalize_byte_add_sub_carry(ops);
    let mut out = Vec::new();
    let mut temp_widths = collect_temp_widths(&ops);
    refine_temp_widths_from_storage_loads(&ops, routine_id, layout, &mut temp_widths);
    #[cfg(test)]
    let delayed_byte_indexes = collect_delayed_byte_index_plan(&ops);
    #[cfg(not(test))]
    let delayed_byte_indexes = indexes::DelayedByteIndexPlan::empty();
    let mut index = 0;
    while index < ops.len() {
        #[cfg(test)]
        if delayed_byte_indexes.producer_ops().contains(&index) {
            peephole_stats.record(routine_id, "delayed-byte-index-producer");
            index += 1;
            continue;
        }

        #[cfg(test)]
        {
            let call_arg_expr = try_materialize_call_arg_expr_producers(
                &ops, index, config, layout, helpers, &mut out,
            );
            if call_arg_expr.consumed > 0 {
                peephole_stats.record(routine_id, "call-arg-expr-consumer");
                peephole_stats.record_many(
                    routine_id,
                    "indexed-word-load-ax-call-arg",
                    call_arg_expr.indexed_word_loads,
                );
                peephole_stats.record_many(
                    routine_id,
                    "indexed-word-arithmetic-ax-call-arg",
                    call_arg_expr.indexed_word_arithmetic,
                );
                index += call_arg_expr.consumed;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_fuse_cast_store_consumer(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "cast-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused =
                try_fuse_address_store_consumer(&ops, index, routine_id, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "address-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused =
                try_fuse_indexed_byte_copy(&ops, index, layout, &delayed_byte_indexes, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "indexed-byte-copy");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_fuse_indexed_word_copy(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "indexed-word-copy");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_fuse_dynamic_inline_byte_index(&ops, index, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "dynamic-inline-byte-index");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_prepare_dynamic_byte_index(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "prepare-dynamic-byte-index");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused =
                try_prepare_dynamic_word_index(&ops, index, routine_id, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "prepare-dynamic-word-index");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if let Some(stat) = byte_binary_compare_consumer_observation(&ops, index, terminator) {
            peephole_stats.record(routine_id, "byte-binary-compare-candidates");
            peephole_stats.record(routine_id, stat);
        }
        record_binary_temp_consumer_observation(
            &ops,
            index,
            terminator,
            routine_id,
            peephole_stats,
        );

        #[cfg(test)]
        {
            let maybe_fused =
                try_fuse_byte_binary_compare_consumer(&ops, index, terminator, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "byte-binary-compare-consumer");
                index += maybe_fused;
                continue;
            }

            let maybe_fused = try_fuse_compare_operand_producers(&ops, index, terminator, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "compare-operand-consumer");
                index += maybe_fused;
                continue;
            }

            let maybe_fused = try_fuse_byte_compare_consumer(&ops, index, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "byte-compare-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_fuse_byte_mul_add_sub_word_store_consumer(
                &ops,
                index,
                config,
                layout,
                &temp_widths,
                helpers,
                &mut out,
            );
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "byte-mul-add-sub-word-store-consumer");
                index += maybe_fused;
                continue;
            }

            let maybe_fused = try_fuse_byte_mul_word_store_consumer(
                &ops,
                index,
                config,
                layout,
                &temp_widths,
                helpers,
                &mut out,
            );
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "byte-mul-word-store-consumer");
                index += maybe_fused;
                continue;
            }

            let maybe_fused = try_fuse_word_store_consumer(&ops, index, config, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "word-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_fuse_loaded_arg_call_result_store_consumer(
                &ops,
                index,
                routine_id,
                layout,
                &temp_widths,
                &delayed_byte_indexes,
                peephole_stats,
                &mut out,
            );
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "call-result-loaded-arg-store-consumer");
                index += maybe_fused;
                continue;
            }

            let maybe_fused = try_fuse_call_result_store_consumer(
                &ops,
                index,
                routine_id,
                layout,
                &temp_widths,
                &delayed_byte_indexes,
                peephole_stats,
                &mut out,
            );
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "call-result-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        let materialized = try_materialize_loaded_arg_forwarded_call_result_store(
            &ops,
            index,
            routine_id,
            layout,
            &temp_widths,
            &delayed_byte_indexes,
            peephole_stats,
            &mut out,
        );
        if materialized > 0 {
            peephole_stats.record(routine_id, "call-result-loaded-arg-store-consumer");
            index += materialized;
            continue;
        }

        let materialized = try_materialize_forwarded_call_result_store(
            &ops,
            index,
            routine_id,
            layout,
            &temp_widths,
            &delayed_byte_indexes,
            peephole_stats,
            &mut out,
        );
        if materialized > 0 {
            peephole_stats.record(routine_id, "call-result-store-consumer");
            index += materialized;
            continue;
        }

        #[cfg(test)]
        {
            let maybe_fused = try_fuse_direct_copy_store_consumer(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "direct-copy-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_fuse_byte_store_consumer(
                &ops,
                index,
                terminator,
                routine_id,
                _block_id,
                layout,
                &temp_widths,
                &delayed_byte_indexes,
                peephole_stats,
                &mut out,
            );
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "byte-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_materialize_store_expr_producers(
                &ops, index, terminator, config, layout, &mut out,
            );
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "store-expr-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        {
            let maybe_fused = try_fuse_pointer_temp_deref(
                &ops,
                index,
                routine_id,
                layout,
                &temp_widths,
                &mut out,
            );
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "pointer-temp-deref");
                index += maybe_fused;
                continue;
            }
        }

        match ops[index].clone() {
            MirOp::Load {
                dst,
                src: MirAddr::Direct(src),
                width: MirWidth::Word,
            } => {
                if let Some((lo, hi)) = split_def(dst.clone()) {
                    out.push(MirOp::Load {
                        dst: lo,
                        src: MirAddr::Direct(src.clone()),
                        width: MirWidth::Byte,
                    });
                    materialize_byte_load_or_zero(
                        hi,
                        offset_mem(&src, 1),
                        routine_id,
                        layout,
                        &mut out,
                    );
                } else {
                    out.push(MirOp::Load {
                        dst,
                        src: MirAddr::Direct(src),
                        width: MirWidth::Word,
                    });
                }
            }
            MirOp::Load {
                dst,
                src: MirAddr::Deref { ptr, offset },
                width: MirWidth::Word,
            } => {
                if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                    materialize_pointer_deref_read(
                        lo_dst,
                        hi_dst,
                        ptr,
                        offset,
                        routine_id,
                        layout,
                        &temp_widths,
                        &mut out,
                    );
                } else {
                    out.push(MirOp::Load {
                        dst,
                        src: MirAddr::Deref { ptr, offset },
                        width: MirWidth::Word,
                    });
                }
            }
            MirOp::Load {
                dst,
                src: MirAddr::PointerCell { ptr, offset },
                width: MirWidth::Word,
            } => {
                if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                    materialize_pointer_deref_read(
                        lo_dst,
                        hi_dst,
                        pointer_value_from_mem(&ptr),
                        offset,
                        routine_id,
                        layout,
                        &temp_widths,
                        &mut out,
                    );
                } else {
                    out.push(MirOp::Load {
                        dst,
                        src: MirAddr::PointerCell { ptr, offset },
                        width: MirWidth::Word,
                    });
                }
            }
            MirOp::Load {
                dst,
                src: MirAddr::Deref { ptr, offset },
                width: MirWidth::Byte,
            } => materialize_pointer_deref_read_byte(
                dst,
                ptr,
                offset,
                routine_id,
                layout,
                &temp_widths,
                &mut out,
            ),
            MirOp::Load {
                dst,
                src: MirAddr::PointerCell { ptr, offset },
                width: MirWidth::Byte,
            } => materialize_pointer_deref_read_byte(
                dst,
                pointer_value_from_mem(&ptr),
                offset,
                routine_id,
                layout,
                &temp_widths,
                &mut out,
            ),
            MirOp::Load {
                dst,
                src: src @ (MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. }),
                width,
            } => {
                let parts = indexed_addr_parts(&src).expect("indexed load matched above");
                if materialize_indexed_read_to_def(
                    dst,
                    parts,
                    width,
                    layout,
                    Some(&delayed_byte_indexes),
                    &mut out,
                ) {
                    peephole_stats.record(routine_id, "delayed-byte-index-consumer");
                }
            }
            MirOp::Load {
                dst,
                src: MirAddr::Direct(src),
                width: MirWidth::Byte,
            } if layout.is_synthetic_byte_storage_high(routine_id, &src) => {
                materialize_zero_to_def(dst, &mut out);
            }
            MirOp::Store {
                dst: MirAddr::Deref { ptr, offset },
                src,
                width: MirWidth::Word,
            } => materialize_pointer_deref_write(
                ptr,
                offset,
                src,
                routine_id,
                layout,
                &temp_widths,
                &mut out,
            ),
            MirOp::Store {
                dst: MirAddr::PointerCell { ptr, offset },
                src,
                width: MirWidth::Word,
            } => materialize_pointer_deref_write(
                pointer_value_from_mem(&ptr),
                offset,
                src,
                routine_id,
                layout,
                &temp_widths,
                &mut out,
            ),
            MirOp::Store {
                dst: MirAddr::Deref { ptr, offset },
                src,
                width: MirWidth::Byte,
            } => materialize_pointer_deref_write_byte(
                src,
                ptr,
                offset,
                routine_id,
                layout,
                &temp_widths,
                &mut out,
            ),
            MirOp::Store {
                dst: MirAddr::PointerCell { ptr, offset },
                src,
                width: MirWidth::Byte,
            } => materialize_pointer_deref_write_byte(
                src,
                pointer_value_from_mem(&ptr),
                offset,
                routine_id,
                layout,
                &temp_widths,
                &mut out,
            ),
            MirOp::Store {
                dst: dst @ (MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. }),
                src,
                width,
            } => {
                let parts = indexed_addr_parts(&dst).expect("indexed store matched above");
                if materialize_indexed_write_from_value(
                    parts,
                    src,
                    width,
                    layout,
                    Some(&delayed_byte_indexes),
                    &mut out,
                ) {
                    peephole_stats.record(routine_id, "delayed-byte-index-consumer");
                }
            }
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                src,
                width: MirWidth::Word,
            } => {
                let (lo, hi) =
                    split_value_with_storage_widths(src, routine_id, layout, &temp_widths);
                out.push(MirOp::Store {
                    dst: MirAddr::Direct(dst.clone()),
                    src: lo,
                    width: MirWidth::Byte,
                });
                if !layout.is_byte_scalar_storage(routine_id, &dst) {
                    out.push(MirOp::Store {
                        dst: MirAddr::Direct(offset_mem(&dst, 1)),
                        src: hi,
                        width: MirWidth::Byte,
                    });
                }
                if next_op_is_machine_block(&ops, index + 1) {
                    reload_low_byte_for_machine_block(dst, &mut out);
                }
            }
            MirOp::Move {
                dst,
                src,
                width: MirWidth::Word,
            } => {
                if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                    let (lo_src, hi_src) =
                        split_value_with_storage_widths(src, routine_id, layout, &temp_widths);
                    out.push(MirOp::Move {
                        dst: lo_dst,
                        src: lo_src,
                        width: MirWidth::Byte,
                    });
                    out.push(MirOp::Move {
                        dst: hi_dst,
                        src: hi_src,
                        width: MirWidth::Byte,
                    });
                } else {
                    out.push(MirOp::Move {
                        dst,
                        src,
                        width: MirWidth::Word,
                    });
                }
            }
            MirOp::Binary {
                op,
                dst,
                left,
                right,
                width: MirWidth::Word,
                ..
            } if matches!(op, MirBinaryOp::And | MirBinaryOp::Or | MirBinaryOp::Xor) => {
                if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                    let (left_lo, left_hi) =
                        split_value_with_storage_widths(left, routine_id, layout, &temp_widths);
                    let (right_lo, right_hi) =
                        split_value_with_storage_widths(right, routine_id, layout, &temp_widths);
                    out.push(MirOp::Binary {
                        op,
                        dst: lo_dst,
                        left: left_lo,
                        right: right_lo,
                        width: MirWidth::Byte,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    });
                    out.push(MirOp::Binary {
                        op,
                        dst: hi_dst,
                        left: left_hi,
                        right: right_hi,
                        width: MirWidth::Byte,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    });
                } else {
                    out.push(MirOp::Binary {
                        op,
                        dst,
                        left,
                        right,
                        width: MirWidth::Word,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    });
                }
            }
            MirOp::Binary {
                op,
                dst,
                left,
                right,
                width: MirWidth::Word,
                ..
            } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
                if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                    let (left_lo, left_hi) =
                        split_value_with_storage_widths(left, routine_id, layout, &temp_widths);
                    let (right_lo, right_hi) =
                        split_value_with_storage_widths(right, routine_id, layout, &temp_widths);
                    out.push(MirOp::Binary {
                        op,
                        dst: lo_dst,
                        left: left_lo,
                        right: right_lo,
                        width: MirWidth::Byte,
                        carry_in: Some(match op {
                            MirBinaryOp::Add => MirCarryIn::Clear,
                            MirBinaryOp::Sub => MirCarryIn::Set,
                            _ => unreachable!(),
                        }),
                        carry_out: MirCarryOut::Produce,
                    });
                    out.push(MirOp::Binary {
                        op,
                        dst: hi_dst,
                        left: left_hi,
                        right: right_hi,
                        width: MirWidth::Byte,
                        carry_in: Some(MirCarryIn::FromPrevious),
                        carry_out: MirCarryOut::Ignore,
                    });
                } else {
                    out.push(MirOp::Binary {
                        op,
                        dst,
                        left,
                        right,
                        width: MirWidth::Word,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    });
                }
            }
            MirOp::Unary {
                op: MirUnaryOp::Neg,
                dst,
                src,
                width: MirWidth::Word,
            } => {
                if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                    let (src_lo, src_hi) =
                        split_value_with_storage_widths(src, routine_id, layout, &temp_widths);
                    out.push(MirOp::Binary {
                        op: MirBinaryOp::Sub,
                        dst: lo_dst,
                        left: MirValue::ConstU8(0),
                        right: src_lo,
                        width: MirWidth::Byte,
                        carry_in: Some(MirCarryIn::Set),
                        carry_out: MirCarryOut::Produce,
                    });
                    out.push(MirOp::Binary {
                        op: MirBinaryOp::Sub,
                        dst: hi_dst,
                        left: MirValue::ConstU8(0),
                        right: src_hi,
                        width: MirWidth::Byte,
                        carry_in: Some(MirCarryIn::FromPrevious),
                        carry_out: MirCarryOut::Ignore,
                    });
                } else {
                    out.push(MirOp::Unary {
                        op: MirUnaryOp::Neg,
                        dst,
                        src,
                        width: MirWidth::Word,
                    });
                }
            }
            MirOp::Compare {
                dst,
                op,
                left,
                right,
                width: MirWidth::Byte,
                signed,
            } => out.push(MirOp::Compare {
                dst,
                op,
                left: normalize_synthetic_high_value(left, routine_id, layout),
                right: normalize_synthetic_high_value(right, routine_id, layout),
                width: MirWidth::Byte,
                signed,
            }),
            MirOp::Compare {
                dst,
                op,
                left,
                right,
                width: MirWidth::Word,
                signed: true,
            } if matches!(op, MirCompareOp::Lt | MirCompareOp::Ge)
                && is_zero_word_value(&right) =>
            {
                let (_, left_hi) =
                    split_value_with_storage_widths(left, routine_id, layout, &temp_widths);
                out.push(MirOp::Compare {
                    dst,
                    op: match op {
                        MirCompareOp::Lt => MirCompareOp::Ge,
                        MirCompareOp::Ge => MirCompareOp::Lt,
                        _ => unreachable!(),
                    },
                    left: left_hi,
                    right: MirValue::ConstU8(0x80),
                    width: MirWidth::Byte,
                    signed: false,
                });
            }
            MirOp::LeaAddr {
                dst,
                target,
                width: MirWidth::Word,
            } => {
                if can_resolve_address_early(&target)
                    && let Some(address) = layout.mem_address(routine_id, &target)
                {
                    lower_address_to_def(dst, address, &mut out);
                } else {
                    out.push(MirOp::LeaAddr {
                        dst,
                        target,
                        width: MirWidth::Word,
                    });
                }
            }
            MirOp::Call {
                target,
                abi,
                args,
                result,
                effects,
            } => materialize_call(target, abi, args, result, effects, layout, &mut out),
            MirOp::Binary {
                op,
                dst,
                left,
                right,
                width: MirWidth::Byte,
                ..
            } if config.select_runtime_helpers
                && matches!(op, MirBinaryOp::Lsh | MirBinaryOp::Rsh)
                && !matches!(right, MirValue::ConstU8(_) | MirValue::ConstU16(_)) =>
            {
                let helper = match op {
                    MirBinaryOp::Lsh => MirRuntimeHelper::Lsh,
                    MirBinaryOp::Rsh => MirRuntimeHelper::Rsh,
                    _ => unreachable!(),
                };
                helpers.push(helper.clone());
                materialize_runtime_helper_binary(
                    helper,
                    Some(dst),
                    left,
                    right,
                    MirWidth::Byte,
                    MirWidth::Byte,
                    layout,
                    &temp_widths,
                    &mut out,
                );
            }
            MirOp::Binary {
                op,
                dst,
                left,
                right,
                width,
                ..
            } if config.select_runtime_helpers && helper_for_binary(op, width).is_some() => {
                let helper = helper_for_binary(op, width).expect("helper exists");
                let result_width = runtime_helper_result_width(&helper, width, &dst);
                helpers.push(helper.clone());
                materialize_runtime_helper_binary(
                    helper,
                    Some(dst),
                    left,
                    right,
                    width,
                    result_width,
                    layout,
                    &temp_widths,
                    &mut out,
                );
            }
            MirOp::Binary {
                op,
                dst,
                left,
                right,
                width: MirWidth::Byte,
                carry_in: None,
                carry_out,
            } if matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub) => {
                out.push(MirOp::Binary {
                    op,
                    dst,
                    left,
                    right,
                    width: MirWidth::Byte,
                    carry_in: Some(default_byte_add_sub_carry(op)),
                    carry_out,
                });
            }
            MirOp::MaterializeAddress { .. }
            | MirOp::AdvanceAddress { .. }
            | MirOp::LoadIndirect { .. }
            | MirOp::StoreIndirect { .. }
            | MirOp::UpdateMem { .. } => out.push(ops[index].clone()),
            other => out.push(other),
        }
        index += 1;
    }
    out
}

fn normalize_byte_add_sub_carry(ops: Vec<MirOp>) -> Vec<MirOp> {
    ops.into_iter()
        .map(|op| match op {
            MirOp::Binary {
                op: binary_op,
                dst,
                left,
                right,
                width: MirWidth::Byte,
                carry_in: None,
                carry_out,
            } if matches!(binary_op, MirBinaryOp::Add | MirBinaryOp::Sub) => MirOp::Binary {
                op: binary_op,
                dst,
                left,
                right,
                width: MirWidth::Byte,
                carry_in: Some(default_byte_add_sub_carry(binary_op)),
                carry_out,
            },
            other => other,
        })
        .collect()
}

fn default_byte_add_sub_carry(op: MirBinaryOp) -> MirCarryIn {
    match op {
        MirBinaryOp::Add => MirCarryIn::Clear,
        MirBinaryOp::Sub => MirCarryIn::Set,
        _ => unreachable!("default carry is only defined for add/sub"),
    }
}

fn materialize_byte_load_or_zero(
    dst: MirDef,
    src: MirMem,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    if layout.is_synthetic_byte_storage_high(routine_id, &src) {
        materialize_zero_to_def(dst, out);
        return;
    }
    out.push(MirOp::Load {
        dst,
        src: MirAddr::Direct(src),
        width: MirWidth::Byte,
    });
}

fn materialize_zero_to_def(dst: MirDef, out: &mut Vec<MirOp>) {
    if matches!(dst, MirDef::Reg(_)) {
        out.push(MirOp::LoadImm {
            dst,
            value: 0,
            width: MirWidth::Byte,
        });
    } else {
        out.push(MirOp::Move {
            dst,
            src: MirValue::ConstU8(0),
            width: MirWidth::Byte,
        });
    }
}

fn normalize_synthetic_high_value(
    value: MirValue,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> MirValue {
    match value {
        MirValue::PointerCell(mem) if layout.is_synthetic_byte_storage_high(routine_id, &mem) => {
            MirValue::ConstU8(0)
        }
        other => other,
    }
}

fn normalize_synthetic_byte_storage_high_ops(
    ops: Vec<MirOp>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Vec<MirOp> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            MirOp::Load {
                dst,
                src: MirAddr::Direct(src),
                width: MirWidth::Byte,
            } if layout.is_synthetic_byte_storage_high(routine_id, &src) => {
                materialize_zero_to_def(dst, &mut out);
            }
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                width: MirWidth::Byte,
                ..
            } if layout.is_synthetic_byte_storage_high(routine_id, &dst) => {}
            MirOp::Store { dst, src, width } => out.push(MirOp::Store {
                dst,
                src: normalize_synthetic_high_value(src, routine_id, layout),
                width,
            }),
            MirOp::Move { dst, src, width } => out.push(MirOp::Move {
                dst,
                src: normalize_synthetic_high_value(src, routine_id, layout),
                width,
            }),
            MirOp::Unary {
                op,
                dst,
                src,
                width,
            } => out.push(MirOp::Unary {
                op,
                dst,
                src: normalize_synthetic_high_value(src, routine_id, layout),
                width,
            }),
            MirOp::Binary {
                op,
                dst,
                left,
                right,
                width,
                carry_in,
                carry_out,
            } => out.push(MirOp::Binary {
                op,
                dst,
                left: normalize_synthetic_high_value(left, routine_id, layout),
                right: normalize_synthetic_high_value(right, routine_id, layout),
                width,
                carry_in,
                carry_out,
            }),
            MirOp::Compare {
                dst,
                op,
                left,
                right,
                width,
                signed,
            } => out.push(MirOp::Compare {
                dst,
                op,
                left: normalize_synthetic_high_value(left, routine_id, layout),
                right: normalize_synthetic_high_value(right, routine_id, layout),
                width,
                signed,
            }),
            MirOp::Call {
                target,
                args,
                result,
                abi,
                effects,
            } => out.push(MirOp::Call {
                target,
                args: args
                    .into_iter()
                    .map(|arg| MirCallArg {
                        value: normalize_synthetic_high_value(arg.value, routine_id, layout),
                        ..arg
                    })
                    .collect(),
                result,
                abi,
                effects,
            }),
            other => out.push(other),
        }
    }
    out
}

fn refine_temp_widths_from_storage_loads(
    ops: &[MirOp],
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &mut BTreeMap<MirTempId, MirWidth>,
) {
    for op in ops {
        let MirOp::Load {
            dst: MirDef::VTemp(id),
            src: MirAddr::Direct(mem),
            width: MirWidth::Word,
        } = op
        else {
            continue;
        };
        if layout.is_byte_scalar_storage(routine_id, mem) {
            temp_widths.insert(*id, MirWidth::Byte);
        }
    }
}

fn record_binary_temp_consumer_observation(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    routine_id: RoutineId,
    peephole_stats: &mut MirPeepholeStats,
) {
    let Some(classification) = binary_temp_consumer_observation(ops, index) else {
        return;
    };

    peephole_stats.record(routine_id, "binary-temp-consumer-candidates");
    peephole_stats.record(routine_id, classification.consumer);
    peephole_stats.record(routine_id, classification.width);
    peephole_stats.record(routine_id, classification.op);
    if classification.has_temp_operand {
        peephole_stats.record(routine_id, "binary-temp-consumer-temp-operands");
    }
    if temp_is_used_after(ops, index + 2, classification.temp)
        || terminator_uses_temp(terminator, classification.temp)
    {
        peephole_stats.record(routine_id, "binary-temp-consumer-live-after");
    } else {
        peephole_stats.record(routine_id, "binary-temp-consumer-single-use");
    }
}

struct BinaryTempConsumerObservation {
    consumer: &'static str,
    width: &'static str,
    op: &'static str,
    temp: MirTempId,
    has_temp_operand: bool,
}

fn binary_temp_consumer_observation(
    ops: &[MirOp],
    index: usize,
) -> Option<BinaryTempConsumerObservation> {
    let MirOp::Binary {
        op,
        dst,
        left,
        right,
        width,
        ..
    } = ops.get(index)?
    else {
        return None;
    };
    let temp = binary_consumer_temp_id(dst)?;
    let dst_value = MirValue::Def(dst.clone());
    let next = ops.get(index + 1)?;
    let consumer = match next {
        MirOp::Store { src, .. } if src == &dst_value => "binary-temp-consumer-store",
        MirOp::StoreIndirect { src, .. } if src == &dst_value => {
            "binary-temp-consumer-store-indirect"
        }
        MirOp::Call { args, .. } if args.iter().any(|arg| arg.value == dst_value) => {
            "binary-temp-consumer-call-arg"
        }
        MirOp::Binary {
            left: next_left,
            right: next_right,
            ..
        } if next_left == &dst_value || next_right == &dst_value => "binary-temp-consumer-binary",
        MirOp::Compare { .. } => return None,
        other if op_uses_temp(other, temp) => "binary-temp-consumer-other",
        _ => return None,
    };

    Some(BinaryTempConsumerObservation {
        consumer,
        width: match width {
            MirWidth::Byte => "binary-temp-consumer-byte",
            MirWidth::Word => "binary-temp-consumer-word",
        },
        op: binary_temp_consumer_op_stat(*op),
        temp,
        has_temp_operand: value_uses_temp(left) || value_uses_temp(right),
    })
}

fn binary_consumer_temp_id(def: &MirDef) -> Option<MirTempId> {
    match def {
        MirDef::VTemp(temp) | MirDef::VTempByte { id: temp, .. } => Some(*temp),
        _ => None,
    }
}

fn binary_temp_consumer_op_stat(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "binary-temp-consumer-op-add",
        MirBinaryOp::Sub => "binary-temp-consumer-op-sub",
        MirBinaryOp::Mul => "binary-temp-consumer-op-mul",
        MirBinaryOp::Div => "binary-temp-consumer-op-div",
        MirBinaryOp::Mod => "binary-temp-consumer-op-mod",
        MirBinaryOp::Lsh => "binary-temp-consumer-op-lsh",
        MirBinaryOp::Rsh => "binary-temp-consumer-op-rsh",
        MirBinaryOp::And => "binary-temp-consumer-op-and",
        MirBinaryOp::Or => "binary-temp-consumer-op-or",
        MirBinaryOp::Xor => "binary-temp-consumer-op-xor",
    }
}

fn try_fuse_address_store_consumer(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(MirOp::LeaAddr {
        dst,
        target,
        width: MirWidth::Word,
    }) = ops.get(index)
    else {
        return 0;
    };
    let Some(address_temp) = split_def_as_temp(dst) else {
        return 0;
    };
    let Some(MirOp::Store {
        dst: MirAddr::Direct(store_dst),
        src: MirValue::Def(MirDef::VTemp(store_temp)),
        width: MirWidth::Word,
    }) = ops.get(index + 1)
    else {
        return 0;
    };
    if *store_temp != address_temp {
        return 0;
    }
    let (lo, hi) = if can_resolve_address_early(target)
        && let Some(address) = layout.mem_address(routine_id, target)
    {
        split_address(address)
    } else {
        storage_address_bytes(target)
    };
    materialize_value_to_mem(lo, store_dst.clone(), out);
    materialize_value_to_mem(hi, offset_mem(store_dst, 1), out);
    if next_op_is_machine_block(ops, index + 2) {
        reload_low_byte_for_machine_block(store_dst.clone(), out);
    }
    2
}

fn storage_address_bytes(mem: &MirMem) -> (MirValue, MirValue) {
    (
        MirValue::StorageAddrByte {
            mem: mem.clone(),
            byte: 0,
        },
        MirValue::StorageAddrByte {
            mem: mem.clone(),
            byte: 1,
        },
    )
}

fn next_op_is_machine_block(ops: &[MirOp], index: usize) -> bool {
    matches!(ops.get(index), Some(MirOp::MachineBlock { .. }))
}

fn reload_low_byte_for_machine_block(src: MirMem, out: &mut Vec<MirOp>) {
    out.push(MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(src),
        width: MirWidth::Byte,
    });
}

fn can_resolve_address_early(mem: &MirMem) -> bool {
    matches!(mem, MirMem::Absolute(_))
}

fn materialize_byte_value_to_a(value: MirValue, out: &mut Vec<MirOp>) -> MirValue {
    match value {
        MirValue::ConstU8(_) | MirValue::PointerCell(_) => {
            out.push(MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: value,
                width: MirWidth::Byte,
            });
            MirValue::Def(MirDef::Reg(MirReg::A))
        }
        MirValue::Def(MirDef::Reg(MirReg::A)) => value,
        other => other,
    }
}

#[cfg(test)]
mod tests;
