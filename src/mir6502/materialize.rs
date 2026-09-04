use super::analysis::cfg::{MirCfg, MirCfgError};
use super::analysis::known_callees::MirKnownCalleeSummaries;
use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirAddr, MirAddressConsumer, MirArgHome, MirBinaryOp, MirBlockId, MirByteIndexedSource,
    MirCallAbi, MirCallArg, MirCallTarget, MirCarryIn, MirCarryOut, MirCompareOp, MirCond,
    MirCondDest, MirDef, MirEffects, MirFixedZpSlot, MirFlagTest, MirMachineAtom, MirMachineItem,
    MirMem, MirMemoryEffect, MirMemoryRegion, MirMemoryRegionKind, MirOp, MirOpRef, MirPhase,
    MirPointerPair, MirProgram, MirReg, MirResultHome, MirRuntimeHelper, MirSpillId,
    MirStorageBase, MirStorageInit, MirTemp, MirTempId, MirTerminator, MirUnaryOp, MirUpdateOp,
    MirValue, MirWidth, RoutineId,
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
mod copies;
mod dead_spills;
mod defs;
mod dynamic_loops;
mod flags;
mod home_census;
mod indexes;
mod layout;
mod lea;
mod machine_value_census;
mod memory;
mod narrowing;
mod peepholes;
mod pointers;
mod regs;
mod runtime;
mod small_loops;
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
mod word_sources;
mod word_values;
mod zp;

use super::rewrite::driver::{
    MirPostHomeRewriteDriver, MirPreHomeRewriteDriver, MirRewriteRunResult,
};
#[cfg(test)]
use super::rewrite::pilots::discover_dual_indirect_compares;
use super::rewrite::pilots::{
    byte_binary_compare_consumer_rank, compare_narrowing_rank, discover_affine_static_byte_indexes,
    discover_byte_binary_compare_consumers, discover_compare_narrowing, discover_compare_producers,
    discover_dual_indirect_compares_with_layout, discover_inclusive_compare_reversals,
    discover_index_rewrites, discover_pointer_rewrites, discover_unused_lea_addrs,
    inclusive_compare_reversal_rank,
};
use abi::{
    coalesce_leaf_word_param_with_result_home, elide_write_only_param_homes,
    prepend_action_abi_param_prologue, width_bytes,
};
use block_args::lower_block_arguments;
use calls::{
    CallArgExprRewriteCandidate, CallArgProducerRewriteCandidate, CallResultStoreRewriteCandidate,
    LoadedArgCallResultStoreRewriteCandidate, StoredCallResultAliasCandidate,
    call_arg_expr_rewrite_candidate, call_arg_producer_rewrite_candidate,
    call_result_store_rewrite_candidate, loaded_arg_call_result_store_rewrite_candidate,
    materialize_call, stored_call_result_alias_candidate,
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
use cfg::{collapse_empty_jump_blocks, layout_blocks_in_reverse_postorder};
#[cfg(test)]
use compare_branch::fold_compare_operand_producers_before_branches;
use compare_branch::{
    ByteAddWordCompareCandidate, ByteBinaryCompareChainRewriteCandidate,
    ByteBinaryCompareRewriteCandidate, CompareNarrowingCandidate, CompareOperandRewriteCandidate,
    DirectWordEqualityCompareCandidate, DirectWordRelationalCompareCandidate,
    InclusiveCompareReversalCandidate, SignedWordZeroCompareCandidate,
    WordArithmeticCompareCandidate, byte_add_word_compare_candidate,
    byte_binary_compare_chain_rewrite_candidate, byte_binary_compare_rewrite_candidate,
    byte_bitwise_zero_compare_narrowing_candidate, compare_branch_plan,
    compare_operand_rewrite_candidate, direct_word_equality_compare_candidate,
    direct_word_relational_compare_candidate, expand_compare_branch_consumers,
    expand_proven_byte_add_word_compare_branches,
    expand_proven_direct_word_equality_compare_branches,
    expand_proven_direct_word_relational_compare_branches,
    expand_proven_signed_word_zero_compare_branches,
    expand_proven_word_arithmetic_compare_branches, fold_posthome_signed_word_relations,
    inclusive_compare_reversal_candidate, signed_word_zero_compare_candidate,
    word_arithmetic_compare_candidate,
};
pub(in crate::mir6502) use compare_branch::{
    addressed_byte_compare_candidate, direct_indexed_byte_compare_candidate,
    dual_indirect_compare_candidate,
};
#[cfg(test)]
use compare_branch::{
    byte_binary_compare_consumer_observation, try_fuse_byte_binary_compare_consumer,
    try_fuse_byte_compare_consumer, try_fuse_compare_operand_producers,
};
use copies::select_aggregate_copies;
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
pub(in crate::mir6502) use indexes::ByteIndexUpperBound;
#[cfg(test)]
use indexes::{
    DelayedByteIndexExpr, materialize_computed_index_read, materialize_computed_index_write,
    materialize_delayed_byte_indexed_read, materialize_delayed_byte_indexed_write,
    materialize_dynamic_byte_index_read, materialize_dynamic_byte_index_write,
};
use indexes::{
    collect_delayed_byte_index_plan, indexed_addr_parts,
    indexed_word_copy_rematerialized_producer_ops, materialize_base_address,
    materialize_index_to_y, materialize_indexed_address_for_consumer,
    materialize_indexed_read_to_def, materialize_indexed_write_from_value, storage_address_value,
    try_fuse_dynamic_inline_byte_index, try_fuse_indexed_byte_copy,
    try_fuse_indexed_byte_inc_dec_update, try_fuse_indexed_to_indirect_word_copy,
    try_fuse_indexed_word_copy, try_fuse_indirect_to_indexed_word_copy,
    try_fuse_private_indirect_word_copy, try_prepare_dynamic_byte_index,
    try_prepare_dynamic_word_index,
};
pub(super) use layout::MaterializeLayout;
use lea::{lower_address_to_def, lower_lea_addrs_with_final_layout};
use machine_value_census::fold_redundant_register_reloads;
pub(in crate::mir6502) use memory::op_may_write_mem;
#[cfg(test)]
use memory::{mem_is_read_after, op_definitely_writes_mem};
use memory::{op_may_have_unknown_memory_effects, op_reads_mem};
use narrowing::narrow_discarded_high_constant_products;
#[cfg(test)]
use peepholes::{
    dead_private_scratch_store_at, fixed_pointer_consumer, fold_dead_private_scratch_stores,
    fold_dead_reg_writes_before_overwrite, fold_indirect_byte_const_compounds,
    fold_indirect_byte_const_stores, fold_indirect_byte_direct_compounds,
    fold_indirect_y_const_stores, fold_word_array_store_value_staging, staged_compare_rhs_at,
};
use peepholes::{fold_structural_before_cleanup_migrations, fold_structural_machine_tail};
use pointers::{
    is_zero_word_value, materialize_pointer_deref_address, materialize_pointer_deref_read,
    materialize_pointer_deref_read_byte, materialize_pointer_deref_write,
    materialize_pointer_deref_write_byte, pointer_value_from_mem,
    select_direct_pointer_temp_rematerialization, select_pointer_temp_deref,
    word_value_splits_to_constants,
};
#[cfg(test)]
use pointers::{rematerialize_direct_pointer_temp_derefs, try_fuse_pointer_temp_deref};
#[cfg(test)]
use regs::value_reads_reg;
use regs::{op_reads_reg, op_writes_reg};
pub(in crate::mir6502) use runtime::helper_for_binary;
use runtime::{
    ensure_helper_decl, helper_for_typed_binary, materialize_runtime_helper_binary,
    runtime_helper_result_width,
};
pub(super) use runtime::{helper_abi, helper_args, helper_effects};
#[cfg(test)]
use spills::op_may_clobber_reg;
#[cfg(test)]
pub(super) use spills::spill_accounting_for_routine;
#[cfg(test)]
use spills::{
    can_remove_spill_reload_at, can_remove_spill_reload_before_later_a_use,
    can_remove_spill_store_reload_pair_at, fold_indirect_load_spill_consumers,
    forward_block_local_spill_accumulator,
};
use spills::{
    color_basic_block_spills, color_routine_spills, lower_block_local_byte_spills_to_zero_page,
    lower_hot_induction_address_spills_to_zero_page,
    lower_known_call_result_spills_to_reused_zero_page, prune_unused_spills,
};
#[cfg(test)]
use ssa_lite::scan_ssa_lite_v2_observability;
#[cfg(test)]
use ssa_lite::{
    MirCopyPropByteValue, SsaLiteValueKey, classify_mir_copy_prop_byte_value,
    fold_mir_copy_prop_const_uses, fold_mir_copy_prop_const_uses_with_terminator,
    fold_ssa_lite_byte_loads, fold_ssa_lite_single_predecessor_loads, scan_ssa_lite_block_env,
    temp_byte_binary_candidate_reason_for_test,
};
use ssa_lite::{
    fold_mir_copy_prop_const_uses_with_terminator_and_live_out, record_ssa_lite_block_facts,
    record_ssa_lite_v2_observability,
};
use stats::{MirPeepholeStats, maybe_report_peepholes};
use store_consumers::{
    materialize_value_to_mem, select_absolute_word_sub_indirect_store_consumer,
    select_abstract_byte_inc_dec_store_consumer, select_byte_mul_add_sub_word_store_consumer,
    select_byte_store_consumer, select_direct_copy_store_consumer, select_store_expr_producers,
    select_widened_byte_shift_store_consumer, select_word_arithmetic_dual_indirect_store_consumer,
    select_word_arithmetic_indirect_store_consumer, select_word_arithmetic_pointer_store_consumer,
    select_word_arithmetic_result_consumer, select_word_carry_chain_store_consumer,
    select_word_helper_store_consumer, select_word_store_consumer,
    try_fuse_byte_mul_word_store_consumer, try_fuse_cast_store_consumer,
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
use temp_widths::{collect_routine_temp_widths, collect_temp_widths};
use temps::{
    cleanup_pre_materialization_temp_artifacts,
    cleanup_pre_materialization_temp_artifacts_with_liveness, def_is_used_after,
    materialize_fused_compare_dest, materialize_temp_ops_with_routine_widths_and_address_reuse,
    materialize_terminator, store_a_to_spill, temp_is_used_after,
};
#[cfg(test)]
use temps::{materialize_temp_ops, materialize_temp_ops_with_routine_widths};
use values::{
    offset_mem, return_slot_mem, split_address, split_def, split_value, split_value_as_word,
    split_value_with_storage_widths, split_value_with_temp_widths,
};
use word_values::forward_unique_word_load_address_consumers;
use zp::{
    allocate_zero_page_slots, find_zp_range, mark_zp_range, reserve_pointer_scratch_slots,
    reserve_used_fixed_zero_page_slots, resolve_virtual_address_consumers, source_zero_page_slots,
};

const POINTER_SCRATCH_LO: u8 = 0xAC;
const POINTER_SCRATCH_HI: u8 = 0xAD;
const POINTER_INDEX_SCRATCH_LO: u8 = 0xAE;
const POINTER_INDEX_SCRATCH_HI: u8 = 0xAF;
const INDIRECT_CALL_TARGET_LO: u8 = 0xE4;
const INDIRECT_CALL_TARGET_HI: u8 = 0xE5;
const DEST_POINTER_SCRATCH_LO: u8 = 0xAA;
const MAX_INLINE_WORD_CONSTANT_SHIFT: u16 = 2;

const DEFAULT_POINTER_PAIR: MirAddressConsumer =
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(POINTER_SCRATCH_LO),
    });
const DEFAULT_PAGED_Y_POINTER_PAIR: MirAddressConsumer =
    MirAddressConsumer::PagedIndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(POINTER_SCRATCH_LO),
    });
