//! Shared entry obligations, separate from exit preservation and full memory
//! footprints. Facts own their source bodies, so stale sites cannot reuse them.
use super::*;
use crate::mir6502::analysis::{
    effects::{MirFlagSet, classify_terminator},
    home_liveness::{MirHomeBlockTransfers, MirHomeLiveness, MirHomeTransfer},
    machine_liveness::{MirMachineLiveness, MirMachineLiveSet},
};
use crate::mir6502::ir::{
    MirDef, MirFlag, MirParamLane, MirRoutineAbi, MirValue, MirWidth,
};
use std::sync::Arc;

mod acyclic;
pub(in crate::mir6502) use acyclic::MirAcyclicEntryAnalysis;

/// The caller establishes this boundary; bindings and operation prefixes cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir6502) enum MirCalleeEntryStage {
    PlannedCapture,
    ActualEntry,
}

/// Incoming machine values observed before a guaranteed definition. Program
/// memory observations belong to the separate read footprint, not this set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirEntryRequirements {
    pub registers: MirRegisterSet,
    pub flags: MirFlagSet,
    pub fixed: BTreeSet<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct MirCalleeEntryFact {
    pub source: Arc<MirRoutine>,
    stage: MirCalleeEntryStage,
    machines: Vec<MirMachineBlock>,
    helper_bindings: Vec<crate::mir6502::ir::MirRuntimeHelperDecl>,
    library_bindings: Vec<Arc<crate::mir6502::ir::MirLibraryBinding>>,
    global_backings: Vec<(crate::nir::SymbolId, MirGlobalBacking, u16)>,
    pub inputs: MirEntryRequirements,
    dependencies: BTreeMap<RoutineId, Arc<MirCalleeEntryFact>>,
    /// Declared result bytes definitely defined on every normal-return path.
    pub defined_results: BTreeSet<u8>,
}
impl MirCalleeEntryFact {
    pub fn analyze(
        program: &MirProgram,
        target: RoutineId,
        stage: MirCalleeEntryStage,
    ) -> Option<Self> {
        Self::analyze_with_dependencies(program, target, stage, None)
    }
    fn analyze_with_dependencies(
        program: &MirProgram, target: RoutineId, stage: MirCalleeEntryStage,
        dependencies: Option<BTreeMap<RoutineId, Arc<Self>>>,
    ) -> Option<Self> {
        let routine = program.routines.iter().find(|r| r.id == target)?;
        if routine.effects.opaque { return None; }
        if routine.effects.may_call_os {
            // Explain the aggregate flag through current returning boundary
            // contracts. An unexplained OS effect remains an entry barrier.
            let mut covered = false;
            for op in routine.blocks.iter().flat_map(|b| &b.ops) {
                let effects = match op {
                    MirOp::Call { effects, .. } | MirOp::RuntimeHelper { effects, .. }
                        | MirOp::SArgsCapture { effects, .. } | MirOp::MachineBlock { effects, .. }
                        | MirOp::Barrier { effects } => effects,
                    _ => continue,
                };
                if !effects.may_call_os { continue; }
                let returning = crate::mir6502::runtime_contracts::invocation(program, routine, op).is_some()
                    || matches!(op, MirOp::Call { target: MirCallTarget::Routine(id), .. }
                        if dependencies.as_ref().is_some_and(|d| d.contains_key(id)));
                if !returning { return None; }
                covered = true;
            }
            if !covered { return None; }
        }
        let machines: Vec<_> = routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|o| {
                if let MirOp::MachineBlock { id, .. } = o {
                    Some(*id)
                } else {
                    None
                }
            })
            .map(|id| program.machine_blocks.iter().find(|m| m.id == id).cloned())
            .collect::<Option<_>>()?;
        let (inputs, defined_results) = if !machines.is_empty() {
            if dependencies.is_some() { return None; }
            (BoundedMachine::analyze(routine, program)?.entry_inputs(program)?, BTreeSet::new())
        } else {
            scalar_inputs(program, routine, stage, dependencies.as_ref())?
        };
        Some(Self {
            source: Arc::new(routine.clone()),
            stage,
            machines,
            helper_bindings: program.runtime_helpers.clone(),
            library_bindings: routine.blocks.iter().flat_map(|b| &b.ops).filter_map(|o| match o {
                MirOp::Call { abi, .. } => abi.library.clone(), _ => None,
            }).collect(),
            global_backings: program
                .globals
                .iter()
                .map(|g| (g.id, g.backing.clone(), g.storage_size))
                .collect(),
            inputs,
            dependencies: dependencies.unwrap_or_default(),
            defined_results,
        })
    }
    pub fn is_current(&self, program: &MirProgram, stage: MirCalleeEntryStage) -> bool {
        self.stage == stage
            && self.library_bindings.iter().all(|b| b.implementation.as_ref()
                .is_none_or(|i| i.is_current(program, &i.target)))
            && self.helper_bindings == program.runtime_helpers
            && self.helper_bindings.iter().all(|d| d.binding.as_ref()
                .is_none_or(|b| b.is_current(program, &d.target)))
            && self.dependencies.values().all(|d| d.is_current(program, stage))
            && program.routines.iter().any(|r| r == self.source.as_ref())
            && self
                .machines
                .iter()
                .all(|m| program.machine_blocks.iter().any(|now| now == m))
            && self.global_backings
                == program
                    .globals
                    .iter()
                    .map(|g| (g.id, g.backing.clone(), g.storage_size))
                    .collect::<Vec<_>>()
    }
}

