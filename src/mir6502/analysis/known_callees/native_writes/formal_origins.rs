//! Follow copies of incoming ABI bytes through selected instructions. A missing
//! lane in a reachable state means that the entry value is no longer known.
use super::*;
use crate::mir6502::ir::{MirBlock, MirBlockId};

/// If an earlier byte store went through this same entry pointer, the entry
/// address must differ from every carrier retained across that store.
#[derive(Debug, Copy, Clone, Default, Eq, PartialEq, Ord, PartialOrd)]
pub(in super::super) struct ExcludedBases([u64; 4]);

impl ExcludedBases {
    fn insert(&mut self, slot: u8) {
        self.0[usize::from(slot / 64)] |= 1 << (slot % 64);
    }

    fn union(&mut self, other: Self) {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a |= b;
        }
    }

    pub(super) fn contains(self, address: u16) -> bool {
        u8::try_from(address)
            .is_ok_and(|byte| self.0[usize::from(byte / 64)] & (1 << (byte % 64)) != 0)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(in super::super) struct EntryByte {
    pub(super) index: u16,
    pub(super) excluded: ExcludedBases,
}

type Origins = BTreeMap<Lane, EntryByte>;

struct FormalOrigins<'a> {
    program: &'a MirProgram,
    routine: &'a MirRoutine,
    known: &'a MirKnownCalleeSummaries,
    purpose: TracePurpose,
    cfg: MirCfg,
    inputs: Origins,
}

pub(in super::super) fn formal_origins(
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
    purpose: TracePurpose,
) -> BTreeMap<MirSite, EntryByte> {
    let Some(params) = &routine.frame.direct_parameter_abi else {
        return BTreeMap::new();
    };
    if !params.iter().any(|p| p.width().bytes() == 2) {
        return BTreeMap::new();
    }
    let Some(lanes) =
        crate::mir6502::abi::parameter_bytes(params).and_then(crate::mir6502::abi::argument_lanes)
    else {
        return BTreeMap::new();
    };
    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return BTreeMap::new();
    };
    let inputs = lanes
        .into_iter()
        .enumerate()
        .map(|(i, lane)| {
            (
                lane.into(),
                EntryByte {
                    index: i as u16,
                    excluded: Default::default(),
                },
            )
        })
        .collect();
    let problem = FormalOrigins {
        program,
        routine,
        known,
        cfg,
        purpose,
        inputs,
    };
    let facts = solve_dataflow(&problem.cfg, &problem);
    let mut sites = BTreeMap::new();
    for block in &routine.blocks {
        if let Some(Some(state)) = facts.in_state(block.id) {
            problem.walk(block, state.clone(), |site, argument| {
                sites.insert(site, argument);
            });
        }
    }
    sites
}