const DEFAULT_SCALED_Y_POINTER_PAIR: MirAddressConsumer =
    MirAddressConsumer::ScaledIndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(POINTER_SCRATCH_LO),
    });
const INDEX_POINTER_PAIR: MirAddressConsumer =
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(POINTER_INDEX_SCRATCH_LO),
    });
const INDEX_SCALED_Y_POINTER_PAIR: MirAddressConsumer =
    MirAddressConsumer::ScaledIndirectIndexedY(MirPointerPair::Fixed {
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

pub(in crate::mir6502) fn analyzed_inclusive_compare_reversal_candidate(
    op: &MirOp,
    layout: &MaterializeLayout,
) -> Option<InclusiveCompareReversalCandidate> {
    inclusive_compare_reversal_candidate(op, layout)
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

pub(in crate::mir6502) fn analyzed_byte_binary_compare_chain_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<ByteBinaryCompareChainRewriteCandidate> {
    byte_binary_compare_chain_rewrite_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_byte_add_word_compare_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<ByteAddWordCompareCandidate> {
    byte_add_word_compare_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_word_arithmetic_compare_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<WordArithmeticCompareCandidate> {
    word_arithmetic_compare_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_direct_word_equality_compare_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<DirectWordEqualityCompareCandidate> {
    direct_word_equality_compare_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_direct_word_relational_compare_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<DirectWordRelationalCompareCandidate> {
    direct_word_relational_compare_candidate(ops, index)
}

pub(in crate::mir6502) fn analyzed_signed_word_zero_compare_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<SignedWordZeroCompareCandidate> {
    signed_word_zero_compare_candidate(ops, index)
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

pub(in crate::mir6502) fn analyzed_stored_call_result_alias_candidate(
    ops: &[MirOp],
    index: usize,
) -> Option<StoredCallResultAliasCandidate> {
    stored_call_result_alias_candidate(ops, index)
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
    pub required_upper_bound: Option<indexes::ByteIndexUpperBound>,
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

pub(in crate::mir6502) fn analyzed_word_carry_chain_store_candidates(
    block: &super::ir::MirBlock,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
) -> Vec<(usize, StoreConsumerRewriteCandidate)> {
    let ops = &block.ops;
    (0..ops.len())
        .filter_map(|index| {
            let mut replacement = Vec::new();
            let consumed = select_word_carry_chain_store_consumer(
                ops,
                index,
                config,
                layout,
                &mut replacement,
            );
            (consumed > 0).then_some((
                index,
                StoreConsumerRewriteCandidate {
                    start: index,
                    consumed,
                    replacement,
                    stat: "word-carry-chain-store-consumer",
                    family_priority: 120,
                },
            ))
        })
        .collect()
}

pub(in crate::mir6502) fn analyzed_widened_byte_shift_store_candidates(
    routine_id: RoutineId,
    block: &super::ir::MirBlock,
    layout: &MaterializeLayout,
) -> Vec<(usize, StoreConsumerRewriteCandidate)> {
    let ops = &block.ops;
    (0..ops.len())
        .filter_map(|index| {
            let mut replacement = Vec::new();
            let consumed = select_widened_byte_shift_store_consumer(
                ops,
                index,
                routine_id,
                layout,
                &mut replacement,
            );
            (consumed > 0).then_some((
                index,
                StoreConsumerRewriteCandidate {
                    start: index,
                    consumed,
                    replacement,
                    stat: "widened-byte-shift-store-consumer",
                    family_priority: 119,
                },
            ))
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
        .flat_map(|index| {
            let primary = analyzed_index_rewrite_candidate_at(
                routine_id,
                ops,
                index,
                layout,
                &delayed_byte_indexes,
                true,
            );
            let fallback = primary
                .as_ref()
                .is_some_and(|candidate| candidate.stat == "adjacent-static-indexed-byte-copy")
                .then(|| {
                    analyzed_index_rewrite_candidate_at(
                        routine_id,
                        ops,
                        index,
                        layout,
                        &delayed_byte_indexes,
                        false,
                    )
                })
                .flatten();
            [primary, fallback]
                .into_iter()
                .flatten()
                .map(|candidate| (candidate.start, candidate))
        })
        .collect()
}

pub(in crate::mir6502) fn analyzed_affine_static_byte_index_candidates(
    block: &super::ir::MirBlock,
) -> Vec<(usize, IndexRewriteCandidate)> {
    let ops = &block.ops;
    let indexes = collect_delayed_byte_index_plan(ops);
    (0..ops.len())
        .filter_map(|index| {
            affine_static_byte_index_rewrite_candidate_at(ops, index, &indexes)
                .map(|candidate| (candidate.start, candidate))
        })
        .collect()
}

pub(in crate::mir6502) fn analyzed_indexed_to_indirect_word_copy_candidates(
    block: &super::ir::MirBlock,
    layout: &MaterializeLayout,
) -> Vec<(usize, IndexRewriteCandidate)> {
    let ops = &block.ops;
    (0..ops.len())
        .filter_map(|index| {
            let mut replacement = Vec::new();
            let consumed =
                try_fuse_indexed_to_indirect_word_copy(ops, index, layout, &mut replacement);
            if consumed == 0 {
                return None;
            }
            let candidate = IndexRewriteCandidate {
                start: index,
                consumed,
                replacement,
                stat: "indexed-to-indirect-word-copy",
                observations: Vec::new(),
                family_priority: 111,
                required_upper_bound: None,
            };
            let candidate = expand_index_rewrite_window_with_producers(
                ops,
                index,
                candidate,
                indexed_word_copy_rematerialized_producer_ops(ops, index),
            );
            Some((candidate.start, candidate))
        })
        .collect()
}

pub(in crate::mir6502) fn analyzed_indirect_to_indexed_word_copy_candidates(
    block: &super::ir::MirBlock,
    layout: &MaterializeLayout,
) -> Vec<(usize, IndexRewriteCandidate)> {
    let ops = &block.ops;
    (0..ops.len())
        .filter_map(|index| {
            let mut replacement = Vec::new();
            let consumed =
                try_fuse_indirect_to_indexed_word_copy(ops, index, layout, &mut replacement);
            if consumed == 0 {
                return None;
            }
            let candidate = IndexRewriteCandidate {
                start: index,
                consumed,
                replacement,
                stat: "indirect-to-indexed-word-copy",
                observations: Vec::new(),
                family_priority: 112,
                required_upper_bound: None,
            };
            let candidate = expand_index_rewrite_window_with_producers(
                ops,
                index,
                candidate,
                indexed_word_copy_rematerialized_producer_ops(ops, index),
            );
            Some((candidate.start, candidate))
        })
        .collect()
}

pub(in crate::mir6502) fn analyzed_private_indirect_word_copy_candidates(
    block: &super::ir::MirBlock,
    layout: &MaterializeLayout,
) -> Vec<(usize, IndexRewriteCandidate)> {
    let ops = &block.ops;
    (0..ops.len())
        .filter_map(|index| {
            let mut replacement = Vec::new();
            let consumed =
                try_fuse_private_indirect_word_copy(ops, index, layout, &mut replacement);
            if consumed == 0 {
                return None;
            }
            let candidate = IndexRewriteCandidate {
                start: index,
                consumed,
                replacement,
                stat: "private-indirect-word-copy",
                observations: Vec::new(),
                family_priority: 113,
                required_upper_bound: None,
            };
            let candidate = expand_index_rewrite_window_with_producers(
                ops,
                index,
                candidate,
                indexed_word_copy_rematerialized_producer_ops(ops, index),
            );
            Some((candidate.start, candidate))
        })
        .collect()
}

fn analyzed_index_rewrite_candidate_at(
    routine_id: RoutineId,
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    delayed_byte_indexes: &indexes::DelayedByteIndexPlan,
    allow_adjacent_direct_copy: bool,
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
                required_upper_bound: None,
            })
        };

    if let Some(candidate) =
        affine_static_byte_index_rewrite_candidate_at(ops, index, delayed_byte_indexes)
    {
        return Some(candidate);
    }

    if allow_adjacent_direct_copy
        && let Some(direct) =
            indexes::adjacent_static_indexed_byte_copy_candidate(ops, index, delayed_byte_indexes)
    {
        let producer_count = direct.producer_ops.len();
        let mut candidate = expand_index_rewrite_window_with_producers(
            ops,
            index,
            IndexRewriteCandidate {
                start: index,
                consumed: direct.consumed,
                replacement: direct.replacement,
                stat: "adjacent-static-indexed-byte-copy",
                observations: Vec::new(),
                family_priority: 90,
                required_upper_bound: Some(direct.required_upper_bound),
            },
            direct.producer_ops,
        );
        if producer_count != 0 {
            candidate
                .observations
                .push(("delayed-byte-index-producer", producer_count));
        }
        return Some(candidate);
    }

    let mut replacement = Vec::new();
    let consumed = indexes::try_fuse_same_base_indexed_byte_copy(
        ops,
        index,
        layout,
        delayed_byte_indexes,
        &mut replacement,
    );
    if consumed > 0 {
        return Some(expand_delayed_index_rewrite_window(
            ops,
            index,
            IndexRewriteCandidate {
                start: index,
                consumed,
                replacement,
                stat: "same-base-indexed-byte-copy",
                observations: Vec::new(),
                family_priority: 95,
                required_upper_bound: None,
            },
            delayed_byte_indexes,
        ));
    }

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
                required_upper_bound: None,
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
    let consumed = try_fuse_indexed_to_indirect_word_copy(ops, index, layout, &mut replacement);
    if let Some(candidate) = selected(consumed, replacement, "indexed-to-indirect-word-copy", 111) {
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

fn affine_static_byte_index_rewrite_candidate_at(
    ops: &[MirOp],
    index: usize,
    indexes: &indexes::DelayedByteIndexPlan,
) -> Option<IndexRewriteCandidate> {
    let addr = match ops.get(index)? {
        MirOp::Load {
            src,
            width: MirWidth::Byte,
            ..
        } => src,
        MirOp::Store {
            dst,
            width: MirWidth::Byte,
            ..
        } => dst,
        _ => return None,
    };
    let affine = indexes::canonical_static_affine_byte_index(ops, index, addr, indexes)?;
    let mut replacement = Vec::new();
    indexes::materialize_index_to_y(affine.root, &mut replacement);
    match ops.get(index)? {
        MirOp::Load { dst, .. } => replacement.push(MirOp::Load {
            dst: dst.clone(),
            src: MirAddr::AbsoluteIndexedY {
                base: affine.indexed_base,
            },
            width: MirWidth::Byte,
        }),
        MirOp::Store { src, .. } => {
            let src = materialize_byte_value_to_a(src.clone(), &mut replacement);
            replacement.push(MirOp::Store {
                dst: MirAddr::AbsoluteIndexedY {
                    base: affine.indexed_base,
                },
                src,
                width: MirWidth::Byte,
            });
        }
        _ => unreachable!("affine address was selected from a byte load or store"),
    }
    Some(expand_index_rewrite_window_with_producers(
        ops,
        index,
        IndexRewriteCandidate {
            start: index,
            consumed: 1,
            replacement,
            stat: "affine-static-byte-index",
            observations: Vec::new(),
            family_priority: 89,
            required_upper_bound: None,
        },
        affine.producer_ops,
    ))
}

fn delayed_byte_index_rewrite_candidate_at(
    ops: &[MirOp],
    index: usize,
    layout: &MaterializeLayout,
    delayed_byte_indexes: &indexes::DelayedByteIndexPlan,
) -> Option<IndexRewriteCandidate> {
    let indexed_addr = match ops.get(index)? {
        MirOp::Load {
            src: src @ (MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. }),
            ..
        } => src,
        MirOp::Store {
            dst: dst @ (MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. }),
            ..
        } => dst,
        _ => return None,
    };
    let mut parts = indexed_addr_parts(indexed_addr)?;
    let original_base = parts.base.clone();
    parts.base = indexes::resolve_indexed_base_producer(ops, index, parts.base);
    let base_producer = (parts.base != original_base)
        .then(|| {
            let MirValue::Def(MirDef::VTemp(temp)) = original_base else {
                return None;
            };
            ops[..index]
                .iter()
                .enumerate()
                .rev()
                .find_map(|(producer_index, op)| {
                    (op_def(op).and_then(split_def_as_temp) == Some(temp)).then_some(producer_index)
                })
        })
        .flatten();
    let mut replacement = Vec::new();
    let used_delayed_index = match ops.get(index)? {
        MirOp::Load { dst, width, .. } => materialize_indexed_read_to_def(
            dst.clone(),
            parts,
            *width,
            layout,
            Some(delayed_byte_indexes),
            &mut replacement,
        ),
        MirOp::Store { src, width, .. } => materialize_indexed_write_from_value(
            parts,
            src.clone(),
            *width,
            layout,
            Some(delayed_byte_indexes),
            &mut replacement,
        ),
        _ => false,
    };
    used_delayed_index.then(|| {
        let mut producer_ops = delayed_producer_ops_for_window(ops, index, 1, delayed_byte_indexes);
        let delayed_producer_count = producer_ops.len();
        if let Some(base_producer) = base_producer {
            producer_ops.insert(base_producer);
        }
        let mut candidate = expand_index_rewrite_window_with_producers(
            ops,
            index,
            IndexRewriteCandidate {
                start: index,
                consumed: 1,
                replacement,
                stat: "delayed-byte-index-consumer",
                observations: Vec::new(),
                family_priority: 150,
                required_upper_bound: None,
            },
            producer_ops,
        );
        if delayed_producer_count != 0 {
            candidate
                .observations
                .push(("delayed-byte-index-producer", delayed_producer_count));
        }
        candidate
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
    let consumed = select_absolute_word_sub_indirect_store_consumer(
        ops,
        index,
        routine_id,
        layout,
        &mut replacement,
    );
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "absolute-word-sub-indirect-store-consumer",
            family_priority: 117,
        });
    }

    let consumed = select_word_arithmetic_result_consumer(ops, index, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "word-arithmetic-result-consumer",
            family_priority: 113,
        });
    }

    let consumed = select_word_arithmetic_pointer_store_consumer(ops, index, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "word-arithmetic-pointer-store-consumer",
            family_priority: 114,
        });
    }

    let consumed =
        select_word_arithmetic_dual_indirect_store_consumer(ops, index, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "word-arithmetic-dual-indirect-store-consumer",
            family_priority: 116,
        });
    }

    let consumed = select_word_arithmetic_indirect_store_consumer(ops, index, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "word-arithmetic-indirect-store-consumer",
            family_priority: 115,
        });
    }

    let consumed = try_fuse_indexed_byte_inc_dec_update(ops, index, layout, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "indexed-byte-inc-dec-update",
            family_priority: 90,
        });
    }

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
    let consumed =
        select_word_carry_chain_store_consumer(ops, index, config, layout, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "word-carry-chain-store-consumer",
            family_priority: 120,
        });
    }

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
            family_priority: 130,
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
            family_priority: 140,
        });
    }

    let consumed = select_word_helper_store_consumer(
        ops,
        index,
        config,
        layout,
        temp_widths,
        false,
        &mut selected_helpers,
        &mut replacement,
    );
    if consumed > 0 {
        let stat = if selected_helpers.contains(&MirRuntimeHelper::MulByte) {
            "widening-byte-multiply-selected"
        } else {
            "word-helper-store-consumer"
        };
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat,
            family_priority: 145,
        });
    }

    let consumed = select_word_store_consumer(ops, index, config, layout, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "word-store-consumer",
            family_priority: 150,
        });
    }

    let consumed = select_direct_copy_store_consumer(ops, index, layout, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "direct-copy-store-consumer",
            family_priority: 160,
        });
    }

    let consumed =
        select_abstract_byte_inc_dec_store_consumer(ops, index, terminator, &mut replacement);
    if consumed > 0 {
        return Some(StoreConsumerRewriteCandidate {
            start: index,
            consumed,
            replacement,
            stat: "abstract-byte-inc-dec-store-consumer",
            family_priority: 165,
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
            family_priority: if start < index { 90 } else { 170 },
        });
    }

    let consumed =
        select_store_expr_producers(ops, index, terminator, config, layout, &mut replacement);
    (consumed > 0).then_some(StoreConsumerRewriteCandidate {
        start: index,
        consumed,
        replacement,
        stat: "store-expr-consumer",
        family_priority: 180,
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
    refine_terminal_indirect_jump_effects(&mut program);
    reserve_pointer_scratch_slots(&mut program);
    allocate_zero_page_slots(&mut program);
    let source_zero_page = source_zero_page_slots(&program);
    {
        let (routines, machine_blocks) = (&mut program.routines, &mut program.machine_blocks);
        for routine in routines {
            prepend_action_abi_param_prologue(routine, machine_blocks, &mut helpers);
        }
    }
    let layout = MaterializeLayout::new(&program, object_origin);
    for routine in &mut program.routines {
        let aggregate_copies = select_aggregate_copies(routine);
        peephole_stats.record_many(
            routine.id,
            "aggregate-copy-retained",
            aggregate_copies.retained,
        );
        peephole_stats.record_many(
            routine.id,
            "aggregate-copy-selected",
            aggregate_copies.selected,
        );
        peephole_stats.record_many(
            routine.id,
            "aggregate-copy-prepared-address",
            aggregate_copies.prepared_address,
        );
        peephole_stats.record_many(
            routine.id,
            "aggregate-copy-scalar-fallback",
            aggregate_copies.scalar_fallback,
        );
        peephole_stats.record_many(
            routine.id,
            "aggregate-copy-blocked-address-form",
            aggregate_copies.blocked_address_form,
        );
        peephole_stats.record_many(
            routine.id,
            "aggregate-copy-blocked-offset-range",
            aggregate_copies.blocked_offset_range,
        );
        run_cfg_group(routine, &layout)?;
        let narrow_products = narrow_discarded_high_constant_products(routine);
        peephole_stats.record_many(
            routine.id,
            "discarded-high-product-candidate",
            narrow_products.candidates,
        );
        peephole_stats.record_many(
            routine.id,
            "discarded-high-product-narrowed",
            narrow_products.applied,
        );
        peephole_stats.record_many(
            routine.id,
            "discarded-high-product-blocked-high-lane-live",
            narrow_products.blocked_high_lane_live,
        );
        peephole_stats.record_many(
            routine.id,
            "discarded-high-product-blocked-multiple-definitions",
            narrow_products.blocked_multiple_definitions,
        );
        peephole_stats.record_many(
            routine.id,
            "discarded-high-product-blocked-operand-width",
            narrow_products.blocked_operand_width,
        );
        peephole_stats.record_many(
            routine.id,
            "discarded-high-product-blocked-carry-contract",
            narrow_products.blocked_carry_contract,
        );
        strength_reduce_constant_multiplications(
            routine,
            &layout,
            &narrow_products.low_only_results,
            &mut peephole_stats,
        );
        run_analyzed_widened_byte_shift_store_consumers(routine, &layout, &mut peephole_stats)?;
        lower_constant_word_shift_projections(routine, &layout, &mut peephole_stats);
        lower_small_constant_word_shifts(routine, &layout, &mut peephole_stats);
        let routine_temp_widths = collect_routine_temp_widths(routine);
        run_prehome_canonicalization_group(routine, config, &layout, &mut peephole_stats)?;
        run_prehome_selection_group(routine, config, &layout, &mut helpers, &mut peephole_stats)?;
        for block in &mut routine.blocks {
            block.ops = materialize_ops_impl(
                routine.id,
                block.id,
                block.ops.clone(),
                &block.terminator,
                config,
                &layout,
                &mut helpers,
                &mut peephole_stats,
                false,
                Some(&routine_temp_widths),
            );
            block.ops = normalize_synthetic_byte_storage_high_ops(
                std::mem::take(&mut block.ops),
                routine.id,
                &layout,
            );
        }
        reserve_used_fixed_zero_page_slots(routine);
        materialize_word_compare_temp_ops(routine, &layout);
        run_pre_home_cleanup_fixed_point(routine, &layout, &mut peephole_stats);
        let home_liveness = analyze_temp_liveness(routine);
        let home_plan = record_home_demand_census(routine, &home_liveness, &mut peephole_stats);
        home_fates.insert(routine.id, HomeFateTracker::from_plan(&home_plan));
        apply_register_home_plan(routine, &home_plan, &mut peephole_stats);
        for block in &mut routine.blocks {
            let (ops, repeated_address_reuses) =
                materialize_temp_ops_with_routine_widths_and_address_reuse(
                    std::mem::take(&mut block.ops),
                    &mut routine.frame.spills,
                    &routine_temp_widths,
                );
            block.ops = ops;
            peephole_stats.record_many(
                routine.id,
                "ssa-lite-redundant-address",
                repeated_address_reuses,
            );
            block.ops = normalize_synthetic_byte_storage_high_ops(
                std::mem::take(&mut block.ops),
                routine.id,
                &layout,
            );
            // LEA temps retain an implicit identity with the two spill lanes
            // selected from their temp ID. Make those writes explicit before
            // spill coloring so definitions and uses are remapped together.
            // Symbolic storage-address bytes remain layout-independent until
            // emission, while descriptors retain their pointer-cell meaning.
            block.ops = lower_lea_addrs_with_final_layout(routine.id, block.ops.clone(), &layout);
        }
        let induction_zp_remap =
            lower_hot_induction_address_spills_to_zero_page(routine, &source_zero_page);
        peephole_stats.record_many(
            routine.id,
            "hot-induction-address-zp-pairs",
            induction_zp_remap.len() / 2,
        );
        if let Some(tracker) = home_fates.get_mut(&routine.id) {
            tracker.apply_zero_page_remap(&induction_zp_remap);
        }
        run_posthome_structural_group(routine, &layout, config, None, &mut peephole_stats)?;
        run_posthome_cleanup_group(
            routine,
            &layout,
            None,
            None,
            home_fates.get_mut(&routine.id),
            &mut peephole_stats,
        )?;
    }
    for routine in &mut program.routines {
        coalesce_leaf_word_param_with_result_home(routine, &mut peephole_stats);
    }
    for routine in &mut program.routines {
        elide_write_only_param_homes(routine, &mut peephole_stats);
    }
    for helper in helpers {
        ensure_helper_decl(&mut program, helper);
    }
    // Home selection may introduce virtual zero-page lanes after the initial
    // reservation pass. Assign those lanes before building known-callee
    // memory summaries so a callee's private scratch writes have exact
    // physical identities for cross-call preservation proofs.
    allocate_zero_page_slots(&mut program);
    let known_callees = MirKnownCalleeSummaries::analyze(&program);
    let layout = MaterializeLayout::new(&program, object_origin);
    for routine in &mut program.routines {
        for block in &mut routine.blocks {
            block.ops = lower_lea_addrs_with_final_layout(routine.id, block.ops.clone(), &layout);
            block.ops = normalize_synthetic_byte_storage_high_ops(
                std::mem::take(&mut block.ops),
                routine.id,
                &layout,
            );
        }
        run_posthome_structural_group(
            routine,
            &layout,
            config,
            Some(&known_callees),
            &mut peephole_stats,
        )?;
        run_posthome_cleanup_group(
            routine,
            &layout,
            Some(config),
            Some(&known_callees),
            home_fates.get_mut(&routine.id),
            &mut peephole_stats,
        )?;
    }
    // Known-callee carrier selection can replace the last parameter reload
    // with a preserved machine register. Re-run the narrow ABI-home elider so
    // the now write-only entry capture is removed in the same pipeline.
    for routine in &mut program.routines {
        elide_write_only_param_homes(routine, &mut peephole_stats);
    }
    let zero_page_remaps = lower_block_local_byte_spills_to_zero_page(&mut program);
    for (routine, remap) in zero_page_remaps {
        if let Some(tracker) = home_fates.get_mut(&routine) {
            tracker.apply_zero_page_remap(&remap);
        }
    }
    allocate_zero_page_slots(&mut program);
    let zero_page_known_callees = MirKnownCalleeSummaries::analyze(&program);
    let reused_zero_page_remaps =
        lower_known_call_result_spills_to_reused_zero_page(&mut program, &zero_page_known_callees);
    for (routine, remap) in reused_zero_page_remaps {
        peephole_stats.record_many(
            routine,
            "known-call-result-preserved-in-reused-zp",
            remap.len() / 2,
        );
        if let Some(tracker) = home_fates.get_mut(&routine) {
            tracker.apply_zero_page_remap(&remap);
        }
    }
    // Zero-page coloring can make two previously distinct logical homes the
    // same physical byte at a CFG edge. Rebuild machine-value and exact Z/N
    // provenance facts after that remap so final physical reloads can be
    // removed safely.
    let final_known_callees = MirKnownCalleeSummaries::analyze(&program);
    let final_layout = MaterializeLayout::new(&program, object_origin);
    for routine in &mut program.routines {
        run_analyzed_ssa_lite_byte_rewrites(
            routine,
            &final_layout,
            true,
            Some(&final_known_callees),
            &mut peephole_stats,
        )?;
        let exact_zn_compares =
            ssa_lite::fold_exact_zn_zero_compares(routine, &final_known_callees);
        peephole_stats.record_many(routine.id, "exact-zn-zero-compare-fold", exact_zn_compares);
        if config.enable_peepholes {
            let selected = small_loops::select_high_bit_shift_xor_diamonds(routine, &final_layout);
            peephole_stats.record_many(
                routine.id,
                "high-bit-shift-xor-carry-branch-selected",
                selected,
            );
            verify_cfg_after_transform(routine, "late high-bit shift/XOR carry selection")?;
        }
        // Exact Z/N folding can expose a load/add-or-sub/store update whose
        // only remaining consumer is the branch on the update result. Re-run
        // the analyzed selector here so countdown latches become DEC/BNE (or
        // INC/BNE) without changing live A/carry/overflow state.
        run_analyzed_direct_inc_dec_updates(routine, &final_layout, &mut peephole_stats)?;
        let carried = cfg::select_counted_loop_register_carriers(routine, &final_layout);
        peephole_stats.record_many(routine.id, "register-carried-induction-selected", carried);
        if carried > 0 && layout_blocks_in_reverse_postorder(routine) {
            peephole_stats.record(routine.id, "register-carried-induction-layout");
        }
        if layout_blocks_in_reverse_postorder(routine) {
            peephole_stats.record(routine.id, "cfg-cost-aware-layout");
        }
        let latch_report = cfg::select_counted_loop_latches_with_report(routine, &final_layout);
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-candidate",
            latch_report.candidates,
        );
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-blocked-initial-guard",
            latch_report.blocked_initial_guard,
        );
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-first-entry-a-required",
            latch_report.first_entry_accumulator_required,
        );
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-first-entry-flags-required",
            latch_report.first_entry_flags_required,
        );
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-blocked-unsupported-or-unsafe",
            latch_report.blocked_unsupported_or_unsafe,
        );
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-blocked-profitability",
            latch_report.blocked_profitability,
        );
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-selected-exact-rotation",
            latch_report.selected_exact_rotation,
        );
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-selected-first-entry-repair",
            latch_report.selected_first_entry_repaired,
        );
        peephole_stats.record_many(
            routine.id,
            "counted-loop-latch-selected",
            latch_report.selected,
        );
        if latch_report.selected > 0 && layout_blocks_in_reverse_postorder(routine) {
            peephole_stats.record(routine.id, "counted-loop-post-selection-layout");
        }
    }
    materialize_remaining_pointer_cell_values(&mut program);
    fold_redundant_register_reloads(&mut program, &final_layout, &mut peephole_stats);
    for routine in &mut program.routines {
        if config.enable_peepholes {
            let selected = small_loops::coalesce_chained_shift_xor_stores(routine, &final_layout);
            peephole_stats.record_many(routine.id, "chained-shift-xor-store-coalesced", selected);
            let entry_stores =
                small_loops::coalesce_selected_shift_entry_stores(routine, &final_layout);
            peephole_stats.record_many(
                routine.id,
                "selected-shift-entry-store-coalesced",
                entry_stores,
            );
            let hoisted = small_loops::hoist_common_shift_xor_stores(routine, &final_layout);
            peephole_stats.record_many(routine.id, "common-shift-xor-store-hoisted", hoisted);
            if selected > 0 || entry_stores > 0 || hoisted > 0 {
                collapse_empty_jump_blocks(routine);
                if layout_blocks_in_reverse_postorder(routine) {
                    peephole_stats.record(routine.id, "chained-shift-xor-layout");
                }
            }
            verify_cfg_after_transform(routine, "chained shift/XOR store coalescing")?;
        }
    }
    // Late exact-Z/N and register-value folding can leave an earlier pure
    // register write dead across a CFG edge. Rebuild machine liveness before
    // the final home-definition cleanup so both exposed register writes and
    // newly dead private stores are removed from the final program.
    for routine in &mut program.routines {
        run_analyzed_dead_register_writes(
            routine,
            &final_layout,
            Some(&final_known_callees),
            &mut peephole_stats,
        )?;
        run_analyzed_dead_private_scratch_stores(routine, &mut peephole_stats)?;
        prune_unused_spills(routine);
    }
    resolve_virtual_address_consumers(&mut program);
    verify_materialization_stage(&program, MirPhase::PostHome, "post-home boundary")?;
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

