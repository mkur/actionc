#![allow(dead_code)] // Production matcher migration begins in the next slice.

use std::collections::{BTreeMap, BTreeSet};

use crate::mir6502::analysis::effects::{
    MirFlagSet, MirHomeByte, MirMemoryRange, MirTempAccess, classify_op,
};
use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::analysis::use_def::{MirDefSite, MirTempLane};
use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirBlockId, MirFixedZpSlot, MirMem, MirOp, MirPointerPair, MirReg,
    MirRegisterSet, MirRoutine, MirValue, MirWidth,
};
use crate::mir6502::rewrite::context::{
    MirBlockedRewriteSite, MirProof, PostHomeRewriteContext, PreHomeRewriteContext,
};
use crate::mir6502::rewrite::plan::{
    MirEffectDelta, MirFactClass, MirPostHomeRewritePlan, MirRewritePlan,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirRewriteError {
    InvalidCfg,
    StalePlan {
        expected: MirRoutineGeneration,
        actual: MirRoutineGeneration,
    },
    UnknownBlock(MirBlockId),
    InvalidRange {
        block: MirBlockId,
        start: usize,
        end: usize,
        op_count: usize,
    },
    InvalidDeclaration {
        stat: &'static str,
        message: String,
    },
    DidNotConverge {
        max_rounds: usize,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirRewriteRunResult {
    pub rounds: usize,
    pub analysis_builds: usize,
    pub candidates: usize,
    pub applied: usize,
    pub overlap_rejections: usize,
    pub applied_by_stat: BTreeMap<&'static str, usize>,
    pub blocked: usize,
    pub blocked_by_reason: BTreeMap<&'static str, usize>,
    pub blocked_by_stat: BTreeMap<&'static str, usize>,
    pub blocked_sites: Vec<MirBlockedRewriteSite>,
    pub estimated_bytes_saved: usize,
    pub estimated_cycles_saved: usize,
    pub converged: bool,
}

impl MirRewriteRunResult {
    fn record_blocked_site(&mut self, site: MirBlockedRewriteSite) {
        if self.blocked_sites.contains(&site) {
            return;
        }
        self.blocked += 1;
        *self.blocked_by_reason.entry(site.reason).or_default() += 1;
        *self.blocked_by_stat.entry(site.stat).or_default() += 1;
        self.blocked_sites.push(site);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) struct MirPreHomeRewriteDriver {
    generation: MirRoutineGeneration,
    max_rounds: usize,
}

impl Default for MirPreHomeRewriteDriver {
    fn default() -> Self {
        Self {
            generation: MirRoutineGeneration::initial(),
            max_rounds: 32,
        }
    }
}

impl MirPreHomeRewriteDriver {
    pub(in crate::mir6502) fn generation(&self) -> MirRoutineGeneration {
        self.generation
    }

    pub(in crate::mir6502) fn with_max_rounds(max_rounds: usize) -> Self {
        Self {
            max_rounds,
            ..Self::default()
        }
    }

    pub(in crate::mir6502) fn run_fixed_point<Discover>(
        &mut self,
        routine: &mut MirRoutine,
        discover: Discover,
    ) -> Result<MirRewriteRunResult, MirRewriteError>
    where
        Discover: FnMut(&MirRoutine, &PreHomeRewriteContext<'_, '_>) -> Vec<MirRewritePlan>,
    {
        self.run_fixed_point_by_key(routine, discover, |routine| {
            routine
                .blocks
                .iter()
                .map(|block| block.ops.len())
                .sum::<usize>()
        })
    }

    pub(in crate::mir6502) fn run_fixed_point_by_key<Discover, Metric, Key>(
        &mut self,
        routine: &mut MirRoutine,
        mut discover: Discover,
        mut metric: Metric,
    ) -> Result<MirRewriteRunResult, MirRewriteError>
    where
        Discover: FnMut(&MirRoutine, &PreHomeRewriteContext<'_, '_>) -> Vec<MirRewritePlan>,
        Metric: FnMut(&MirRoutine) -> Key,
        Key: Ord,
    {
        let mut result = MirRewriteRunResult::default();
        for _ in 0..self.max_rounds {
            result.rounds += 1;
            let snapshot = PreHomeAnalysisSnapshot::new(routine, self.generation)
                .map_err(|_| MirRewriteError::InvalidCfg)?;
            result.analysis_builds += 1;
            let context = PreHomeRewriteContext::new(&snapshot);
            let plans = discover(routine, &context);
            result.candidates += plans.len();
            drop(snapshot);
            if plans.is_empty() {
                result.converged = true;
                return Ok(result);
            }

            let before = metric(routine);
            let batch = self.apply_batch(routine, plans)?;
            result.overlap_rejections += batch.overlap_rejections;
            result.estimated_bytes_saved += batch.estimated_bytes_saved;
            result.estimated_cycles_saved += batch.estimated_cycles_saved;
            result.applied += batch.applied.len();
            for stat in batch.applied {
                *result.applied_by_stat.entry(stat).or_default() += 1;
            }
            for (stat, count) in batch.observations {
                *result.applied_by_stat.entry(stat).or_default() += count;
            }
            let after = metric(routine);
            if after >= before {
                return Err(MirRewriteError::InvalidDeclaration {
                    stat: "pre-home-fixed-point",
                    message: "rewrite batch did not reduce its declared metric".to_string(),
                });
            }
        }
        Err(MirRewriteError::DidNotConverge {
            max_rounds: self.max_rounds,
        })
    }

    pub(in crate::mir6502) fn apply_batch(
        &mut self,
        routine: &mut MirRoutine,
        plans: Vec<MirRewritePlan>,
    ) -> Result<MirAppliedBatch, MirRewriteError> {
        for plan in &plans {
            validate_plan(routine, self.generation, plan)?;
        }
        let candidates = plans.len();
        let selected = select_non_overlapping(routine, plans);
        let overlap_rejections = candidates.saturating_sub(selected.len());
        let estimated_bytes_saved = selected
            .iter()
            .map(|plan| usize::from(plan.estimated_byte_saving))
            .sum();
        let estimated_cycles_saved = selected
            .iter()
            .map(|plan| usize::from(plan.estimated_cycle_saving))
            .sum();
        let mut by_block = BTreeMap::<MirBlockId, Vec<MirRewritePlan>>::new();
        for plan in selected {
            by_block.entry(plan.block).or_default().push(plan);
        }
        let mut applied = Vec::new();
        let mut observations = BTreeMap::new();
        for (block, mut plans) in by_block {
            let block_index = routine
                .blocks
                .iter()
                .position(|candidate| candidate.id == block)
                .expect("validated rewrite block");
            plans.sort_by_key(|plan| std::cmp::Reverse(plan.range.start));
            for plan in plans {
                for (stat, count) in &plan.observations {
                    *observations.entry(*stat).or_default() += *count;
                }
                routine.blocks[block_index]
                    .ops
                    .splice(plan.range.clone(), plan.replacement);
                applied.push(plan.stat);
            }
        }
        if !applied.is_empty() {
            self.generation = self.generation.next();
        }
        Ok(MirAppliedBatch {
            applied,
            overlap_rejections,
            observations,
            estimated_bytes_saved,
            estimated_cycles_saved,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirAppliedBatch {
    pub applied: Vec<&'static str>,
    pub overlap_rejections: usize,
    pub observations: BTreeMap<&'static str, usize>,
    pub estimated_bytes_saved: usize,
    pub estimated_cycles_saved: usize,
}

/// Routine-level transactional driver for physical-home rewrites. A snapshot
/// is rebuilt after every applied batch, so no liveness or availability fact
/// can be reused with shifted operation indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) struct MirPostHomeRewriteDriver {
    generation: MirRoutineGeneration,
    max_rounds: usize,
}

impl Default for MirPostHomeRewriteDriver {
    fn default() -> Self {
        Self {
            generation: MirRoutineGeneration::initial(),
            max_rounds: 32,
        }
    }
}

impl MirPostHomeRewriteDriver {
    pub(in crate::mir6502) fn generation(&self) -> MirRoutineGeneration {
        self.generation
    }

    pub(in crate::mir6502) fn with_max_rounds(max_rounds: usize) -> Self {
        Self {
            max_rounds,
            ..Self::default()
        }
    }

    pub(in crate::mir6502) fn run_fixed_point<Discover>(
        &mut self,
        routine: &mut MirRoutine,
        mut discover: Discover,
    ) -> Result<MirRewriteRunResult, MirRewriteError>
    where
        Discover:
            FnMut(&MirRoutine, &PostHomeRewriteContext<'_, '_>) -> Vec<MirPostHomeRewritePlan>,
    {
        let mut result = MirRewriteRunResult::default();
        for _ in 0..self.max_rounds {
            result.rounds += 1;
            let snapshot = PostHomeAnalysisSnapshot::new(routine, self.generation)
                .map_err(|_| MirRewriteError::InvalidCfg)?;
            result.analysis_builds += 1;
            let context = PostHomeRewriteContext::new(&snapshot);
            let plans = discover(routine, &context);
            result.candidates += plans.len();
            for site in context.take_blocked_sites() {
                result.record_blocked_site(site);
            }
            for plan in &plans {
                validate_posthome_plan(routine, self.generation, &context, plan)?;
            }
            drop(snapshot);
            if plans.is_empty() {
                result.converged = true;
                return Ok(result);
            }

            let candidates = plans.len();
            let selected = select_non_overlapping_posthome(routine, plans);
            result.overlap_rejections += candidates.saturating_sub(selected.len());
            result.estimated_bytes_saved += selected
                .iter()
                .map(|plan| usize::from(plan.estimated_byte_saving))
                .sum::<usize>();
            result.estimated_cycles_saved += selected
                .iter()
                .map(|plan| usize::from(plan.estimated_cycle_saving))
                .sum::<usize>();
            let mut by_block = BTreeMap::<MirBlockId, Vec<MirPostHomeRewritePlan>>::new();
            for plan in selected {
                by_block.entry(plan.block).or_default().push(plan);
            }
            let mut applied_this_round = 0usize;
            for (block, mut plans) in by_block {
                let block_index = routine
                    .blocks
                    .iter()
                    .position(|candidate| candidate.id == block)
                    .expect("validated post-home rewrite block");
                plans.sort_by_key(|plan| std::cmp::Reverse(plan.range.start));
                for plan in plans {
                    for (stat, count) in &plan.observations {
                        *result.applied_by_stat.entry(*stat).or_default() += *count;
                    }
                    routine.blocks[block_index]
                        .ops
                        .splice(plan.range, plan.replacement);
                    *result.applied_by_stat.entry(plan.stat).or_default() += 1;
                    result.applied += 1;
                    applied_this_round += 1;
                }
            }
            if applied_this_round == 0 {
                return Err(MirRewriteError::InvalidDeclaration {
                    stat: "post-home-fixed-point",
                    message: "non-empty candidate set selected no rewrite".to_string(),
                });
            }
            self.generation = self.generation.next();
        }
        Err(MirRewriteError::DidNotConverge {
            max_rounds: self.max_rounds,
        })
    }
}

fn select_non_overlapping_posthome(
    routine: &MirRoutine,
    mut plans: Vec<MirPostHomeRewritePlan>,
) -> Vec<MirPostHomeRewritePlan> {
    let block_order = routine
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.id, index))
        .collect::<BTreeMap<_, _>>();
    plans.sort_by_key(|plan| {
        (
            plan.family_priority,
            std::cmp::Reverse(plan.estimated_byte_saving),
            std::cmp::Reverse(plan.estimated_cycle_saving),
            std::cmp::Reverse(plan.range.len()),
            block_order[&plan.block],
            plan.range.start,
        )
    });
    let mut occupied = BTreeMap::<MirBlockId, Vec<std::ops::Range<usize>>>::new();
    let mut selected = Vec::new();
    for plan in plans {
        let overlaps = occupied.get(&plan.block).is_some_and(|ranges| {
            ranges
                .iter()
                .any(|range| ranges_overlap(range, &plan.range))
        });
        if !overlaps {
            occupied
                .entry(plan.block)
                .or_default()
                .push(plan.range.clone());
            selected.push(plan);
        }
    }
    selected
}

fn validate_posthome_plan(
    routine: &MirRoutine,
    generation: MirRoutineGeneration,
    context: &PostHomeRewriteContext<'_, '_>,
    plan: &MirPostHomeRewritePlan,
) -> Result<(), MirRewriteError> {
    if plan.generation != generation {
        return Err(MirRewriteError::StalePlan {
            expected: generation,
            actual: plan.generation,
        });
    }
    let Some(block) = routine.blocks.iter().find(|block| block.id == plan.block) else {
        return Err(MirRewriteError::UnknownBlock(plan.block));
    };
    if plan.range.start >= plan.range.end || plan.range.end > block.ops.len() {
        return Err(MirRewriteError::InvalidRange {
            block: plan.block,
            start: plan.range.start,
            end: plan.range.end,
            op_count: block.ops.len(),
        });
    }
    if plan.range.len() != plan.replacement.len()
        && !(plan.change_set.invalidates(MirFactClass::HomeLiveness)
            && plan.change_set.invalidates(MirFactClass::MachineLiveness)
            && plan.change_set.invalidates(MirFactClass::ParamAvailability)
            && plan.change_set.invalidates(MirFactClass::MemoryEffects))
    {
        return Err(MirRewriteError::InvalidDeclaration {
            stat: plan.stat,
            message: "operation-count change omitted post-home invalidations".to_string(),
        });
    }

    let end = context.point(MirSite::Op {
        block: plan.block,
        op_index: plan.range.end - 1,
    });
    let mut declared = BTreeSet::new();
    for removed in &plan.removed_homes {
        if removed.store.block() != plan.block
            || !matches!(removed.store, MirSite::Op { op_index, .. } if plan.range.contains(&op_index))
        {
            return Err(MirRewriteError::InvalidDeclaration {
                stat: plan.stat,
                message: format!("removed home store is outside rewrite range: {removed:?}"),
            });
        }
        if !declared.insert(*removed) {
            return Err(MirRewriteError::InvalidDeclaration {
                stat: plan.stat,
                message: format!("duplicate removed home definition: {removed:?}"),
            });
        }
        let MirSite::Op { op_index, .. } = removed.store else {
            unreachable!("checked operation site")
        };
        let effects = classify_op(&block.ops[op_index]);
        if !effects.homes.writes.contains(&removed.home)
            && !effects.addresses.pair_writes.contains(&removed.home)
        {
            return Err(MirRewriteError::InvalidDeclaration {
                stat: plan.stat,
                message: format!("declared store does not write its home: {removed:?}"),
            });
        }
        if !matches!(
            context.home_definition_dead_after(removed.home, context.point(removed.store), end,),
            MirProof::Proven(())
        ) {
            return Err(MirRewriteError::InvalidDeclaration {
                stat: plan.stat,
                message: format!("removed home definition is live: {removed:?}"),
            });
        }
    }
    if !matches!(
        context.exit_state_change_is_unobservable(&plan.exit_state_change, end),
        MirProof::Proven(())
    ) {
        return Err(MirRewriteError::InvalidDeclaration {
            stat: plan.stat,
            message: "declared exit-state change is observable".to_string(),
        });
    }
    Ok(())
}

fn select_non_overlapping(
    routine: &MirRoutine,
    mut plans: Vec<MirRewritePlan>,
) -> Vec<MirRewritePlan> {
    let block_order = routine
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.id, index))
        .collect::<BTreeMap<_, _>>();
    plans.sort_by_key(|plan| {
        (
            plan.family_priority,
            std::cmp::Reverse(plan.estimated_byte_saving),
            std::cmp::Reverse(plan.estimated_cycle_saving),
            std::cmp::Reverse(plan.range.len()),
            block_order[&plan.block],
            plan.range.start,
        )
    });
    let mut occupied = BTreeMap::<MirBlockId, Vec<std::ops::Range<usize>>>::new();
    let mut selected = Vec::new();
    for plan in plans {
        let overlaps = occupied.get(&plan.block).is_some_and(|ranges| {
            ranges
                .iter()
                .any(|range| ranges_overlap(range, &plan.range))
        });
        if overlaps {
            continue;
        }
        occupied
            .entry(plan.block)
            .or_default()
            .push(plan.range.clone());
        selected.push(plan);
    }
    selected
}

fn ranges_overlap(left: &std::ops::Range<usize>, right: &std::ops::Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn validate_plan(
    routine: &MirRoutine,
    generation: MirRoutineGeneration,
    plan: &MirRewritePlan,
) -> Result<(), MirRewriteError> {
    if plan.generation != generation {
        return Err(MirRewriteError::StalePlan {
            expected: generation,
            actual: plan.generation,
        });
    }
    let Some(block) = routine.blocks.iter().find(|block| block.id == plan.block) else {
        return Err(MirRewriteError::UnknownBlock(plan.block));
    };
    if plan.range.start >= plan.range.end || plan.range.end > block.ops.len() {
        return Err(MirRewriteError::InvalidRange {
            block: plan.block,
            start: plan.range.start,
            end: plan.range.end,
            op_count: block.ops.len(),
        });
    }
    if plan.range.len() != plan.replacement.len()
        && !(plan.change_set.invalidates(MirFactClass::TempUseDef)
            && plan
                .change_set
                .invalidates(MirFactClass::ReachingDefinitions)
            && plan.change_set.invalidates(MirFactClass::TempLiveness))
    {
        return Err(MirRewriteError::InvalidDeclaration {
            stat: plan.stat,
            message: "operation-count change omitted site-indexed invalidations".to_string(),
        });
    }
    validate_removed_definitions(block.id, &block.ops[plan.range.clone()], plan)?;
    if !effect_delta_is_valid(
        &block.ops[plan.range.clone()],
        &plan.replacement,
        plan.exit_effect_delta,
    ) {
        return Err(MirRewriteError::InvalidDeclaration {
            stat: plan.stat,
            message: "replacement effects do not match the declared delta".to_string(),
        });
    }
    Ok(())
}

fn effect_delta_is_valid(original: &[MirOp], replacement: &[MirOp], delta: MirEffectDelta) -> bool {
    let original_ops = original;
    let replacement_ops = replacement;
    if matches!(
        delta,
        MirEffectDelta::MaterializedCallArguments | MirEffectDelta::ForwardedCallResultStore { .. }
    ) && !calls_and_effects_are_preserved(original, replacement)
    {
        return false;
    }
    if matches!(delta, MirEffectDelta::MaterializedCallArguments) {
        return calls_and_effects_are_preserved(original, replacement);
    }
    let mut original = observable_effects(original);
    let mut replacement = observable_effects(replacement);
    match delta {
        MirEffectDelta::Unchanged => original == replacement,
        MirEffectDelta::SelectedResultRegister(register) => {
            if !register_is_set(replacement.register_writes, register) {
                return false;
            }
            clear_register(&mut original.register_reads, register);
            clear_register(&mut original.register_writes, register);
            clear_register(&mut original.register_clobbers, register);
            clear_register(&mut replacement.register_reads, register);
            clear_register(&mut replacement.register_writes, register);
            clear_register(&mut replacement.register_clobbers, register);
            original == replacement
        }
        MirEffectDelta::ForwardedReturnSlot { base, width } => {
            let bytes = match width {
                crate::mir6502::ir::MirWidth::Byte => 1,
                crate::mir6502::ir::MirWidth::Word => 2,
            };
            for offset in 0..bytes {
                let slot = crate::mir6502::ir::MirFixedZpSlot(base.0.saturating_add(offset as u8));
                original
                    .home_reads
                    .insert(crate::mir6502::analysis::effects::MirHomeByte::FixedZeroPage(slot));
                original.memory_reads.push(format!("fixed-zp:{}", slot.0));
            }
            original.memory_reads.sort();
            original == replacement
        }
        MirEffectDelta::MaterializedCallArguments => unreachable!("handled before projection"),
        MirEffectDelta::ForwardedCallResultStore {
            base,
            width,
            selected_arg_register,
        } => {
            add_fixed_home_reads(&mut original, base, width);
            if let Some(register) = selected_arg_register {
                if !register_is_set(replacement.register_reads, register)
                    || !register_is_set(replacement.register_writes, register)
                {
                    return false;
                }
                clear_register(&mut original.register_reads, register);
                clear_register(&mut original.register_writes, register);
                clear_register(&mut original.register_clobbers, register);
                clear_register(&mut replacement.register_reads, register);
                clear_register(&mut replacement.register_writes, register);
                clear_register(&mut replacement.register_clobbers, register);
                // Selecting a byte load into a real 6502 register also makes
                // its transient Z/N writes explicit. The immediately
                // following preserved call clobbers those flags before the
                // rewritten window exits.
                original.flag_writes = MirFlagSet::default();
                replacement.flag_writes = MirFlagSet::default();
            }
            original == replacement
        }
        MirEffectDelta::MaterializedStoreConsumer
        | MirEffectDelta::MaterializedPointerConsumer
        | MirEffectDelta::MaterializedIndexConsumer => {
            let materialized_pointer = matches!(delta, MirEffectDelta::MaterializedPointerConsumer);
            if materialized_pointer && !pointer_source_is_preserved(original_ops, replacement_ops) {
                return false;
            }
            clear_machine_effects(&mut original);
            clear_machine_effects(&mut replacement);
            // Storage-address byte operands name a home without reading its
            // contents. Direct memory reads and writes remain checked below;
            // the duplicated home projection is therefore intentionally
            // removed for this target-selection delta.
            original.home_reads.clear();
            original.home_writes.clear();
            replacement.home_reads.clear();
            replacement.home_writes.clear();
            strip_materialized_consumer_projection(&mut original, original_ops);
            strip_materialized_consumer_projection(&mut replacement, replacement_ops);
            if materialized_pointer {
                strip_pointer_producer_projection(&mut original, original_ops);
                strip_pointer_producer_projection(&mut replacement, replacement_ops);
            }
            // A selector may avoid reading lanes which do not contribute to
            // the stored value (for example, the high lane of a word that is
            // immediately truncated). It may not introduce a new data read,
            // except for reloading a location written in the original
            // transaction. Absolute reads remain observable and therefore
            // cannot be dropped.
            if original.memory_reads.iter().any(|read| {
                read.starts_with("absolute:") && !replacement.memory_reads.contains(read)
            }) {
                return false;
            }
            for read in &replacement.memory_reads {
                if !original.memory_reads.contains(read) && original.memory_writes.contains(read) {
                    original.memory_reads.push(read.clone());
                }
            }
            original.memory_reads.sort();
            if replacement
                .memory_reads
                .iter()
                .any(|read| !original.memory_reads.contains(read))
            {
                return false;
            }
            original.memory_reads = replacement.memory_reads.clone();
            original == replacement
        }
    }
}

fn pointer_source_is_preserved(original: &[MirOp], replacement: &[MirOp]) -> bool {
    let sources = original
        .iter()
        .filter_map(|op| match op {
            MirOp::Load {
                dst: crate::mir6502::ir::MirDef::VTemp(_),
                src: MirAddr::Direct(mem),
                width: MirWidth::Word,
            } => Some(mem),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [source] = sources.as_slice() else {
        return false;
    };
    let expected = (0..2)
        .map(|offset| memory_byte_key(source, offset))
        .collect::<BTreeSet<_>>();

    let materializations = replacement
        .iter()
        .filter_map(|op| match op {
            MirOp::MaterializeAddress { consumer, value } => {
                let mut inputs = BTreeSet::new();
                collect_value_pointer_reads(value, &mut inputs);
                Some((*consumer, inputs))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let access_consumers = replacement
        .iter()
        .filter_map(|op| match op {
            MirOp::LoadIndirect { consumer, .. } | MirOp::StoreIndirect { consumer, .. } => {
                Some(*consumer)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some(selected_consumer) = access_consumers.first().copied() else {
        return false;
    };
    if access_consumers
        .iter()
        .any(|consumer| *consumer != selected_consumer)
    {
        return false;
    }
    match materializations.as_slice() {
        [(consumer, inputs)] if *consumer == selected_consumer && *inputs == expected => {
            return true;
        }
        [] => {}
        _ => return false,
    }

    let mut selected = BTreeSet::new();
    collect_consumer_keys(selected_consumer, &mut selected);
    let exact_direct_home = expected
        .iter()
        .map(|key| canonical_zero_page_key(key))
        .collect::<BTreeSet<_>>()
        == selected
            .iter()
            .map(|key| canonical_zero_page_key(key))
            .collect::<BTreeSet<_>>();
    if exact_direct_home {
        return true;
    }

    let source_can_have_a_selected_layout_home = matches!(
        source,
        MirMem::Static { .. }
            | MirMem::Global { .. }
            | MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
    );
    let selected_is_direct_zero_page = !selected.is_empty()
        && selected
            .iter()
            .all(|key| key.starts_with("fixed-zp:") || key.starts_with("zp:"));
    let selected_uses_private_staging = selected.iter().any(|key| {
        key.strip_prefix("fixed-zp:")
            .and_then(|slot| slot.parse::<u8>().ok())
            .is_some_and(|slot| (0xAC..=0xAF).contains(&slot))
    });
    source_can_have_a_selected_layout_home
        && selected_is_direct_zero_page
        && !selected_uses_private_staging
}

fn canonical_zero_page_key(key: &str) -> String {
    key.strip_prefix("absolute:")
        .and_then(|address| address.parse::<u16>().ok())
        .filter(|address| *address <= u8::MAX as u16)
        .map_or_else(|| key.to_string(), |address| format!("fixed-zp:{address}"))
}

fn strip_pointer_producer_projection(effects: &mut ObservableEffects, ops: &[MirOp]) {
    let pointer_loads = ops
        .iter()
        .filter_map(|op| match op {
            MirOp::Load {
                dst: crate::mir6502::ir::MirDef::VTemp(_),
                src: MirAddr::Direct(mem),
                width: MirWidth::Word,
            } => Some(mem),
            _ => None,
        })
        .flat_map(|mem| (0..2).map(move |offset| memory_byte_key(mem, offset)))
        .collect::<BTreeSet<_>>();
    effects
        .memory_reads
        .retain(|key| !pointer_loads.contains(key));
}

fn strip_materialized_consumer_projection(effects: &mut ObservableEffects, ops: &[MirOp]) {
    const PRIVATE_POINTER_SCRATCH_FIRST: u8 = 0xAC;
    const PRIVATE_POINTER_SCRATCH_LAST: u8 = 0xAF;
    let is_private = |key: &String| {
        key.strip_prefix("fixed-zp:")
            .and_then(|value| value.parse::<u8>().ok())
            .is_some_and(|slot| {
                (PRIVATE_POINTER_SCRATCH_FIRST..=PRIVATE_POINTER_SCRATCH_LAST).contains(&slot)
            })
    };
    effects.memory_reads.retain(|key| !is_private(key));
    effects.memory_writes.retain(|key| !is_private(key));

    let (address_reads, address_writes) = store_materialization_address_keys(ops);
    effects
        .memory_reads
        .retain(|key| !address_reads.contains(key));
    effects
        .memory_writes
        .retain(|key| !address_writes.contains(key));
}

fn store_materialization_address_keys(ops: &[MirOp]) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();
    for op in ops {
        match op {
            MirOp::Load { src, .. } | MirOp::Store { dst: src, .. } => {
                collect_addr_carrier_reads(src, &mut reads);
            }
            MirOp::MaterializeAddress { consumer, value } => {
                collect_value_pointer_reads(value, &mut reads);
                collect_consumer_keys(*consumer, &mut writes);
            }
            MirOp::MaterializeIndexedAddress { consumer, base, .. } => {
                collect_consumer_keys(*consumer, &mut reads);
                collect_consumer_keys(*consumer, &mut writes);
                collect_value_pointer_reads(base, &mut reads);
            }
            MirOp::AdvanceAddress { consumer, .. } => {
                collect_consumer_keys(*consumer, &mut reads);
                collect_consumer_keys(*consumer, &mut writes);
            }
            MirOp::LoadIndirect { consumer, .. } | MirOp::StoreIndirect { consumer, .. } => {
                collect_consumer_keys(*consumer, &mut reads);
            }
            MirOp::IndirectByteCompound { target, source, .. } => {
                collect_consumer_keys(*target, &mut reads);
                collect_consumer_keys(*source, &mut reads);
                collect_consumer_keys(*target, &mut writes);
            }
            _ => {}
        }
    }
    (reads, writes)
}

fn collect_addr_carrier_reads(addr: &MirAddr, reads: &mut BTreeSet<String>) {
    match addr {
        MirAddr::AbsoluteIndexedX { base } | MirAddr::AbsoluteIndexedY { base } => {
            reads.insert(memory_byte_key(base, 0));
        }
        MirAddr::PointerCell { ptr, .. } | MirAddr::PointerIndex { ptr, .. } => {
            collect_mem_range_keys(ptr, 2, reads);
        }
        MirAddr::ComputedIndex { base, .. } | MirAddr::Deref { ptr: base, .. } => {
            collect_value_pointer_reads(base, reads);
        }
        MirAddr::IndirectIndexedY { zp } => {
            collect_consumer_keys(
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(*zp)),
                reads,
            );
        }
        MirAddr::FixedIndirectIndexedY { zp } => {
            collect_consumer_keys(
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo: *zp }),
                reads,
            );
        }
        MirAddr::Direct(_) | MirAddr::Label(_) | MirAddr::ZeroPageIndexedX { .. } => {}
    }
}

fn collect_value_pointer_reads(value: &MirValue, reads: &mut BTreeSet<String>) {
    match value {
        MirValue::PointerCell(mem) => {
            reads.insert(memory_byte_key(mem, 0));
        }
        MirValue::Word { lo, hi } => {
            collect_value_pointer_reads(lo, reads);
            collect_value_pointer_reads(hi, reads);
        }
        _ => {}
    }
}

fn collect_consumer_keys(consumer: MirAddressConsumer, keys: &mut BTreeSet<String>) {
    match consumer.pointer_pair() {
        MirPointerPair::Virtual(lo) => {
            keys.insert(memory_byte_key(&MirMem::ZeroPage(lo), 0));
            keys.insert(memory_byte_key(&MirMem::ZeroPage(lo), 1));
        }
        MirPointerPair::Fixed { lo } => {
            keys.insert(memory_byte_key(&MirMem::FixedZeroPage(lo), 0));
            keys.insert(memory_byte_key(&MirMem::FixedZeroPage(lo), 1));
        }
    }
}

fn collect_mem_range_keys(mem: &MirMem, bytes: u16, keys: &mut BTreeSet<String>) {
    for offset in 0..bytes {
        keys.insert(memory_byte_key(mem, offset));
    }
}

fn clear_machine_effects(effects: &mut ObservableEffects) {
    effects.register_reads = MirRegisterSet::default();
    effects.register_writes = MirRegisterSet::default();
    effects.register_clobbers = MirRegisterSet::default();
    effects.flag_reads = MirFlagSet::default();
    effects.flag_writes = MirFlagSet::default();
    effects.flag_clobbers = MirFlagSet::default();
}

fn add_fixed_home_reads(effects: &mut ObservableEffects, base: MirFixedZpSlot, width: MirWidth) {
    let bytes = match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    };
    for offset in 0..bytes {
        let slot = MirFixedZpSlot(base.0.saturating_add(offset as u8));
        effects.home_reads.insert(MirHomeByte::FixedZeroPage(slot));
        effects.memory_reads.push(format!("fixed-zp:{}", slot.0));
    }
    effects.memory_reads.sort();
}

fn calls_and_effects_are_preserved(original: &[MirOp], replacement: &[MirOp]) -> bool {
    let original_calls = original
        .iter()
        .filter_map(call_effect_identity)
        .collect::<Vec<_>>();
    let replacement_calls = replacement
        .iter()
        .filter_map(call_effect_identity)
        .collect::<Vec<_>>();
    !original_calls.is_empty() && original_calls == replacement_calls
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MirCallTargetIdentity {
    Routine(crate::mir6502::ir::RoutineId),
    Indirect(crate::mir6502::ir::MirWidth),
    Builtin { name: String, address: Option<u16> },
    Runtime { name: String, address: Option<u16> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MirCallEffectIdentity {
    target: MirCallTargetIdentity,
    clobbers: MirRegisterSet,
    preserves: MirRegisterSet,
    effects: crate::mir6502::ir::MirEffects,
}

fn call_effect_identity(op: &MirOp) -> Option<MirCallEffectIdentity> {
    let MirOp::Call {
        target,
        abi,
        effects,
        ..
    } = op
    else {
        return None;
    };
    let target = match target {
        crate::mir6502::ir::MirCallTarget::Routine(routine) => {
            MirCallTargetIdentity::Routine(*routine)
        }
        crate::mir6502::ir::MirCallTarget::Indirect { width, .. } => {
            MirCallTargetIdentity::Indirect(*width)
        }
        crate::mir6502::ir::MirCallTarget::Builtin { name, address } => {
            MirCallTargetIdentity::Builtin {
                name: name.clone(),
                address: *address,
            }
        }
        crate::mir6502::ir::MirCallTarget::Runtime { name, address } => {
            MirCallTargetIdentity::Runtime {
                name: name.clone(),
                address: *address,
            }
        }
    };
    Some(MirCallEffectIdentity {
        target,
        clobbers: abi.clobbers,
        preserves: abi.preserves,
        effects: effects.clone(),
    })
}

fn register_is_set(registers: MirRegisterSet, register: MirReg) -> bool {
    match register {
        MirReg::A => registers.a,
        MirReg::X => registers.x,
        MirReg::Y => registers.y,
    }
}

fn clear_register(registers: &mut MirRegisterSet, register: MirReg) {
    match register {
        MirReg::A => registers.a = false,
        MirReg::X => registers.x = false,
        MirReg::Y => registers.y = false,
    }
}

fn validate_removed_definitions(
    block: MirBlockId,
    original: &[MirOp],
    plan: &MirRewritePlan,
) -> Result<(), MirRewriteError> {
    let replacement_lanes = collect_definition_lanes(&plan.replacement);
    let mut expected = BTreeSet::new();
    for (relative_index, op) in original.iter().enumerate() {
        let site = MirSite::Op {
            block,
            op_index: plan.range.start + relative_index,
        };
        for lane in collect_definition_lanes(std::slice::from_ref(op)) {
            if !replacement_lanes.contains(&lane) {
                expected.insert(MirDefSite { site, lane });
            }
        }
    }
    let declared = plan
        .removed_defs
        .iter()
        .map(|removed| removed.definition)
        .collect::<BTreeSet<_>>();
    if declared != expected {
        return Err(MirRewriteError::InvalidDeclaration {
            stat: plan.stat,
            message: format!(
                "removed definitions differ: expected {expected:?}, declared {declared:?}"
            ),
        });
    }
    Ok(())
}

fn collect_definition_lanes(ops: &[MirOp]) -> BTreeSet<MirTempLane> {
    ops.iter()
        .flat_map(|op| classify_op(op).logical.temp_defs)
        .flat_map(|access| match access {
            MirTempAccess::Exact { temp, byte } => vec![MirTempLane { temp, byte }],
            MirTempAccess::Full(temp) => {
                vec![MirTempLane { temp, byte: 0 }, MirTempLane { temp, byte: 1 }]
            }
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ObservableEffects {
    home_reads: BTreeSet<MirHomeByte>,
    home_writes: BTreeSet<MirHomeByte>,
    memory_reads: Vec<String>,
    memory_writes: Vec<String>,
    indirect_reads: bool,
    indirect_writes: bool,
    opaque: bool,
    register_reads: MirRegisterSet,
    register_writes: MirRegisterSet,
    register_clobbers: MirRegisterSet,
    flag_reads: MirFlagSet,
    flag_writes: MirFlagSet,
    flag_clobbers: MirFlagSet,
}

fn observable_effects(ops: &[MirOp]) -> ObservableEffects {
    let mut out = ObservableEffects::default();
    for op in ops {
        let effects = classify_op(op);
        out.home_reads.extend(effects.homes.reads);
        out.home_writes.extend(effects.homes.writes);
        out.memory_reads
            .extend(effects.memory.direct_reads.iter().flat_map(range_byte_keys));
        out.memory_writes.extend(
            effects
                .memory
                .direct_writes
                .iter()
                .flat_map(range_byte_keys),
        );
        if !matches!(
            effects.memory.structured_reads,
            crate::mir6502::ir::MirMemoryEffect::None
        ) {
            out.memory_reads
                .push(format!("structured:{:?}", effects.memory.structured_reads));
        }
        if !matches!(
            effects.memory.structured_writes,
            crate::mir6502::ir::MirMemoryEffect::None
        ) {
            out.memory_writes
                .push(format!("structured:{:?}", effects.memory.structured_writes));
        }
        out.indirect_reads |= effects.memory.indirect_reads;
        out.indirect_writes |= effects.memory.indirect_writes;
        out.opaque |= effects.memory.opaque;
        merge_registers(&mut out.register_reads, effects.machine.register_reads);
        merge_registers(&mut out.register_writes, effects.machine.register_writes);
        merge_registers(
            &mut out.register_clobbers,
            effects.machine.register_clobbers,
        );
        merge_flags(&mut out.flag_reads, effects.machine.flag_reads);
        merge_flags(&mut out.flag_writes, effects.machine.flag_writes);
        merge_flags(&mut out.flag_clobbers, effects.machine.flag_clobbers);
    }
    out.memory_reads.sort();
    out.memory_writes.sort();
    out
}

fn range_byte_keys(range: &MirMemoryRange) -> Vec<String> {
    (0..range.bytes)
        .map(|offset| memory_byte_key(&range.base, offset))
        .collect()
}

fn memory_byte_key(mem: &MirMem, delta: u16) -> String {
    match mem {
        MirMem::Absolute(address) => format!("absolute:{}", address.saturating_add(delta)),
        MirMem::Static { id, offset } => {
            format!("static:{id:?}:{}", offset.saturating_add(delta))
        }
        MirMem::Global { id, offset } => {
            format!("global:{id:?}:{}", offset.saturating_add(delta))
        }
        MirMem::Local { id, offset } => {
            format!("local:{id:?}:{}", offset.saturating_add(delta))
        }
        MirMem::Param { id, offset } => {
            format!("param:{id:?}:{}", offset.saturating_add(delta))
        }
        MirMem::Spill { id, offset } => {
            format!("spill:{id:?}:{}", offset.saturating_add(delta))
        }
        MirMem::ZeroPage(slot) => format!("zp:{slot:?}"),
        MirMem::FixedZeroPage(slot) => format!("fixed-zp:{}", slot.0.saturating_add(delta as u8)),
    }
}

fn merge_registers(into: &mut MirRegisterSet, other: MirRegisterSet) {
    into.a |= other.a;
    into.x |= other.x;
    into.y |= other.y;
    into.flags |= other.flags;
    into.sp |= other.sp;
}

fn merge_flags(into: &mut MirFlagSet, other: MirFlagSet) {
    into.c |= other.c;
    into.z |= other.z;
    into.n |= other.n;
    into.v |= other.v;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::sites::MirSite;
    use crate::mir6502::analysis::use_def::MirTempLane;
    use crate::mir6502::ir::{
        MirAddr, MirBlock, MirDef, MirEffects, MirFrame, MirMem, MirRoutineAbi, MirSpillId,
        MirTempId, MirTerminator, MirValue, MirWidth, RoutineId,
    };
    use crate::mir6502::rewrite::context::MirExitStateChange;
    use crate::mir6502::rewrite::plan::{
        MirChangeSet, MirEffectDelta, MirPostHomeRewritePlan, MirRemovedDefinition,
    };

    fn routine(op: MirOp) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "driver".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![op],
                terminator: MirTerminator::Return,
            }],
            effects: MirEffects::default(),
        }
    }

    fn load(value: u16) -> MirOp {
        MirOp::LoadImm {
            dst: MirDef::VTemp(MirTempId(1)),
            value,
            width: MirWidth::Byte,
        }
    }

    fn spill_store(id: u32) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(id),
                offset: 0,
            }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    }

    fn spill_load(id: u32) -> MirOp {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(id),
                offset: 0,
            }),
            width: MirWidth::Byte,
        }
    }

    fn replacement_plan(stat: &'static str, saving: u16, replacement: MirOp) -> MirRewritePlan {
        MirRewritePlan {
            generation: MirRoutineGeneration::initial(),
            block: MirBlockId(0),
            range: 0..1,
            replacement: vec![replacement],
            removed_defs: Vec::new(),
            exit_effect_delta: MirEffectDelta::Unchanged,
            change_set: MirChangeSet::default(),
            stat,
            observations: Vec::new(),
            family_priority: 0,
            estimated_byte_saving: saving,
            estimated_cycle_saving: 0,
        }
    }

    fn discover_posthome_canonical_load(
        routine: &MirRoutine,
        context: &PostHomeRewriteContext<'_, '_>,
    ) -> Vec<MirPostHomeRewritePlan> {
        if !matches!(
            routine.blocks[0].ops.first(),
            Some(MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: 1,
                width: MirWidth::Byte,
            })
        ) {
            return Vec::new();
        }
        vec![MirPostHomeRewritePlan {
            generation: context.generation(),
            block: MirBlockId(0),
            range: 0..1,
            replacement: vec![MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: 0,
                width: MirWidth::Byte,
            }],
            removed_homes: Vec::new(),
            exit_state_change: MirExitStateChange {
                registers: MirRegisterSet {
                    a: true,
                    ..MirRegisterSet::default()
                },
                flags: MirFlagSet {
                    z: true,
                    n: true,
                    ..MirFlagSet::default()
                },
                ..MirExitStateChange::default()
            },
            change_set: MirChangeSet::posthome_operation_change(),
            stat: "canonical-load",
            observations: Vec::new(),
            family_priority: 0,
            estimated_byte_saving: 1,
            estimated_cycle_saving: 1,
        }]
    }

    #[test]
    fn overlap_resolution_prefers_more_profitable_candidate_deterministically() {
        let mut routine = routine(load(1));
        let plans = vec![
            replacement_plan("small", 1, load(2)),
            replacement_plan("large", 2, load(3)),
        ];
        let batch = MirPreHomeRewriteDriver::default()
            .apply_batch(&mut routine, plans)
            .unwrap();
        assert_eq!(batch.applied, vec!["large"]);
        assert_eq!(batch.overlap_rejections, 1);
        assert_eq!(batch.estimated_bytes_saved, 2);
        assert_eq!(batch.estimated_cycles_saved, 0);
        assert!(matches!(
            routine.blocks[0].ops[0],
            MirOp::LoadImm { value: 3, .. }
        ));
    }

    #[test]
    fn posthome_fixed_point_is_idempotent_and_deterministic() {
        let input = routine(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 1,
            width: MirWidth::Byte,
        });
        let mut left = input.clone();
        let mut right = input;
        let left_result = MirPostHomeRewriteDriver::default()
            .run_fixed_point(&mut left, discover_posthome_canonical_load)
            .unwrap();
        let right_result = MirPostHomeRewriteDriver::default()
            .run_fixed_point(&mut right, discover_posthome_canonical_load)
            .unwrap();
        assert_eq!(left_result, right_result);
        assert_eq!(left, right);
        assert_eq!((left_result.applied, left_result.rounds), (1, 2));
        assert_eq!(left_result.estimated_bytes_saved, 1);
        assert_eq!(left_result.estimated_cycles_saved, 1);

        let stable = left.clone();
        let second = MirPostHomeRewriteDriver::default()
            .run_fixed_point(&mut left, discover_posthome_canonical_load)
            .unwrap();
        assert_eq!(left, stable);
        assert_eq!((second.applied, second.rounds), (0, 1));
        assert_eq!(second.estimated_bytes_saved, 0);
        assert_eq!(second.estimated_cycles_saved, 0);
    }

    #[test]
    fn posthome_fixed_point_reports_stable_proof_blockers() {
        let mut input = routine(spill_store(0));
        input.blocks[0].ops.push(spill_load(0));

        let result = MirPostHomeRewriteDriver::default()
            .run_fixed_point(&mut input, |routine, context| {
                crate::mir6502::rewrite::posthome::structural_plan(
                    routine,
                    context,
                    MirBlockId(0),
                    0..1,
                    Vec::new(),
                    MirExitStateChange::default(),
                    "remove-live-store",
                    0,
                )
                .into_iter()
                .collect()
            })
            .unwrap();

        assert!(result.converged);
        assert_eq!(result.blocked, 1);
        assert_eq!(result.blocked_by_reason["home-definition-live"], 1);
        assert_eq!(result.blocked_by_stat["remove-live-store"], 1);
        assert_eq!(result.blocked_sites[0].block, MirBlockId(0));
        assert_eq!(result.blocked_sites[0].op_index, 0);
    }

    #[test]
    fn posthome_iteration_limit_is_reported_not_silently_accepted() {
        let mut candidate = routine(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 1,
            width: MirWidth::Byte,
        });
        let error = MirPostHomeRewriteDriver::with_max_rounds(1)
            .run_fixed_point(&mut candidate, discover_posthome_canonical_load)
            .unwrap_err();
        assert_eq!(error, MirRewriteError::DidNotConverge { max_rounds: 1 });
    }

    #[test]
    fn validation_rejects_missing_site_indexed_invalidations() {
        let definition = MirDefSite {
            site: MirSite::Op {
                block: MirBlockId(0),
                op_index: 0,
            },
            lane: MirTempLane {
                temp: MirTempId(1),
                byte: 0,
            },
        };
        let removal = MirRewritePlan {
            generation: MirRoutineGeneration::initial(),
            block: MirBlockId(0),
            range: 0..1,
            replacement: Vec::new(),
            removed_defs: vec![MirRemovedDefinition { definition }],
            exit_effect_delta: MirEffectDelta::Unchanged,
            change_set: MirChangeSet::default(),
            stat: "invalid",
            observations: Vec::new(),
            family_priority: 0,
            estimated_byte_saving: 1,
            estimated_cycle_saving: 1,
        };
        assert!(matches!(
            MirPreHomeRewriteDriver::default().apply_batch(&mut routine(load(1)), vec![removal]),
            Err(MirRewriteError::InvalidDeclaration { .. })
        ));
    }

    #[test]
    fn validation_rejects_undeclared_observable_effect_change() {
        let mut routine = routine(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
            src: MirValue::ConstU8(1),
            width: MirWidth::Byte,
        });
        let plan = MirRewritePlan {
            generation: MirRoutineGeneration::initial(),
            block: MirBlockId(0),
            range: 0..1,
            replacement: Vec::new(),
            removed_defs: Vec::new(),
            exit_effect_delta: MirEffectDelta::Unchanged,
            change_set: MirChangeSet::prehome_operation_change(),
            stat: "invalid-effects",
            observations: Vec::new(),
            family_priority: 0,
            estimated_byte_saving: 1,
            estimated_cycle_saving: 1,
        };
        assert!(matches!(
            MirPreHomeRewriteDriver::default().apply_batch(&mut routine, vec![plan]),
            Err(MirRewriteError::InvalidDeclaration { .. })
        ));
    }

    #[test]
    fn store_selection_delta_allows_dead_nonvolatile_load_lanes() {
        let source = MirMem::FixedZeroPage(MirFixedZpSlot(0x90));
        let destination = MirMem::FixedZeroPage(MirFixedZpSlot(0x92));
        let original = vec![
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(1)),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Word,
            },
            MirOp::Truncate {
                dst: MirDef::VTemp(MirTempId(2)),
                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                from_width: MirWidth::Word,
                to_width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(destination.clone()),
                src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                width: MirWidth::Byte,
            },
        ];
        let replacement = vec![
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::PointerCell(source),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(destination),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ];

        assert!(effect_delta_is_valid(
            &original,
            &replacement,
            MirEffectDelta::MaterializedStoreConsumer,
        ));
    }

    #[test]
    fn store_selection_delta_keeps_absolute_loads_observable() {
        let original = vec![MirOp::Load {
            dst: MirDef::VTemp(MirTempId(1)),
            src: MirAddr::Direct(MirMem::Absolute(0xD000)),
            width: MirWidth::Word,
        }];
        let replacement = vec![MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Absolute(0xD000)),
            width: MirWidth::Byte,
        }];

        assert!(!effect_delta_is_valid(
            &original,
            &replacement,
            MirEffectDelta::MaterializedStoreConsumer,
        ));
    }

    #[test]
    fn store_selection_delta_abstracts_address_carrier_homes() {
        let original = vec![MirOp::Store {
            dst: MirAddr::Deref {
                ptr: MirValue::Word {
                    lo: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                        MirFixedZpSlot(0x90),
                    ))),
                    hi: Box::new(MirValue::PointerCell(MirMem::FixedZeroPage(
                        MirFixedZpSlot(0x91),
                    ))),
                },
                offset: 0,
            },
            src: MirValue::ConstU8(1),
            width: MirWidth::Byte,
        }];
        let replacement = vec![MirOp::StoreIndirect {
            consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                lo: MirFixedZpSlot(0xE4),
            }),
            src: MirValue::ConstU8(1),
            offset: 0,
        }];

        assert!(effect_delta_is_valid(
            &original,
            &replacement,
            MirEffectDelta::MaterializedStoreConsumer,
        ));
    }

    #[test]
    fn pointer_selection_delta_rejects_an_uninitialized_address_consumer() {
        let original = vec![
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(1)),
                src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                width: MirWidth::Word,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(2)),
                src: MirAddr::Deref {
                    ptr: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
        ];
        let replacement = vec![MirOp::LoadIndirect {
            consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                lo: MirFixedZpSlot(0xAC),
            }),
            dst: MirDef::VTemp(MirTempId(2)),
            offset: 0,
        }];

        assert!(!effect_delta_is_valid(
            &original,
            &replacement,
            MirEffectDelta::MaterializedPointerConsumer,
        ));
    }

    #[test]
    fn explicit_metric_allows_terminating_same_count_rewrites() {
        let mut routine = routine(MirOp::LoadImm {
            dst: MirDef::VTemp(MirTempId(1)),
            value: 1,
            width: MirWidth::Byte,
        });
        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                &mut routine,
                |routine, context| {
                    let MirOp::LoadImm {
                        dst,
                        value: 1,
                        width,
                    } = &routine.blocks[0].ops[0]
                    else {
                        return Vec::new();
                    };
                    vec![MirRewritePlan {
                        generation: context.generation(),
                        block: MirBlockId(0),
                        range: 0..1,
                        replacement: vec![MirOp::LoadImm {
                            dst: dst.clone(),
                            value: 0,
                            width: *width,
                        }],
                        removed_defs: Vec::new(),
                        exit_effect_delta: MirEffectDelta::Unchanged,
                        change_set: MirChangeSet::prehome_operation_change(),
                        stat: "same-count",
                        observations: Vec::new(),
                        family_priority: 1,
                        estimated_byte_saving: 1,
                        estimated_cycle_saving: 0,
                    }]
                },
                |routine| {
                    usize::from(matches!(
                        routine.blocks[0].ops[0],
                        MirOp::LoadImm { value: 1, .. }
                    ))
                },
            )
            .unwrap();
        assert_eq!((result.applied, result.rounds), (1, 2));
        assert!(result.converged);
    }
}
