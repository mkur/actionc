#![allow(dead_code)] // Matchers migrate to this snapshot in Slice 8.

use crate::mir6502::analysis::cfg::{MirCfg, MirCfgError};
use crate::mir6502::analysis::home_definitions::MirHomeDefinitions;
use crate::mir6502::analysis::home_liveness::MirHomeLiveness;
use crate::mir6502::analysis::known_callees::MirKnownCalleeSummaries;
use crate::mir6502::analysis::machine_liveness::MirMachineLiveness;
use crate::mir6502::analysis::machine_values::{MirMachineMemoryMap, MirMachineValueAvailability};
use crate::mir6502::analysis::param_availability::MirParamRegisterAvailability;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirRoutineSnapshot};
use crate::mir6502::ir::{MirRoutine,MirOp,MirCallTarget,RoutineId};
use std::collections::BTreeSet;

/// Immutable, generation-scoped fact bundle for post-home rewrites.
#[derive(Debug)]
pub(in crate::mir6502) struct PostHomeAnalysisSnapshot<'a> {
    source: &'a MirRoutine,
    routine: MirRoutineSnapshot<'a>,
    home_definitions: MirHomeDefinitions,
    home_liveness: MirHomeLiveness,
    machine_liveness: MirMachineLiveness,
    machine_values: MirMachineValueAvailability,
    param_availability: MirParamRegisterAvailability,
    scalar_entries_ignoring_flags: BTreeSet<RoutineId>,
    entry_flags: std::collections::BTreeMap<RoutineId, super::entry_flags::MirCalleeEntryFlagFact>,
}

impl<'a> PostHomeAnalysisSnapshot<'a> {
    pub(in crate::mir6502) fn new(
        routine: &'a MirRoutine,
        generation: MirRoutineGeneration,
    ) -> Result<Self, Vec<MirCfgError>> {
        Self::new_with_known_callees(routine, generation, &MirKnownCalleeSummaries::default())
    }

    pub(in crate::mir6502) fn new_with_known_callees(
        routine: &'a MirRoutine,
        generation: MirRoutineGeneration,
        known_callees: &MirKnownCalleeSummaries,
    ) -> Result<Self, Vec<MirCfgError>> {
        Self::new_with_known_callees_and_memory_map(
            routine,
            generation,
            known_callees,
            &MirMachineMemoryMap::default(),
        )
    }

    pub(in crate::mir6502) fn new_with_known_callees_and_memory_map(
        routine: &'a MirRoutine,
        generation: MirRoutineGeneration,
        known_callees: &MirKnownCalleeSummaries,
        memory_map: &MirMachineMemoryMap,
    ) -> Result<Self, Vec<MirCfgError>> {
        let routine_snapshot = MirRoutineSnapshot::new(routine, generation)?;
        let cfg = routine_snapshot.cfg();
        Ok(Self {
            source: routine,
            entry_flags: routine.blocks.iter().flat_map(|b| &b.ops).filter_map(|op| match op {
                MirOp::Call { target: MirCallTarget::Routine(id), .. } => known_callees.entry_flag_fact(*id).cloned().map(|f| (*id, f)),
                _ => None,
            }).collect(),
            scalar_entries_ignoring_flags: routine.blocks.iter().flat_map(|b|&b.ops).filter_map(|o|match o {
                MirOp::Call {target:MirCallTarget::Routine(id),..} if known_callees.scalar_entry_ignores_flags(*id)=>Some(*id),_=>None
            }).collect(),
            home_definitions: MirHomeDefinitions::analyze(routine, cfg),
            home_liveness: MirHomeLiveness::analyze(routine, cfg),
            machine_liveness: MirMachineLiveness::analyze_with_known_callees(
                routine,
                cfg,
                known_callees,
            ),
            machine_values: MirMachineValueAvailability::analyze_with_known_callees_and_memory_map(
                routine,
                cfg,
                known_callees,
                memory_map,
            ),
            param_availability: MirParamRegisterAvailability::analyze(routine, cfg),
            routine: routine_snapshot,
        })
    }

    pub(in crate::mir6502) fn routine(&self) -> &MirRoutineSnapshot<'a> {
        &self.routine
    }

    pub(in crate::mir6502) fn scalar_entry_ignores_flags(&self,id:RoutineId)->bool {self.scalar_entries_ignoring_flags.contains(&id)}

    pub(in crate::mir6502) fn entry_flag_fact(&self, id: RoutineId) -> Option<&super::entry_flags::MirCalleeEntryFlagFact> { self.entry_flags.get(&id) }

    pub(in crate::mir6502) fn source(&self) -> &MirRoutine { self.source }

    pub(in crate::mir6502) fn cfg(&self) -> &MirCfg {
        self.routine.cfg()
    }

    pub(in crate::mir6502) fn home_liveness(&self) -> &MirHomeLiveness {
        &self.home_liveness
    }

    pub(in crate::mir6502) fn home_definitions(&self) -> &MirHomeDefinitions {
        &self.home_definitions
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
