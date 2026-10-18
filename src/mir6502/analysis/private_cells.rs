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

pub(in crate::mir6502) fn frame_exposed_in_data(program: &MirProgram, routine: RoutineId) -> bool {
    let image = |image: &MirDataImage| {
        image
            .relocations
            .iter()
            .any(|r| matches!(r.target, MirDataRelocationTarget::Local { routine: owner, .. } | MirDataRelocationTarget::Param { routine: owner, .. } if owner == routine))
    };
    let init = |init: &MirStorageInit| match init {
        MirStorageInit::Bytes { image: data, .. } => image(data),
        MirStorageInit::Descriptor { backing, .. } => image(&backing.image),
        _ => false,
    };
    program.statics.iter().any(|s| image(&s.image))
        || program.globals.iter().any(|g| match &g.init {
            Some(MirGlobalInit::Bytes { image: data, .. }) => image(data),
            Some(MirGlobalInit::Descriptor { backing, .. }) => image(&backing.image),
            _ => false,
        })
        || program
            .routines
            .iter()
            .flat_map(|r| r.frame.params.iter().chain(&r.frame.locals))
            .any(|s| s.init.as_ref().is_some_and(init))
}

pub(in crate::mir6502) fn physical_cells_unaliased(
    p: &MirProgram,
    origin: u16,
    cells: &std::collections::BTreeMap<RoutineId, Vec<MirMem>>,
) -> bool {
    let layout = crate::mir6502::materialize::MaterializeLayout::new(p, origin);
    let mut addresses = vec![];
    for (&id, mems) in cells {
        for mem in mems {
            let Some(a) = layout.mem_address(id, mem) else {
                return false;
            };
            if a < 0x100 {
                return false;
            }
            addresses.push(a);
        }
    }
    let overlaps = |a: u16, bytes: u16| addresses.iter().any(|&x| x.wrapping_sub(a) < bytes);
    for g in &p.globals {
        if g.storage_size > 0
            && layout
                .mem_address(
                    RoutineId(0),
                    &MirMem::Global {
                        id: g.id,
                        offset: 0,
                    },
                )
                .is_none_or(|a| overlaps(a, g.storage_size))
        {
            return false;
        }
    }
    for r in &p.routines {
        for slot in r.frame.locals.iter().chain(&r.frame.params) {
            if let MirStorageBase::Absolute(a) = slot.base {
                if overlaps(a.wrapping_add(slot.offset), slot.storage_size) {
                    return false;
                }
            }
        }
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            let effect = crate::mir6502::analysis::effects::classify_op(op);
            if effect
                .memory
                .direct_reads
                .iter()
                .chain(&effect.memory.direct_writes)
                .any(|range| matches!(range.base,MirMem::Absolute(a) if overlaps(a,range.bytes)))
            {
                return false;
            }
        }
    }
    true
}

/// Prove each stored value unobserved through all successors until overwrite
/// or return. This deliberately does not exempt captures from static-frame
/// alias rules. Reads on a different CASE arm do not observe this definition.
pub(in crate::mir6502) fn scalar_home_unobserved(
    program: &MirProgram,
    routine: &MirRoutine,
    id: crate::nir::LocalId,
) -> bool {
    if frame_exposed_in_data(program, routine.id)
        || !matches!(
            routine.abi,
            MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry
        )
    {
        return false;
    }
    fn frozen(v: &MirValue) -> bool {
        match v {
            MirValue::ConstU8(_)
            | MirValue::ConstU16(_)
            | MirValue::Def(MirDef::VTemp(_) | MirDef::VTempByte { .. }) => true,
            MirValue::Word { lo, hi } => frozen(lo) && frozen(hi),
            _ => false,
        }
    }
    let Ok(snapshot) = super::prehome::PreHomeAnalysisSnapshot::new(
        routine,
        super::sites::MirRoutineGeneration::initial(),
    ) else {
        return false;
    };
    let context = PreHomeRewriteContext::new(&snapshot);
    let mut stores = Vec::new();
    for block in &routine.blocks {
        for (op_index, op) in block.ops.iter().enumerate() {
            let effect = classify_op(op);
            if effect
                .memory
                .direct_references
                .iter()
                .chain(effect.memory.direct_writes.iter().map(|w| &w.base))
                .any(|m| matches!(m, MirMem::Local { id: other, .. } if *other == id))
            {
                let MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Local { id: target, offset }),
                    src,
                    width,
                } = op
                else {
                    return false;
                };
                if *target != id || !frozen(src) {
                    return false;
                }
                for byte in 0..if *width == MirWidth::Byte { 1 } else { 2 } {
                    stores.push((
                        MirMem::Local {
                            id,
                            offset: offset + byte,
                        },
                        MirSite::Op {
                            block: block.id,
                            op_index,
                        },
                    ));
                }
            }
        }
        let effects = super::effects::classify_terminator(&block.terminator);
        if effects
            .memory
            .direct_references
            .iter()
            .any(|m| matches!(m, MirMem::Local { id: other, .. } if *other == id))
        {
            return false;
        }
        if effects
            .memory
            .address_references
            .iter()
            .any(|m| matches!(m, MirMem::Local { .. } | MirMem::Param { .. }))
        {
            return false;
        }
    }
    if stores.is_empty() {
        return false;
    }
    for (cell, site) in stores {
        if !reads_confined(routine, &context, &cell, site, &BTreeSet::new()) {
            return false;
        }
        let MirSite::Op { block, op_index } = site else {
            unreachable!()
        };
        let mut pending = vec![(block, op_index + 1)];
        let mut visited = BTreeSet::new();
        while let Some((block, first)) = pending.pop() {
            if !visited.insert((block, first)) {
                continue;
            }
            let Some(b) = routine.blocks.iter().find(|b| b.id == block) else {
                return false;
            };
            let mut overwritten = false;
            for op in b.ops.iter().skip(first) {
                let e = classify_op(op).memory;
                if e.opaque
                    || e.has_unknown_effects
                    || e.indirect_reads
                    || e.indirect_writes
                    || e.structured_reads != MirMemoryEffect::None
                    || e.structured_writes != MirMemoryEffect::None
                    || e.direct_reads
                        .iter()
                        .any(|r| matches!(r.base, MirMem::Absolute(_)))
                {
                    return false;
                }
                if e.direct_writes.iter().any(|w| {
                    same_object(&w.base, &cell)
                        && offset(&w.base)
                            .is_some_and(|o| offset(&cell).unwrap().wrapping_sub(o) < w.bytes)
                }) {
                    overwritten = true;
                    break;
                }
            }
            if !overwritten {
                if matches!(b.terminator, MirTerminator::Unreachable) {
                    return false;
                }
                pending.extend(
                    context
                        .cfg()
                        .successors(block)
                        .iter()
                        .map(|&next| (next, 0)),
                );
            }
        }
    }
    true
}