fn strength_reduce_constant_multiplications(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    proven_low_only_results: &BTreeSet<MirTempId>,
    peephole_stats: &mut MirPeepholeStats,
) {
    let temp_widths = collect_routine_temp_widths(routine);
    let mut fresh = FreshTemps::new(&routine.temps);
    let (temps, blocks) = (&mut routine.temps, &mut routine.blocks);
    let mut reduced = 0;

    for block in blocks {
        let mut out = Vec::with_capacity(block.ops.len());
        for op in std::mem::take(&mut block.ops) {
            let MirOp::Binary {
                op: MirBinaryOp::Mul,
                dst,
                left,
                right,
                width,
                ..
            } = op
            else {
                out.push(op);
                continue;
            };

            let Some((operand, factor)) = constant_multiply_parts(left.clone(), right.clone())
            else {
                out.push(MirOp::Binary {
                    op: MirBinaryOp::Mul,
                    dst,
                    left,
                    right,
                    width,
                    carry_in: None,
                    carry_out: MirCarryOut::Ignore,
                });
                continue;
            };

            // Pointer-cell values can still represent a memory read at this
            // operation. Keep the helper unless the replacement itself reads
            // the complete value.
            if factor != 1 && value_contains_pointer_cell(&operand) {
                out.push(MirOp::Binary {
                    op: MirBinaryOp::Mul,
                    dst,
                    left,
                    right,
                    width,
                    carry_in: None,
                    carry_out: MirCarryOut::Ignore,
                });
                continue;
            }

            let result_width =
                strength_reduced_multiply_width(width, &dst, proven_low_only_results);

            let replacement = match result_width {
                MirWidth::Byte => strength_reduce_byte_multiply(dst.clone(), operand, factor),
                MirWidth::Word => strength_reduce_word_multiply(
                    dst.clone(),
                    operand,
                    factor,
                    routine.id,
                    layout,
                    &temp_widths,
                    &mut fresh,
                    temps,
                ),
            };
            let Some(replacement) = replacement else {
                out.push(MirOp::Binary {
                    op: MirBinaryOp::Mul,
                    dst,
                    left,
                    right,
                    width,
                    carry_in: None,
                    carry_out: MirCarryOut::Ignore,
                });
                continue;
            };
            out.extend(replacement);
            reduced += 1;
        }
        block.ops = out;
    }

    peephole_stats.record_many(routine.id, "constant-multiply-strength-reduction", reduced);
}

