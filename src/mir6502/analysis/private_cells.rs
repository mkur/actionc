//! Path-sensitive observation proofs for unexposed ordinary source cells.
use super::{comparison_loops::same_object, effects::classify_op, sites::MirSite};
use crate::mir6502::{ir::*, rewrite::context::PreHomeRewriteContext};
use std::collections::BTreeSet;

fn offset(mem: &MirMem) -> Option<u16> {
    match mem {
        MirMem::Local { offset, .. } | MirMem::Param { offset, .. } => Some(*offset),
        _ => None,
    }
}
/// Starting immediately after a defining store, every reachable read of that
/// byte must be enumerated. A definite covering store ends this definition's
/// lifetime. Unknown effects and address exposure conservatively reject it.
pub(in crate::mir6502) fn reads_confined(
    r: &MirRoutine,
    c: &PreHomeRewriteContext<'_, '_>,
    cell: &MirMem,
    start: MirSite,
    allowed: &BTreeSet<MirSite>,
) -> bool {
    let Some(cell_offset) = offset(cell) else {
        return false;
    };
    // A separately declared local alias can expose the same cell without
    // spelling its identity in an address reference or a content read.
    if let MirMem::Local { id, .. } = cell {
        if r.frame
            .locals
            .iter()
            .any(|slot| matches!(slot.base,MirStorageBase::LocalAlias{target,..} if target==*id))
        {
            return false;
        }
    }
    // Address arithmetic from an exposed frame cell can reach neighboring
    // homes. Require the whole frame to remain unexposed until a bounded
    // provenance proof can establish a narrower range.
    if r.blocks.iter().flat_map(|b| &b.ops).any(|op| {
        classify_op(op)
            .memory
            .address_references
            .iter()
            .any(|m| matches!(m, MirMem::Local { .. } | MirMem::Param { .. }))
    }) {
        return false;
    }
    let MirSite::Op { block, op_index } = start else {
        return false;
    };
    let mut pending = vec![(block, op_index + 1)];
    let mut visited = BTreeSet::new();
    while let Some((id, first)) = pending.pop() {
        if !visited.insert((id, first)) {
            continue;
        }
        let Some(b) = r.blocks.iter().find(|b| b.id == id) else {
            return false;
        };
        let mut overwritten = false;
        for (i, op) in b.ops.iter().enumerate().skip(first) {
            let effect = classify_op(op);
            if matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) || effect.memory.indirect_writes
            {
                return false;
            }
            if effect
                .memory
                .direct_reads
                .iter()
                .any(|range| same_object(&range.base, cell))
                && !allowed.contains(&MirSite::Op {
                    block: id,
                    op_index: i,
                })
            {
                return false;
            }
            if effect.memory.direct_writes.iter().any(|range| {
                same_object(&range.base, cell)
                    && offset(&range.base)
                        .is_some_and(|o| cell_offset.wrapping_sub(o) < range.bytes)
            }) {
                overwritten = true;
                break;
            }
        }
        if !overwritten {
            pending.extend(c.cfg().successors(id).iter().map(|&id| (id, 0)));
        }
    }
    true
}
