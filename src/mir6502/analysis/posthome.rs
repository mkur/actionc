#![allow(dead_code)] // Matchers migrate to this snapshot in Slice 8.

use crate::mir6502::analysis::cfg::{MirCfg, MirCfgError};
use crate::mir6502::analysis::home_liveness::MirHomeLiveness;
use crate::mir6502::analysis::machine_liveness::MirMachineLiveness;
use crate::mir6502::analysis::machine_values::MirMachineValueAvailability;
use crate::mir6502::analysis::param_availability::MirParamRegisterAvailability;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirRoutineSnapshot};
use crate::mir6502::ir::MirRoutine;

/// Immutable, generation-scoped fact bundle for post-home rewrites.
#[derive(Debug)]
pub(in crate::mir6502) struct PostHomeAnalysisSnapshot<'a> {
    routine: MirRoutineSnapshot<'a>,
    home_liveness: MirHomeLiveness,
    machine_liveness: MirMachineLiveness,
    machine_values: MirMachineValueAvailability,
    param_availability: MirParamRegisterAvailability,
}

impl<'a> PostHomeAnalysisSnapshot<'a> {
    pub(in crate::mir6502) fn new(
        routine: &'a MirRoutine,
        generation: MirRoutineGeneration,
    ) -> Result<Self, Vec<MirCfgError>> {
        let routine_snapshot = MirRoutineSnapshot::new(routine, generation)?;
        let cfg = routine_snapshot.cfg();
        Ok(Self {
            home_liveness: MirHomeLiveness::analyze(routine, cfg),
            machine_liveness: MirMachineLiveness::analyze(routine, cfg),
            machine_values: MirMachineValueAvailability::analyze(routine, cfg),
            param_availability: MirParamRegisterAvailability::analyze(routine, cfg),
            routine: routine_snapshot,
        })
    }

    pub(in crate::mir6502) fn routine(&self) -> &MirRoutineSnapshot<'a> {
        &self.routine
    }

    pub(in crate::mir6502) fn cfg(&self) -> &MirCfg {
        self.routine.cfg()
    }

    pub(in crate::mir6502) fn home_liveness(&self) -> &MirHomeLiveness {
        &self.home_liveness
    }

    pub(in crate::mir6502) fn machine_liveness(&self) -> &MirMachineLiveness {
        &self.machine_liveness
    }

    pub(in crate::mir6502) fn machine_values(&self) -> &MirMachineValueAvailability {
        &self.machine_values
    }

    pub(in crate::mir6502) fn param_availability(&self) -> &MirParamRegisterAvailability {
        &self.param_availability
    }
}
