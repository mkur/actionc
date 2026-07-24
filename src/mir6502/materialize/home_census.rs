use super::dead_spills::block_successor_indices;
use super::defs::op_def;
use super::spills::{MirHomeStorage, home_access_counts, op_may_clobber_reg};
use super::stats::MirPeepholeStats;
use super::temp_liveness::{MirTempLiveSet, MirTempLiveness};
use super::temp_widths::collect_temp_widths;
use crate::mir6502::ir::{
    MirAddr, MirArgHome, MirCallTarget, MirCarryIn, MirCarryOut, MirCond, MirCondDest, MirDef,
    MirOp, MirProgram, MirReg, MirResultHome, MirRoutine, MirSpillId, MirTempId, MirTerminator,
    MirValue, MirWidth, MirZpSlot,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TempLane {
    id: MirTempId,
    byte: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DefSite {
    block: usize,
    op: usize,
    natural_reg: Option<MirReg>,
    coupled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UseSite {
    block: usize,
    op: Option<usize>,
    accepts_a: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegisterHomeRange {
    block: usize,
    def_op: usize,
    use_op: usize,
}

#[derive(Debug, Default)]
struct LaneFacts {
    defs: Vec<DefSite>,
    uses: Vec<UseSite>,
}

// Slice 2 establishes the full decision vocabulary. Rematerialization and
// direct forwarding are populated only by their later profitability slices.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HomeDecision {
    ElideInRegister(MirReg),
    Rematerialize(MirValue),
    ForwardToConsumer(MirValue),
    MustMaterialize(HomeMaterializationReason),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum HomeMaterializationReason {
    Unused,
    NonSingleDefinition,
    MultipleUses,
    TerminatorUse,
    CoupledLanes,
    LiveAcrossCall,
    LiveAcrossMachineBlock,
    LiveAcrossBarrier,
    LiveAcrossBackedge,
    LiveAtJoin,
    CrossBlock,
    NonAccumulatorProducer,
    AccumulatorClobber,
    UnsupportedConsumer,
    ObservableStorage,
    Profitability,
}

impl HomeMaterializationReason {
    fn name(self) -> &'static str {
        match self {
            Self::Unused => "unused",
            Self::NonSingleDefinition => "non-single-def",
            Self::MultipleUses => "multi-use",
            Self::TerminatorUse => "terminator",
            Self::CoupledLanes => "coupled",
            Self::LiveAcrossCall => "call-live",
            Self::LiveAcrossMachineBlock => "machine-live",
            Self::LiveAcrossBarrier => "barrier-live",
            Self::LiveAcrossBackedge => "backedge-live",
            Self::LiveAtJoin => "join-live",
            Self::CrossBlock => "cross-block",
            Self::NonAccumulatorProducer => "non-accumulator",
            Self::AccumulatorClobber => "accumulator-clobber",
            Self::UnsupportedConsumer => "unsupported-consumer",
            Self::ObservableStorage => "observable-storage",
            Self::Profitability => "profitability",
        }
    }

    fn metric_name(self) -> &'static str {
        match self {
            Self::Unused => "home-plan-materialize-unused-lanes",
            Self::NonSingleDefinition => "home-plan-materialize-non-single-def-lanes",
            Self::MultipleUses => "home-plan-materialize-multi-use-lanes",
            Self::TerminatorUse => "home-plan-materialize-terminator-lanes",
            Self::CoupledLanes => "home-plan-materialize-coupled-lanes",
            Self::LiveAcrossCall => "home-plan-materialize-call-live-lanes",
            Self::LiveAcrossMachineBlock => "home-plan-materialize-machine-live-lanes",
            Self::LiveAcrossBarrier => "home-plan-materialize-barrier-live-lanes",
            Self::LiveAcrossBackedge => "home-plan-materialize-backedge-live-lanes",
            Self::LiveAtJoin => "home-plan-materialize-join-live-lanes",
            Self::CrossBlock => "home-plan-materialize-cross-block-lanes",
            Self::NonAccumulatorProducer => "home-plan-materialize-non-accumulator-lanes",
            Self::AccumulatorClobber => "home-plan-materialize-accumulator-clobber-lanes",
            Self::UnsupportedConsumer => "home-plan-materialize-unsupported-consumer-lanes",
            Self::ObservableStorage => "home-plan-materialize-observable-storage-lanes",
            Self::Profitability => "home-plan-materialize-profitability-lanes",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct HomePlan {
    decisions: BTreeMap<TempLane, HomeDecision>,
    register_ranges: BTreeMap<TempLane, RegisterHomeRange>,
    attributions: BTreeMap<TempLane, LaneAttribution>,
}

impl HomePlan {
    #[cfg(test)]
    fn decision(&self, id: MirTempId, byte: u8) -> Option<&HomeDecision> {
        self.decisions.get(&TempLane { id, byte })
    }

    fn len(&self) -> usize {
        self.decisions.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LaneAttribution {
    producer: &'static str,
    consumer: &'static str,
    width: &'static str,
    coupled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackedHome {
    Spill(MirSpillId),
    ZeroPage(MirZpSlot),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedLane {
    attribution: LaneAttribution,
    decision: HomeDecision,
    home: Option<TrackedHome>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct HomeFateTracker {
    lanes: BTreeMap<TempLane, TrackedLane>,
}

impl HomeFateTracker {
    pub(super) fn from_plan(plan: &HomePlan) -> Self {
        let lanes = plan
            .decisions
            .iter()
            .map(|(lane, decision)| {
                let home = matches!(decision, HomeDecision::MustMaterialize(_)).then_some(
                    TrackedHome::Spill(MirSpillId(
                        lane.id
                            .0
                            .saturating_mul(2)
                            .saturating_add(u32::from(lane.byte)),
                    )),
                );
                let attribution = plan
                    .attributions
                    .get(lane)
                    .copied()
                    .expect("every planned lane has attribution");
                (
                    *lane,
                    TrackedLane {
                        attribution,
                        decision: decision.clone(),
                        home,
                    },
                )
            })
            .collect();
        Self { lanes }
    }

    pub(super) fn apply_spill_remap(&mut self, remap: &BTreeMap<MirSpillId, MirSpillId>) {
        for lane in self.lanes.values_mut() {
            let Some(TrackedHome::Spill(spill)) = lane.home else {
                continue;
            };
            if let Some(replacement) = remap.get(&spill) {
                lane.home = Some(TrackedHome::Spill(*replacement));
            }
        }
    }

    pub(super) fn apply_zero_page_remap(&mut self, remap: &BTreeMap<MirSpillId, MirZpSlot>) {
        for lane in self.lanes.values_mut() {
            let Some(TrackedHome::Spill(spill)) = lane.home else {
                continue;
            };
            if let Some(replacement) = remap.get(&spill) {
                lane.home = Some(TrackedHome::ZeroPage(*replacement));
            }
        }
    }

    pub(super) fn record_final_fates(&self, routine: &MirRoutine, stats: &mut MirPeepholeStats) {
        let accesses = home_access_counts(routine);
        let ram = routine
            .frame
            .spills
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let zp = routine
            .frame
            .virtual_zero_page
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut surviving_homes = BTreeSet::new();
        let mut reconciled = 0usize;

        for (lane, tracked) in &self.lanes {
            let (fate, home) = match tracked.home {
                None => ("elided-plan", None),
                Some(TrackedHome::Spill(spill)) if ram.contains(&spill) => {
                    ("ram", Some(MirHomeStorage::Spill(spill)))
                }
                Some(TrackedHome::ZeroPage(slot)) if zp.contains(&slot) => {
                    ("zp", Some(MirHomeStorage::ZeroPage(slot)))
                }
                Some(TrackedHome::Spill(_)) | Some(TrackedHome::ZeroPage(_)) => {
                    ("eliminated", None)
                }
            };
            reconciled = reconciled.saturating_add(1);
            stats.record_dynamic(routine.id, format!("residual-lane-final-{fate}"));
            stats.record_dynamic(
                routine.id,
                format!("residual-lane-producer-{}", tracked.attribution.producer),
            );
            stats.record_dynamic(
                routine.id,
                format!("residual-lane-consumer-{}", tracked.attribution.consumer),
            );
            stats.record_dynamic(
                routine.id,
                format!(
                    "residual-lane-{}-to-{}",
                    tracked.attribution.producer, tracked.attribution.consumer
                ),
            );
            stats.record_dynamic(
                routine.id,
                format!(
                    "residual-lane-decision-{}-to-{fate}",
                    decision_name(&tracked.decision)
                ),
            );
            stats.record_dynamic(
                routine.id,
                format!(
                    "residual-lane-width-{}-to-{fate}",
                    tracked.attribution.width
                ),
            );
            if tracked.attribution.coupled {
                stats.record_dynamic(routine.id, format!("residual-lane-coupled-to-{fate}"));
            }

            let access = home.and_then(|home| {
                surviving_homes.insert(home);
                accesses.get(&home).copied()
            });
            if access.is_some_and(|count| count.writes > 0) {
                stats.record(routine.id, "residual-lane-final-with-stores");
            }
            if access.is_some_and(|count| count.reads > 0) {
                stats.record(routine.id, "residual-lane-final-with-reloads");
            }
            stats.record_site(
                routine.id,
                "residual-lane-final-fate",
                format!(
                    "temp={} byte={} producer={} consumer={} decision={} fate={} reads={} writes={}",
                    lane.id.0,
                    lane.byte,
                    tracked.attribution.producer,
                    tracked.attribution.consumer,
                    decision_name(&tracked.decision),
                    fate,
                    access.map_or(0, |count| count.reads),
                    access.map_or(0, |count| count.writes),
                ),
            );
        }

        stats.record_many(
            routine.id,
            "residual-lane-final-reconciled-lanes",
            reconciled,
        );
        for home in surviving_homes {
            let fate = match home {
                MirHomeStorage::Spill(_) => "ram",
                MirHomeStorage::ZeroPage(_) => "zp",
            };
            stats.record_dynamic(routine.id, format!("residual-home-final-{fate}"));
            let access = accesses.get(&home).copied().unwrap_or_default();
            stats.record_many_dynamic(
                routine.id,
                format!("residual-home-final-{fate}-stores"),
                access.writes,
            );
            stats.record_many_dynamic(
                routine.id,
                format!("residual-home-final-{fate}-reloads"),
                access.reads,
            );
        }
    }
}

fn decision_name(decision: &HomeDecision) -> &'static str {
    match decision {
        HomeDecision::ElideInRegister(MirReg::A) => "elide-a",
        HomeDecision::ElideInRegister(MirReg::X) => "elide-x",
        HomeDecision::ElideInRegister(MirReg::Y) => "elide-y",
        HomeDecision::Rematerialize(_) => "rematerialize",
        HomeDecision::ForwardToConsumer(_) => "forward",
        HomeDecision::MustMaterialize(reason) => reason.name(),
    }
}

struct HomeDemandAnalysis {
    census: HomeDemandCensus,
    plan: HomePlan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct HomeDemandCensus {
    pub(super) temp_lanes: usize,
    pub(super) definitions: usize,
    pub(super) uses: usize,
    pub(super) single_use_lanes: usize,
    pub(super) multi_use_lanes: usize,
    pub(super) same_block_lanes: usize,
    pub(super) cross_block_lanes: usize,
    pub(super) terminator_lanes: usize,
    pub(super) join_live_lanes: usize,
    pub(super) backedge_live_lanes: usize,
    pub(super) call_live_lanes: usize,
    pub(super) machine_live_lanes: usize,
    pub(super) barrier_live_lanes: usize,
    pub(super) natural_a_lanes: usize,
    pub(super) natural_x_lanes: usize,
    pub(super) natural_y_lanes: usize,
    pub(super) coupled_lanes: usize,
    pub(super) blocked_clobber_lanes: usize,
    pub(super) unsupported_consumer_lanes: usize,
    pub(super) same_block_a_eligible_lanes: usize,
    pub(super) same_block_a_candidates: usize,
    pub(super) retained_unused_lanes: usize,
    pub(super) retained_non_single_def_lanes: usize,
    pub(super) retained_multi_use_lanes: usize,
    pub(super) retained_terminator_lanes: usize,
    pub(super) retained_coupled_lanes: usize,
    pub(super) retained_call_live_lanes: usize,
    pub(super) retained_machine_live_lanes: usize,
    pub(super) retained_barrier_live_lanes: usize,
    pub(super) retained_backedge_live_lanes: usize,
    pub(super) retained_join_live_lanes: usize,
    pub(super) retained_cross_block_lanes: usize,
    pub(super) retained_non_accumulator_lanes: usize,
    pub(super) retained_clobber_lanes: usize,
    pub(super) retained_unsupported_consumer_lanes: usize,
    pub(super) retained_profitability_lanes: usize,
    pub(super) gross_store_instructions: usize,
    pub(super) gross_reload_instructions: usize,
    pub(super) gross_absolute_code_bytes: usize,
    pub(super) gross_storage_bytes: usize,
}

pub(super) fn record_home_demand_census(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
    stats: &mut MirPeepholeStats,
) -> HomePlan {
    let analysis = analyze_home_demand(routine, liveness);
    let census = analysis.census;
    let routine_id = routine.id;
    stats.record_many(
        routine_id,
        "home-demand-preexisting-virtual-zp-cells",
        routine.frame.virtual_zero_page.len(),
    );
    for (name, count) in [
        ("home-demand-temp-lanes", census.temp_lanes),
        ("home-demand-definitions", census.definitions),
        ("home-demand-uses", census.uses),
        ("home-demand-single-use-lanes", census.single_use_lanes),
        ("home-demand-multi-use-lanes", census.multi_use_lanes),
        ("home-demand-same-block-lanes", census.same_block_lanes),
        ("home-demand-cross-block-lanes", census.cross_block_lanes),
        ("home-demand-terminator-lanes", census.terminator_lanes),
        ("home-demand-join-live-lanes", census.join_live_lanes),
        (
            "home-demand-backedge-live-lanes",
            census.backedge_live_lanes,
        ),
        ("home-demand-call-live-lanes", census.call_live_lanes),
        ("home-demand-machine-live-lanes", census.machine_live_lanes),
        ("home-demand-barrier-live-lanes", census.barrier_live_lanes),
        ("home-demand-natural-a-lanes", census.natural_a_lanes),
        ("home-demand-natural-x-lanes", census.natural_x_lanes),
        ("home-demand-natural-y-lanes", census.natural_y_lanes),
        ("home-demand-coupled-lanes", census.coupled_lanes),
        (
            "home-demand-blocked-accumulator-clobber-lanes",
            census.blocked_clobber_lanes,
        ),
        (
            "home-demand-unsupported-consumer-lanes",
            census.unsupported_consumer_lanes,
        ),
        (
            "home-demand-same-block-a-eligible-lanes",
            census.same_block_a_eligible_lanes,
        ),
        (
            "home-demand-same-block-a-candidates",
            census.same_block_a_candidates,
        ),
        (
            "home-demand-retained-unused-lanes",
            census.retained_unused_lanes,
        ),
        (
            "home-demand-retained-non-single-def-lanes",
            census.retained_non_single_def_lanes,
        ),
        (
            "home-demand-retained-multi-use-lanes",
            census.retained_multi_use_lanes,
        ),
        (
            "home-demand-retained-terminator-lanes",
            census.retained_terminator_lanes,
        ),
        (
            "home-demand-retained-coupled-lanes",
            census.retained_coupled_lanes,
        ),
        (
            "home-demand-retained-call-live-lanes",
            census.retained_call_live_lanes,
        ),
        (
            "home-demand-retained-machine-live-lanes",
            census.retained_machine_live_lanes,
        ),
        (
            "home-demand-retained-barrier-live-lanes",
            census.retained_barrier_live_lanes,
        ),
        (
            "home-demand-retained-backedge-live-lanes",
            census.retained_backedge_live_lanes,
        ),
        (
            "home-demand-retained-join-live-lanes",
            census.retained_join_live_lanes,
        ),
        (
            "home-demand-retained-cross-block-lanes",
            census.retained_cross_block_lanes,
        ),
        (
            "home-demand-retained-non-accumulator-lanes",
            census.retained_non_accumulator_lanes,
        ),
        (
            "home-demand-retained-clobber-lanes",
            census.retained_clobber_lanes,
        ),
        (
            "home-demand-retained-unsupported-consumer-lanes",
            census.retained_unsupported_consumer_lanes,
        ),
        (
            "home-demand-retained-profitability-lanes",
            census.retained_profitability_lanes,
        ),
        (
            "home-demand-gross-store-instructions",
            census.gross_store_instructions,
        ),
        (
            "home-demand-gross-reload-instructions",
            census.gross_reload_instructions,
        ),
        (
            "home-demand-gross-absolute-code-bytes",
            census.gross_absolute_code_bytes,
        ),
        (
            "home-demand-gross-storage-bytes",
            census.gross_storage_bytes,
        ),
    ] {
        stats.record_many(routine_id, name, count);
    }
    record_home_plan(routine_id, &analysis.plan, stats);
    analysis.plan
}

/// Applies the register-resident subset of a home plan before generic temp
/// materialization. A lane is changed only when both planned endpoints still
/// match, so stale analysis falls back to the existing spill path.
pub(super) fn apply_register_home_plan(
    routine: &mut MirRoutine,
    plan: &HomePlan,
    stats: &mut MirPeepholeStats,
) {
    let routine_id = routine.id;
    for (lane, range) in &plan.register_ranges {
        let Some(HomeDecision::ElideInRegister(reg)) = plan.decisions.get(lane) else {
            continue;
        };
        let Some(block) = routine.blocks.get_mut(range.block) else {
            stats.record(routine_id, "home-elision-stale-plan");
            continue;
        };
        let (Some(producer), Some(consumer)) =
            (block.ops.get(range.def_op), block.ops.get(range.use_op))
        else {
            stats.record(routine_id, "home-elision-stale-plan");
            continue;
        };

        let mut rewritten_producer = producer.clone();
        let mut rewritten_consumer = consumer.clone();
        let producer_kind = op_kind(producer);
        let consumer_kind = op_kind(consumer);
        if !replace_op_def_lane(&mut rewritten_producer, *lane, *reg)
            || replace_op_use_lane(&mut rewritten_consumer, *lane, *reg) != 1
        {
            stats.record(routine_id, "home-elision-stale-plan");
            continue;
        }

        block.ops[range.def_op] = rewritten_producer;
        block.ops[range.use_op] = rewritten_consumer;
        match reg {
            MirReg::A => stats.record(routine_id, "home-elision-register-a-lanes"),
            MirReg::X => stats.record(routine_id, "home-elision-register-x-lanes"),
            MirReg::Y => stats.record(routine_id, "home-elision-register-y-lanes"),
        }
        stats.record_site(
            routine_id,
            "home-elision-register-range",
            format!(
                "block {} ops {}..{}: {producer_kind} -> {consumer_kind} via {reg:?}",
                range.block, range.def_op, range.use_op
            ),
        );
    }
}

fn replace_op_def_lane(op: &mut MirOp, lane: TempLane, reg: MirReg) -> bool {
    let Some(def) = op_def_mut(op) else {
        return false;
    };
    let matches = match def {
        MirDef::VTemp(id) => *id == lane.id && lane.byte == 0,
        MirDef::VTempByte { id, byte } => *id == lane.id && *byte == lane.byte,
        MirDef::Reg(_) => false,
    };
    if matches {
        *def = MirDef::Reg(reg);
    }
    matches
}

fn op_def_mut(op: &mut MirOp) -> Option<&mut MirDef> {
    match op {
        MirOp::Move { dst, .. } | MirOp::Binary { dst, .. } => Some(dst),
        _ => None,
    }
}

fn replace_op_use_lane(op: &mut MirOp, lane: TempLane, reg: MirReg) -> usize {
    let replacement = MirValue::Def(MirDef::Reg(reg));
    match op {
        MirOp::Store {
            src,
            width: MirWidth::Byte,
            ..
        }
        | MirOp::Compare {
            left: src,
            width: MirWidth::Byte,
            ..
        } => replace_value_lane(src, lane, &replacement),
        _ => 0,
    }
}

fn replace_value_lane(value: &mut MirValue, lane: TempLane, replacement: &MirValue) -> usize {
    let matches = match value {
        MirValue::Def(MirDef::VTemp(id)) => *id == lane.id && lane.byte == 0,
        MirValue::Def(MirDef::VTempByte { id, byte }) => *id == lane.id && *byte == lane.byte,
        _ => false,
    };
    if matches {
        *value = replacement.clone();
        return 1;
    }
    match value {
        MirValue::Word { lo, hi } => {
            replace_value_lane(lo, lane, replacement) + replace_value_lane(hi, lane, replacement)
        }
        _ => 0,
    }
}

fn op_kind(op: &MirOp) -> &'static str {
    match op {
        MirOp::LoadImm { .. } => "load-imm",
        MirOp::Load { .. } => "load",
        MirOp::Move { .. } => "move",
        MirOp::LeaAddr { .. } => "lea",
        MirOp::Extend { .. } => "extend",
        MirOp::Truncate { .. } => "truncate",
        MirOp::Unary { .. } => "unary",
        MirOp::Binary { .. } => "binary",
        MirOp::Store { .. } => "store",
        MirOp::Compare { .. } => "compare",
        MirOp::CompareIndirectBytes { .. } => "compare-indirect-bytes",
        MirOp::Call { .. } => "call",
        MirOp::RuntimeHelper { .. } => "runtime-helper",
        MirOp::MaterializeAddress { .. } => "materialize-address",
        MirOp::MaterializeIndexedAddress { .. } => "materialize-indexed-address",
        MirOp::AdvanceAddress { .. } => "advance-address",
        MirOp::LoadIndirect { .. } => "load-indirect",
        MirOp::StoreIndirect { .. } => "store-indirect",
        MirOp::CopyIndirectWord { .. } => "copy-indirect-word",
        MirOp::IndirectByteCompound { .. } => "indirect-byte-compound",
        MirOp::UpdateMem { .. } => "update-mem",
        MirOp::UpdateIndexedMem { .. } => "update-indexed-mem",
        MirOp::AddByteToWordMem { .. } => "add-byte-to-word",
        MirOp::SubByteFromWordMem { .. } => "sub-byte-from-word",
        MirOp::OffsetPointerByIndirectByte { .. } => "offset-pointer-by-indirect-byte",
        MirOp::Barrier { .. } => "barrier",
        MirOp::MachineBlock { .. } => "machine-block",
    }
}

fn record_home_plan(
    routine_id: crate::mir6502::ir::RoutineId,
    plan: &HomePlan,
    stats: &mut MirPeepholeStats,
) {
    stats.record_many(routine_id, "home-plan-temp-lanes", plan.len());
    let mut must_materialize = 0usize;
    for decision in plan.decisions.values() {
        match decision {
            HomeDecision::ElideInRegister(MirReg::A) => {
                stats.record(routine_id, "home-plan-elide-register-a-lanes")
            }
            HomeDecision::ElideInRegister(MirReg::X) => {
                stats.record(routine_id, "home-plan-elide-register-x-lanes")
            }
            HomeDecision::ElideInRegister(MirReg::Y) => {
                stats.record(routine_id, "home-plan-elide-register-y-lanes")
            }
            HomeDecision::Rematerialize(_) => {
                stats.record(routine_id, "home-plan-rematerialize-lanes")
            }
            HomeDecision::ForwardToConsumer(_) => {
                stats.record(routine_id, "home-plan-forward-to-consumer-lanes")
            }
            HomeDecision::MustMaterialize(reason) => {
                must_materialize = must_materialize.saturating_add(1);
                stats.record(routine_id, reason.metric_name());
            }
        }
    }
    stats.record_many(
        routine_id,
        "home-plan-must-materialize-lanes",
        must_materialize,
    );
}

pub(super) fn record_final_home_allocations(program: &MirProgram, stats: &mut MirPeepholeStats) {
    for routine in &program.routines {
        let ram = routine.frame.spills.len();
        let zp = routine.frame.virtual_zero_page.len();
        let preexisting_zp =
            stats.count_for(routine.id, "home-demand-preexisting-virtual-zp-cells");
        let new_zp = zp.saturating_sub(preexisting_zp);
        stats.record_many(routine.id, "home-demand-final-ram-spill-cells", ram);
        stats.record_many(routine.id, "home-demand-final-virtual-zp-cells", zp);
        stats.record_many(routine.id, "home-demand-final-new-virtual-zp-cells", new_zp);
        stats.record_many(
            routine.id,
            "home-demand-final-temp-home-cells",
            ram.saturating_add(new_zp),
        );
        stats.record_many(
            routine.id,
            "home-demand-final-frame-storage-cells",
            ram.saturating_add(zp),
        );
    }
}

#[cfg(test)]
pub(super) fn scan_home_demand_census(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
) -> HomeDemandCensus {
    analyze_home_demand(routine, liveness).census
}

#[cfg(test)]
fn scan_home_plan(routine: &MirRoutine, liveness: &MirTempLiveness) -> HomePlan {
    analyze_home_demand(routine, liveness).plan
}

fn analyze_home_demand(routine: &MirRoutine, liveness: &MirTempLiveness) -> HomeDemandAnalysis {
    let widths = routine_temp_widths(routine);
    let mut facts = BTreeMap::<TempLane, LaneFacts>::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        for param in &block.params {
            for lane in lanes_for_width(param.dest, param.width) {
                facts.entry(lane).or_default().defs.push(DefSite {
                    block: block_index,
                    op: 0,
                    natural_reg: None,
                    coupled: param.width == MirWidth::Word,
                });
            }
        }
        for (op_index, op) in block.ops.iter().enumerate() {
            record_op_defs(op, block_index, op_index, &mut facts);
            record_op_uses(op, block_index, op_index, &widths, &mut facts);
        }
        record_terminator_uses(&block.terminator, block_index, &widths, &mut facts);
    }

    let keys = facts.keys().copied().collect::<BTreeSet<_>>();
    let predecessors = predecessor_counts(routine);
    let backedge_live = live_across_cycle_edges(routine, liveness, &keys);
    let (call_live, machine_live, barrier_live) =
        live_across_barriers(routine, liveness, &widths, &keys);

    let mut census = HomeDemandCensus::default();
    let mut plan = HomePlan::default();
    census.temp_lanes = facts.len();
    for (lane, lane_facts) in &facts {
        census.definitions = census.definitions.saturating_add(lane_facts.defs.len());
        census.uses = census.uses.saturating_add(lane_facts.uses.len());
        census.gross_store_instructions = census
            .gross_store_instructions
            .saturating_add(lane_facts.defs.len());
        census.gross_reload_instructions = census
            .gross_reload_instructions
            .saturating_add(lane_facts.uses.len());
        census.gross_storage_bytes = census.gross_storage_bytes.saturating_add(1);

        match lane_facts.uses.len() {
            1 => census.single_use_lanes = census.single_use_lanes.saturating_add(1),
            2.. => census.multi_use_lanes = census.multi_use_lanes.saturating_add(1),
            _ => {}
        }

        let same_block = lane_facts.defs.len() == 1
            && lane_facts
                .uses
                .iter()
                .all(|site| site.block == lane_facts.defs[0].block);
        if same_block {
            census.same_block_lanes = census.same_block_lanes.saturating_add(1);
        } else if !lane_facts.uses.is_empty() {
            census.cross_block_lanes = census.cross_block_lanes.saturating_add(1);
        }

        let terminator_use = lane_facts.uses.iter().any(|site| site.op.is_none());
        if terminator_use {
            census.terminator_lanes = census.terminator_lanes.saturating_add(1);
        }
        if lane_live_at_join(routine, liveness, &predecessors, *lane) {
            census.join_live_lanes = census.join_live_lanes.saturating_add(1);
        }
        if backedge_live.contains(lane) {
            census.backedge_live_lanes = census.backedge_live_lanes.saturating_add(1);
        }
        if call_live.contains(lane) {
            census.call_live_lanes = census.call_live_lanes.saturating_add(1);
        }
        if machine_live.contains(lane) {
            census.machine_live_lanes = census.machine_live_lanes.saturating_add(1);
        }
        if barrier_live.contains(lane) {
            census.barrier_live_lanes = census.barrier_live_lanes.saturating_add(1);
        }

        let natural_reg = (lane_facts.defs.len() == 1)
            .then(|| lane_facts.defs[0].natural_reg)
            .flatten();
        match natural_reg {
            Some(MirReg::A) => census.natural_a_lanes = census.natural_a_lanes.saturating_add(1),
            Some(MirReg::X) => census.natural_x_lanes = census.natural_x_lanes.saturating_add(1),
            Some(MirReg::Y) => census.natural_y_lanes = census.natural_y_lanes.saturating_add(1),
            None => {}
        }

        let coupled = widths.get(&lane.id) == Some(&MirWidth::Word)
            || lane_facts.defs.iter().any(|site| site.coupled);
        if coupled {
            census.coupled_lanes = census.coupled_lanes.saturating_add(1);
        }

        if lane_facts.uses.is_empty() {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::Unused,
            );
            continue;
        }
        if lane_facts.defs.len() != 1 {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::NonSingleDefinition,
            );
            continue;
        }
        if lane_facts.uses.len() != 1 {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::MultipleUses,
            );
            continue;
        }
        if terminator_use {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::TerminatorUse,
            );
            continue;
        }
        if coupled {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::CoupledLanes,
            );
            continue;
        }
        if call_live.contains(lane) {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::LiveAcrossCall,
            );
            continue;
        }
        if machine_live.contains(lane) {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::LiveAcrossMachineBlock,
            );
            continue;
        }
        if barrier_live.contains(lane) {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::LiveAcrossBarrier,
            );
            continue;
        }
        if backedge_live.contains(lane) {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::LiveAcrossBackedge,
            );
            continue;
        }
        if lane_live_at_join(routine, liveness, &predecessors, *lane) {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::LiveAtJoin,
            );
            continue;
        }
        if !same_block {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::CrossBlock,
            );
            continue;
        }
        if natural_reg != Some(MirReg::A) {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::NonAccumulatorProducer,
            );
            continue;
        }

        let def = lane_facts.defs[0];
        let use_site = lane_facts.uses[0];
        let Some(use_op) = use_site.op else {
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::TerminatorUse,
            );
            continue;
        };
        if accumulator_clobbered_between(routine, def.block, def.op, use_op) {
            census.blocked_clobber_lanes = census.blocked_clobber_lanes.saturating_add(1);
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::AccumulatorClobber,
            );
        } else if !use_site.accepts_a {
            census.unsupported_consumer_lanes = census.unsupported_consumer_lanes.saturating_add(1);
            retain_home(
                &mut census,
                &mut plan,
                *lane,
                HomeMaterializationReason::UnsupportedConsumer,
            );
        } else {
            census.same_block_a_eligible_lanes =
                census.same_block_a_eligible_lanes.saturating_add(1);
            if !register_home_range_is_profitable(routine, def, use_op) {
                retain_home(
                    &mut census,
                    &mut plan,
                    *lane,
                    HomeMaterializationReason::Profitability,
                );
                continue;
            }
            census.same_block_a_candidates = census.same_block_a_candidates.saturating_add(1);
            let previous = plan
                .decisions
                .insert(*lane, HomeDecision::ElideInRegister(MirReg::A));
            debug_assert!(previous.is_none());
            let previous = plan.register_ranges.insert(
                *lane,
                RegisterHomeRange {
                    block: def.block,
                    def_op: def.op,
                    use_op,
                },
            );
            debug_assert!(previous.is_none());
        }
    }
    census.gross_absolute_code_bytes = census
        .gross_store_instructions
        .saturating_add(census.gross_reload_instructions)
        .saturating_mul(3);
    for (lane, lane_facts) in &facts {
        let coupled = widths.get(&lane.id) == Some(&MirWidth::Word)
            || lane_facts.defs.iter().any(|site| site.coupled);
        plan.attributions.insert(
            *lane,
            LaneAttribution {
                producer: lane_producer_kind(routine, *lane, lane_facts),
                consumer: lane_consumer_kind(routine, lane_facts),
                width: if widths.get(&lane.id) == Some(&MirWidth::Word) {
                    "word"
                } else {
                    "byte"
                },
                coupled,
            },
        );
    }
    debug_assert_eq!(plan.len(), census.temp_lanes);
    debug_assert_eq!(plan.attributions.len(), census.temp_lanes);
    HomeDemandAnalysis { census, plan }
}

