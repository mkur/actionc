//! Resident-base extension. All call ranges are resolved with their real owner.
use super::*;
use crate::mir6502::analysis::known_callees::{MirKnownCalleeSummaries, MirKnownMemoryWrites};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct CallWrites {
    routines: BTreeMap<RoutineId, Option<BTreeSet<u16>>>,
}
impl CallWrites {
    pub fn analyze(program: &MirProgram, memory: &MirPhysicalMemory) -> Self {
        let known = MirKnownCalleeSummaries::analyze_for_rewriting(program);
        let mut result = Self::default();
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
    if matches!(op, MirOp::Call { .. }) {
        if let Some(writes) = calls.for_call(op, routine, memory) {
            for address in writes {
                state.write(address, 1);
            }
            // Call-result/register precision remains owned by machine_values.
            state.registers = [None; 3];
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
    let window = b.ops.get(op_index..op_index.checked_add(4)?)?;
    let pair = setup_pair(window)?;
    let mut state = State::default();
    for (i, op) in b.ops[..op_index].iter().enumerate() {
        observe(&mut state, op, i, r.id, memory, calls);
    }
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
