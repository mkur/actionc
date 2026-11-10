//! Resident-base extension. All call ranges are resolved with their real owner.
use super::*;
use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::known_callees::{MirKnownCalleeSummaries, MirKnownMemoryWrites};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct CallWrites {
    routines: BTreeMap<RoutineId, Option<BTreeSet<u16>>>,
    known: MirKnownCalleeSummaries,
}
impl CallWrites {
    pub fn analyze(program: &MirProgram, memory: &MirPhysicalMemory) -> Self {
        let known = MirKnownCalleeSummaries::analyze_for_rewriting(program);
        let mut result = Self {
            known: known.clone(),
            ..Self::default()
        };
        for r in &program.routines {
            result.collect(r.id, program, memory, &known, &mut BTreeSet::new());
        }
        result
    }
    fn collect(
        &mut self,
        id: RoutineId,
        program: &MirProgram,
        memory: &MirPhysicalMemory,
        known: &MirKnownCalleeSummaries,
        active: &mut BTreeSet<RoutineId>,
    ) -> Option<BTreeSet<u16>> {
        if let Some(cached) = self.routines.get(&id) {
            return cached.clone();
        }
        if active.len() >= 16 || !active.insert(id) {
            return None;
        }
        let result = (|| {
            let r = program.routines.iter().find(|r| r.id == id)?;
            if r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() > 512 {
                return None;
            }
            let mut writes = BTreeSet::new();
            if r.blocks.len() == 1
                && matches!(r.blocks[0].ops.as_slice(), [MirOp::MachineBlock { effects, .. }] if !effects.opaque && !effects.may_call_os)
            {
                let MirKnownMemoryWrites::Exact(bytes) = known.get(id)?.writes() else {
                    return None;
                };
                for byte in bytes {
                    writes.insert(memory.address(id, byte)?);
                }
                return Some(writes);
            }
            for op in r.blocks.iter().flat_map(|b| &b.ops) {
                if let MirOp::Call {
                    target: MirCallTarget::Routine(callee),
                    effects,
                    ..
                } = op
                {
                    if effects.opaque || effects.may_call_os {
                        return None;
                    }
                    writes.extend(self.collect(*callee, program, memory, known, active)?);
                    writes.extend(argument_writes(op, r.id, memory)?);
                    continue;
                }
                if matches!(
                    op,
                    MirOp::Call { .. }
                        | MirOp::RuntimeHelper { .. }
                        | MirOp::MachineBlock { .. }
                        | MirOp::Barrier { .. }
                ) {
                    return None;
                }
                let effect = classify_op(op);
                if effect.memory.indirect_writes
                    || effect.memory.opaque
                    || effect.memory.may_write_any
                    || effect.memory.has_unknown_effects
                    || effect.homes.unknown_writes
                {
                    return None;
                }
                for range in &effect.memory.direct_writes {
                    let start = memory.address(id, &range.base)?;
                    writes.extend((0..range.bytes).map(|n| start.wrapping_add(n)));
                }
                for home in &effect.addresses.pair_writes {
                    use crate::mir6502::analysis::effects::MirHomeByte;
                    match home {
                        MirHomeByte::FixedZeroPage(slot) => {
                            writes.insert(u16::from(slot.0));
                        }
                        _ => return None,
                    }
                }
            }
            Some(writes)
        })();
        active.remove(&id);
        self.routines.insert(id, result.clone());
        result
    }
    pub fn for_call(
        &self,
        op: &MirOp,
        owner: RoutineId,
        memory: &MirPhysicalMemory,
    ) -> Option<BTreeSet<u16>> {
        let MirOp::Call {
            target: MirCallTarget::Routine(id),
            effects,
            ..
        } = op
        else {
            return None;
        };
        if effects.opaque || effects.may_call_os {
            return None;
        }
        let mut writes = self.routines.get(id)?.clone()?;
        writes.extend(argument_writes(op, owner, memory)?);
        Some(writes)
    }
}

