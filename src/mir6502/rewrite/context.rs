#![allow(dead_code)] // Matchers migrate to this facade in later slices.

use std::cell::RefCell;
use std::collections::BTreeSet;

use crate::mir6502::analysis::effects::{MirFlagSet, MirHomeByte};
use crate::mir6502::analysis::home_liveness::MirHomeLivenessError;
use crate::mir6502::analysis::machine_liveness::MirMachineLivenessError;
use crate::mir6502::analysis::param_availability::{MirParamAvailabilityError, MirParamHomeByte};
use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
use crate::mir6502::analysis::reaching_defs::MirReachingDefinitionError;
use crate::mir6502::analysis::sites::{
    MirProgramPoint, MirProgramPointError, MirRoutineGeneration, MirSite,
};
use crate::mir6502::analysis::use_def::{MirDefSite, MirTempLane, MirUseSite};
use crate::mir6502::ir::{
    MirAddressConsumer, MirBlockId, MirFixedZpSlot, MirPointerPair, MirReg, MirRegisterSet,
    MirTempId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirProof<T> {
    Proven(T),
    Blocked(MirProofBlocker),
}

impl<T> MirProof<T> {
    pub(in crate::mir6502) fn is_proven(&self) -> bool {
        matches!(self, Self::Proven(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirProofBlocker {
    InvalidPoint(MirProgramPointError),
    ReachingDefinitions(MirReachingDefinitionError),
    RequirementDoesNotUseLane(MirTempLane),
    NoReachingDefinition(MirTempLane),
    MultipleReachingDefinitions {
        lane: MirTempLane,
        count: usize,
    },
    DefinitionDoesNotDominateUse {
        definition: MirDefSite,
        usage: MirUseSite,
    },
    DefinitionUnavailable {
        definition: MirDefSite,
        point: MirSite,
    },
    InvalidWindow {
        definition: MirDefSite,
        end: MirSite,
    },
    UseOutsideWindow(MirUseSite),
    HomeLiveness(MirHomeLivenessError),
    MachineLiveness(MirMachineLivenessError),
    ParamAvailability(MirParamAvailabilityError),
    HomeDefinitionLive {
        home: MirHomeByte,
        store: MirSite,
        end: MirSite,
    },
    HomeLive {
        home: MirHomeByte,
        point: MirSite,
    },
    RegisterLive {
        reg: MirReg,
        point: MirSite,
    },
    StackPointerLive {
        point: MirSite,
    },
    FlagsLive {
        flags: MirFlagSet,
        point: MirSite,
    },
    ParameterRegisterUnavailable {
        home: MirParamHomeByte,
        point: MirSite,
    },
    UnsupportedPointerPair(MirAddressConsumer),
}

impl MirProofBlocker {
    pub(in crate::mir6502) fn category(&self) -> &'static str {
        match self {
            Self::InvalidPoint(_) => "invalid-point",
            Self::ReachingDefinitions(_) => "reaching-definitions",
            Self::RequirementDoesNotUseLane(_) => "requirement-does-not-use-lane",
            Self::NoReachingDefinition(_) => "no-reaching-definition",
            Self::MultipleReachingDefinitions { .. } => "multiple-reaching-definitions",
            Self::DefinitionDoesNotDominateUse { .. } => "definition-does-not-dominate-use",
            Self::DefinitionUnavailable { .. } => "definition-unavailable",
            Self::InvalidWindow { .. } => "invalid-window",
            Self::UseOutsideWindow(_) => "use-outside-window",
            Self::HomeLiveness(_) => "home-liveness-error",
            Self::MachineLiveness(_) => "machine-liveness-error",
            Self::ParamAvailability(_) => "parameter-availability-error",
            Self::HomeDefinitionLive { .. } => "home-definition-live",
            Self::HomeLive { .. } => "home-live",
            Self::RegisterLive { .. } => "register-live",
            Self::StackPointerLive { .. } => "stack-pointer-live",
            Self::FlagsLive { .. } => "flags-live",
            Self::ParameterRegisterUnavailable { .. } => "parameter-register-unavailable",
            Self::UnsupportedPointerPair(_) => "unsupported-pointer-pair",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::mir6502) struct MirBlockedRewriteSite {
    pub stat: &'static str,
    pub block: MirBlockId,
    pub op_index: usize,
    pub reason: &'static str,
}

/// Machine and private-home locations whose final values differ between an
/// original rewrite window and its replacement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirExitStateChange {
    pub registers: MirRegisterSet,
    pub flags: MirFlagSet,
    pub homes: BTreeSet<MirHomeByte>,
}

/// Read-only query facade. Matchers receive this type instead of raw analysis
/// maps, so proof policy and blocker accounting remain centralized.
#[derive(Debug, Clone, Copy)]
pub(in crate::mir6502) struct PreHomeRewriteContext<'snapshot, 'routine> {
    snapshot: &'snapshot PreHomeAnalysisSnapshot<'routine>,
}

impl<'snapshot, 'routine> PreHomeRewriteContext<'snapshot, 'routine> {
    pub(in crate::mir6502) fn new(snapshot: &'snapshot PreHomeAnalysisSnapshot<'routine>) -> Self {
        Self { snapshot }
    }

    pub(in crate::mir6502) fn generation(&self) -> MirRoutineGeneration {
        self.snapshot.routine().generation()
    }

    pub(in crate::mir6502) fn point(&self, site: MirSite) -> MirProgramPoint {
        self.snapshot.routine().point(site)
    }

    pub(in crate::mir6502) fn unique_reaching_definition(
        &self,
        usage: MirUseSite,
        lane: MirTempLane,
    ) -> MirProof<MirDefSite> {
        if !usage.requirement.requires(lane) {
            return MirProof::Blocked(MirProofBlocker::RequirementDoesNotUseLane(lane));
        }
        let definitions = match self
            .snapshot
            .reaching_definitions()
            .definitions_reaching_site(usage.site, lane)
        {
            Ok(definitions) => definitions,
            Err(error) => {
                return MirProof::Blocked(MirProofBlocker::ReachingDefinitions(error));
            }
        };
        let Some(definition) = (definitions.len() == 1)
            .then(|| *definitions.first().expect("one reaching definition"))
        else {
            return MirProof::Blocked(if definitions.is_empty() {
                MirProofBlocker::NoReachingDefinition(lane)
            } else {
                MirProofBlocker::MultipleReachingDefinitions {
                    lane,
                    count: definitions.len(),
                }
            });
        };
        if !self
            .snapshot
            .dominance()
            .definition_dominates_use(definition, usage)
        {
            return MirProof::Blocked(MirProofBlocker::DefinitionDoesNotDominateUse {
                definition,
                usage,
            });
        }
        MirProof::Proven(definition)
    }

    pub(in crate::mir6502) fn definition_reaches_use(
        &self,
        definition: MirDefSite,
        usage: MirUseSite,
    ) -> MirProof<()> {
        match self
            .snapshot
            .reaching_definitions()
            .definition_reaches_use(definition, usage)
        {
            Ok(true) => MirProof::Proven(()),
            Ok(false) => MirProof::Blocked(MirProofBlocker::DefinitionUnavailable {
                definition,
                point: usage.site,
            }),
            Err(error) => MirProof::Blocked(MirProofBlocker::ReachingDefinitions(error)),
        }
    }

    pub(in crate::mir6502) fn value_available_at(
        &self,
        definition: MirDefSite,
        point: MirProgramPoint,
    ) -> MirProof<()> {
        if let Err(error) = self.snapshot.routine().validate_point(point) {
            return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
        }
        let available = self
            .snapshot
            .reaching_definitions()
            .value_available_at(definition, point.site);
        match available {
            Ok(true)
                if self
                    .snapshot
                    .dominance()
                    .site_dominates(definition.site, point.site) =>
            {
                MirProof::Proven(())
            }
            Ok(_) => MirProof::Blocked(MirProofBlocker::DefinitionUnavailable {
                definition,
                point: point.site,
            }),
            Err(error) => MirProof::Blocked(MirProofBlocker::ReachingDefinitions(error)),
        }
    }

    pub(in crate::mir6502) fn temp_definition_dead_after(
        &self,
        definition: MirDefSite,
        window_end: MirProgramPoint,
    ) -> MirProof<()> {
        if let Err(error) = self.snapshot.routine().validate_point(window_end) {
            return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
        }
        if !valid_window(definition.site, window_end.site) {
            return MirProof::Blocked(MirProofBlocker::InvalidWindow {
                definition,
                end: window_end.site,
            });
        }

        for usage in self.snapshot.use_def().uses_of_lane(definition.lane) {
            let reaches = self
                .snapshot
                .reaching_definitions()
                .definition_reaches_use(definition, *usage);
            match reaches {
                Ok(false) => continue,
                Err(error) => {
                    return MirProof::Blocked(MirProofBlocker::ReachingDefinitions(error));
                }
                Ok(true) if use_is_inside_window(*usage, definition.site, window_end.site) => {
                    continue;
                }
                Ok(true) => {
                    return MirProof::Blocked(MirProofBlocker::UseOutsideWindow(*usage));
                }
            }
        }
        MirProof::Proven(())
    }

    pub(in crate::mir6502) fn temp_live_out(&self, block: MirBlockId, lane: MirTempLane) -> bool {
        self.snapshot
            .temp_liveness()
            .block_by_id(block)
            .is_some_and(|facts| {
                facts.live_out.exact_lane_live(lane.temp, lane.byte)
                    || facts.live_out.full_temp_live(lane.temp)
            })
    }

    pub(in crate::mir6502) fn definitions_at(
        &self,
        temp: MirTempId,
        site: MirSite,
    ) -> Vec<MirDefSite> {
        self.snapshot
            .use_def()
            .definitions_of_temp(temp)
            .copied()
            .filter(|definition| definition.site == site)
            .collect()
    }

    pub(in crate::mir6502) fn uses_at(&self, temp: MirTempId, site: MirSite) -> Vec<MirUseSite> {
        self.snapshot
            .use_def()
            .uses_of_temp(temp)
            .iter()
            .copied()
            .filter(|usage| usage.site == site)
            .collect()
    }

    pub(in crate::mir6502) fn parameter_register_at(
        &self,
        home: MirParamHomeByte,
        point: MirProgramPoint,
    ) -> MirProof<MirReg> {
        if let Err(error) = self.snapshot.routine().validate_point(point) {
            return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
        }
        match self
            .snapshot
            .param_availability()
            .register_at(home, point.site)
        {
            Ok(Some(reg)) => MirProof::Proven(reg),
            Ok(None) => MirProof::Blocked(MirProofBlocker::ParameterRegisterUnavailable {
                home,
                point: point.site,
            }),
            Err(error) => MirProof::Blocked(MirProofBlocker::ParamAvailability(error)),
        }
    }
}

fn register_set_contains(registers: MirRegisterSet, reg: MirReg) -> bool {
    match reg {
        MirReg::A => registers.a,
        MirReg::X => registers.x,
        MirReg::Y => registers.y,
    }
}

/// Read-only post-home query facade. Temp identity queries are intentionally
/// absent so matchers cannot accidentally cross the materialization boundary.
#[derive(Debug)]
pub(in crate::mir6502) struct PostHomeRewriteContext<'snapshot, 'routine> {
    snapshot: &'snapshot PostHomeAnalysisSnapshot<'routine>,
    blocked_sites: RefCell<Vec<MirBlockedRewriteSite>>,
}

impl<'snapshot, 'routine> PostHomeRewriteContext<'snapshot, 'routine> {
    pub(in crate::mir6502) fn new(snapshot: &'snapshot PostHomeAnalysisSnapshot<'routine>) -> Self {
        Self {
            snapshot,
            blocked_sites: RefCell::new(Vec::new()),
        }
    }

    pub(in crate::mir6502) fn generation(&self) -> MirRoutineGeneration {
        self.snapshot.routine().generation()
    }

    pub(in crate::mir6502) fn point(&self, site: MirSite) -> MirProgramPoint {
        self.snapshot.routine().point(site)
    }

    pub(in crate::mir6502) fn record_blocker(
        &self,
        stat: &'static str,
        block: MirBlockId,
        op_index: usize,
        blocker: &MirProofBlocker,
    ) {
        self.blocked_sites.borrow_mut().push(MirBlockedRewriteSite {
            stat,
            block,
            op_index,
            reason: blocker.category(),
        });
    }

    pub(in crate::mir6502) fn take_blocked_sites(&self) -> Vec<MirBlockedRewriteSite> {
        std::mem::take(&mut *self.blocked_sites.borrow_mut())
    }

    pub(in crate::mir6502) fn home_definition_dead_after(
        &self,
        home: MirHomeByte,
        store: MirProgramPoint,
        window_end: MirProgramPoint,
    ) -> MirProof<()> {
        for point in [store, window_end] {
            if let Err(error) = self.snapshot.routine().validate_point(point) {
                return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
            }
        }
        match self.snapshot.home_liveness().home_definition_dead_after(
            home,
            store.site,
            window_end.site,
        ) {
            Ok(true) => MirProof::Proven(()),
            Ok(false) => MirProof::Blocked(MirProofBlocker::HomeDefinitionLive {
                home,
                store: store.site,
                end: window_end.site,
            }),
            Err(error) => MirProof::Blocked(MirProofBlocker::HomeLiveness(error)),
        }
    }

    pub(in crate::mir6502) fn register_dead_after(
        &self,
        reg: MirReg,
        point: MirProgramPoint,
    ) -> MirProof<()> {
        if let Err(error) = self.snapshot.routine().validate_point(point) {
            return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
        }
        match self
            .snapshot
            .machine_liveness()
            .register_dead_after(reg, point.site)
        {
            Ok(true) => MirProof::Proven(()),
            Ok(false) => MirProof::Blocked(MirProofBlocker::RegisterLive {
                reg,
                point: point.site,
            }),
            Err(error) => MirProof::Blocked(MirProofBlocker::MachineLiveness(error)),
        }
    }

    pub(in crate::mir6502) fn flags_dead_after(
        &self,
        flags: MirFlagSet,
        point: MirProgramPoint,
    ) -> MirProof<()> {
        if let Err(error) = self.snapshot.routine().validate_point(point) {
            return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
        }
        match self
            .snapshot
            .machine_liveness()
            .flags_dead_after(flags, point.site)
        {
            Ok(true) => MirProof::Proven(()),
            Ok(false) => MirProof::Blocked(MirProofBlocker::FlagsLive {
                flags,
                point: point.site,
            }),
            Err(error) => MirProof::Blocked(MirProofBlocker::MachineLiveness(error)),
        }
    }

    pub(in crate::mir6502) fn pointer_pair_dead_after(
        &self,
        consumer: MirAddressConsumer,
        point: MirProgramPoint,
    ) -> MirProof<()> {
        if let Err(error) = self.snapshot.routine().validate_point(point) {
            return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
        }
        let MirPointerPair::Fixed { lo } = consumer.pointer_pair() else {
            return MirProof::Blocked(MirProofBlocker::UnsupportedPointerPair(consumer));
        };
        for slot in [lo, MirFixedZpSlot(lo.0.saturating_add(1))] {
            let home = MirHomeByte::FixedZeroPage(slot);
            match self.snapshot.home_liveness().live_after(home, point.site) {
                Ok(false) => {}
                Ok(true) => {
                    return MirProof::Blocked(MirProofBlocker::HomeLive {
                        home,
                        point: point.site,
                    });
                }
                Err(error) => {
                    return MirProof::Blocked(MirProofBlocker::HomeLiveness(error));
                }
            }
        }
        MirProof::Proven(())
    }

    pub(in crate::mir6502) fn parameter_register_at(
        &self,
        home: MirParamHomeByte,
        point: MirProgramPoint,
    ) -> MirProof<MirReg> {
        if let Err(error) = self.snapshot.routine().validate_point(point) {
            return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
        }
        match self
            .snapshot
            .param_availability()
            .register_at(home, point.site)
        {
            Ok(Some(reg)) => MirProof::Proven(reg),
            Ok(None) => MirProof::Blocked(MirProofBlocker::ParameterRegisterUnavailable {
                home,
                point: point.site,
            }),
            Err(error) => MirProof::Blocked(MirProofBlocker::ParamAvailability(error)),
        }
    }

    pub(in crate::mir6502) fn exit_state_change_is_unobservable(
        &self,
        change: &MirExitStateChange,
        point: MirProgramPoint,
    ) -> MirProof<()> {
        if let Err(error) = self.snapshot.routine().validate_point(point) {
            return MirProof::Blocked(MirProofBlocker::InvalidPoint(error));
        }
        for reg in [MirReg::A, MirReg::X, MirReg::Y] {
            if register_set_contains(change.registers, reg) {
                match self
                    .snapshot
                    .machine_liveness()
                    .register_dead_after(reg, point.site)
                {
                    Ok(true) => {}
                    Ok(false) => {
                        return MirProof::Blocked(MirProofBlocker::RegisterLive {
                            reg,
                            point: point.site,
                        });
                    }
                    Err(error) => {
                        return MirProof::Blocked(MirProofBlocker::MachineLiveness(error));
                    }
                }
            }
        }
        if change.registers.sp {
            match self
                .snapshot
                .machine_liveness()
                .stack_pointer_dead_after(point.site)
            {
                Ok(true) => {}
                Ok(false) => {
                    return MirProof::Blocked(MirProofBlocker::StackPointerLive {
                        point: point.site,
                    });
                }
                Err(error) => {
                    return MirProof::Blocked(MirProofBlocker::MachineLiveness(error));
                }
            }
        }
        let changed_flags = if change.registers.flags {
            MirFlagSet::all()
        } else {
            change.flags
        };
        match self
            .snapshot
            .machine_liveness()
            .flags_dead_after(changed_flags, point.site)
        {
            Ok(true) => {}
            Ok(false) => {
                return MirProof::Blocked(MirProofBlocker::FlagsLive {
                    flags: changed_flags,
                    point: point.site,
                });
            }
            Err(error) => {
                return MirProof::Blocked(MirProofBlocker::MachineLiveness(error));
            }
        }
        for home in &change.homes {
            match self.snapshot.home_liveness().live_after(*home, point.site) {
                Ok(false) => {}
                Ok(true) => {
                    return MirProof::Blocked(MirProofBlocker::HomeLive {
                        home: *home,
                        point: point.site,
                    });
                }
                Err(error) => {
                    return MirProof::Blocked(MirProofBlocker::HomeLiveness(error));
                }
            }
        }
        MirProof::Proven(())
    }
}

fn valid_window(definition: MirSite, end: MirSite) -> bool {
    if definition.block() != end.block() {
        return false;
    }
    match (definition, end) {
        (MirSite::BlockEntry { .. }, _) => true,
        (
            MirSite::Op {
                op_index: start, ..
            },
            MirSite::Op { op_index: end, .. },
        ) => start <= end,
        (MirSite::Op { .. }, MirSite::Terminator { .. }) => true,
        (MirSite::Terminator { .. }, MirSite::Terminator { .. }) => true,
        (MirSite::Terminator { .. }, _) | (_, MirSite::BlockEntry { .. }) => false,
    }
}

fn use_is_inside_window(usage: MirUseSite, definition: MirSite, end: MirSite) -> bool {
    if usage.site.block() != definition.block() {
        return false;
    }
    let starts_before = match (definition, usage.site) {
        (MirSite::BlockEntry { .. }, MirSite::Op { .. } | MirSite::Terminator { .. }) => true,
        (
            MirSite::Op {
                op_index: definition,
                ..
            },
            MirSite::Op {
                op_index: usage, ..
            },
        ) => definition < usage,
        (MirSite::Op { .. }, MirSite::Terminator { .. }) => true,
        (MirSite::Terminator { .. }, _) | (_, MirSite::BlockEntry { .. }) => false,
    };
    let ends_after = match (usage.site, end) {
        (
            MirSite::Op {
                op_index: usage, ..
            },
            MirSite::Op { op_index: end, .. },
        ) => usage <= end,
        (MirSite::Op { .. }, MirSite::Terminator { .. }) => true,
        (MirSite::Terminator { .. }, MirSite::Terminator { .. }) => true,
        _ => false,
    };
    starts_before && ends_after
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::effects::MirTempUseKind;
    use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
    use crate::mir6502::analysis::use_def::MirTempRequirement;
    use crate::mir6502::ir::{
        MirAddr, MirAddressConsumer, MirBlock, MirDef, MirEffects, MirFixedZpSlot, MirFrame,
        MirMem, MirOp, MirPointerPair, MirReg, MirRoutine, MirRoutineAbi, MirSpillId, MirTempId,
        MirTerminator, MirValue, MirWidth, RoutineId,
    };
    use crate::nir::ParamId;

    fn routine() -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "context".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![
                    MirOp::LoadImm {
                        dst: MirDef::VTemp(MirTempId(1)),
                        value: 1,
                        width: MirWidth::Byte,
                    },
                    MirOp::Move {
                        dst: MirDef::VTemp(MirTempId(2)),
                        src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    },
                    MirOp::Move {
                        dst: MirDef::VTemp(MirTempId(3)),
                        src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    },
                ],
                terminator: MirTerminator::Return,
            }],
            effects: MirEffects::default(),
        }
    }

    #[test]
    fn facade_proves_unique_dominating_definition_and_reports_later_use() {
        let routine = routine();
        let snapshot =
            PreHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let context = PreHomeRewriteContext::new(&snapshot);
        let lane = MirTempLane {
            temp: MirTempId(1),
            byte: 0,
        };
        let usage = MirUseSite {
            site: MirSite::Op {
                block: MirBlockId(0),
                op_index: 1,
            },
            requirement: MirTempRequirement::Full(MirTempId(1)),
            kind: MirTempUseKind::Operand,
        };
        let MirProof::Proven(definition) = context.unique_reaching_definition(usage, lane) else {
            panic!("expected unique definition proof");
        };
        assert!(
            context
                .value_available_at(definition, context.point(usage.site))
                .is_proven()
        );
        assert!(
            context
                .temp_definition_dead_after(definition, context.point(usage.site))
                .is_proven()
                == false
        );
        assert!(matches!(
            context.temp_definition_dead_after(
                definition,
                context.point(MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 2,
                }),
            ),
            MirProof::Proven(())
        ));
    }

    #[test]
    fn facade_rejects_stale_points() {
        let routine = routine();
        let snapshot =
            PreHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let context = PreHomeRewriteContext::new(&snapshot);
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
        let stale = MirProgramPoint {
            generation: MirRoutineGeneration::initial().next(),
            site: MirSite::Op {
                block: MirBlockId(0),
                op_index: 1,
            },
        };
        assert!(matches!(
            context.value_available_at(definition, stale),
            MirProof::Blocked(MirProofBlocker::InvalidPoint(
                MirProgramPointError::StaleGeneration { .. }
            ))
        ));
    }

    #[test]
    fn posthome_facade_exposes_home_machine_pointer_and_param_proofs() {
        let param = MirParamHomeByte {
            param: ParamId(0),
            offset: 0,
        };
        let spill = MirHomeByte::Spill {
            id: MirSpillId(0),
            offset: 0,
        };
        let pointer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0xAC),
        });
        let routine = MirRoutine {
            id: RoutineId(0),
            name: "posthome-context".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame {
                spills: vec![MirSpillId(0)],
                fixed_zero_page: vec![MirFixedZpSlot(0xAC), MirFixedZpSlot(0xAD)],
                ..MirFrame::default()
            },
            temps: Vec::new(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Param {
                            id: ParamId(0),
                            offset: 0,
                        }),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Spill {
                            id: MirSpillId(0),
                            offset: 0,
                        }),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                    MirOp::Load {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirAddr::Direct(MirMem::Spill {
                            id: MirSpillId(0),
                            offset: 0,
                        }),
                        width: MirWidth::Byte,
                    },
                    MirOp::MaterializeAddress {
                        consumer: pointer,
                        value: MirValue::ConstU16(0x4000),
                    },
                    MirOp::LoadIndirect {
                        consumer: pointer,
                        dst: MirDef::Reg(MirReg::A),
                        offset: 0,
                    },
                ],
                terminator: MirTerminator::Return,
            }],
            effects: MirEffects::default(),
        };
        let snapshot =
            PostHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let context = PostHomeRewriteContext::new(&snapshot);

        assert_eq!(
            context.parameter_register_at(
                param,
                context.point(MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 1,
                })
            ),
            MirProof::Proven(MirReg::A)
        );
        assert!(matches!(
            context.home_definition_dead_after(
                spill,
                context.point(MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 1,
                }),
                context.point(MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 1,
                }),
            ),
            MirProof::Blocked(MirProofBlocker::HomeDefinitionLive { .. })
        ));
        assert!(matches!(
            context.pointer_pair_dead_after(
                pointer,
                context.point(MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 3,
                }),
            ),
            MirProof::Blocked(MirProofBlocker::HomeLive { .. })
        ));
        assert!(
            context
                .register_dead_after(
                    MirReg::A,
                    context.point(MirSite::Op {
                        block: MirBlockId(0),
                        op_index: 4,
                    })
                )
                .is_proven()
        );
        assert!(
            context
                .flags_dead_after(
                    MirFlagSet {
                        z: true,
                        ..MirFlagSet::default()
                    },
                    context.point(MirSite::Op {
                        block: MirBlockId(0),
                        op_index: 4,
                    })
                )
                .is_proven()
        );
        assert!(
            context
                .exit_state_change_is_unobservable(
                    &MirExitStateChange {
                        registers: MirRegisterSet {
                            a: true,
                            ..MirRegisterSet::default()
                        },
                        ..MirExitStateChange::default()
                    },
                    context.point(MirSite::Op {
                        block: MirBlockId(0),
                        op_index: 4,
                    })
                )
                .is_proven()
        );
        assert!(matches!(
            context.exit_state_change_is_unobservable(
                &MirExitStateChange {
                    registers: MirRegisterSet {
                        sp: true,
                        ..MirRegisterSet::default()
                    },
                    ..MirExitStateChange::default()
                },
                context.point(MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 4,
                })
            ),
            MirProof::Blocked(MirProofBlocker::StackPointerLive { .. })
        ));
    }
}