fn lane_producer_kind(routine: &MirRoutine, lane: TempLane, facts: &LaneFacts) -> &'static str {
    let [def] = facts.defs.as_slice() else {
        return if facts.defs.is_empty() {
            "none"
        } else {
            "multiple"
        };
    };
    let Some(block) = routine.blocks.get(def.block) else {
        return "unknown";
    };
    if block.params.iter().any(|param| param.dest == lane.id) {
        return "block-param";
    }
    block.ops.get(def.op).map_or("unknown", op_kind)
}

fn lane_consumer_kind(routine: &MirRoutine, facts: &LaneFacts) -> &'static str {
    let [use_site] = facts.uses.as_slice() else {
        return if facts.uses.is_empty() {
            "unused"
        } else {
            "multiple"
        };
    };
    let Some(op) = use_site.op else {
        return "terminator";
    };
    routine
        .blocks
        .get(use_site.block)
        .and_then(|block| block.ops.get(op))
        .map_or("unknown", op_kind)
}

fn register_home_range_is_profitable(routine: &MirRoutine, def: DefSite, use_op: usize) -> bool {
    let Some(block) = routine.blocks.get(def.block) else {
        return false;
    };
    let (Some(producer), Some(consumer)) = (block.ops.get(def.op), block.ops.get(use_op)) else {
        return false;
    };
    matches!(
        (producer, consumer),
        (MirOp::Move { .. }, MirOp::Store { .. }) | (MirOp::Binary { .. }, MirOp::Compare { .. })
    )
}