fn argument_writes(
    op: &MirOp,
    owner: RoutineId,
    memory: &MirPhysicalMemory,
) -> Option<BTreeSet<u16>> {
    let MirOp::Call { args, .. } = op else {
        return None;
    };
    let mut writes = BTreeSet::new();
    for arg in args {
        let address = match arg.home {
            MirArgHome::Reg(_) | MirArgHome::RegisterPair { .. } => continue,
            MirArgHome::FixedZeroPage(slot) => u16::from(slot.0),
            MirArgHome::ZeroPage(slot) => memory.address(owner, &MirMem::ZeroPage(slot))?,
            _ => return None,
        };
        let bytes = if arg.width == MirWidth::Word { 2 } else { 1 };
        writes.extend((0..bytes).map(|n| address.wrapping_add(n)));
    }
    Some(writes)
}

fn observe(
    state: &mut State,
    op: &MirOp,
    index: usize,
    routine: RoutineId,
    memory: &MirPhysicalMemory,
    calls: &CallWrites,
) {
    observe_retained(state, op, index, routine, memory, calls, false);
}

fn observe_retained(
    state: &mut State,
    op: &MirOp,
    index: usize,
    routine: RoutineId,
    memory: &MirPhysicalMemory,
    calls: &CallWrites,
    extended: bool,
) {
    if matches!(op, MirOp::Call { .. }) {
        if let Some(writes) = calls.for_call(op, routine, memory) {
            for address in writes {
                state.write(address, 1);
            }
            for reg_value in [MirReg::A, MirReg::X, MirReg::Y] {
                let preserved = extended
                    && matches!(op,MirOp::Call{target,..} if calls.known.for_target(target).is_some_and(|s|s.preserves_register(reg_value)));
                if !preserved {
                    state.registers[reg(reg_value)] = None;
                }
            }
        } else {
            *state = State::default();
        }
    } else if extended
        && matches!(op,MirOp::Load {dst:MirDef::Reg(_),src:MirAddr::Direct(mem),width:MirWidth::Byte} if memory.address(routine,mem).is_some_and(|a|a<0x100) && !memory.ordinary(routine,mem))
    {
        // The read still executes. This does not license deleting absolute reads.
        if let MirOp::Load {
            dst: MirDef::Reg(r),
            ..
        } = op
        {
            state.registers[reg(*r)] = None;
        }
    } else if extended
        && let MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY { zp },
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } = op
    {
        let base = state
            .pairs
            .get(&zp.0)
            .zip(state.pairs.get(&zp.0.wrapping_add(1)))
            .and_then(|(lo, hi)| match (lo.0, hi.0) {
                (ByteValue::Constant(lo), ByteValue::Constant(hi)) => {
                    Some(u16::from_le_bytes([lo, hi]))
                }
                _ => None,
            });
        let (offset, bytes) = match state.registers[reg(MirReg::Y)] {
            Some(ByteValue::Constant(y)) => (u16::from(y), 1),
            _ => (0, 256),
        };
        if let Some(start) = base
            .map(|base| base.wrapping_add(offset))
            .filter(|start| memory.contains_write(*start, bytes))
        {
            state.write(start, bytes);
        } else {
            *state = State::default();
        }
    } else {
        state.observe(
            op,
            index,
            routine,
            memory,
            RecordBaseOptions {
                writes: true,
                parameter_origins: true,
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct RetentionProof {
    pub routine: RoutineId,
    pub generation: MirRoutineGeneration,
    pub site: MirSite,
    pub pair: u8,
    values: [ByteValue; 2],
}

pub(in crate::mir6502) fn prove(
    r: &MirRoutine,
    generation: MirRoutineGeneration,
    site: MirSite,
    memory: &MirPhysicalMemory,
    calls: &CallWrites,
) -> Option<RetentionProof> {
    let MirSite::Op { block, op_index } = site else {
        return None;
    };
    let b = r.blocks.iter().find(|b| b.id == block)?;
    let mut state = State::default();
    for (i, op) in b.ops[..op_index].iter().enumerate() {
        observe(&mut state, op, i, r.id, memory, calls);
    }
    proof_at(r, generation, site, memory, &state)
}

fn proof_at(
    r: &MirRoutine,
    generation: MirRoutineGeneration,
    site: MirSite,
    memory: &MirPhysicalMemory,
    state: &State,
) -> Option<RetentionProof> {
    let MirSite::Op { block, op_index } = site else {
        return None;
    };
    let b = r.blocks.iter().find(|b| b.id == block)?;
    let window = b.ops.get(op_index..op_index.checked_add(4)?)?;
    let pair = setup_pair(window)?;
    let values = [
        state.value(
            &load_source(&window[0])?,
            r.id,
            memory,
            RecordBaseOptions {
                writes: true,
                parameter_origins: true,
            },
        )?,
        state.value(
            &load_source(&window[2])?,
            r.id,
            memory,
            RecordBaseOptions {
                writes: true,
                parameter_origins: true,
            },
        )?,
    ];
    if [state.pairs.get(&pair)?.0, state.pairs.get(&(pair + 1))?.0] != values {
        return None;
    }
    // Source reads must be ordinary and cannot name the overwritten pair.
    for input in [load_source(&window[0])?, load_source(&window[2])?] {
        if let MirValue::PointerCell(mem) = input {
            if !memory.ordinary(r.id, &mem)
                || [u16::from(pair), u16::from(pair) + 1].contains(&memory.address(r.id, &mem)?)
            {
                return None;
            }
        }
    }
    Some(RetentionProof {
        routine: r.id,
        generation,
        site,
        pair,
        values,
    })
}

/// Must-agree complete-byte facts. No entry/backedge fact is assumed.
pub(in crate::mir6502) struct ResidentFacts<'routine> {
    source: &'routine MirRoutine,
    entries: BTreeMap<MirBlockId, Option<State>>,
}
impl<'routine> ResidentFacts<'routine> {
    pub fn analyze(
        r: &'routine MirRoutine,
        memory: &MirPhysicalMemory,
        calls: &CallWrites,
    ) -> Option<Self> {
        if r.blocks.len() > 256 || r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() > 4096 {
            return None;
        }
        let cfg = MirCfg::from_routine(r).ok()?;
        let problem = ResidentProblem {
            r,
            memory,
            calls,
            entry: cfg.entry(),
        };
        let result = solve_dataflow(&cfg, &problem);
        let entries = r
            .blocks
            .iter()
            .map(|b| (b.id, result.in_state(b.id).cloned().flatten()))
            .collect();
        Some(Self { source: r, entries })
    }
    fn at(
        &self,
        r: &MirRoutine,
        site: MirSite,
        memory: &MirPhysicalMemory,
        calls: &CallWrites,
    ) -> Option<State> {
        if !std::ptr::eq(self.source, r) {
            return None;
        }
        let MirSite::Op { block, op_index } = site else {
            return None;
        };
        let mut state = self.entries.get(&block)?.clone()?;
        let b = r.blocks.iter().find(|b| b.id == block)?;
        for (i, op) in b.ops.get(..op_index)?.iter().enumerate() {
            observe_retained(&mut state, op, i, r.id, memory, calls, true);
        }
        Some(state)
    }
    pub fn prove(
        &self,
        r: &MirRoutine,
        generation: MirRoutineGeneration,
        site: MirSite,
        memory: &MirPhysicalMemory,
        calls: &CallWrites,
    ) -> Option<RetentionProof> {
        proof_at(
            r,
            generation,
            site,
            memory,
            &self.at(r, site, memory, calls)?,
        )
    }
}

impl ResidentFacts<'_> {
    pub fn matches_pair(
        &self,
        r: &MirRoutine,
        site: MirSite,
        pair: u8,
        base: &MirValue,
        memory: &MirPhysicalMemory,
        calls: &CallWrites,
    ) -> bool {
        let Some(state) = self.at(r, site, memory, calls) else {
            return false;
        };
        let MirValue::Word { lo, hi } = base else {
            return false;
        };
        for value in [lo.as_ref(), hi.as_ref()] {
            match value {
                MirValue::PointerCell(mem)
                    if memory.ordinary(r.id, mem)
                        && memory
                            .address(r.id, mem)
                            .is_some_and(|a| a != u16::from(pair) && a != u16::from(pair) + 1) => {}
                MirValue::ConstU8(_)
                | MirValue::ConstU16(0..=255)
                | MirValue::StorageAddrByte { byte: 0..=1, .. } => {}
                _ => return false,
            }
        }
        let options = RecordBaseOptions {
            writes: true,
            parameter_origins: true,
        };
        let Some(low) = state.value(lo, r.id, memory, options) else {
            return false;
        };
        let Some(high) = state.value(hi, r.id, memory, options) else {
            return false;
        };
        [
            state.pairs.get(&pair).map(|v| v.0),
            state.pairs.get(&(pair + 1)).map(|v| v.0),
        ] == [Some(low), Some(high)]
    }
}

struct ResidentProblem<'a> {
    r: &'a MirRoutine,
    memory: &'a MirPhysicalMemory,
    calls: &'a CallWrites,
    entry: Option<MirBlockId>,
}

