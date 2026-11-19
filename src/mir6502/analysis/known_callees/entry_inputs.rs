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

/// Entry obligations for the requested consumer. Physical entry analysis asks
/// which machine values can be read before definition. Home placement asks
/// which values a caller must supply. Program-data reads have a separate footprint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirEntryRequirements {
    pub registers: MirRegisterSet,
    pub flags: MirFlagSet,
    pub fixed: BTreeSet<u8>,
}
/// ABI values and reviewed fixed-byte definitions available on normal return.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HomeResults {
    slots: BTreeSet<u8>,
    /// Physical writes declared by a review or established on all wrapper returns.
    defined_zp: BTreeSet<u8>,
    // A checked A=$A0 relation also supplies A. Track a possible observation
    // of the old slot separately from the ABI arguments needed at entry.
    accumulator: Option<ReviewedSlotInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewedSlotInput { Independent, MayReadOldSlot }

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
    home_results: HomeResults,
    ordinary_globals_above_zp: bool,
}
impl MirCalleeEntryFact {
    pub fn analyze(
        program: &MirProgram,
        target: RoutineId,
        stage: MirCalleeEntryStage,
    ) -> Option<Self> {
        Self::analyze_with_dependencies(program, target, stage, None, false)
    }
    fn analyze_with_dependencies(
        program: &MirProgram, target: RoutineId, stage: MirCalleeEntryStage,
        dependencies: Option<BTreeMap<RoutineId, Arc<Self>>>,
        ordinary_globals_above_zp: bool,
    ) -> Option<Self> {
        let routine = program.routines.iter().find(|r| r.id == target)?;
        let review = super::reviewed_contract(program, routine);
        if routine.effects.opaque && review.is_none() { return None; }
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
                let returning = matches!(op, MirOp::MachineBlock { .. }) && review.is_some()
                    || crate::mir6502::runtime_contracts::invocation(program, routine, op).is_some()
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
        let (inputs, defined_results, home_results) = if !machines.is_empty() {
            // Only the acyclic home analysis supplies a dependency map.
            let home_placement = dependencies.is_some();
            let mut results = HomeResults::default();
            let mut old_values = BTreeSet::new();
            let inputs = if routine.frame.contains_source_asm || routine.frame.callee_review.is_some() {
                let certified = if let Some(review) = review {
                    if stage == MirCalleeEntryStage::ActualEntry {
                        results.defined_zp.clone_from(&review.defines_on_return);
                        old_values.extend(review.defines_on_return.iter().copied().filter(|slot|
                            reviewed_slot_input(program, review, *slot, ordinary_globals_above_zp)
                                == ReviewedSlotInput::MayReadOldSlot));
                    }
                    if home_placement && review.returns_a_equals_a0
                        && crate::mir6502::abi::direct_result_slots(routine.direct_result_abi?).contains(&0xa0) {
                        results.slots.insert(0xa0);
                        results.accumulator = Some(reviewed_slot_input(program, review, 0xa0, ordinary_globals_above_zp));
                    }
                    if home_placement { review.abi_inputs_only || review.physical_abi_inputs }
                    else { review.physical_abi_inputs }
                } else {
                    home_placement && routine.frame.callee_review.is_none()
                        && routine.frame.abi_inputs_only_review.is_some_and(|expected|
                            super::abi_inputs_review_fingerprint(program, routine) == Some(expected))
                };
                if !certified { return None; }
                let abi = routine.frame.direct_parameter_abi.as_ref()?;
                let bytes = crate::mir6502::abi::parameter_bytes(abi)?;
                let mut inputs = MirEntryRequirements { fixed: old_values, ..Default::default() };
                for lane in crate::mir6502::abi::argument_lanes(bytes)? {
                    match lane {
                        MirParamLane::Reg(MirReg::A) => inputs.registers.a = true,
                        MirParamLane::Reg(MirReg::X) => inputs.registers.x = true,
                        MirParamLane::Reg(MirReg::Y) => inputs.registers.y = true,
                        MirParamLane::ZeroPage(slot) => { inputs.fixed.insert(slot.0); }
                    }
                }
                inputs
            } else { BoundedMachine::analyze(routine, program)?.entry_inputs(program)? };
            let result_bytes = routine.direct_result_abi.map(crate::mir6502::abi::direct_result_slots)
                .unwrap_or_default();
            let defined_results: BTreeSet<_> = results.defined_zp.iter().copied()
                .filter(|s| result_bytes.contains(s)).collect();
            results.slots.extend(&defined_results);
            (inputs, defined_results, results)
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
            home_results,
            ordinary_globals_above_zp,
        })
    }
    pub fn is_current(&self, program: &MirProgram, stage: MirCalleeEntryStage) -> bool {
        self.is_current_for_layout(program, stage, false)
    }

    fn is_current_for_layout(&self, program: &MirProgram, stage: MirCalleeEntryStage, ordinary_globals_above_zp: bool) -> bool {
        self.stage == stage && self.ordinary_globals_above_zp == ordinary_globals_above_zp
            && (self.source.frame.callee_review.is_none() || super::reviewed_contract(program, &self.source).is_some())
            && self.library_bindings.iter().all(|b| b.implementation.as_ref()
                .is_none_or(|i| i.is_current(program, &i.target)))
            && self.helper_bindings == program.runtime_helpers
            && self.helper_bindings.iter().all(|d| d.binding.as_ref()
                .is_none_or(|b| b.is_current(program, &d.target)))
            && self.dependencies.values().all(|d| d.is_current_for_layout(program, stage, ordinary_globals_above_zp))
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

fn reviewed_slot_input(
    program: &MirProgram,
    facts: &crate::callee6502::CallFacts<crate::nir::SymbolId>,
    slot: u8,
    ordinary_globals_above_zp: bool,
) -> ReviewedSlotInput {
    // A physical entry certificate bounds observations of incoming machine
    // bytes. The caller handles incoming ABI argument lanes separately.
    // The invocation may still read it after definition or read through a
    // pointer. Its complete program-data read footprint stays independent.
    if facts.physical_abi_inputs { return ReviewedSlotInput::Independent; }
    let disjoint = facts.reads.as_ref().is_some_and(|ranges| ranges.iter().all(|r| {
        let Some(id) = r.global else { return !(r.start..=r.end).contains(&u16::from(slot)); };
        // The review matcher binds the full range to an in-bounds, inline
        // array. Successful layouts place its entire payload at/above origin.
        ordinary_globals_above_zp && program.globals.iter().any(|g| g.id == id
            && matches!(g.backing, MirGlobalBacking::Ordinary { .. }) && r.end < g.storage_size)
    }));
    if disjoint && !facts.buffers.iter().any(|b| !b.write) { ReviewedSlotInput::Independent }
    else { ReviewedSlotInput::MayReadOldSlot }
}

fn scalar_inputs(
    program: &MirProgram,
    routine: &MirRoutine,
    stage: MirCalleeEntryStage,
    dependencies: Option<&BTreeMap<RoutineId, Arc<MirCalleeEntryFact>>>,
) -> Option<(MirEntryRequirements, BTreeSet<u8>, HomeResults)> {
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
    let mut returned_calls = Vec::new();
    let mut reviewed_definitions = BTreeSet::new();
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
                if fact.home_results.accumulator == Some(ReviewedSlotInput::MayReadOldSlot) {
                    reads.insert(MirHomeByte::FixedZeroPage(MirFixedZpSlot(0xa0)));
                }
                if fact.home_results.slots != fact.defined_results {
                    returned_calls.push((block.id, ops.len(), &fact.home_results.slots));
                }
                reviewed_definitions.extend(fact.home_results.defined_zp.iter().copied());
                ops.push(MirHomeTransfer {
                    reads,
                    writes: fact.defined_results.union(&fact.home_results.defined_zp).map(|s|
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
    let result_bytes = routine.direct_result_abi.map(crate::mir6502::abi::direct_result_slots)
        .unwrap_or_default();
    let mut candidates = reviewed_definitions.clone();
    if stage == MirCalleeEntryStage::ActualEntry { candidates.extend(result_bytes); }
    let physical = acyclic::slots_defined_on_all_returns(routine, &cfg, &transfers, &candidates);
    let defined_results: BTreeSet<_> = physical.iter().copied().filter(|s| result_bytes.contains(s)).collect();
    reviewed_definitions.retain(|s| physical.contains(s));
    // Physical must-writes are established above. Apply the additional ABI
    // values only in the caller view, then compose them over the same CFG.
    let slots = if returned_calls.is_empty() {
        defined_results.clone()
    } else {
        for (block, index, slots) in returned_calls {
            transfers.get_mut(&block)?.ops[index].writes.extend(slots.iter()
                .map(|s| MirHomeByte::FixedZeroPage(MirFixedZpSlot(*s))));
        }
        acyclic::slots_defined_on_all_returns(routine, &cfg, &transfers, &result_bytes.iter().copied().collect())
    };
    let homes = MirHomeLiveness::analyze_transfers(routine, &cfg, transfers);
    let live = MirMachineLiveness::entry_inputs_with_calls(routine, &cfg, &|op| {
        if let Some(invocation) = crate::mir6502::runtime_contracts::invocation(program, routine, op) {
            return Some(invocation.machine_transfer());
        }
        let MirOp::Call { target: MirCallTarget::Routine(target), args, .. } = op else { return None; };
        let fact = &dependencies?[target];
        let inputs = &fact.inputs;
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
        let mut supplied = MirMachineLiveSet::default();
        supplied.registers.a = fact.home_results.accumulator.is_some();
        Some((MirMachineLiveSet { registers, flags: inputs.flags }, supplied))
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
    }, defined_results, HomeResults { slots, defined_zp: reviewed_definitions, accumulator: None }))
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