fn retain_home(
    census: &mut HomeDemandCensus,
    plan: &mut HomePlan,
    lane: TempLane,
    reason: HomeMaterializationReason,
) {
    match reason {
        HomeMaterializationReason::Unused => {
            census.retained_unused_lanes = census.retained_unused_lanes.saturating_add(1)
        }
        HomeMaterializationReason::NonSingleDefinition => {
            census.retained_non_single_def_lanes =
                census.retained_non_single_def_lanes.saturating_add(1)
        }
        HomeMaterializationReason::MultipleUses => {
            census.retained_multi_use_lanes = census.retained_multi_use_lanes.saturating_add(1)
        }
        HomeMaterializationReason::TerminatorUse => {
            census.retained_terminator_lanes = census.retained_terminator_lanes.saturating_add(1)
        }
        HomeMaterializationReason::CoupledLanes => {
            census.retained_coupled_lanes = census.retained_coupled_lanes.saturating_add(1)
        }
        HomeMaterializationReason::LiveAcrossCall => {
            census.retained_call_live_lanes = census.retained_call_live_lanes.saturating_add(1)
        }
        HomeMaterializationReason::LiveAcrossMachineBlock => {
            census.retained_machine_live_lanes =
                census.retained_machine_live_lanes.saturating_add(1)
        }
        HomeMaterializationReason::LiveAcrossBarrier => {
            census.retained_barrier_live_lanes =
                census.retained_barrier_live_lanes.saturating_add(1)
        }
        HomeMaterializationReason::LiveAcrossBackedge => {
            census.retained_backedge_live_lanes =
                census.retained_backedge_live_lanes.saturating_add(1)
        }
        HomeMaterializationReason::LiveAtJoin => {
            census.retained_join_live_lanes = census.retained_join_live_lanes.saturating_add(1)
        }
        HomeMaterializationReason::CrossBlock => {
            census.retained_cross_block_lanes = census.retained_cross_block_lanes.saturating_add(1)
        }
        HomeMaterializationReason::NonAccumulatorProducer => {
            census.retained_non_accumulator_lanes =
                census.retained_non_accumulator_lanes.saturating_add(1)
        }
        HomeMaterializationReason::AccumulatorClobber => {
            census.retained_clobber_lanes = census.retained_clobber_lanes.saturating_add(1)
        }
        HomeMaterializationReason::UnsupportedConsumer => {
            census.retained_unsupported_consumer_lanes =
                census.retained_unsupported_consumer_lanes.saturating_add(1)
        }
        HomeMaterializationReason::Profitability => {
            census.retained_profitability_lanes =
                census.retained_profitability_lanes.saturating_add(1)
        }
        HomeMaterializationReason::ObservableStorage => {}
    }
    let previous = plan
        .decisions
        .insert(lane, HomeDecision::MustMaterialize(reason));
    debug_assert!(previous.is_none());
}