fn strength_reduced_multiply_width(
    width: MirWidth,
    dst: &MirDef,
    proven_low_only_results: &BTreeSet<MirTempId>,
) -> MirWidth {
    let result_is_proven_low_only = match dst {
        MirDef::VTemp(id) | MirDef::VTempByte { id, .. } => proven_low_only_results.contains(id),
        MirDef::Reg(_) => false,
    };
    if width == MirWidth::Byte && split_def(dst.clone()).is_some() && !result_is_proven_low_only {
        // Action!'s byte multiply helper returns A:X. Preserve that word
        // result unless an analyzed rewrite proved the synthetic high lane
        // unobservable throughout the complete routine.
        MirWidth::Word
    } else {
        width
    }
}

fn constant_multiply_parts(left: MirValue, right: MirValue) -> Option<(MirValue, u16)> {
    let (operand, factor) = if let Some(factor) = constant_value_u16(&right) {
        (left, factor)
    } else {
        (right, constant_value_u16(&left)?)
    };
    (factor <= 1 || factor.is_power_of_two()).then_some((operand, factor))
}

fn constant_value_u16(value: &MirValue) -> Option<u16> {
    match value {
        MirValue::ConstU8(value) => Some(u16::from(*value)),
        MirValue::ConstU16(value) => Some(*value),
        _ => None,
    }
}

fn lower_constant_word_shift_projections(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) {
    let temp_widths = collect_routine_temp_widths(routine);
    let mut lowered = 0;

    for block in &mut routine.blocks {
        let mut out = Vec::with_capacity(block.ops.len());
        for op in std::mem::take(&mut block.ops) {
            let MirOp::Binary {
                op: shift_op @ (MirBinaryOp::Lsh | MirBinaryOp::Rsh),
                dst,
                left,
                right,
                width: MirWidth::Word,
                carry_in,
                carry_out,
            } = op
            else {
                out.push(op);
                continue;
            };
            let Some(count) = constant_value_u16(&right) else {
                out.push(MirOp::Binary {
                    op: shift_op,
                    dst,
                    left,
                    right,
                    width: MirWidth::Word,
                    carry_in,
                    carry_out,
                });
                continue;
            };
            if carry_in.is_some() || carry_out != MirCarryOut::Ignore {
                out.push(MirOp::Binary {
                    op: shift_op,
                    dst,
                    left,
                    right,
                    width: MirWidth::Word,
                    carry_in,
                    carry_out,
                });
                continue;
            }

            if count == 0 {
                out.push(MirOp::Move {
                    dst,
                    src: left,
                    width: MirWidth::Word,
                });
                lowered += 1;
                continue;
            }

            // Pointer-cell values still represent a complete word read at
            // this operation. Keep the helper when a projection or zero
            // result would otherwise skip one or both source bytes.
            if value_contains_pointer_cell(&left) || count < 8 {
                out.push(MirOp::Binary {
                    op: shift_op,
                    dst,
                    left,
                    right,
                    width: MirWidth::Word,
                    carry_in,
                    carry_out,
                });
                continue;
            }

            if count >= 16 {
                out.push(MirOp::Move {
                    dst,
                    src: MirValue::ConstU16(0),
                    width: MirWidth::Word,
                });
                lowered += 1;
                continue;
            }

            let Some((dst_lo, dst_hi)) = split_def(dst.clone()) else {
                out.push(MirOp::Binary {
                    op: shift_op,
                    dst,
                    left,
                    right,
                    width: MirWidth::Word,
                    carry_in,
                    carry_out,
                });
                continue;
            };
            let (left_lo, left_hi) =
                split_value_with_storage_widths(left, routine.id, layout, &temp_widths);
            let remaining = (count - 8) as u8;
            let (zero_dst, projected_dst, projected_src) = match shift_op {
                MirBinaryOp::Lsh => (dst_lo, dst_hi, left_lo),
                MirBinaryOp::Rsh => (dst_hi, dst_lo, left_hi),
                _ => unreachable!(),
            };
            if remaining == 0 {
                out.push(MirOp::Move {
                    dst: projected_dst,
                    src: projected_src,
                    width: MirWidth::Byte,
                });
            } else {
                out.push(MirOp::Binary {
                    op: shift_op,
                    dst: projected_dst,
                    left: projected_src,
                    right: MirValue::ConstU8(remaining),
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: MirCarryOut::Ignore,
                });
            }
            out.push(MirOp::Move {
                dst: zero_dst,
                src: MirValue::ConstU8(0),
                width: MirWidth::Byte,
            });
            lowered += 1;
        }
        block.ops = out;
    }

    peephole_stats.record_many(routine.id, "constant-word-shift-projection", lowered);
}