impl FormalOrigins<'_> {
    fn cell(&self, mem: &MirMem) -> Option<Lane> {
        Some(Lane::Cell(physical_cell(self.routine, self.known, mem)?))
    }

    fn copy(&self, op: &MirOp) -> Option<(Lane, Lane)> {
        let (dst, value) = match op {
            MirOp::Load {
                dst: MirDef::Reg(r),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            } => return Some((Lane::Reg(*r), self.cell(mem)?)),
            MirOp::Move {
                dst: MirDef::Reg(r),
                src,
                width: MirWidth::Byte,
            } => (Lane::Reg(*r), src),
            MirOp::Store {
                dst: MirAddr::Direct(mem),
                src,
                width: MirWidth::Byte,
            } => (self.cell(mem)?, src),
            _ => return None,
        };
        let src = match value {
            MirValue::Def(MirDef::Reg(r)) => Lane::Reg(*r),
            MirValue::PointerCell(mem) => self.cell(mem)?,
            _ => return None,
        };
        Some((dst, src))
    }

    fn argument(&self, pair: MirPointerPair, state: &Origins) -> Option<EntryByte> {
        let lo = fixed_pair(self.routine, pair)?;
        let byte = |slot| {
            state
                .get(&Lane::Cell(MirMem::FixedZeroPage(MirFixedZpSlot(slot))))
                .copied()
        };
        let mut start = byte(lo)?;
        let high = byte(lo.wrapping_add(1))?;
        start.excluded.union(high.excluded);
        if high.index != start.index.checked_add(1)? {
            return None;
        }
        let mut offset = 0;
        for param in self.routine.frame.direct_parameter_abi.as_ref()? {
            if start.index == offset && param.width().bytes() == 2 {
                return Some(start);
            }
            offset += param.width().bytes();
        }
        None
    }

    fn walk(
        &self,
        block: &MirBlock,
        mut state: Origins,
        mut store: impl FnMut(MirSite, EntryByte),
    ) -> Origins {
        for (op_index, op) in block.ops.iter().enumerate() {
            let dest = destination(op).and_then(|(pair, offset, bytes, indexed)| {
                Some((self.argument(pair, &state)?, offset, bytes, indexed))
            });
            if let Some((arg, ..)) = dest {
                store(
                    MirSite::Op {
                        block: block.id,
                        op_index,
                    },
                    arg,
                );
            }
            if matches!(op, MirOp::SArgsCapture { .. }) {
                let mut next = Origins::new();
                if let Some(capture) =
                    crate::mir6502::runtime_contracts::sargs_capture(self.program, self.routine, op)
                {
                    for (input, dst) in capture.inputs.iter().zip(capture.definitions) {
                        if let Some(lanes) =
                            crate::mir6502::abi::physical_lanes(input, MirWidth::Byte)
                            && let [input] = lanes.as_slice()
                            && let Some(origin) = state.get(&Lane::from(*input))
                            && let Some(cell) = self.cell(&dst)
                        {
                            next.insert(cell, *origin);
                        }
                    }
                }
                state = next;
                continue;
            }
            let copied = self
                .copy(op)
                .and_then(|(dst, src)| Some((dst, *state.get(&src)?)));
            let effects = classify_op(op);
            let mut writes = MirKnownMemoryWrites::exact();
            if matches!(op, MirOp::Call { target, .. } if self.known.for_target(target).is_none()) {
                writes.make_unknown();
            } else {
                summarize_op_writes(op, self.routine, self.known, &mut writes);
            }
            state.retain(|lane, origin| match lane {
                Lane::Reg(r) => !effects.may_clobber_reg_compat(*r),
                // A possible numeric observation survives a call in the
                // inventory even when preservation cannot be established.
                Lane::Cell(_)
                    if self.purpose == TracePurpose::NumericObservation
                        && matches!(op, MirOp::Call { .. }) =>
                {
                    true
                }
                Lane::Cell(MirMem::FixedZeroPage(slot)) => {
                    if writes.preserves_fixed_byte(*slot, &self.known.fixed_storage) {
                        return true;
                    }
                    // Only this pure indirect byte store has a single dynamic
                    // write at the unindexed entry address. Preserve copies of
                    // its own pointer under the corresponding call-site guard.
                    if let MirOp::StoreIndirect { .. } = op
                        && let Some((arg, 0, 1, false)) = dest
                        && (arg.index..=arg.index + 1).contains(&origin.index)
                    {
                        origin.excluded.union(arg.excluded);
                        origin.excluded.insert(slot.0);
                        return true;
                    }
                    false
                }
                Lane::Cell(mem) => !writes.may_write_mem(mem, &self.known.fixed_storage),
            });
            if let Some((dst, origin)) = copied {
                state.insert(dst, origin);
            }
        }
        let effects = super::super::super::effects::classify_terminator(&block.terminator);
        if effects.machine.possible_register_writes() != MirRegisterSet::default() {
            state.retain(|lane, _| matches!(lane, Lane::Cell(_)));
        }
        state
    }
}

impl DataflowProblem<MirCfg> for FormalOrigins<'_> {
    type State = Option<Origins>;
    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Forward
    }
    fn bottom(&self) -> Self::State {
        None
    }
    fn boundary(&self, block: MirBlockId) -> Option<Self::State> {
        (self.cfg.entry() == Some(block)).then(|| Some(self.inputs.clone()))
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        if let Some(other) = other {
            match into {
                Some(state) => state.retain(|lane, value| {
                    let Some(incoming) = other.get(lane) else {
                        return false;
                    };
                    if value.index != incoming.index {
                        return false;
                    }
                    value.excluded.union(incoming.excluded);
                    true
                }),
                None => *into = Some(other.clone()),
            }
        }
    }
    fn transfer(&self, block: MirBlockId, state: &Self::State) -> Self::State {
        let block = &self.routine.blocks[self.cfg.block_index(block)?];
        Some(self.walk(block, state.as_ref()?.clone(), |_, _| {}))
    }
}