fn routine_temp_widths(routine: &MirRoutine) -> BTreeMap<MirTempId, MirWidth> {
    let mut widths = BTreeMap::new();
    for block in &routine.blocks {
        for param in &block.params {
            note_width(&mut widths, param.dest, param.width);
        }
        for (id, width) in collect_temp_widths(&block.ops) {
            note_width(&mut widths, id, width);
        }
        for op in &block.ops {
            if let MirOp::LoadIndirect { dst, .. } = op {
                note_def_width(&mut widths, dst, MirWidth::Byte);
            }
        }
    }
    widths
}

fn note_def_width(widths: &mut BTreeMap<MirTempId, MirWidth>, def: &MirDef, width: MirWidth) {
    match def {
        MirDef::VTemp(id) => note_width(widths, *id, width),
        MirDef::VTempByte { id, byte } => note_width(
            widths,
            *id,
            if *byte == 0 {
                MirWidth::Byte
            } else {
                MirWidth::Word
            },
        ),
        MirDef::Reg(_) => {}
    }
}

fn note_width(widths: &mut BTreeMap<MirTempId, MirWidth>, id: MirTempId, width: MirWidth) {
    widths
        .entry(id)
        .and_modify(|existing| {
            if width == MirWidth::Word {
                *existing = MirWidth::Word;
            }
        })
        .or_insert(width);
}