fn lower_small_constant_word_shifts(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) {
    let routine_id = routine.id;
    let temp_widths = collect_routine_temp_widths(routine);
    let mut fresh = FreshTemps::new(&routine.temps);
    let (temps, blocks) = (&mut routine.temps, &mut routine.blocks);
    let mut lowered = 0;

    for block in blocks {
        let mut out = Vec::with_capacity(block.ops.len());
        for op in std::mem::take(&mut block.ops) {
            let MirOp::Binary {
                op: shift_op @ (MirBinaryOp::Lsh | MirBinaryOp::Rsh),
                dst,
                left,
                right,
                width: MirWidth::Word,
                carry_in,
                carry_out,
            } = op
            else {
                out.push(op);
                continue;
            };
            let Some(count) = constant_value_u16(&right) else {
                out.push(MirOp::Binary {
                    op: shift_op,
                    dst,
                    left,
                    right,
                    width: MirWidth::Word,
                    carry_in,
                    carry_out,
                });
                continue;
            };
            if !(1..=MAX_INLINE_WORD_CONSTANT_SHIFT).contains(&count)
                || carry_in.is_some()
                || carry_out != MirCarryOut::Ignore
                || value_contains_pointer_cell(&left)
                || split_def(dst.clone()).is_none()
            {
                out.push(MirOp::Binary {
                    op: shift_op,
                    dst,
                    left,
                    right,
                    width: MirWidth::Word,
                    carry_in,
                    carry_out,
                });
                continue;
            }

            let (mut source_lo, mut source_hi) =
                split_value_with_storage_widths(left, routine_id, layout, &temp_widths);
            for step in 0..count {
                let stage_dst = if step + 1 == count {
                    dst.clone()
                } else {
                    MirDef::VTemp(fresh.fresh(temps))
                };
                let (stage_lo, stage_hi) =
                    split_def(stage_dst).expect("word shift destination was validated");
                match shift_op {
                    MirBinaryOp::Lsh => {
                        out.push(MirOp::Binary {
                            op: MirBinaryOp::Lsh,
                            dst: stage_lo.clone(),
                            left: source_lo,
                            right: MirValue::ConstU8(1),
                            width: MirWidth::Byte,
                            carry_in: None,
                            carry_out: MirCarryOut::Produce,
                        });
                        out.push(MirOp::Binary {
                            op: MirBinaryOp::Add,
                            dst: stage_hi.clone(),
                            left: source_hi.clone(),
                            right: source_hi,
                            width: MirWidth::Byte,
                            carry_in: Some(MirCarryIn::FromPrevious),
                            carry_out: MirCarryOut::Ignore,
                        });
                    }
                    MirBinaryOp::Rsh => {
                        out.push(MirOp::Binary {
                            op: MirBinaryOp::Rsh,
                            dst: stage_hi.clone(),
                            left: source_hi,
                            right: MirValue::ConstU8(1),
                            width: MirWidth::Byte,
                            carry_in: None,
                            carry_out: MirCarryOut::Produce,
                        });
                        out.push(MirOp::Binary {
                            op: MirBinaryOp::Rsh,
                            dst: stage_lo.clone(),
                            left: source_lo,
                            right: MirValue::ConstU8(1),
                            width: MirWidth::Byte,
                            carry_in: Some(MirCarryIn::FromPrevious),
                            carry_out: MirCarryOut::Ignore,
                        });
                    }
                    _ => unreachable!(),
                }
                source_lo = MirValue::Def(stage_lo);
                source_hi = MirValue::Def(stage_hi);
            }
            lowered += 1;
        }
        block.ops = out;
    }

    peephole_stats.record_many(routine.id, "small-constant-word-shift", lowered);
}

fn strength_reduce_byte_multiply(
    dst: MirDef,
    operand: MirValue,
    factor: u16,
) -> Option<Vec<MirOp>> {
    if factor == 0 || factor >= 0x100 {
        return Some(vec![MirOp::Move {
            dst,
            src: MirValue::ConstU8(0),
            width: MirWidth::Byte,
        }]);
    }
    if factor == 1 {
        return Some(vec![MirOp::Move {
            dst,
            src: operand,
            width: MirWidth::Byte,
        }]);
    }
    Some(vec![MirOp::Binary {
        op: MirBinaryOp::Lsh,
        dst,
        left: operand,
        right: MirValue::ConstU8(factor.trailing_zeros() as u8),
        width: MirWidth::Byte,
        carry_in: None,
        carry_out: MirCarryOut::Ignore,
    }])
}

#[allow(clippy::too_many_arguments)]
fn strength_reduce_word_multiply(
    dst: MirDef,
    operand: MirValue,
    factor: u16,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    fresh: &mut FreshTemps,
    temps: &mut Vec<MirTemp>,
) -> Option<Vec<MirOp>> {
    if factor == 0 {
        return Some(vec![MirOp::Move {
            dst,
            src: MirValue::ConstU16(0),
            width: MirWidth::Word,
        }]);
    }
    if factor == 1 {
        return Some(vec![MirOp::Move {
            dst,
            src: operand,
            width: MirWidth::Word,
        }]);
    }

    let (dst_lo, dst_hi) = split_def(dst.clone())?;
    let (mut source_lo, mut source_hi) =
        split_value_with_storage_widths(operand, routine_id, layout, temp_widths);
    let shift = factor.trailing_zeros() as u8;
    if shift >= 16 {
        return Some(vec![MirOp::Move {
            dst,
            src: MirValue::ConstU16(0),
            width: MirWidth::Word,
        }]);
    }

    if shift >= 8 {
        let mut out = Vec::with_capacity(2);
        let high_shift = shift - 8;
        if high_shift == 0 {
            out.push(MirOp::Move {
                dst: dst_hi,
                src: source_lo,
                width: MirWidth::Byte,
            });
        } else {
            out.push(MirOp::Binary {
                op: MirBinaryOp::Lsh,
                dst: dst_hi,
                left: source_lo,
                right: MirValue::ConstU8(high_shift),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            });
        }
        out.push(MirOp::Move {
            dst: dst_lo,
            src: MirValue::ConstU8(0),
            width: MirWidth::Byte,
        });
        return Some(out);
    }

    let mut out = Vec::with_capacity(usize::from(shift) * 2);
    for step in 0..shift {
        let stage_dst = if step + 1 == shift {
            dst.clone()
        } else {
            MirDef::VTemp(fresh.fresh(temps))
        };
        let (stage_lo, stage_hi) = split_def(stage_dst.clone())?;
        out.push(MirOp::Binary {
            op: MirBinaryOp::Lsh,
            dst: stage_lo,
            left: source_lo,
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Produce,
        });
        out.push(MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: stage_hi,
            left: source_hi.clone(),
            right: source_hi,
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::FromPrevious),
            carry_out: MirCarryOut::Ignore,
        });
        source_lo = MirValue::Def(match &stage_dst {
            MirDef::VTemp(id) => MirDef::VTempByte { id: *id, byte: 0 },
            _ => return None,
        });
        source_hi = MirValue::Def(match stage_dst {
            MirDef::VTemp(id) => MirDef::VTempByte { id, byte: 1 },
            _ => return None,
        });
    }
    Some(out)
}

/// A terminal `JMP (word-local)` fed by a compiler-known table containing only
/// parameterless Action routines is an indirect Action tail dispatch, not an
/// arbitrary inline instruction stream. The opcode consumes the two-byte
/// vector but no incoming register, flag, or compiler pointer scratch state.
/// Keep it as a full memory-write barrier for the dispatched routine while
/// exposing that narrower machine-input contract to MIR6502 liveness.
fn refine_terminal_indirect_jump_effects(program: &mut MirProgram) {
    let machine_blocks = program
        .machine_blocks
        .iter()
        .map(|block| (block.id, block.items.clone()))
        .collect::<BTreeMap<_, _>>();
    let parameterless_action_routines = program
        .routines
        .iter()
        .filter(|routine| {
            matches!(
                routine.abi,
                super::ir::MirRoutineAbi::Action
                    | super::ir::MirRoutineAbi::ProgramEntry
                    | super::ir::MirRoutineAbi::ProgramEntryObservable
                    | super::ir::MirRoutineAbi::ActionObservable
            ) && routine.frame.params.is_empty()
        })
        .map(|routine| routine.name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let action_routine_tables = program
        .routines
        .iter()
        .filter_map(|routine| {
            let [block] = routine.blocks.as_slice() else {
                return None;
            };
            let [MirOp::MachineBlock { id, .. }] = block.ops.as_slice() else {
                return None;
            };
            if !matches!(block.terminator, MirTerminator::Unreachable) {
                return None;
            }
            machine_blocks
                .get(id)
                .is_some_and(|items| {
                    machine_table_contains_only_action_routines(
                        items,
                        &parameterless_action_routines,
                    )
                })
                .then_some(routine.id)
        })
        .collect::<BTreeSet<_>>();

    for routine in &mut program.routines {
        for block_index in 0..routine.blocks.len() {
            let Some(local) = ({
                let block = &routine.blocks[block_index];
                if !matches!(block.terminator, MirTerminator::Unreachable) {
                    None
                } else {
                    let id = match block.ops.last() {
                        Some(MirOp::MachineBlock { id, .. }) => Some(*id),
                        _ => None,
                    };
                    id.and_then(|id| machine_blocks.get(&id))
                        .and_then(|items| terminal_indirect_jump_vector_name(items))
                        .and_then(|name| {
                            proven_action_dispatch_vector(
                                routine,
                                block,
                                name,
                                &action_routine_tables,
                            )
                        })
                }
            }) else {
                continue;
            };
            let Some(MirOp::MachineBlock { effects, .. }) =
                routine.blocks[block_index].ops.last_mut()
            else {
                unreachable!("dispatch proof requires terminal machine block")
            };

            *effects = MirEffects {
                memory_reads: MirMemoryEffect::Regions(vec![MirMemoryRegion {
                    kind: MirMemoryRegionKind::Local(local),
                    offset: 0,
                    size: 2,
                }]),
                // The tail-dispatched routine may mutate arbitrary program
                // memory, but it cannot observe MIR6502's transient pointer
                // pair merely by being entered through JMP.
                memory_writes: MirMemoryEffect::All,
                reads: Default::default(),
                clobbers: super::abi::action_call_clobbers(),
                preserves: Default::default(),
                stack_depth_delta: None,
                may_call_os: true,
                opaque: false,
            };
        }
    }
}

fn machine_table_contains_only_action_routines(
    items: &[MirMachineItem],
    parameterless_action_routines: &BTreeSet<String>,
) -> bool {
    !items.is_empty()
        && items.len() % 2 == 0
        && items.chunks_exact(2).all(|pair| {
            let name = match pair {
                [
                    MirMachineItem::AddressExpr {
                        selector: Some(super::ir::MirMachineByteSelector::Low),
                        atom: MirMachineAtom::Name(low),
                        offset: 0,
                        ..
                    },
                    MirMachineItem::AddressExpr {
                        selector: Some(super::ir::MirMachineByteSelector::High),
                        atom: MirMachineAtom::Name(high),
                        offset: 0,
                        ..
                    },
                ] if low.eq_ignore_ascii_case(high) => Some(low),
                [
                    MirMachineItem::AddressByte {
                        high: false,
                        name: low,
                    },
                    MirMachineItem::AddressByte {
                        high: true,
                        name: high,
                    },
                ] if low.eq_ignore_ascii_case(high) => Some(low),
                _ => None,
            };
            name.is_some_and(|name| {
                parameterless_action_routines.contains(&name.to_ascii_lowercase())
            })
        })
}

fn proven_action_dispatch_vector(
    routine: &super::ir::MirRoutine,
    block: &super::ir::MirBlock,
    vector_name: &str,
    action_routine_tables: &BTreeSet<RoutineId>,
) -> Option<crate::nir::LocalId> {
    let machine_index = block.ops.len().checked_sub(1)?;
    let store_index = machine_index.checked_sub(1)?;
    let MirOp::Store {
        dst:
            MirAddr::Direct(MirMem::Local {
                id: vector,
                offset: 0,
            }),
        src: MirValue::Def(MirDef::VTemp(value)),
        width: MirWidth::Word,
    } = &block.ops[store_index]
    else {
        return None;
    };
    let vector_slot = routine.frame.locals.iter().find(|slot| {
        slot.base == MirStorageBase::Local(*vector)
            && slot.scalar_width == Some(MirWidth::Word)
            && slot.offset == 0
            && slot
                .name
                .as_deref()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(vector_name))
    })?;
    if vector_slot.storage != super::ir::MirStorageClass::Scalar {
        return None;
    }

    let mut definitions = block.ops[..store_index].iter().filter_map(|op| {
        let MirOp::Load {
            dst: MirDef::VTemp(candidate),
            src:
                MirAddr::PointerIndex {
                    ptr:
                        MirMem::Local {
                            id: table,
                            offset: 0,
                        },
                    elem_size: 2,
                    offset: 0,
                    ..
                },
            width: MirWidth::Word,
        } = op
        else {
            return None;
        };
        (candidate == value).then_some(*table)
    });
    let table = definitions.next()?;
    if definitions.next().is_some() {
        return None;
    }
    let table_slot = routine.frame.locals.iter().find(|slot| {
        slot.base == MirStorageBase::Local(table)
            && slot.offset == 0
            && matches!(
                slot.init,
                Some(MirStorageInit::RoutineAddress { routine, .. })
                    if action_routine_tables.contains(&routine)
            )
    })?;
    matches!(table_slot.storage, super::ir::MirStorageClass::Array).then_some(*vector)
}

