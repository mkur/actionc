//! Shared entry obligations, separate from exit preservation and full memory
//! footprints. Facts own their source bodies, so stale sites cannot reuse them.
use super::*;
use crate::mir6502::analysis::{
    effects::{MirFlagSet, classify_terminator},
    home_liveness::{MirHomeBlockTransfers, MirHomeLiveness, MirHomeTransfer},
    machine_liveness::MirMachineLiveness,
};
use crate::mir6502::ir::{
    MirDef, MirFlag, MirParamLane, MirRoutineAbi, MirStorageClass, MirValue, MirWidth,
};
use std::sync::Arc;

/// The caller establishes this boundary; bindings and operation prefixes cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir6502) enum MirCalleeEntryStage {
    PlannedCapture,
    ActualEntry,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct Inputs {
    pub registers: MirRegisterSet,
    pub flags: MirFlagSet,
    pub fixed: BTreeSet<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct MirCalleeEntryFact {
    pub source: Arc<MirRoutine>,
    stage: MirCalleeEntryStage,
    machines: Vec<MirMachineBlock>,
    global_backings: Vec<(crate::nir::SymbolId, MirGlobalBacking, u16)>,
    pub inputs: Inputs,
}
impl MirCalleeEntryFact {
    pub fn analyze(
        program: &MirProgram,
        target: RoutineId,
        stage: MirCalleeEntryStage,
    ) -> Option<Self> {
        let routine = program.routines.iter().find(|r| r.id == target)?;
        if routine.effects.opaque || routine.effects.may_call_os {
            return None;
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
        let inputs = if !machines.is_empty() {
            BoundedMachine::analyze(routine, program)?.entry_inputs()?
        } else {
            scalar_inputs(program, routine, stage)?
        };
        Some(Self {
            source: Arc::new(routine.clone()),
            stage,
            machines,
            global_backings: program
                .globals
                .iter()
                .map(|g| (g.id, g.backing.clone(), g.storage_size))
                .collect(),
            inputs,
        })
    }
    pub fn is_current(&self, program: &MirProgram, stage: MirCalleeEntryStage) -> bool {
        self.stage == stage
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
) -> Option<Inputs> {
    if !direct_parameter_abi_is_complete(routine)
        || routine.blocks.iter().flat_map(|b| &b.ops).any(|o| {
            matches!(
                o,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::Barrier { .. }
                    | MirOp::MachineBlock { .. }
            )
        })
    {
        return None;
    }
    // Only certified register-store capture participates in this slice. A
    // memory-sourced capture expands through A and changes N/Z: treating its
    // pseudo Store as flag-preserving would be unsound. SArgs is excluded by
    // the complete direct ABI check, and machine entries use their own proof.
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
            MirMem::Global { id, offset } => {
                let g = program.globals.iter().find(|g| g.id == *id)?;
                let MirGlobalBacking::Absolute(a) = g.backing else {
                    return None;
                };
                u8::try_from(a.wrapping_add(*offset).wrapping_add(delta)).ok()
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
        for (ranges, into) in [
            (&memory.direct_reads, &mut result.reads),
            (&memory.direct_writes, &mut result.writes),
        ] {
            for range in ranges {
                for offset in 0..range.bytes {
                    if let Some(slot) = physical(&range.base, offset) {
                        into.insert(MirHomeByte::FixedZeroPage(MirFixedZpSlot(slot)));
                    }
                }
            }
        }
        result
    };
    let mut transfers = BTreeMap::new();
    for block in &routine.blocks {
        let mut ops = Vec::new();
        for op in &block.ops {
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
                    .writes
                    .union(&e.addresses.pair_writes)
                    .copied()
                    .collect(),
            ));
        }
        let e = classify_terminator(&block.terminator);
        transfers.insert(
            block.id,
            MirHomeBlockTransfers {
                ops,
                terminator: transfer(&e.memory, &BTreeSet::new(), &BTreeSet::new()),
            },
        );
    }
    let homes = MirHomeLiveness::analyze_transfers(routine, &cfg, transfers);
    let live = MirMachineLiveness::entry_inputs(routine, &cfg)?;
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
    Some(Inputs {
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
    })
}

/// Byte widths alone cannot distinguish a pointer from an aggregate or a
/// variadic declaration. Lowering carries the complete finite ABI explicitly;
/// validate it against the current frame rather than consulting source meaning.
fn direct_parameter_abi_is_complete(routine: &MirRoutine) -> bool {
    let Some(widths) = &routine.frame.direct_parameter_abi else {
        return false;
    };
    matches!(
        routine.abi,
        MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry
    ) && widths.len() == routine.frame.params.len()
        && widths
            .iter()
            .copied()
            .map(crate::mir6502::abi::action_arg_width_bytes)
            .sum::<u16>()
            <= crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES
        && widths
            .iter()
            .zip(&routine.frame.params)
            .all(|(width, slot)| {
                slot.storage == MirStorageClass::Scalar
                    && slot.scalar_width == Some(*width)
                    && slot.storage_size == crate::mir6502::abi::action_arg_width_bytes(*width)
                    && slot.offset == 0
                    && slot.init.is_none()
                    && matches!(
                        slot.base,
                        MirStorageBase::Param(_) | MirStorageBase::ParamAbiOnly(_)
                    )
            })
}
