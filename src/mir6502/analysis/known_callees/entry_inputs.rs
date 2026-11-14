//! Shared entry obligations, separate from exit preservation and full memory
//! footprints. Facts own their source bodies, so stale sites cannot reuse them.
use super::*;
use crate::mir6502::analysis::{
    effects::{MirFlagSet, classify_terminator},
    home_liveness::{MirHomeBlockTransfers, MirHomeLiveness, MirHomeTransfer},
    machine_liveness::MirMachineLiveness,
};
use crate::mir6502::ir::MirParamLane;
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
            scalar_inputs(program, routine)?
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

fn scalar_inputs(program: &MirProgram, routine: &MirRoutine) -> Option<Inputs> {
    if routine.scalar_signature.is_none()
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
            MirMem::Param { id, offset } => match routine
                .frame
                .parameter_lane(*id, offset.checked_add(delta)?)?
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
    let machines = MirMachineLiveness::analyze(routine, &cfg);
    let live = machines.live_in(entry)?;
    Some(Inputs {
        registers: MirRegisterSet {
            a: live.register_live(MirReg::A),
            x: live.register_live(MirReg::X),
            y: live.register_live(MirReg::Y),
            sp: live.stack_pointer_live(),
            flags: false,
        },
        flags: MirMachineLiveness::entry_flag_inputs(routine, &cfg)?,
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
