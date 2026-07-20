#![allow(dead_code)] // Matchers migrate to this snapshot in later slices.

use crate::mir6502::analysis::cfg::{MirCfg, MirCfgError};
use crate::mir6502::analysis::dominance::MirDominance;
use crate::mir6502::analysis::reaching_defs::MirReachingDefinitions;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirRoutineSnapshot};
use crate::mir6502::analysis::temp_liveness::MirTempLiveness;
use crate::mir6502::analysis::use_def::MirTempUseDefIndex;
use crate::mir6502::ir::MirRoutine;

/// Immutable, generation-scoped fact bundle for pre-home rewrites.
#[derive(Debug)]
pub(in crate::mir6502) struct PreHomeAnalysisSnapshot<'a> {
    routine: MirRoutineSnapshot<'a>,
    use_def: MirTempUseDefIndex,
    reaching_definitions: MirReachingDefinitions,
    temp_liveness: MirTempLiveness,
    dominance: MirDominance,
}

impl<'a> PreHomeAnalysisSnapshot<'a> {
    pub(in crate::mir6502) fn new(
        routine: &'a MirRoutine,
        generation: MirRoutineGeneration,
    ) -> Result<Self, Vec<MirCfgError>> {
        let routine_snapshot = MirRoutineSnapshot::new(routine, generation)?;
        let cfg = routine_snapshot.cfg();
        Ok(Self {
            use_def: MirTempUseDefIndex::from_routine(routine),
            reaching_definitions: MirReachingDefinitions::analyze(routine, cfg),
            temp_liveness: MirTempLiveness::analyze(routine, cfg),
            dominance: MirDominance::from_cfg(cfg),
            routine: routine_snapshot,
        })
    }

    pub(in crate::mir6502) fn routine(&self) -> &MirRoutineSnapshot<'a> {
        &self.routine
    }

    pub(in crate::mir6502) fn cfg(&self) -> &MirCfg {
        self.routine.cfg()
    }

    pub(in crate::mir6502) fn use_def(&self) -> &MirTempUseDefIndex {
        &self.use_def
    }

    pub(in crate::mir6502) fn reaching_definitions(&self) -> &MirReachingDefinitions {
        &self.reaching_definitions
    }

    pub(in crate::mir6502) fn temp_liveness(&self) -> &MirTempLiveness {
        &self.temp_liveness
    }

    pub(in crate::mir6502) fn dominance(&self) -> &MirDominance {
        &self.dominance
    }
}