fn record_op_defs(
    op: &MirOp,
    block: usize,
    op_index: usize,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    let (def, width) = match op {
        MirOp::LoadImm { dst, width, .. }
        | MirOp::Load { dst, width, .. }
        | MirOp::Move { dst, width, .. }
        | MirOp::LeaAddr { dst, width, .. }
        | MirOp::Unary { dst, width, .. }
        | MirOp::Binary { dst, width, .. } => (Some(dst), Some(*width)),
        MirOp::Extend { dst, to_width, .. } => (Some(dst), Some(*to_width)),
        MirOp::Truncate { dst, to_width, .. } => (Some(dst), Some(*to_width)),
        MirOp::LoadIndirect { dst, .. } => (Some(dst), Some(MirWidth::Byte)),
        MirOp::Call {
            result: Some(result),
            ..
        } => (Some(&result.dst), Some(result.width)),
        _ => (None, None),
    };
    if let (Some(def), Some(width)) = (def, width) {
        for lane in def_lanes(def, width) {
            facts.entry(lane).or_default().defs.push(DefSite {
                block,
                op: op_index,
                natural_reg: natural_result_reg(op, lane.byte),
                coupled: op_couples_result_lanes(op),
            });
        }
    }
    if let MirOp::Compare {
        dst: MirCondDest::Temp(id),
        ..
    } = op
    {
        facts
            .entry(TempLane { id: *id, byte: 0 })
            .or_default()
            .defs
            .push(DefSite {
                block,
                op: op_index,
                natural_reg: None,
                coupled: false,
            });
    }
}

fn def_lanes(def: &MirDef, width: MirWidth) -> Vec<TempLane> {
    match def {
        MirDef::VTemp(id) => lanes_for_width(*id, width),
        MirDef::VTempByte { id, byte } => vec![TempLane {
            id: *id,
            byte: *byte,
        }],
        MirDef::Reg(_) => Vec::new(),
    }
}

fn lanes_for_width(id: MirTempId, width: MirWidth) -> Vec<TempLane> {
    match width {
        MirWidth::Byte => vec![TempLane { id, byte: 0 }],
        MirWidth::Word => vec![TempLane { id, byte: 0 }, TempLane { id, byte: 1 }],
    }
}

fn natural_result_reg(op: &MirOp, byte: u8) -> Option<MirReg> {
    match op {
        MirOp::LoadImm { width, .. }
        | MirOp::Load { width, .. }
        | MirOp::Unary { width, .. }
        | MirOp::Binary { width, .. }
            if *width == MirWidth::Byte && byte == 0 =>
        {
            Some(MirReg::A)
        }
        MirOp::Move {
            src: MirValue::Def(MirDef::Reg(reg)),
            width: MirWidth::Byte,
            ..
        } if byte == 0 => Some(*reg),
        MirOp::Move {
            width: MirWidth::Byte,
            ..
        }
        | MirOp::LoadIndirect { .. }
            if byte == 0 =>
        {
            Some(MirReg::A)
        }
        MirOp::Call {
            result: Some(result),
            ..
        } => result_home_reg(&result.home, byte),
        _ => None,
    }
}

fn result_home_reg(home: &MirResultHome, byte: u8) -> Option<MirReg> {
    match home {
        MirResultHome::Reg(reg) if byte == 0 => Some(*reg),
        MirResultHome::RegisterPair { lo, hi } => match byte {
            0 => Some(*lo),
            1 => Some(*hi),
            _ => None,
        },
        _ => None,
    }
}

fn op_couples_result_lanes(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Binary {
            carry_in: Some(MirCarryIn::FromPrevious),
            ..
        } | MirOp::Binary {
            carry_out: MirCarryOut::Produce,
            ..
        } | MirOp::Extend {
            to_width: MirWidth::Word,
            ..
        }
    )
}