#[derive(Clone, Debug)]
pub(in crate::mir6502) struct EnteredLoop {
    pub preheader: MirBlockId,
    pub body: MirBlockId,
    pub guard: MirBlockId,
    pub exit: MirBlockId,
    pub counter: MirMem,
    pub iterations: u16,
}

/// A small entered byte loop: zero initialization, one increment and `< N`.
/// This proves the actual backedge/entry and rejects aliases of its counter.
pub(in crate::mir6502) fn entered_loops(
    r: &MirRoutine,
    map: &MirPhysicalMemory,
    calls: &CallWrites,
) -> Vec<EnteredLoop> {
    let Ok(cfg) = MirCfg::from_routine(r) else {
        return vec![];
    };
    let dominance = crate::mir6502::analysis::dominance::MirDominance::from_cfg(&cfg);
    let mut result = vec![];
    for guard in &r.blocks {
        let [
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(counter),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                op: MirCompareOp::Lt,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right,
                width: MirWidth::Byte,
                signed: false,
                ..
            },
        ] = guard.ops.as_slice()
        else {
            continue;
        };
        let bound = match right {
            MirValue::ConstU8(n) => u16::from(*n),
            MirValue::ConstU16(1..=255) => {
                if let MirValue::ConstU16(n) = right {
                    *n
                } else {
                    unreachable!()
                }
            }
            _ => continue,
        };
        if bound == 0 || !map.ordinary(r.id, counter) {
            continue;
        }
        let MirTerminator::Branch {
            cond,
            then_edge,
            else_edge,
        } = &guard.terminator
        else {
            continue;
        };
        if !matches!(
            cond,
            MirCond::FlagTest(MirFlagTest::CClear)
                | MirCond::FusedCompare {
                    flag_test: MirFlagTest::CClear,
                    ..
                }
        ) || !then_edge.args.is_empty()
            || !else_edge.args.is_empty()
        {
            continue;
        }
        let Some(body) = r.blocks.iter().find(|b| b.id == then_edge.target) else {
            continue;
        };
        if !matches!(&body.terminator,MirTerminator::Jump(e) if e.target==guard.id && e.args.is_empty())
            || cfg.predecessors(guard.id) != &BTreeSet::from([body.id])
        {
            continue;
        }
        let outside: Vec<_> = cfg
            .predecessors(body.id)
            .iter()
            .copied()
            .filter(|id| *id != guard.id)
            .collect();
        let [pre] = outside.as_slice() else {
            continue;
        };
        let Some(preheader) = r.blocks.iter().find(|b| b.id == *pre) else {
            continue;
        };
        if !dominance.block_dominates(*pre, body.id) {
            continue;
        }
        if !matches!(&preheader.terminator,MirTerminator::Jump(e) if e.target==body.id && e.args.is_empty())
            || cfg.predecessors(body.id).len() != 2
        {
            continue;
        }
        if !matches!(preheader.ops.as_slice(),[..,MirOp::LoadImm {dst:MirDef::Reg(MirReg::A),value:0,width:MirWidth::Byte},MirOp::Store {dst:MirAddr::Direct(m),src:MirValue::Def(MirDef::Reg(MirReg::A)),width:MirWidth::Byte}] if m==counter)
        {
            continue;
        }
        let Some(address) = map.address(r.id, counter) else {
            continue;
        };
        let mut increments = 0;
        let safe=body.ops.iter().all(|op| {
            if matches!(op,MirOp::UpdateMem {op:MirUpdateOp::Inc,mem,width:MirWidth::Byte} if mem==counter) {increments+=1;return true;}
            if matches!(op,MirOp::Call {..}) {return calls.for_call(op,r.id,map).is_some_and(|writes|!writes.contains(&address));}
            if matches!(op,MirOp::MachineBlock {..} | MirOp::RuntimeHelper {..} | MirOp::Barrier {..}) {return false;}
            let e=classify_op(op);
            if e.memory.indirect_writes || e.memory.may_write_any || e.memory.opaque || e.homes.unknown_writes {return false;}
            e.memory.direct_writes.iter().all(|w|map.address(r.id,&w.base).is_some_and(|a|address.wrapping_sub(a)>=w.bytes))
        });
        if safe && increments == 1 {
            result.push(EnteredLoop {
                preheader: *pre,
                body: body.id,
                guard: guard.id,
                exit: else_edge.target,
                counter: counter.clone(),
                iterations: bound,
            });
        }
    }
    result
}
impl DataflowProblem<MirCfg> for ResidentProblem<'_> {
    type State = Option<State>;
    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Forward
    }
    fn bottom(&self) -> Self::State {
        None
    }
    fn boundary(&self, node: MirBlockId) -> Option<Self::State> {
        (Some(node) == self.entry).then(|| Some(State::default()))
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        let Some(other) = other else {
            return;
        };
        let Some(into) = into else {
            *into = Some(other.clone());
            return;
        };
        for i in 0..3 {
            if into.registers[i] != other.registers[i] {
                into.registers[i] = None;
            }
        }
        into.cells
            .retain(|cell, value| other.cells.get(cell) == Some(value));
        into.pairs
            .retain(|slot, (value, _)| other.pairs.get(slot).is_some_and(|(v, _)| v == value));
        // Definition positions are local to legacy block proofs. CFG equality
        // uses exact values and the enclosing immutable routine generation.
        for (_, definition) in into.pairs.values_mut() {
            *definition = 0;
        }
    }
    fn transfer(&self, node: MirBlockId, state: &Self::State) -> Self::State {
        let mut state = state.clone()?;
        let block = self.r.blocks.iter().find(|b| b.id == node)?;
        for (i, op) in block.ops.iter().enumerate() {
            observe_retained(&mut state, op, i, self.r.id, self.memory, self.calls, true);
        }
        if matches!(
            block.terminator,
            MirTerminator::Branch {
                cond: MirCond::BoolValue(_) | MirCond::Deferred,
                ..
            }
        ) {
            state = State::default();
        }
        for (_, definition) in state.pairs.values_mut() {
            *definition = 0;
        }
        Some(state)
    }
}

