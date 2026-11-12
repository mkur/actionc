//! Complete post-copy regions, with a sealed physical and value witness.
use super::*;
use crate::mir6502::analysis::address_ranges::NumericHomeRange;
use crate::mir6502::analysis::effects::MirHomeByte;
use crate::mir6502::analysis::sites::MirProgramPoint;
use crate::mir6502::rewrite::context::{MirExitStateChange, MirProof, PostHomeRewriteContext};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct CompoundRegionProof {
    source: MirRoutine,
    point: MirProgramPoint,
    original: Vec<MirOp>,
    replacement: Vec<MirOp>,
    calls: CallWrites,
    memory: MirPhysicalMemory,
}
impl CompoundRegionProof {
    pub fn point(&self) -> MirProgramPoint {
        self.point
    }
    pub fn original(&self) -> &[MirOp] {
        &self.original
    }
    pub fn replacement(&self) -> &[MirOp] {
        &self.replacement
    }
    pub fn validate(
        &self,
        r: &MirRoutine,
        context: &PostHomeRewriteContext<'_, '_>,
        memory: &MirPhysicalMemory,
    ) -> bool {
        self.source == *r
            && self.point.generation == context.generation()
            && self.memory == *memory
            && prove_compound_region(r, context, self.point.site, memory, &self.calls).as_ref()
                == Some(self)
    }
}

fn fixed(pair: u8) -> MirAddressConsumer {
    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(pair),
    })
}
fn pair_values(state: &State, pair: u8) -> Option<[ByteValue; 2]> {
    Some([
        state.pairs.get(&pair)?.0,
        state.pairs.get(&pair.checked_add(1)?)?.0,
    ])
}
fn requested(
    state: &State,
    window: &[MirOp],
    owner: RoutineId,
    memory: &MirPhysicalMemory,
) -> Option<[ByteValue; 2]> {
    setup_pair(window)?;
    let options = RecordBaseOptions {
        writes: true,
        parameter_origins: true,
    };
    let mut result = Vec::new();
    for op in [&window[0], &window[2]] {
        let v = load_source(op)?;
        if let MirValue::PointerCell(mem) = &v {
            if !memory.ordinary(owner, mem) {
                return None;
            }
        }
        result.push(state.value(&v, owner, memory, options)?);
    }
    Some([result[0], result[1]])
}
fn numeric(
    values: [ByteValue; 2],
    homes: &[NumericHomeRange],
    owner: RoutineId,
    memory: &MirPhysicalMemory,
) -> Option<(u16, u16)> {
    match values {
        [ByteValue::Constant(lo), ByteValue::Constant(hi)] => {
            let n = u16::from_le_bytes([lo, hi]);
            Some((n, n))
        }
        [ByteValue::Memory(lo), ByteValue::Memory(hi)] => homes.iter().find_map(|h| {
            (memory.address(owner, &h.lo) == Some(lo) && memory.address(owner, &h.hi) == Some(hi))
                .then_some((h.min, h.max))
        }),
        _ => None,
    }
}

pub(in crate::mir6502) fn prove_compound_region(
    r: &MirRoutine,
    c: &PostHomeRewriteContext<'_, '_>,
    site: MirSite,
    memory: &MirPhysicalMemory,
    calls: &CallWrites,
) -> Option<CompoundRegionProof> {
    // Published/current-location entries can expose scratch exit values.
    if !calls.compatible(r) || r.abi != MirRoutineAbi::Action {
        return None;
    }
    let MirSite::Op {
        block,
        op_index: index,
    } = site
    else {
        return None;
    };
    let b = r.blocks.iter().find(|b| b.id == block)?;
    if index == 0
        || index.checked_add(9)? != b.ops.len()
        || !matches!(b.terminator, MirTerminator::Return)
    {
        return None;
    }
    let MirOp::Call {
        target: MirCallTarget::Routine(callee),
        ..
    } = &b.ops[index - 1]
    else {
        return None;
    };
    if !calls.ranged.contains_key(&(r.id, *callee)) {
        return None;
    }
    let homes = calls.homes.get(&(r.id, *callee))?;
    let ops = b.ops.get(index..)?;
    let old_target = setup_pair(&ops[..4])?;
    let old_source = setup_pair(&ops[4..8])?;
    if old_target == old_source {
        return None;
    }
    let MirOp::IndirectByteCompound {
        op: MirBinaryOp::Add,
        target,
        source,
        offset: 0,
    } = &ops[8]
    else {
        return None;
    };
    if *target != fixed(old_target) || *source != fixed(old_source) {
        return None;
    }
    let facts = ResidentFacts::analyze(r, memory, calls)?;
    let state = facts.at(r, site, memory, calls)?;
    let target_values = requested(&state, &ops[..4], r.id, memory)?;
    let source_values = requested(&state, &ops[4..8], r.id, memory)?;
    let resident = |values| {
        [0xAC, 0xAE, 0xAA]
            .into_iter()
            .find(|&p| pair_values(&state, p) == Some(values))
    };
    let new_target = resident(target_values)?;
    let new_source = resident(source_values)?;
    if new_target == new_source {
        return None;
    }
    let changed = [old_target, old_target + 1, old_source, old_source + 1];
    for values in [target_values, source_values] {
        let (lo, hi) = numeric(values, homes, r.id, memory)?;
        // The final indirect reads happen after the copy. Neither can observe
        // changed pair cells, code, stack or hardware. No old length is reused.
        if !memory.retention_ram_range(lo, u32::from(hi) + 1) {
            return None;
        }
        for v in values {
            if let ByteValue::Memory(a) = v {
                if changed.iter().any(|b| a == u16::from(*b)) {
                    return None;
                }
            }
        }
    }
    // Scratch may not also be a source-visible global or frame cell.
    for byte in changed {
        let a = u16::from(byte);
        if memory
            .globals
            .values()
            .any(|&(base, size, _)| a.wrapping_sub(base) < size)
            || memory.entries.iter().any(|(_, m, p, _)| {
                *p == a && matches!(m, MirMem::Param { .. } | MirMem::Local { .. })
            })
        {
            return None;
        }
    }
    let mut exit = MirExitStateChange::default();
    exit.homes
        .extend(changed.map(|b| MirHomeByte::FixedZeroPage(MirFixedZpSlot(b))));
    let end = c.point(MirSite::Op {
        block,
        op_index: index + 8,
    });
    if !matches!(
        c.exit_state_change_is_unobservable(&exit, end),
        MirProof::Proven(())
    ) {
        return None;
    }
    // Loads/stores removed here affect only A/N/Z, all defined by the retained
    // compound. X, Y, SP, carry/overflow and all published facts stay equal.
    Some(CompoundRegionProof {
        source: r.clone(),
        point: c.point(site),
        original: ops.to_vec(),
        replacement: vec![MirOp::IndirectByteCompound {
            op: MirBinaryOp::Add,
            target: fixed(new_target),
            source: fixed(new_source),
            offset: 0,
        }],
        calls: calls.clone(),
        memory: memory.clone(),
    })
}

#[cfg(test)]
#[path = "compound_tests.rs"]
mod tests;
