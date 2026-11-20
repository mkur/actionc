//! Follow copies of incoming ABI bytes through selected instructions. A missing
//! lane in a reachable state means that the entry value is no longer known.
use super::*;
use crate::mir6502::ir::{MirBlock, MirBlockId};

type Origins = BTreeMap<Lane, u16>;

struct FormalOrigins<'a> {
    program: &'a MirProgram,
    routine: &'a MirRoutine,
    known: &'a MirKnownCalleeSummaries,
    cfg: MirCfg,
    inputs: Origins,
}

pub(in super::super) fn formal_origins(
    program: &MirProgram,
    routine: &MirRoutine,
    known: &MirKnownCalleeSummaries,
) -> BTreeMap<MirSite, u16> {
    let Some(params) = &routine.frame.direct_parameter_abi else {
        return BTreeMap::new();
    };
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
        .map(|(i, lane)| (lane.into(), i as u16))
        .collect();
    let problem = FormalOrigins {
        program,
        routine,
        known,
        cfg,
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

    fn argument(&self, pair: MirPointerPair, state: &Origins) -> Option<u16> {
        let lo = fixed_pair(self.routine, pair)?;
        let byte = |slot| {
            state
                .get(&Lane::Cell(MirMem::FixedZeroPage(MirFixedZpSlot(slot))))
                .copied()
        };
        let start = byte(lo)?;
        if byte(lo.wrapping_add(1))? != start.checked_add(1)? {
            return None;
        }
        let mut offset = 0;
        for param in self.routine.frame.direct_parameter_abi.as_ref()? {
            if start == offset && param.width().bytes() == 2 {
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
        mut store: impl FnMut(MirSite, u16),
    ) -> Origins {
        for (op_index, op) in block.ops.iter().enumerate() {
            if let Some((pair, ..)) = destination(op)
                && let Some(arg) = self.argument(pair, &state)
            {
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
            state.retain(|lane, _| match lane {
                Lane::Reg(r) => !effects.may_clobber_reg_compat(*r),
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
                Some(state) => state.retain(|lane, value| other.get(lane) == Some(value)),
                None => *into = Some(other.clone()),
            }
        }
    }
    fn transfer(&self, block: MirBlockId, state: &Self::State) -> Self::State {
        let block = &self.routine.blocks[self.cfg.block_index(block)?];
        Some(self.walk(block, state.as_ref()?.clone(), |_, _| {}))
    }
}