pub(in crate::mir6502) struct RedundantSetup {
    pub proof: RetentionProof,
    pub len: usize,
    pub loads: Vec<MirOp>,
}
impl ResidentFacts<'_> {
    /// Two complete stores, with up to two ordinary accumulator producers.
    /// This admits existing A/X/Y placement without requiring four LDA/STA ops.
    pub fn redundant_setup(
        &self,
        r: &MirRoutine,
        generation: MirRoutineGeneration,
        site: MirSite,
        memory: &MirPhysicalMemory,
        calls: &CallWrites,
    ) -> Option<RedundantSetup> {
        let MirSite::Op { block, op_index } = site else {
            return None;
        };
        let before = self.at(r, site, memory, calls)?;
        let mut after = before.clone();
        let b = r.blocks.iter().find(|b| b.id == block)?;
        let mut low = None;
        let mut loads = vec![];
        for (offset, op) in b.ops.get(op_index..)?.iter().take(4).enumerate() {
            match op {
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(slot)),
                    src: MirValue::Def(MirDef::Reg(_)),
                    width: MirWidth::Byte,
                } => {
                    if low.is_none() && [0xAA, 0xAC, 0xAE].contains(&slot.0) {
                        low = Some(slot.0);
                    } else if low.is_some_and(|lo| slot.0 == lo + 1) {
                        observe_retained(&mut after, op, offset, r.id, memory, calls, true);
                        let pair = low?;
                        let values = [before.pairs.get(&pair)?.0, before.pairs.get(&(pair + 1))?.0];
                        if [after.pairs.get(&pair)?.0, after.pairs.get(&(pair + 1))?.0] != values {
                            return None;
                        }
                        return Some(RedundantSetup {
                            proof: RetentionProof {
                                routine: r.id,
                                generation,
                                site,
                                pair,
                                values,
                            },
                            len: offset + 1,
                            loads,
                        });
                    } else {
                        return None;
                    }
                }
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if memory.ordinary(r.id, mem) => {
                    loads.push(op.clone());
                }
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::A),
                    src,
                    width: MirWidth::Byte,
                } if after
                    .value(
                        src,
                        r.id,
                        memory,
                        RecordBaseOptions {
                            writes: true,
                            parameter_origins: true,
                        },
                    )
                    .is_some() =>
                {
                    loads.push(op.clone());
                }
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    width: MirWidth::Byte,
                    ..
                } => {
                    loads.push(op.clone());
                }
                _ => return None,
            }
            observe_retained(&mut after, op, offset, r.id, memory, calls, true);
        }
        None
    }
}