fn record_op_uses(
    op: &MirOp,
    block: usize,
    op_index: usize,
    widths: &BTreeMap<MirTempId, MirWidth>,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    match op {
        MirOp::Load { src, .. } => record_addr_uses(src, block, op_index, widths, facts),
        MirOp::Store { dst, src, width } => {
            record_addr_uses(dst, block, op_index, widths, facts);
            record_op_value_use(
                src,
                *width,
                *width == MirWidth::Byte,
                block,
                op_index,
                facts,
            );
        }
        MirOp::Move { src, width, .. } | MirOp::Unary { src, width, .. } => record_op_value_use(
            src,
            *width,
            *width == MirWidth::Byte,
            block,
            op_index,
            facts,
        ),
        MirOp::Extend {
            src, from_width, ..
        }
        | MirOp::Truncate {
            src, from_width, ..
        } => record_op_value_use(
            src,
            *from_width,
            *from_width == MirWidth::Byte,
            block,
            op_index,
            facts,
        ),
        MirOp::Binary {
            left, right, width, ..
        }
        | MirOp::Compare {
            left, right, width, ..
        } => {
            record_op_value_use(
                left,
                *width,
                *width == MirWidth::Byte,
                block,
                op_index,
                facts,
            );
            record_op_value_use(right, *width, false, block, op_index, facts);
        }
        MirOp::AddByteToWordMem { value, .. } | MirOp::SubByteFromWordMem { value, .. } => {
            record_op_value_use(value, MirWidth::Byte, false, block, op_index, facts)
        }
        MirOp::Call { target, args, .. } => {
            if let MirCallTarget::Indirect { target, width } = target {
                record_op_value_use(target, *width, false, block, op_index, facts);
            }
            for arg in args {
                record_op_value_use(
                    &arg.value,
                    arg.width,
                    matches!(arg.home, MirArgHome::Reg(MirReg::A)) && arg.width == MirWidth::Byte,
                    block,
                    op_index,
                    facts,
                );
            }
        }
        MirOp::MaterializeAddress { value, .. } => {
            record_op_value_use(value, MirWidth::Word, false, block, op_index, facts)
        }
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            record_op_value_use(base, MirWidth::Word, false, block, op_index, facts);
            record_op_value_use(
                index,
                inferred_value_width(index, widths),
                false,
                block,
                op_index,
                facts,
            );
        }
        MirOp::AdvanceAddress { index, .. } => record_op_value_use(
            index,
            inferred_value_width(index, widths),
            true,
            block,
            op_index,
            facts,
        ),
        MirOp::StoreIndirect { src, .. } => {
            record_op_value_use(src, MirWidth::Byte, true, block, op_index, facts)
        }
        MirOp::LoadImm { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::OffsetPointerByIndirectByte { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn record_op_value_use(
    value: &MirValue,
    width: MirWidth,
    accepts_a: bool,
    block: usize,
    op: usize,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    record_value_uses(value, width, block, Some(op), accepts_a, facts);
}

fn record_addr_uses(
    addr: &MirAddr,
    block: usize,
    op: usize,
    widths: &BTreeMap<MirTempId, MirWidth>,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            record_value_uses(base, MirWidth::Word, block, Some(op), false, facts);
            record_value_uses(
                index,
                inferred_value_width(index, widths),
                block,
                Some(op),
                false,
                facts,
            );
        }
        MirAddr::PointerIndex { index, .. } => record_value_uses(
            index,
            inferred_value_width(index, widths),
            block,
            Some(op),
            false,
            facts,
        ),
        MirAddr::Deref { ptr, .. } => {
            record_value_uses(ptr, MirWidth::Word, block, Some(op), false, facts)
        }
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::PointerCell { .. } => {}
    }
}

fn inferred_value_width(value: &MirValue, widths: &BTreeMap<MirTempId, MirWidth>) -> MirWidth {
    match value {
        MirValue::ConstU16(_)
        | MirValue::Word { .. }
        | MirValue::Def(MirDef::VTempByte { byte: 1, .. }) => MirWidth::Word,
        MirValue::Def(MirDef::VTemp(id)) => widths.get(id).copied().unwrap_or(MirWidth::Byte),
        _ => MirWidth::Byte,
    }
}

fn record_value_uses(
    value: &MirValue,
    width: MirWidth,
    block: usize,
    op: Option<usize>,
    accepts_a: bool,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    match value {
        MirValue::Def(MirDef::VTemp(id)) => {
            for lane in lanes_for_width(*id, width) {
                facts.entry(lane).or_default().uses.push(UseSite {
                    block,
                    op,
                    accepts_a,
                });
            }
        }
        MirValue::Def(MirDef::VTempByte { id, byte }) => {
            facts
                .entry(TempLane {
                    id: *id,
                    byte: *byte,
                })
                .or_default()
                .uses
                .push(UseSite {
                    block,
                    op,
                    accepts_a,
                });
        }
        MirValue::Word { lo, hi } => {
            record_value_uses(lo, MirWidth::Byte, block, op, accepts_a, facts);
            record_value_uses(hi, MirWidth::Byte, block, op, accepts_a, facts);
        }
        _ => {}
    }
}

fn record_terminator_uses(
    terminator: &MirTerminator,
    block: usize,
    _widths: &BTreeMap<MirTempId, MirWidth>,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    if let MirTerminator::Branch {
        cond: MirCond::BoolValue(value),
        ..
    } = terminator
    {
        record_value_uses(value, MirWidth::Byte, block, None, false, facts);
    }
    match terminator {
        MirTerminator::Jump(edge) => record_edge_uses(edge, block, facts),
        MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            record_edge_uses(then_edge, block, facts);
            record_edge_uses(else_edge, block, facts);
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
    }
}

fn record_edge_uses(
    edge: &crate::mir6502::ir::MirEdge,
    block: usize,
    facts: &mut BTreeMap<TempLane, LaneFacts>,
) {
    for arg in &edge.args {
        record_value_uses(&arg.value, arg.width, block, None, false, facts);
    }
}

fn predecessor_counts(routine: &MirRoutine) -> Vec<usize> {
    let mut predecessors = vec![0usize; routine.blocks.len()];
    for block in &routine.blocks {
        for successor in block_successor_indices(routine, &block.terminator) {
            predecessors[successor] = predecessors[successor].saturating_add(1);
        }
    }
    predecessors
}

fn lane_live_at_join(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
    predecessors: &[usize],
    lane: TempLane,
) -> bool {
    routine.blocks.iter().enumerate().any(|(block_index, _)| {
        predecessors.get(block_index).copied().unwrap_or(0) > 1
            && liveness
                .live_in(block_index)
                .is_some_and(|live| lane_live(live, lane))
    })
}

fn live_across_cycle_edges(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
    keys: &BTreeSet<TempLane>,
) -> BTreeSet<TempLane> {
    let successors = routine
        .blocks
        .iter()
        .map(|block| block_successor_indices(routine, &block.terminator))
        .collect::<Vec<_>>();
    let mut live = BTreeSet::new();
    for (from, targets) in successors.iter().enumerate() {
        for target in targets {
            if !path_exists(&successors, *target, from) {
                continue;
            }
            for lane in keys {
                if liveness
                    .live_out(from)
                    .is_some_and(|set| lane_live(set, *lane))
                    && liveness
                        .live_in(*target)
                        .is_some_and(|set| lane_live(set, *lane))
                {
                    live.insert(*lane);
                }
            }
        }
    }
    live
}

fn path_exists(successors: &[Vec<usize>], start: usize, target: usize) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(block) = pending.pop() {
        if block == target {
            return true;
        }
        if !visited.insert(block) {
            continue;
        }
        pending.extend(successors.get(block).into_iter().flatten().copied());
    }
    false
}

fn live_across_barriers(
    routine: &MirRoutine,
    liveness: &MirTempLiveness,
    widths: &BTreeMap<MirTempId, MirWidth>,
    keys: &BTreeSet<TempLane>,
) -> (BTreeSet<TempLane>, BTreeSet<TempLane>, BTreeSet<TempLane>) {
    let mut call_live = BTreeSet::new();
    let mut machine_live = BTreeSet::new();
    let mut barrier_live = BTreeSet::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        let mut live = keys
            .iter()
            .copied()
            .filter(|lane| {
                liveness
                    .live_out(block_index)
                    .is_some_and(|set| lane_live(set, *lane))
            })
            .collect::<BTreeSet<_>>();
        let mut terminator_facts = BTreeMap::new();
        record_terminator_uses(
            &block.terminator,
            block_index,
            widths,
            &mut terminator_facts,
        );
        live.extend(terminator_facts.into_keys());

        for (op_index, op) in block.ops.iter().enumerate().rev() {
            let live_after = live.clone();
            let mut op_facts = BTreeMap::new();
            record_op_defs(op, block_index, op_index, &mut op_facts);
            record_op_uses(op, block_index, op_index, widths, &mut op_facts);
            for (lane, facts) in &op_facts {
                if !facts.defs.is_empty() {
                    live.remove(lane);
                }
            }
            for (lane, facts) in &op_facts {
                if !facts.uses.is_empty() {
                    live.insert(*lane);
                }
            }
            if matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. }) {
                call_live.extend(live.intersection(&live_after).copied());
            }
            if matches!(op, MirOp::MachineBlock { .. }) {
                machine_live.extend(live.intersection(&live_after).copied());
            }
            if matches!(op, MirOp::Barrier { .. }) {
                barrier_live.extend(live.intersection(&live_after).copied());
            }
        }
    }
    (call_live, machine_live, barrier_live)
}

fn lane_live(set: &MirTempLiveSet, lane: TempLane) -> bool {
    set.full_temp_live(lane.id) || set.exact_lane_live(lane.id, lane.byte)
}

fn accumulator_clobbered_between(
    routine: &MirRoutine,
    block_index: usize,
    def_op: usize,
    use_op: usize,
) -> bool {
    let Some(block) = routine.blocks.get(block_index) else {
        return true;
    };
    if use_op <= def_op {
        return true;
    }
    block.ops[def_op.saturating_add(1)..use_op]
        .iter()
        .any(op_may_clobber_accumulator_during_materialization)
}