fn scalar_inputs(
    program: &MirProgram,
    routine: &MirRoutine,
    stage: MirCalleeEntryStage,
    dependencies: Option<&BTreeMap<RoutineId, Arc<MirCalleeEntryFact>>>,
) -> Option<(MirEntryRequirements, BTreeSet<u8>)> {
    // An observable native entry still uses the Action ABI. Its actual body
    // can supply a contract without granting permission to invent capture or
    // move its own storage. Planned capture remains compiler-owned only.
    let supported_entry = matches!(routine.abi, MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry)
        || (routine.abi == MirRoutineAbi::ActionObservable
            && stage == MirCalleeEntryStage::ActualEntry);
    if !supported_entry || !direct_parameter_abi_is_complete(program, routine, stage)
        || routine.blocks.iter().flat_map(|b| &b.ops).any(|o| {
            matches!(
                o,
                MirOp::Barrier { .. }
                    | MirOp::MachineBlock { .. }
            ) || (matches!(o, MirOp::RuntimeHelper { .. } | MirOp::SArgsCapture { .. })
                && (stage != MirCalleeEntryStage::ActualEntry
                    || crate::mir6502::runtime_contracts::invocation(program, routine, o).is_none()))
                || (matches!(o, MirOp::Call { .. })
                && crate::mir6502::runtime_contracts::library_invocation(program, o).is_none()
                && (stage != MirCalleeEntryStage::ActualEntry || dependencies.is_none()))
        })
    {
        return None;
    }
    // Only certified register-store capture participates in this slice. A
    // memory-sourced capture expands through A and changes N/Z: treating its
    // pseudo Store as flag-preserving would be unsound. Actual SArgs captures
    // use their selected-implementation summary below.
    let mut captured;
    let routine = if stage == MirCalleeEntryStage::PlannedCapture {
        let mut capture = crate::mir6502::abi::direct_param_capture(routine);
        if capture.iter().any(|op| {
            !matches!(
                op,
                MirOp::Store {
                    src: MirValue::Def(MirDef::Reg(_)),
                    width: MirWidth::Byte,
                    ..
                }
            )
        }) {
            return None;
        }
        captured = routine.clone();
        let entry = captured.blocks.first_mut()?;
        capture.append(&mut entry.ops);
        entry.ops = capture;
        &captured
    } else {
        routine
    };
    let cfg = MirCfg::from_routine(routine).ok()?;
    let entry = cfg.entry()?;
    let physical = |mem: &MirMem, delta: u16| -> Option<u8> {
        match mem {
            MirMem::FixedZeroPage(slot) => slot.0.checked_add(u8::try_from(delta).ok()?),
            MirMem::Absolute(a) => u8::try_from(a.wrapping_add(delta)).ok(),
            MirMem::Global { .. } => {
                u8::try_from(fixed_storage_address(program, mem)?.checked_add(delta)?).ok()
            }
            MirMem::Param { .. } | MirMem::Local { .. } => match routine.frame.object_lane(mem, delta)?
            {
                MirParamLane::ZeroPage(s) => Some(s.0),
                _ => None,
            },
            _ => None,
        }
    };
    let transfer = |memory: &super::super::effects::MirMemoryEffects,
                    reads: &BTreeSet<MirHomeByte>,
                    writes: &BTreeSet<MirHomeByte>| {
        let mut result = MirHomeTransfer {
            reads: reads.clone(),
            writes: writes.clone(),
            ..Default::default()
        };
        for (ranges, into, reading) in [
            (&memory.direct_reads, &mut result.reads, true),
            (&memory.direct_writes, &mut result.writes, false),
        ] {
            for range in ranges {
                // Classify the access before projecting it to an address. A
                // global at the same address must not erase a separate raw read.
                if reading && program_state_access(program, &range.base, range.bytes)? {
                    continue;
                }
                for offset in 0..range.bytes {
                    if let Some(slot) = physical(&range.base, offset) {
                        into.insert(MirHomeByte::FixedZeroPage(MirFixedZpSlot(slot)));
                    }
                }
            }
        }
        Some(result)
    };
    let mut transfers = BTreeMap::new();
    for block in &routine.blocks {
        let mut ops = Vec::new();
        for op in &block.ops {
            if let Some(invocation) = crate::mir6502::runtime_contracts::invocation(program, routine, op) {
                ops.push(invocation.home_transfer());
                continue;
            }
            if let MirOp::Call { target: MirCallTarget::Routine(target), args, .. } = op {
                let fact = dependencies?.get(target)?;
                let callee = program.routines.iter().find(|r| r.id == *target)?;
                let lanes = crate::mir6502::abi::canonical_call_lanes(callee, args).ok()?;
                crate::mir6502::abi::validate_direct_call_result(callee, op).ok()?;
                let mut reads = fact.inputs.fixed.iter().map(|s|
                    MirHomeByte::FixedZeroPage(MirFixedZpSlot(*s))).collect::<BTreeSet<_>>();
                reads.extend(lanes.iter().filter_map(|lane| match lane {
                    MirParamLane::ZeroPage(s) => Some(MirHomeByte::FixedZeroPage(*s)), _ => None,
                }));
                ops.push(MirHomeTransfer {
                    reads,
                    writes: fact.defined_results.iter().map(|s|
                        MirHomeByte::FixedZeroPage(MirFixedZpSlot(*s))).collect(),
                });
                continue;
            }
            if matches!(op, MirOp::Call { .. }) { return None; }
            let e = classify_op(op);
            if e.memory.opaque {
                return None;
            }
            ops.push(transfer(
                &e.memory,
                &e.homes
                    .reads
                    .union(&e.addresses.pair_reads)
                    .copied()
                    .collect(),
                &e.homes
                    .definite_writes
                    .union(&e.addresses.pair_writes)
                    .copied()
                    .collect(),
            )?);
        }
        let e = classify_terminator(&block.terminator);
        transfers.insert(
            block.id,
            MirHomeBlockTransfers {
                ops,
                terminator: transfer(&e.memory, &BTreeSet::new(), &BTreeSet::new())?,
            },
        );
    }
    // A typed ABI result is a definite definition only when every returning
    // path really defines it. Never infer this from a possible write footprint.
    let defined_results = if stage == MirCalleeEntryStage::ActualEntry {
        acyclic::defined_results(routine, &cfg, &transfers)
    } else { BTreeSet::new() };
    let homes = MirHomeLiveness::analyze_transfers(routine, &cfg, transfers);
    let live = MirMachineLiveness::entry_inputs_with_calls(routine, &cfg, &|op| {
        if let Some(invocation) = crate::mir6502::runtime_contracts::invocation(program, routine, op) {
            return Some(invocation.machine_transfer());
        }
        let MirOp::Call { target: MirCallTarget::Routine(target), args, .. } = op else { return None; };
        let inputs = &dependencies.unwrap()[target].inputs;
        let mut registers = inputs.registers;
        for arg in args {
            for lane in crate::mir6502::abi::physical_lanes(&arg.home, arg.width).unwrap() {
                match lane {
                    MirParamLane::Reg(MirReg::A) => registers.a = true,
                    MirParamLane::Reg(MirReg::X) => registers.x = true,
                    MirParamLane::Reg(MirReg::Y) => registers.y = true,
                    _ => {},
                }
            }
        }
        Some((MirMachineLiveSet { registers, flags: inputs.flags }, MirMachineLiveSet::default()))
    })?;
    // A retained lane has no memory capture. Conservatively require its
    // canonical register even if this body happens not to observe it.
    let retained = |reg| {
        routine
            .frame
            .parameter_homes
            .as_ref()
            .is_some_and(|bindings| {
                bindings.iter().any(|b| {
                    routine
                        .frame
                        .parameter_lanes(b.param)
                        .is_some_and(|lanes| lanes.contains(&MirParamLane::Reg(reg)))
                })
            })
    };
    Some((MirEntryRequirements {
        registers: MirRegisterSet {
            a: live.register_live(MirReg::A) || retained(MirReg::A),
            x: live.register_live(MirReg::X) || retained(MirReg::X),
            y: live.register_live(MirReg::Y) || retained(MirReg::Y),
            sp: live.stack_pointer_live() || routine.effects.reads.sp,
            flags: false,
        },
        flags: MirFlagSet {
            c: live.flag_live(MirFlag::C),
            z: live.flag_live(MirFlag::Z),
            n: live.flag_live(MirFlag::N),
            v: live.flag_live(MirFlag::V),
        },
        fixed: homes
            .live_in(entry)?
            .iter()
            .filter_map(|h| {
                if let MirHomeByte::FixedZeroPage(s) = h {
                    Some(s.0)
                } else {
                    None
                }
            })
            .collect(),
    }, defined_results))
}

