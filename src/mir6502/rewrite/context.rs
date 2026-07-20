#![allow(dead_code)] // Matchers migrate to this facade in later slices.

use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
use crate::mir6502::analysis::reaching_defs::MirReachingDefinitionError;
use crate::mir6502::analysis::sites::{
    MirProgramPoint, MirProgramPointError, MirRoutineGeneration, MirSite,
};
use crate::mir6502::analysis::use_def::{MirDefSite, MirTempLane, MirUseSite};
use crate::mir6502::ir::MirBlockId;

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
    use crate::mir6502::analysis::use_def::MirTempRequirement;
    use crate::mir6502::ir::{
        MirBlock, MirDef, MirEffects, MirFrame, MirOp, MirRoutine, MirRoutineAbi, MirTempId,
        MirTerminator, MirValue, MirWidth, RoutineId,
    };

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
}