fn terminal_indirect_jump_vector_name(items: &[MirMachineItem]) -> Option<&str> {
    match items {
        [MirMachineItem::Byte(0x6C), MirMachineItem::Name(name)] => Some(name),
        [
            MirMachineItem::Byte(0x6C),
            MirMachineItem::AddressExpr {
                selector: None,
                atom: MirMachineAtom::Name(name),
                offset: 0,
                ..
            },
        ] => Some(name),
        _ => None,
    }
}

fn run_cfg_group(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
) -> Result<(), Vec<MirDiagnostic>> {
    cleanup_pre_materialization_temp_artifacts(routine, layout);
    lower_block_arguments(routine).map_err(|diagnostic| vec![diagnostic])?;
    verify_cfg_after_transform(routine, "CFG normalization")
}

fn verify_materialization_stage(
    program: &MirProgram,
    phase: MirPhase,
    stage: &str,
) -> Result<(), Vec<MirDiagnostic>> {
    super::verify::verify_program(program, phase).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|mut diagnostic| {
                diagnostic.message = format!("{stage}: {}", diagnostic.message);
                diagnostic
            })
            .collect()
    })
}

fn run_prehome_canonicalization_group(
    routine: &mut super::ir::MirRoutine,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    if config.enable_peepholes {
        let loops = dynamic_loops::select_dynamic_word_index_loops(routine);
        peephole_stats.record_many(routine.id, "dynamic-word-loop-candidate", loops.candidates);
        peephole_stats.record_many(routine.id, "dynamic-word-loop-rotated", loops.selected);
        peephole_stats.record_many(routine.id, "indexed-loop-cursor-selected", loops.selected);
        peephole_stats.record_many(
            routine.id,
            "dynamic-word-loop-blocked-final-index",
            loops.blocked_final_index,
        );
        peephole_stats.record_many(
            routine.id,
            "dynamic-word-loop-blocked-bound-invariance",
            loops.blocked_bound_invariance,
        );
        peephole_stats.record_many(
            routine.id,
            "indexed-loop-cursor-blocked-index-use",
            loops.blocked_index_use,
        );
        peephole_stats.record_many(
            routine.id,
            "indexed-loop-cursor-blocked-alias",
            loops.blocked_alias,
        );
        peephole_stats.record_many(
            routine.id,
            "dynamic-word-loop-blocked-shape",
            loops.blocked_shape,
        );
        verify_cfg_after_transform(routine, "dynamic word-index loop selection")?;
    }
    if config.enable_small_loop_unrolling {
        let loops = small_loops::unroll_small_counted_loops(routine, 8);
        peephole_stats.record_many(
            routine.id,
            "small-counted-loop-unroll-candidate",
            loops.candidates,
        );
        peephole_stats.record_many(routine.id, "small-counted-loop-unrolled", loops.selected);
        peephole_stats.record_many(
            routine.id,
            "small-counted-loop-unroll-blocked-growth",
            loops.blocked_growth,
        );
        peephole_stats.record_many(
            routine.id,
            "small-counted-loop-unroll-blocked-effects",
            loops.blocked_effects,
        );
        peephole_stats.record_many(
            routine.id,
            "small-counted-loop-unroll-blocked-observable-induction",
            loops.blocked_observable_induction,
        );
        verify_cfg_after_transform(routine, "small counted-loop unrolling")?;
    }
    run_analyzed_compare_producer_rewrites(routine, peephole_stats)?;
    run_analyzed_inclusive_compare_reversals(routine, layout, peephole_stats)?;
    run_analyzed_compare_narrowing(routine, peephole_stats)?;
    run_analyzed_byte_binary_compare_consumers(routine, peephole_stats)?;
    run_analyzed_dual_indirect_compares(routine, layout, peephole_stats)?;
    let signed_word_zero_sites = super::rewrite::pilots::proven_signed_word_zero_compare_branches(
        routine,
    )
    .map_err(|_| {
        vec![MirDiagnostic::routine(
            &routine.name,
            "signed word zero compare analysis failed",
        )]
    })?;
    let signed_word_zero_compares = expand_proven_signed_word_zero_compare_branches(
        &mut routine.blocks,
        &signed_word_zero_sites,
    );
    peephole_stats.record_many(
        routine.id,
        "signed-return-word-zero-compare-branch",
        signed_word_zero_compares,
    );
    let direct_word_equality_sites =
        super::rewrite::pilots::proven_direct_word_equality_compare_branches(routine).map_err(
            |_| {
                vec![MirDiagnostic::routine(
                    &routine.name,
                    "direct word equality compare analysis failed",
                )]
            },
        )?;
    let direct_word_equality_compares = expand_proven_direct_word_equality_compare_branches(
        &mut routine.blocks,
        &direct_word_equality_sites,
    );
    peephole_stats.record_many(
        routine.id,
        "word-load-equality-compare-branch",
        direct_word_equality_compares,
    );
    let direct_word_relational_sites =
        super::rewrite::pilots::proven_direct_word_relational_compare_branches(routine).map_err(
            |_| {
                vec![MirDiagnostic::routine(
                    &routine.name,
                    "direct word relational compare analysis failed",
                )]
            },
        )?;
    let direct_word_relational_compares = expand_proven_direct_word_relational_compare_branches(
        &mut routine.blocks,
        &direct_word_relational_sites,
    );
    peephole_stats.record_many(
        routine.id,
        "word-load-relational-compare-branch",
        direct_word_relational_compares,
    );
    let word_arithmetic_compare_sites =
        super::rewrite::pilots::proven_word_arithmetic_compare_branches(routine).map_err(|_| {
            vec![MirDiagnostic::routine(
                &routine.name,
                "word-arithmetic compare analysis failed",
            )]
        })?;
    let word_arithmetic_compares = expand_proven_word_arithmetic_compare_branches(
        &mut routine.blocks,
        &word_arithmetic_compare_sites,
    );
    peephole_stats.record_many(
        routine.id,
        "word-arithmetic-compare-branch",
        word_arithmetic_compares,
    );
    let byte_add_word_compare_sites =
        super::rewrite::pilots::proven_byte_add_word_compare_branches(routine).map_err(|_| {
            vec![MirDiagnostic::routine(
                &routine.name,
                "byte-add word compare analysis failed",
            )]
        })?;
    let byte_add_word_compares = expand_proven_byte_add_word_compare_branches(
        &mut routine.blocks,
        &byte_add_word_compare_sites,
    );
    peephole_stats.record_many(
        routine.id,
        "byte-add-word-compare-branch",
        byte_add_word_compares,
    );
    expand_compare_branch_consumers(&mut routine.blocks, layout, config);
    verify_cfg_after_transform(routine, "compare/branch expansion")?;
    collapse_empty_jump_blocks(routine);
    verify_cfg_after_transform(routine, "empty-jump collapse")
}

fn run_prehome_selection_group(
    routine: &mut super::ir::MirRoutine,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    run_analyzed_word_carry_chain_store_consumers(routine, config, layout, peephole_stats)?;
    run_analyzed_private_indirect_word_copies(routine, layout, peephole_stats)?;
    run_analyzed_indexed_to_indirect_word_copies(routine, layout, peephole_stats)?;
    run_analyzed_indirect_to_indexed_word_copies(routine, layout, peephole_stats)?;
    run_analyzed_pointer_rewrites(routine, layout, peephole_stats)?;
    // Canonicalize address expressions before store-consumer selection can
    // absorb them into a wider arithmetic transaction. A later index pass
    // still handles computed addresses exposed by those rewrites.
    run_analyzed_affine_static_byte_indexes(routine, peephole_stats)?;
    run_analyzed_call_arg_producers(routine, peephole_stats)?;
    run_analyzed_return_slot_call_arg_forwards(routine, peephole_stats)?;
    run_analyzed_param_home_consumers(routine, peephole_stats)?;
    for block in &mut routine.blocks {
        block.ops = normalize_byte_add_sub_carry(std::mem::take(&mut block.ops));
    }
    run_analyzed_call_arg_exprs(routine, config, layout, helpers, peephole_stats)?;
    run_analyzed_stored_call_result_aliases(routine, peephole_stats)?;
    run_analyzed_call_result_store_consumers(routine, peephole_stats)?;
    run_analyzed_store_consumers(routine, config, layout, helpers, peephole_stats)?;
    run_analyzed_unused_lea_addrs(routine, peephole_stats)?;
    let word_load_address_forwards = forward_unique_word_load_address_consumers(routine, layout);
    peephole_stats.record_many(
        routine.id,
        "word-load-address-consumer-forwards",
        word_load_address_forwards,
    );
    run_analyzed_index_rewrites(routine, layout, peephole_stats)
}