/// Only typed global accesses establish a program-state role. Literal machine
/// addresses retain their entry requirements even when a global overlaps them.
/// Invalid identity, extent or backing declines the entry certificate.
pub(super) fn program_state_access(program: &MirProgram, mem: &MirMem, bytes: u16) -> Option<bool> {
    let MirMem::Global { id, offset } = mem else {
        return Some(false);
    };
    let global = program.globals.iter().find(|g| g.id == *id)?;
    let end = offset.checked_add(bytes)?;
    if bytes == 0 || end > global.storage_size {
        return None;
    }
    let last = MirMem::Global { id: *id, offset: end - 1 };
    (super::observations::valid_backing(program, mem)
        && super::observations::valid_backing(program, &last))
        .then_some(true)
}

/// Byte widths alone cannot distinguish a pointer from an aggregate or a
/// variadic declaration. Lowering carries the complete finite ABI explicitly;
/// validate it against the current frame rather than consulting source meaning.
fn direct_parameter_abi_is_complete(program: &MirProgram, routine: &MirRoutine, stage: MirCalleeEntryStage) -> bool {
    let Some(parameters) = &routine.frame.direct_parameter_abi else {
        return false;
    };
    crate::mir6502::abi::direct_parameter_frame_matches(routine)
        && crate::mir6502::abi::parameter_bytes(parameters)
            .and_then(crate::mir6502::abi::argument_lanes).is_some()
        && (crate::mir6502::abi::parameter_bytes(parameters).is_some_and(|bytes|
                bytes <= crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES)
            || (stage == MirCalleeEntryStage::ActualEntry && routine.blocks.first()
                .and_then(|b| b.ops.first())
                .and_then(|op| crate::mir6502::runtime_contracts::sargs_capture(program, routine, op)).is_some()))
}

#[cfg(test)]
mod state_tests;
