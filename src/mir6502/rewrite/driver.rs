#![allow(dead_code)] // Production matcher migration begins in the next slice.

use std::collections::{BTreeMap, BTreeSet};

use crate::mir6502::analysis::effects::{
    MirFlagSet, MirHomeByte, MirMemoryRange, MirTempAccess, classify_op,
};
use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::analysis::use_def::{MirDefSite, MirTempLane};
use crate::mir6502::ir::{MirBlockId, MirMem, MirOp, MirReg, MirRegisterSet, MirRoutine};
use crate::mir6502::rewrite::context::PreHomeRewriteContext;
use crate::mir6502::rewrite::plan::{MirEffectDelta, MirFactClass, MirRewritePlan};

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
    pub converged: bool,
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
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirAppliedBatch {
    pub applied: Vec<&'static str>,
    pub overlap_rejections: usize,
    pub observations: BTreeMap<&'static str, usize>,
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
    }
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
        MirAddr, MirBlock, MirDef, MirEffects, MirFrame, MirMem, MirRoutineAbi, MirTempId,
        MirTerminator, MirValue, MirWidth, RoutineId,
    };
    use crate::mir6502::rewrite::plan::{MirChangeSet, MirEffectDelta, MirRemovedDefinition};

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
        assert!(matches!(
            routine.blocks[0].ops[0],
            MirOp::LoadImm { value: 3, .. }
        ));
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