fn run_analyzed_indexed_to_indirect_word_copies(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| {
                super::rewrite::pilots::discover_indexed_to_indirect_word_copies(
                    routine, context, layout,
                )
            },
            super::rewrite::pilots::index_rewrite_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("indexed-to-indirect word-copy selection failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_private_indirect_word_copies(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| {
                super::rewrite::pilots::discover_private_indirect_word_copies(
                    routine, context, layout,
                )
            },
            super::rewrite::pilots::index_rewrite_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("private indirect word-copy selection failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_indirect_to_indexed_word_copies(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| {
                super::rewrite::pilots::discover_indirect_to_indexed_word_copies(
                    routine, context, layout,
                )
            },
            super::rewrite::pilots::index_rewrite_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("indirect-to-indexed word-copy selection failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_posthome_structural_group(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    config: &Mir6502Config,
    known_callees: Option<&MirKnownCalleeSummaries>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    run_posthome_signed_word_relations(routine, layout, known_callees, peephole_stats)?;
    if config.enable_peepholes {
        let selected = small_loops::select_high_bit_shift_xor_diamonds(routine, layout);
        peephole_stats.record_many(
            routine.id,
            "high-bit-shift-xor-carry-branch-selected",
            selected,
        );
        verify_cfg_after_transform(routine, "high-bit shift/XOR carry selection")?;
    }
    run_analyzed_param_home_reloads(routine, peephole_stats)?;
    run_analyzed_spill_forwards(routine, peephole_stats)?;
    run_analyzed_direct_inc_dec_updates(routine, layout, peephole_stats)?;
    run_analyzed_staged_word_forwards(
        routine,
        layout,
        config.enable_direct_byte_word_update,
        peephole_stats,
    )?;
    run_analyzed_indirect_constant_stores(routine, layout, peephole_stats)?;
    run_analyzed_word_array_value_staging(routine, layout, peephole_stats)?;
    run_analyzed_word_rsh8_high_projections(routine, layout, peephole_stats)?;
    run_analyzed_indirect_stores_and_compounds(routine, layout, peephole_stats)?;
    run_analyzed_helper_indexed_store_placements(routine, layout, peephole_stats)?;
    for block in &mut routine.blocks {
        let ops = std::mem::take(&mut block.ops);
        block.ops =
            fold_structural_before_cleanup_migrations(ops, routine.id, layout, peephole_stats);
    }
    run_analyzed_static_indexed_byte_stores(routine, layout, peephole_stats)?;
    run_analyzed_direct_indexed_byte_binaries(routine, layout, peephole_stats)?;
    run_analyzed_rhs_and_adjacent_reloads(routine, layout, peephole_stats)?;
    run_analyzed_known_callee_word_result_placements(
        routine,
        layout,
        known_callees,
        peephole_stats,
    )?;
    run_analyzed_known_callee_preserved_index_param_carriers(
        routine,
        layout,
        known_callees,
        peephole_stats,
    )?;
    run_analyzed_ssa_lite_byte_rewrites(routine, layout, false, known_callees, peephole_stats)?;
    run_analyzed_dead_private_scratch_stores(routine, peephole_stats)?;
    run_analyzed_txa_direct_store_folds(routine, peephole_stats)?;
    run_analyzed_dead_register_writes(routine, layout, known_callees, peephole_stats)?;
    for block in &mut routine.blocks {
        let ops = std::mem::take(&mut block.ops);
        block.ops = fold_structural_machine_tail(ops, routine.id, layout, peephole_stats);
    }
    run_analyzed_indexed_base_pointer_staging(routine, peephole_stats)?;
    run_analyzed_scaled_y_word_reads(routine, layout, peephole_stats)?;
    run_analyzed_scaled_y_word_stores(routine, layout, peephole_stats)
}

fn run_analyzed_static_indexed_byte_stores(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            indexes::discover_static_indexed_byte_stores(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home static indexed byte store recovery failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_posthome_signed_word_relations(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    known_callees: Option<&MirKnownCalleeSummaries>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let (direct_relations, overflow_corrections) =
        fold_posthome_signed_word_relations(routine, layout, known_callees).map_err(|_| {
            vec![MirDiagnostic::routine(
                &routine.name,
                "post-home signed word relation rewrite failed",
            )]
        })?;
    peephole_stats.record_many(
        routine.id,
        "posthome-signed-word-direct-compare",
        direct_relations,
    );
    peephole_stats.record_many(
        routine.id,
        "posthome-signed-overflow-correction",
        overflow_corrections,
    );
    Ok(())
}

fn run_analyzed_direct_indexed_byte_binaries(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            indexes::discover_direct_indexed_byte_binaries(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home direct indexed byte binary selection failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_posthome_cleanup_group(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    terminator_config: Option<&Mir6502Config>,
    known_callees: Option<&MirKnownCalleeSummaries>,
    mut home_fates: Option<&mut HomeFateTracker>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    if let Some(config) = terminator_config {
        for block in &mut routine.blocks {
            block.terminator =
                materialize_terminator(block.id, &block.terminator, &block.ops, config);
            materialize_fused_compare_dest(block.id, &block.terminator, &mut block.ops);
        }
    }
    run_analyzed_redundant_indexed_address_materializations(
        routine,
        layout,
        known_callees,
        peephole_stats,
    )?;
    run_analyzed_ssa_lite_byte_rewrites(routine, layout, true, known_callees, peephole_stats)?;
    run_analyzed_call_result_y_placements(routine, peephole_stats)?;
    run_analyzed_dead_register_writes(routine, layout, known_callees, peephole_stats)?;
    remove_dead_spill_stores(routine);
    let remap = color_basic_block_spills(routine);
    if let Some(tracker) = home_fates.as_deref_mut() {
        tracker.apply_spill_remap(&remap);
    }
    let routine_remap = color_routine_spills(routine);
    peephole_stats.record_many(
        routine.id,
        "routine-spill-color-remaps",
        routine_remap.len(),
    );
    if let Some(tracker) = home_fates {
        tracker.apply_spill_remap(&routine_remap);
    }
    prune_unused_spills(routine);
    reserve_used_fixed_zero_page_slots(routine);
    Ok(())
}

fn run_analyzed_redundant_indexed_address_materializations(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    known_callees: Option<&MirKnownCalleeSummaries>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let discover =
        |routine: &super::ir::MirRoutine,
         context: &crate::mir6502::rewrite::context::PostHomeRewriteContext<'_, '_>| {
            ssa_lite::discover_redundant_indexed_address_materializations(routine, context, layout)
        };
    let result = match known_callees {
        Some(known_callees) => {
            driver.run_fixed_point_with_known_callees(routine, known_callees, discover)
        }
        None => driver.run_fixed_point(routine, discover),
    }
    .map_err(|error| {
        vec![MirDiagnostic::routine(
            &routine.name,
            format!("post-home indexed-address reuse failed: {error:?}"),
        )]
    })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
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

fn run_analyzed_spill_forwards(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, spills::discover_spill_forwards)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home spill forwarding failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_param_home_reloads(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, calls::discover_param_home_reloads)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home parameter reload forwarding failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_direct_inc_dec_updates(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            store_consumers::discover_direct_inc_dec_updates(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home direct inc/dec rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_dead_register_writes(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    known_callees: Option<&MirKnownCalleeSummaries>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let discover =
        |routine: &super::ir::MirRoutine,
         context: &crate::mir6502::rewrite::context::PostHomeRewriteContext<'_, '_>| {
            peepholes::discover_dead_register_writes(routine, context, layout)
        };
    let result = match known_callees {
        Some(known_callees) => {
            driver.run_fixed_point_with_known_callees(routine, known_callees, discover)
        }
        None => driver.run_fixed_point(routine, discover),
    }
    .map_err(|error| {
        vec![MirDiagnostic::routine(
            &routine.name,
            format!("post-home dead register-write rewrite failed: {error:?}"),
        )]
    })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_txa_direct_store_folds(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, peepholes::discover_txa_direct_store_folds)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home TXA/direct-store rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_call_result_y_placements(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, peepholes::discover_call_result_y_placements)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home call-result Y placement failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_known_callee_word_result_placements(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    known_callees: Option<&MirKnownCalleeSummaries>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let Some(known_callees) = known_callees else {
        return Ok(());
    };
    let mut driver = MirPostHomeRewriteDriver::default();
    let preserve_exit_accumulator = known_callees.accumulator_summary_is_observable(routine.id);
    let result = driver
        .run_fixed_point_with_known_callees(routine, known_callees, |routine, context| {
            peepholes::discover_known_callee_word_result_placements(
                routine,
                context,
                layout,
                preserve_exit_accumulator,
            )
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home known-callee word-result placement failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_known_callee_preserved_index_param_carriers(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    known_callees: Option<&MirKnownCalleeSummaries>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let Some(known_callees) = known_callees else {
        return Ok(());
    };
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_with_known_callees(routine, known_callees, |routine, context| {
            peepholes::discover_known_callee_preserved_index_param_carriers(
                routine,
                context,
                known_callees,
                layout,
            )
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home known-callee index-carrier rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_ssa_lite_byte_rewrites(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    allow_cross_block: bool,
    known_callees: Option<&MirKnownCalleeSummaries>,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let discover =
        |routine: &super::ir::MirRoutine,
         context: &crate::mir6502::rewrite::context::PostHomeRewriteContext<'_, '_>| {
            ssa_lite::discover_ssa_lite_byte_rewrites(routine, context, layout, allow_cross_block)
        };
    let result = match known_callees {
        Some(known_callees) => {
            driver.run_fixed_point_with_known_callees(routine, known_callees, discover)
        }
        None => driver.run_fixed_point(routine, discover),
    }
    .map_err(|error| {
        vec![MirDiagnostic::routine(
            &routine.name,
            format!("post-home SSA-lite rewrite failed: {error:?}"),
        )]
    })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_rhs_and_adjacent_reloads(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            peepholes::discover_rhs_and_adjacent_reloads(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home staged RHS/reload rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_dead_private_scratch_stores(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, peepholes::discover_dead_private_scratch_stores)
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home dead scratch-store rewrite failed: {error:?}"),
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

fn run_analyzed_word_rsh8_high_projections(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            peepholes::discover_word_rsh8_high_projections(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home word-high projection rewrite failed: {error:?}"),
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

fn run_analyzed_helper_indexed_store_placements(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            indexes::discover_helper_indexed_store_placements(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home helper indexed-store placement failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_scaled_y_word_reads(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            indexes::discover_scaled_y_word_reads(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home scaled-Y word-read selection failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_scaled_y_word_stores(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPostHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            indexes::discover_scaled_y_word_stores(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("post-home scaled-Y word-store selection failed: {error:?}"),
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

fn run_analyzed_inclusive_compare_reversals(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| discover_inclusive_compare_reversals(routine, context, layout),
            |routine| inclusive_compare_reversal_rank(routine, layout),
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("inclusive compare reversal failed: {error:?}"),
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

fn run_analyzed_dual_indirect_compares(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point(routine, |routine, context| {
            discover_dual_indirect_compares_with_layout(routine, context, layout)
        })
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("dual indirect compare selection failed: {error:?}"),
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

fn run_analyzed_affine_static_byte_indexes(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            discover_affine_static_byte_indexes,
            super::rewrite::pilots::index_rewrite_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("affine static byte-index selection failed: {error:?}"),
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

fn run_analyzed_param_home_consumers(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            calls::discover_param_home_consumers,
            calls::param_home_consumer_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("parameter-home consumer rewrite failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
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

fn run_analyzed_stored_call_result_aliases(
    routine: &mut super::ir::MirRoutine,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            super::rewrite::pilots::discover_stored_call_result_aliases,
            super::rewrite::pilots::stored_call_result_alias_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("stored call-result alias forwarding failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_word_carry_chain_store_consumers(
    routine: &mut super::ir::MirRoutine,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| {
                super::rewrite::pilots::discover_word_carry_chain_store_consumers(
                    routine, context, config, layout,
                )
            },
            super::rewrite::pilots::store_consumer_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("word carry-chain store selection failed: {error:?}"),
            )]
        })?;
    record_prehome_rewrite_result(routine.id, result, peephole_stats);
    Ok(())
}

fn run_analyzed_widened_byte_shift_store_consumers(
    routine: &mut super::ir::MirRoutine,
    layout: &MaterializeLayout,
    peephole_stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut driver = MirPreHomeRewriteDriver::default();
    let result = driver
        .run_fixed_point_by_key(
            routine,
            |routine, context| {
                super::rewrite::pilots::discover_widened_byte_shift_store_consumers(
                    routine, context, layout,
                )
            },
            super::rewrite::pilots::store_consumer_rank,
        )
        .map_err(|error| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("widened-byte shift store selection failed: {error:?}"),
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
    for site in &result.blocked_sites {
        peephole_stats.record_site(
            routine_id,
            "analyzed-rewrite-blocked",
            format!(
                "stat={} block=b{} op=#{} reason={}",
                site.stat, site.block.0, site.op_index, site.reason
            ),
        );
    }
    for (reason, count) in &result.blocked_by_reason {
        peephole_stats.record_many_dynamic(
            routine_id,
            format!("analyzed-rewrite-blocked-{reason}"),
            *count,
        );
    }
    for (stat, count) in &result.blocked_by_stat {
        peephole_stats.record_many_dynamic(
            routine_id,
            format!("analyzed-rewrite-blocked-stat-{stat}"),
            *count,
        );
    }
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
    peephole_stats.record_many(routine_id, "analyzed-rewrite-blocked", result.blocked);
    peephole_stats.record_many(
        routine_id,
        "analyzed-rewrite-estimated-bytes-saved",
        result.estimated_bytes_saved,
    );
    peephole_stats.record_many(
        routine_id,
        "analyzed-rewrite-estimated-cycles-saved",
        result.estimated_cycles_saved,
    );
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

fn materialize_packed_real_addr(
    addr: MirAddr,
    consumer: MirAddressConsumer,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) -> (MirAddr, u16) {
    let fixed_indirect = || match consumer.pointer_pair() {
        MirPointerPair::Fixed { lo } => MirAddr::FixedIndirectIndexedY { zp: lo },
        MirPointerPair::Virtual(zp) => MirAddr::IndirectIndexedY { zp },
    };
    match addr {
        MirAddr::Direct(mem) => (MirAddr::Direct(mem), 0),
        MirAddr::Deref { ptr, offset } => {
            let (lo, hi) = split_value_with_temp_widths(ptr, layout, temp_widths);
            out.push(MirOp::MaterializeAddress {
                consumer,
                value: MirValue::Word {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                },
            });
            (fixed_indirect(), offset)
        }
        MirAddr::PointerCell { ptr, offset } => {
            let (lo, hi) =
                split_value_with_temp_widths(pointer_value_from_mem(&ptr), layout, temp_widths);
            out.push(MirOp::MaterializeAddress {
                consumer,
                value: MirValue::Word {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                },
            });
            (fixed_indirect(), offset)
        }
        addr @ (MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. }) => {
            let parts = indexed_addr_parts(&addr).expect("packed REAL indexed address");
            let offset = parts.offset;
            materialize_indexed_address_for_consumer(parts, consumer, layout, None, out);
            (fixed_indirect(), offset)
        }
        other => (other, 0),
    }
}

fn materialize_ops_impl(
    routine_id: RoutineId,
    _block_id: MirBlockId,
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    peephole_stats: &mut MirPeepholeStats,
    legacy_test_peepholes: bool,
    routine_temp_widths: Option<&BTreeMap<MirTempId, MirWidth>>,
) -> Vec<MirOp> {
    #[cfg(not(test))]
    let _ = legacy_test_peepholes;
    #[cfg(test)]
    let ops = if legacy_test_peepholes {
        rematerialize_direct_pointer_temp_derefs(ops)
    } else {
        ops
    };
    #[cfg(test)]
    let ops = if legacy_test_peepholes {
        fold_call_arg_producers(ops)
    } else {
        ops
    };
    #[cfg(test)]
    let (ops, call_result_forwards) = if legacy_test_peepholes {
        forward_return_slot_call_result_args(ops, terminator)
    } else {
        (ops, calls::ReturnSlotCallArgForwardStats::default())
    };
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
    let ops = if legacy_test_peepholes {
        normalize_byte_add_sub_carry(ops)
    } else {
        ops
    };
    let mut out = Vec::new();
    let mut temp_widths = collect_temp_widths(&ops);
    if let Some(routine_temp_widths) = routine_temp_widths {
        for (id, width) in routine_temp_widths {
            temp_widths.entry(*id).or_insert(*width);
        }
    }
    refine_temp_widths_from_storage_loads(&ops, routine_id, layout, &mut temp_widths);
    #[cfg(test)]
    let delayed_byte_indexes = if legacy_test_peepholes {
        collect_delayed_byte_index_plan(&ops)
    } else {
        indexes::DelayedByteIndexPlan::empty()
    };
    #[cfg(not(test))]
    let delayed_byte_indexes = indexes::DelayedByteIndexPlan::empty();
    let mut index = 0;
    while index < ops.len() {
        #[cfg(test)]
        if legacy_test_peepholes && delayed_byte_indexes.producer_ops().contains(&index) {
            peephole_stats.record(routine_id, "delayed-byte-index-producer");
            index += 1;
            continue;
        }

        #[cfg(test)]
        if legacy_test_peepholes {
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
                peephole_stats.record_many(
                    routine_id,
                    "word-arithmetic-direct-action-call-arg",
                    call_arg_expr.direct_word_arithmetic,
                );
                peephole_stats.record_many(
                    routine_id,
                    "indexed-byte-direct-fixed-action-call-arg",
                    call_arg_expr.direct_indexed_byte_fixed_args,
                );
                peephole_stats.record_many(
                    routine_id,
                    "direct-binary-rhs-call-arg-candidates",
                    call_arg_expr.direct_binary_rhs_candidates,
                );
                peephole_stats.record_many(
                    routine_id,
                    "direct-binary-rhs-call-arg",
                    call_arg_expr.direct_binary_rhs_selected,
                );
                peephole_stats.record_many(
                    routine_id,
                    "direct-binary-rhs-call-arg-blocked-overlap",
                    call_arg_expr.direct_binary_rhs_blocked_overlap,
                );
                peephole_stats.record_many(
                    routine_id,
                    "direct-binary-rhs-call-arg-blocked-nonordinary",
                    call_arg_expr.direct_binary_rhs_blocked_nonordinary,
                );
                index += call_arg_expr.consumed;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused = try_fuse_cast_store_consumer(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "cast-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused =
                try_fuse_address_store_consumer(&ops, index, routine_id, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "address-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused =
                try_fuse_indexed_byte_copy(&ops, index, layout, &delayed_byte_indexes, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "indexed-byte-copy");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused = try_fuse_indexed_word_copy(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "indexed-word-copy");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused = try_fuse_indexed_to_indirect_word_copy(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "indexed-to-indirect-word-copy");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused = try_fuse_indirect_to_indexed_word_copy(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "indirect-to-indexed-word-copy");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused = try_fuse_dynamic_inline_byte_index(&ops, index, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "dynamic-inline-byte-index");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused = try_prepare_dynamic_byte_index(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "prepare-dynamic-byte-index");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
            let maybe_fused =
                try_prepare_dynamic_word_index(&ops, index, routine_id, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "prepare-dynamic-word-index");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes
            && let Some(stat) = byte_binary_compare_consumer_observation(&ops, index, terminator)
        {
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
        if legacy_test_peepholes {
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
        if legacy_test_peepholes {
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

            let maybe_fused = select_word_helper_store_consumer(
                &ops,
                index,
                config,
                layout,
                &temp_widths,
                true,
                helpers,
                &mut out,
            );
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "word-helper-store-consumer");
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
        if legacy_test_peepholes {
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
        if legacy_test_peepholes {
            let maybe_fused = try_fuse_direct_copy_store_consumer(&ops, index, layout, &mut out);
            if maybe_fused > 0 {
                peephole_stats.record(routine_id, "direct-copy-store-consumer");
                index += maybe_fused;
                continue;
            }
        }

        #[cfg(test)]
        if legacy_test_peepholes {
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
        if legacy_test_peepholes {
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
        if legacy_test_peepholes {
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
            MirOp::PackedRealCopy {
                source,
                destination,
                source_offset,
                destination_offset,
                negate,
            } => {
                let (source, address_source_offset) = materialize_packed_real_addr(
                    source,
                    DEFAULT_POINTER_PAIR,
                    layout,
                    &temp_widths,
                    &mut out,
                );
                let (destination, address_destination_offset) = materialize_packed_real_addr(
                    destination,
                    DEST_POINTER_PAIR,
                    layout,
                    &temp_widths,
                    &mut out,
                );
                out.push(MirOp::PackedRealCopy {
                    source,
                    destination,
                    source_offset: source_offset.saturating_add(address_source_offset),
                    destination_offset: destination_offset
                        .saturating_add(address_destination_offset),
                    negate,
                });
            }
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
                let mut parts = indexed_addr_parts(&src).expect("indexed load matched above");
                parts.base = indexes::resolve_indexed_base_producer(&ops, index, parts.base);
                let (parts, narrowed_byte_index) =
                    if delayed_byte_indexes.expr_for_value(&parts.index).is_some() {
                        (parts, false)
                    } else {
                        indexes::narrow_known_byte_index(parts, &temp_widths)
                    };
                if narrowed_byte_index {
                    peephole_stats.record(routine_id, "typed-byte-index");
                }
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
                let mut parts = indexed_addr_parts(&dst).expect("indexed store matched above");
                parts.base = indexes::resolve_indexed_base_producer(&ops, index, parts.base);
                let (parts, narrowed_byte_index) =
                    if delayed_byte_indexes.expr_for_value(&parts.index).is_some() {
                        (parts, false)
                    } else {
                        indexes::narrow_known_byte_index(parts, &temp_widths)
                    };
                if narrowed_byte_index {
                    peephole_stats.record(routine_id, "typed-byte-index");
                }
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
            } if config.select_runtime_helpers
                && helper_for_typed_binary(
                    op,
                    width,
                    &left,
                    &right,
                    &temp_widths,
                    config.select_widening_byte_multiply,
                )
                .is_some() =>
            {
                let selection = helper_for_typed_binary(
                    op,
                    width,
                    &left,
                    &right,
                    &temp_widths,
                    config.select_widening_byte_multiply,
                )
                .expect("helper selection exists");
                let helper = selection.helper;
                let result_width = if helper == MirRuntimeHelper::MulByte {
                    peephole_stats.record(routine_id, "widening-byte-multiply-selected");
                    selection.result_width
                } else {
                    runtime_helper_result_width(&helper, width, &dst)
                };
                helpers.push(helper.clone());
                materialize_runtime_helper_binary(
                    helper,
                    Some(dst),
                    left,
                    right,
                    selection.operand_width,
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

/// Test-only adapter for the legacy shape-oracle unit tests. Compiler
/// workflows call `materialize_ops_impl` with this path disabled.
#[cfg(test)]
fn materialize_ops(
    routine_id: RoutineId,
    block_id: MirBlockId,
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    config: &Mir6502Config,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
    peephole_stats: &mut MirPeepholeStats,
) -> Vec<MirOp> {
    materialize_ops_impl(
        routine_id,
        block_id,
        ops,
        terminator,
        config,
        layout,
        helpers,
        peephole_stats,
        true,
        None,
    )
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