/// Diagnostic attempts are not additive savings or executable assumptions.
pub(in crate::mir6502) fn blocker(
    r: &MirRoutine,
    site: MirSite,
    memory: &MirPhysicalMemory,
    calls: &CallWrites,
) -> &'static str {
    let MirSite::Op { block, op_index } = site else {
        return "pointer-retention-unsupported-site";
    };
    let Some(b) = r.blocks.iter().find(|b| b.id == block) else {
        return "pointer-retention-unsupported-site";
    };
    let Some(window) = b.ops.get(op_index..op_index + 4) else {
        return "pointer-retention-unsupported-setup";
    };
    let Some(pair) = setup_pair(window) else {
        return "pointer-retention-unsupported-setup";
    };
    let sources: Vec<_> = [load_source(&window[0]), load_source(&window[2])]
        .into_iter()
        .flatten()
        .filter_map(|v| {
            if let MirValue::PointerCell(m) = v {
                memory.address(r.id, &m)
            } else {
                None
            }
        })
        .collect();
    let mut reason = "pointer-retention-unavailable-base";
    for op in &b.ops[..op_index] {
        if matches!(op, MirOp::Call { .. }) {
            match calls.for_call(op, r.id, memory) {
                None => reason = "pointer-retention-unknown-call-footprint",
                Some(writes)
                    if writes.contains(&u16::from(pair))
                        || writes.contains(&(u16::from(pair) + 1)) =>
                {
                    reason = "pointer-retention-pair-clobber"
                }
                Some(writes) if sources.iter().any(|s| writes.contains(s)) => {
                    reason = "pointer-retention-source-mutation"
                }
                _ => {}
            }
        } else if classify_op(op).memory.indirect_writes {
            reason = "pointer-retention-unbounded-write";
        }
    }
    reason
}