/// A closed program with only named reads cannot observe an otherwise unread
/// home through a pointer. This permits removing stores while retaining the
/// allocated home (and therefore all pointer-write/layout behavior). Calls are
/// admitted only when their complete MIR bodies are in this same census.
/// Final physical layout must separately exclude absolute/global aliases.
pub(in crate::mir6502) fn has_only_named_reads(program: &MirProgram) -> bool {
    program.routines.iter().all(|routine| {
        routine.blocks.iter().all(|block| {
            block.ops.iter().all(|op| {
                let e = classify_op(op).memory;
                // Only the explicit, nonvolatile copy contract admits a
                // write through an unknown address. General pointer writes
                // and hardware/absolute accesses remain ordering barriers.
                if e.indirect_writes
                    && !matches!(
                        op,
                        MirOp::CopyBytes {
                            source_volatile: false,
                            destination_volatile: false,
                            ..
                        } | MirOp::Call {
                            target: MirCallTarget::Routine(_),
                            ..
                        }
                    )
                    || matches!(
                        op,
                        MirOp::CopyBytes {
                            source_volatile: true,
                            ..
                        } | MirOp::CopyBytes {
                            destination_volatile: true,
                            ..
                        }
                    )
                    || e.direct_reads.iter().chain(&e.direct_writes).any(|access| {
                        match access.base {
                            MirMem::Absolute(_) => true,
                            MirMem::Global { id, .. } => !program.globals.iter().any(|g| {
                                g.id == id && matches!(g.backing, MirGlobalBacking::Ordinary { .. })
                            }),
                            _ => false,
                        }
                    })
                {
                    return false;
                }
                match op {
                    MirOp::Call {
                        target: MirCallTarget::Routine(id),
                        ..
                    } => program.routines.iter().any(|r| {
                        r.id == *id
                            && !r.blocks.is_empty()
                            && !matches!(r.abi, MirRoutineAbi::ExternalInterface)
                    }),
                    MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. } => false,
                    _ => {
                        !e.indirect_reads
                            && !e.opaque
                            && !e.has_unknown_effects
                            && e.structured_reads == MirMemoryEffect::None
                    }
                }
            }) && !super::effects::classify_terminator(&block.terminator)
                .memory
                .indirect_reads
        })
    })
}

pub(in crate::mir6502) fn home_has_only_captured_stores(
    routine: &MirRoutine,
    id: crate::nir::LocalId,
) -> bool {
    fn captured(v: &MirValue) -> bool {
        match v {
            MirValue::ConstU8(_)
            | MirValue::ConstU16(_)
            | MirValue::Def(MirDef::VTemp(_) | MirDef::VTempByte { .. }) => true,
            MirValue::Word { lo, hi } => captured(lo) && captured(hi),
            _ => false,
        }
    }
    if routine
        .frame
        .locals
        .iter()
        .any(|s| matches!(s.base, MirStorageBase::LocalAlias { target, .. } if target == id))
    {
        return false;
    }
    let mut stores = 0;
    for b in &routine.blocks {
        for op in &b.ops {
            let effect = classify_op(op).memory;
            if effect
                .direct_references
                .iter()
                .chain(effect.direct_writes.iter().map(|w| &w.base))
                .any(|m| matches!(m, MirMem::Local { id: other, .. } if *other == id))
            {
                if !matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id: target, .. }), src, .. } if *target == id && captured(src))
                {
                    return false;
                }
                stores += 1;
            }
        }
        if super::effects::classify_terminator(&b.terminator)
            .memory
            .direct_references
            .iter()
            .any(|m| matches!(m, MirMem::Local { id: other, .. } if *other == id))
        {
            return false;
        }
    }
    stores > 0
}