fn op_may_clobber_accumulator_during_materialization(op: &MirOp) -> bool {
    if op_may_clobber_reg(op, MirReg::A) {
        return true;
    }
    if matches!(
        op,
        MirOp::Move {
            dst: MirDef::VTemp(_) | MirDef::VTempByte { .. },
            src: MirValue::Def(MirDef::Reg(MirReg::X | MirReg::Y)),
            width: MirWidth::Byte,
        }
    ) {
        return false;
    }
    if op_def(op).is_some_and(|def| matches!(def, MirDef::VTemp(_) | MirDef::VTempByte { .. })) {
        return true;
    }
    match op {
        MirOp::Compare {
            width: MirWidth::Byte,
            ..
        }
        | MirOp::StoreIndirect { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. } => true,
        MirOp::Store {
            src:
                MirValue::ConstU8(_)
                | MirValue::PointerCell(_)
                | MirValue::StorageAddrByte { .. }
                | MirValue::RoutineAddrByte { .. }
                | MirValue::Def(MirDef::VTemp(_) | MirDef::VTempByte { .. }),
            width: MirWidth::Byte,
            ..
        } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirBlockId, MirEdge, MirEffects, MirFrame, MirMem, MirRoutineAbi, MirTemp,
        RoutineId,
    };
    use crate::mir6502::materialize::temp_liveness::analyze_temp_liveness;
    use crate::mir6502::materialize::temps::materialize_temp_ops;

    fn routine(blocks: Vec<MirBlock>, temps: u32) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "HomeCensus".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: (0..temps).map(|id| MirTemp { id: MirTempId(id) }).collect(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn temp(id: u32) -> MirDef {
        MirDef::VTemp(MirTempId(id))
    }

    fn temp_value(id: u32) -> MirValue {
        MirValue::Def(temp(id))
    }

    fn store_temp(id: u32) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
            src: temp_value(id),
            width: MirWidth::Byte,
        }
    }

    fn store_indirect_temp(id: u32) -> MirOp {
        MirOp::StoreIndirect {
            consumer: crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
                crate::mir6502::ir::MirPointerPair::Fixed {
                    lo: crate::mir6502::ir::MirFixedZpSlot(0xAC),
                },
            ),
            src: temp_value(id),
            offset: 0,
        }
    }

    fn store_word_temp(id: u32) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
            src: temp_value(id),
            width: MirWidth::Word,
        }
    }

    fn scan(routine: &MirRoutine) -> HomeDemandCensus {
        let liveness = analyze_temp_liveness(routine);
        let census = scan_home_demand_census(routine, &liveness);
        assert_eq!(
            census.temp_lanes,
            census.same_block_a_candidates
                + census.retained_unused_lanes
                + census.retained_non_single_def_lanes
                + census.retained_multi_use_lanes
                + census.retained_terminator_lanes
                + census.retained_coupled_lanes
                + census.retained_call_live_lanes
                + census.retained_machine_live_lanes
                + census.retained_barrier_live_lanes
                + census.retained_backedge_live_lanes
                + census.retained_join_live_lanes
                + census.retained_cross_block_lanes
                + census.retained_non_accumulator_lanes
                + census.retained_clobber_lanes
                + census.retained_unsupported_consumer_lanes
                + census.retained_profitability_lanes,
            "candidate and retained-home reasons must partition temp lanes"
        );
        census
    }

    fn plan(routine: &MirRoutine) -> HomePlan {
        let liveness = analyze_temp_liveness(routine);
        scan_home_plan(routine, &liveness)
    }

    fn apply_plan(routine: &mut MirRoutine) -> MirPeepholeStats {
        let home_plan = plan(routine);
        let mut stats = MirPeepholeStats::default();
        apply_register_home_plan(routine, &home_plan, &mut stats);
        stats
    }

    #[test]
    fn census_finds_same_block_single_use_accumulator_candidate() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Move {
                        dst: temp(0),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.temp_lanes, 1);
        assert_eq!(census.definitions, 1);
        assert_eq!(census.uses, 1);
        assert_eq!(census.single_use_lanes, 1);
        assert_eq!(census.same_block_lanes, 1);
        assert_eq!(census.natural_a_lanes, 1);
        assert_eq!(census.same_block_a_eligible_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 1);
        assert_eq!(census.gross_absolute_code_bytes, 6);
        assert_eq!(census.gross_storage_bytes, 1);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::ElideInRegister(MirReg::A))
        );
    }

    #[test]
    fn census_retains_safe_but_unprofitable_load_indirect_store_home() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Load {
                        dst: temp(0),
                        src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                        width: MirWidth::Byte,
                    },
                    store_indirect_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.same_block_a_eligible_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
        assert_eq!(census.retained_profitability_lanes, 1);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::Profitability
            ))
        );
    }

    #[test]
    fn register_home_plan_elides_byte_move_store_home() {
        let mut routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Move {
                        dst: temp(0),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        apply_plan(&mut routine);
        let ops = materialize_temp_ops(
            std::mem::take(&mut routine.blocks[0].ops),
            &mut routine.frame.spills,
        );

        assert_eq!(ops.len(), 1, "the redundant A-to-A move is removed");
        assert!(matches!(
            ops[0],
            MirOp::Store {
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                ..
            }
        ));
        assert!(routine.frame.spills.is_empty());
    }

    #[test]
    fn register_home_plan_keeps_byte_binary_compare_in_accumulator() {
        let mut routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Binary {
                        op: crate::mir6502::ir::MirBinaryOp::Add,
                        dst: temp(0),
                        left: MirValue::Def(MirDef::Reg(MirReg::A)),
                        right: MirValue::ConstU8(1),
                        width: MirWidth::Byte,
                        carry_in: Some(MirCarryIn::Clear),
                        carry_out: MirCarryOut::Ignore,
                    },
                    MirOp::Compare {
                        dst: MirCondDest::Flags,
                        op: crate::mir6502::ir::MirCompareOp::Eq,
                        left: temp_value(0),
                        right: MirValue::ConstU8(2),
                        width: MirWidth::Byte,
                        signed: false,
                    },
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let stats = apply_plan(&mut routine);
        let ops = materialize_temp_ops(
            std::mem::take(&mut routine.blocks[0].ops),
            &mut routine.frame.spills,
        );

        assert!(matches!(
            ops[0],
            MirOp::Binary {
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                ..
            }
        ));
        assert!(matches!(
            ops[1],
            MirOp::Compare {
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                ..
            }
        ));
        assert!(routine.frame.spills.is_empty());
        assert_eq!(
            stats
                .aggregate_counts()
                .get("home-elision-register-a-lanes"),
            Some(&1)
        );
    }

    #[test]
    fn register_home_binary_still_legalizes_pointer_cell_source() {
        let mut routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Binary {
                        op: crate::mir6502::ir::MirBinaryOp::Xor,
                        dst: temp(0),
                        left: MirValue::PointerCell(MirMem::Global {
                            id: crate::nir::SymbolId(0),
                            offset: 0,
                        }),
                        right: MirValue::ConstU8(1),
                        width: MirWidth::Byte,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    },
                    MirOp::Compare {
                        dst: MirCondDest::Flags,
                        op: crate::mir6502::ir::MirCompareOp::Eq,
                        left: temp_value(0),
                        right: MirValue::ConstU8(2),
                        width: MirWidth::Byte,
                        signed: false,
                    },
                ],
                MirTerminator::Return,
            )],
            1,
        );

        apply_plan(&mut routine);
        let ops = materialize_temp_ops(
            std::mem::take(&mut routine.blocks[0].ops),
            &mut routine.frame.spills,
        );

        assert_eq!(ops.len(), 3);
        assert!(matches!(
            ops[0],
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                ..
            }
        ));
        assert!(matches!(
            ops[1],
            MirOp::Binary {
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                ..
            }
        ));
        assert!(matches!(
            ops[2],
            MirOp::Compare {
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                ..
            }
        ));
        assert!(routine.frame.spills.is_empty());
    }

    #[test]
    fn census_retains_safe_but_unprofitable_accumulator_call_argument_home() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadIndirect {
                        consumer: crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
                            crate::mir6502::ir::MirPointerPair::Fixed {
                                lo: crate::mir6502::ir::MirFixedZpSlot(0xAC),
                            },
                        ),
                        dst: temp(0),
                        offset: 0,
                    },
                    MirOp::Move {
                        dst: MirDef::Reg(MirReg::A),
                        src: temp_value(0),
                        width: MirWidth::Byte,
                    },
                    MirOp::Call {
                        target: MirCallTarget::Builtin {
                            name: "consume_a".to_string(),
                            address: Some(0x5000),
                        },
                        abi: crate::mir6502::ir::MirCallAbi {
                            params: vec![MirArgHome::Reg(MirReg::A)],
                            result: None,
                            clobbers: Default::default(),
                            preserves: Default::default(),
                        },
                        args: vec![crate::mir6502::ir::MirCallArg {
                            value: MirValue::Def(MirDef::Reg(MirReg::A)),
                            width: MirWidth::Byte,
                            home: MirArgHome::Reg(MirReg::A),
                        }],
                        result: None,
                        effects: MirEffects::default(),
                    },
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.same_block_a_eligible_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
        assert_eq!(census.retained_profitability_lanes, 1);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::Profitability
            ))
        );
    }

    #[test]
    fn register_home_plan_leaves_clobbered_candidate_materialized() {
        let original_ops = vec![
            MirOp::LoadImm {
                dst: temp(0),
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: 9,
                width: MirWidth::Byte,
            },
            store_temp(0),
        ];
        let mut routine = routine(
            vec![block(0, original_ops.clone(), MirTerminator::Return)],
            1,
        );

        let stats = apply_plan(&mut routine);

        assert_eq!(routine.blocks[0].ops, original_ops);
        assert_eq!(
            stats
                .aggregate_counts()
                .get("home-elision-register-a-lanes"),
            None
        );
    }

    #[test]
    fn register_home_plan_falls_back_atomically_when_consumer_is_stale() {
        let mut routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Move {
                        dst: temp(0),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );
        let home_plan = plan(&routine);
        let MirOp::Store { src, .. } = &mut routine.blocks[0].ops[1] else {
            unreachable!()
        };
        *src = MirValue::ConstU8(1);
        let mut stats = MirPeepholeStats::default();

        apply_register_home_plan(&mut routine, &home_plan, &mut stats);

        assert!(matches!(
            routine.blocks[0].ops[0],
            MirOp::Move {
                dst: MirDef::VTemp(MirTempId(0)),
                ..
            }
        ));
        assert_eq!(
            stats.aggregate_counts().get("home-elision-stale-plan"),
            Some(&1)
        );
    }

    #[test]
    fn census_records_aggregate_reporting_counters() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Move {
                        dst: temp(0),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );
        let liveness = analyze_temp_liveness(&routine);
        let mut stats = MirPeepholeStats::default();

        record_home_demand_census(&routine, &liveness, &mut stats);

        let counts = stats.aggregate_counts();
        assert_eq!(counts.get("home-demand-temp-lanes"), Some(&1));
        assert_eq!(
            counts.get("home-demand-same-block-a-eligible-lanes"),
            Some(&1)
        );
        assert_eq!(counts.get("home-demand-same-block-a-candidates"), Some(&1));
        assert_eq!(
            counts.get("home-demand-gross-absolute-code-bytes"),
            Some(&6)
        );
        assert_eq!(counts.get("home-plan-temp-lanes"), Some(&1));
        assert_eq!(counts.get("home-plan-elide-register-a-lanes"), Some(&1));
    }

    #[test]
    fn final_fate_tracker_attributes_zero_page_home_and_accesses() {
        let mut routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Load {
                        dst: temp(0),
                        src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );
        let plan = plan(&routine);
        let mut tracker = HomeFateTracker::from_plan(&plan);
        let slot = MirZpSlot(0);
        tracker.apply_zero_page_remap(&BTreeMap::from([(MirSpillId(0), slot)]));
        routine.frame.virtual_zero_page.push(slot);
        routine.blocks[0].ops = vec![
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::ZeroPage(slot)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::ZeroPage(slot)),
                width: MirWidth::Byte,
            },
        ];
        let mut stats = MirPeepholeStats::default();

        tracker.record_final_fates(&routine, &mut stats);

        let counts = stats.aggregate_counts();
        assert_eq!(counts.get("residual-lane-final-reconciled-lanes"), Some(&1));
        assert_eq!(counts.get("residual-lane-final-zp"), Some(&1));
        assert_eq!(counts.get("residual-home-final-zp"), Some(&1));
        assert_eq!(counts.get("residual-home-final-zp-stores"), Some(&1));
        assert_eq!(counts.get("residual-home-final-zp-reloads"), Some(&1));
        assert_eq!(counts.get("residual-lane-load-to-store"), Some(&1));
        assert_eq!(
            counts.get("residual-lane-decision-profitability-to-zp"),
            Some(&1)
        );
    }

    #[test]
    fn census_retains_safe_but_unprofitable_direct_load_store_home() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Load {
                        dst: temp(0),
                        src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.same_block_a_eligible_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
        assert_eq!(census.retained_profitability_lanes, 1);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::Profitability
            ))
        );
    }

    #[test]
    fn census_reports_accumulator_clobber_before_single_use() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::LoadImm {
                        dst: MirDef::Reg(MirReg::A),
                        value: 9,
                        width: MirWidth::Byte,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.blocked_clobber_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::AccumulatorClobber
            ))
        );
    }

    #[test]
    fn census_treats_implicit_temp_materialization_as_accumulator_clobber() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::LoadIndirect {
                        consumer: crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
                            crate::mir6502::ir::MirPointerPair::Fixed {
                                lo: crate::mir6502::ir::MirFixedZpSlot(0xAC),
                            },
                        ),
                        dst: temp(1),
                        offset: 0,
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            2,
        );

        let census = scan(&routine);

        assert_eq!(census.blocked_clobber_lanes, 1);
        assert_eq!(census.retained_unused_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::AccumulatorClobber
            ))
        );
    }

    #[test]
    fn census_blocks_accumulator_home_across_pointer_materialization() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::Load {
                        dst: temp(0),
                        src: MirAddr::Direct(MirMem::Absolute(0x005C)),
                        width: MirWidth::Byte,
                    },
                    MirOp::MaterializeAddress {
                        consumer: crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
                            crate::mir6502::ir::MirPointerPair::Fixed {
                                lo: crate::mir6502::ir::MirFixedZpSlot(0xAC),
                            },
                        ),
                        value: MirValue::Word {
                            lo: Box::new(MirValue::PointerCell(MirMem::Absolute(0x3000))),
                            hi: Box::new(MirValue::PointerCell(MirMem::Absolute(0x3001))),
                        },
                    },
                    store_indirect_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.blocked_clobber_lanes, 1);
        assert_eq!(census.same_block_a_eligible_lanes, 0);
        assert_eq!(census.same_block_a_candidates, 0);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::AccumulatorClobber
            ))
        );
    }

    #[test]
    fn census_tracks_values_live_across_calls_machine_blocks_and_barriers() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::Call {
                        target: MirCallTarget::Builtin {
                            name: "noop".to_string(),
                            address: Some(0x4000),
                        },
                        abi: crate::mir6502::ir::MirCallAbi {
                            params: Vec::new(),
                            result: None,
                            clobbers: Default::default(),
                            preserves: Default::default(),
                        },
                        args: Vec::new(),
                        result: None,
                        effects: MirEffects::default(),
                    },
                    MirOp::MachineBlock {
                        id: crate::mir6502::ir::MirMachineBlockId(0),
                        effects: MirEffects::default(),
                    },
                    MirOp::Barrier {
                        effects: MirEffects::default(),
                    },
                    store_temp(0),
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.call_live_lanes, 1);
        assert_eq!(census.machine_live_lanes, 1);
        assert_eq!(census.barrier_live_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::LiveAcrossCall
            ))
        );
    }

    #[test]
    fn census_tracks_join_backedge_and_terminator_liveness() {
        let routine = routine(
            vec![
                block(
                    0,
                    vec![MirOp::LoadImm {
                        dst: temp(0),
                        value: 1,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Branch {
                        cond: MirCond::BoolValue(temp_value(0)),
                        then_edge: MirEdge::plain(MirBlockId(1)),
                        else_edge: MirEdge::plain(MirBlockId(2)),
                    },
                ),
                block(
                    1,
                    Vec::new(),
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
                block(
                    2,
                    Vec::new(),
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
                block(
                    3,
                    vec![store_temp(0)],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
            ],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.cross_block_lanes, 1);
        assert_eq!(census.terminator_lanes, 1);
        assert_eq!(census.join_live_lanes, 1);
        assert_eq!(census.backedge_live_lanes, 1);
    }

    #[test]
    fn census_tracks_word_coupling_and_non_accumulator_producers() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 0x1234,
                        width: MirWidth::Word,
                    },
                    store_word_temp(0),
                    MirOp::Move {
                        dst: temp(1),
                        src: MirValue::Def(MirDef::Reg(MirReg::X)),
                        width: MirWidth::Byte,
                    },
                    store_temp(1),
                ],
                MirTerminator::Return,
            )],
            2,
        );

        let census = scan(&routine);

        assert_eq!(census.temp_lanes, 3);
        assert_eq!(census.coupled_lanes, 2);
        assert_eq!(census.natural_x_lanes, 1);
        assert_eq!(census.same_block_a_candidates, 0);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::CoupledLanes
            ))
        );
        assert_eq!(
            plan(&routine).decision(MirTempId(1), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::NonAccumulatorProducer
            ))
        );
    }

    #[test]
    fn census_reports_unsupported_accumulator_consumer() {
        let routine = routine(
            vec![block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: temp(0),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::Binary {
                        op: crate::mir6502::ir::MirBinaryOp::And,
                        dst: MirDef::Reg(MirReg::A),
                        left: MirValue::ConstU8(0xff),
                        right: temp_value(0),
                        width: MirWidth::Byte,
                        carry_in: None,
                        carry_out: MirCarryOut::Ignore,
                    },
                ],
                MirTerminator::Return,
            )],
            1,
        );

        let census = scan(&routine);

        assert_eq!(census.unsupported_consumer_lanes, 1);
        assert_eq!(census.coupled_lanes, 0);
        assert_eq!(
            plan(&routine).decision(MirTempId(0), 0),
            Some(&HomeDecision::MustMaterialize(
                HomeMaterializationReason::UnsupportedConsumer
            ))
        );
    }
}
