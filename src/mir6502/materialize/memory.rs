use crate::mir6502::analysis::effects::classify_op;
#[cfg(test)]
use crate::mir6502::analysis::effects::{MirOpKind, classify_value};
#[cfg(test)]
use crate::mir6502::ir::{MirCond, MirTerminator};
use crate::mir6502::ir::{
    MirEffects, MirFixedZpSlot, MirMem, MirMemoryEffect, MirMemoryRegionKind, MirOp,
};

pub(super) fn effects_may_write_fixed_pair(effects: &MirEffects, lo: MirFixedZpSlot) -> bool {
    effects.opaque || memory_effect_may_write_fixed_pair(&effects.memory_writes, lo)
}

fn memory_effect_may_write_fixed_pair(effect: &MirMemoryEffect, lo: MirFixedZpSlot) -> bool {
    match effect {
        MirMemoryEffect::None => false,
        MirMemoryEffect::Unknown | MirMemoryEffect::All => true,
        MirMemoryEffect::Regions(regions) => regions.iter().any(|region| {
            matches!(
                region.kind,
                MirMemoryRegionKind::ZeroPage | MirMemoryRegionKind::AbsoluteRange
            ) && ranges_overlap(region.offset, region.size, u16::from(lo.0), 2)
        }),
    }
}

pub(super) fn ranges_overlap(
    left_start: u16,
    left_len: u16,
    right_start: u16,
    right_len: u16,
) -> bool {
    let left_end = left_start.saturating_add(left_len);
    let right_end = right_start.saturating_add(right_len);
    left_start < right_end && right_start < left_end
}

#[cfg(test)]
pub(super) fn mem_is_read_after(
    ops: &[MirOp],
    start: usize,
    terminator: &MirTerminator,
    mem: &MirMem,
) -> bool {
    ops[start..].iter().any(|op| op_reads_mem(op, mem)) || terminator_reads_mem(terminator, mem)
}

#[cfg(test)]
fn terminator_reads_mem(terminator: &MirTerminator, mem: &MirMem) -> bool {
    // This compatibility query historically considered only the branch
    // condition. Routine-wide analyses consume the complete terminator summary,
    // including edge arguments.
    matches!(
        terminator,
        MirTerminator::Branch {
            cond: MirCond::BoolValue(value),
            ..
        } if classify_value(value).memory.reads(mem)
    )
}

pub(super) fn op_reads_mem(op: &MirOp, mem: &MirMem) -> bool {
    // Runtime-helper ABI homes were outside the old local memory query.
    !matches!(op, MirOp::RuntimeHelper { .. }) && classify_op(op).memory.reads(mem)
}

#[cfg(test)]
pub(super) fn op_definitely_writes_mem(op: &MirOp, mem: &MirMem) -> bool {
    let effects = classify_op(op);
    matches!(
        effects.kind,
        MirOpKind::Store | MirOpKind::UpdateMem | MirOpKind::UpdateIndexedMem
    ) && effects.memory.definitely_writes(mem)
}

pub(super) fn op_may_have_unknown_memory_effects(op: &MirOp) -> bool {
    classify_op(op).memory.has_unknown_effects_compat
}

pub(super) fn op_may_write_mem(op: &MirOp, mem: &MirMem) -> bool {
    let effects = classify_op(op);
    if matches!(op, MirOp::RuntimeHelper { .. }) {
        effects.memory.may_write_any_compat
    } else {
        effects.memory.may_write_compat(mem)
    }
}
