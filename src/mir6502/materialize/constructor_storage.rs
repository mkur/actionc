//! Deleting a constructor store requires more than forwarding its value.
//! Initial scope: complete small captures in terminal blocks, closed direct
//! callers with exact result buffers, and independently checked physical aliases.
use super::{MirPeepholeStats, address_values::Cost, copy_values, temp_widths};
use crate::mir6502::{
    Mir6502Config,
    analysis::{
        cfg::MirCfg,
        effects::{classify_op, classify_terminator},
        known_callees::{MirKnownCalleeSummaries, MirKnownMemoryWrites},
        private_cells,
        sites::MirSite,
        use_def::{MirTempLane, MirTempUseDefIndex},
    },
    ir::*,
    standalone::{
        visit_data_image_routines, visit_global_init_routines, visit_op_routines,
        visit_storage_init_routines, visit_terminator_routines,
    },
};
use crate::nir::{LocalId, ParamId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Candidate {
    routine: RoutineId,
    local: LocalId,
    bytes: u16,
    stored_bytes: usize,
    protected: BTreeMap<RoutineId, Vec<MirMem>>,
}

#[derive(Default)]
pub(super) struct Discovery {
    candidates: Vec<Candidate>,
    stats: MirPeepholeStats,
}

pub(super) fn discover(program: &MirProgram, config: &Mir6502Config) -> Discovery {
    let mut out = Discovery::default();
    if !config.enable_constructor_store_elision
        || !program.routines.iter().any(|r| {
            r.frame.locals.iter().any(|s| {
                s.purpose == MirStoragePurpose::AggregateCapture
                    && (1..=8).contains(&s.storage_size)
            })
        })
    {
        return out;
    }
    let escaped = escaped_routines(program);
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(program);
    for r in &program.routines {
        for slot in &r.frame.locals {
            let MirStorageBase::Local(local) = slot.base else {
                continue;
            };
            if slot.purpose != MirStoragePurpose::AggregateCapture
                || !(1..=8).contains(&slot.storage_size)
            {
                continue;
            }
            let result = prove(program, r, slot, local, &escaped, &summaries);
            match result {
                Ok(candidate) => out.candidates.push(candidate),
                Err(reason) => out.stats.record(r.id, reason),
            }
        }
    }
    out
}

fn prove(
    program: &MirProgram,
    r: &MirRoutine,
    slot: &MirStorageSlot,
    local: LocalId,
    escaped: &BTreeSet<RoutineId>,
    summaries: &MirKnownCalleeSummaries,
) -> Result<Candidate, &'static str> {
    let observation = "constructor-storage-blocked-observation";
    let destination = "constructor-storage-blocked-destination";
    if r.abi != MirRoutineAbi::Action
        || escaped.contains(&r.id)
        || slot.offset != 0
        || slot.init.is_some()
        || frame_exposed(program, r)
    {
        return Err(observation);
    }
    let mut stores = Vec::new();
    let mut copy = None;
    for (bi, b) in r.blocks.iter().enumerate() {
        if mentions(
            &classify_terminator(&b.terminator).memory.direct_references,
            local,
        ) {
            return Err(observation);
        }
        for (oi, op) in b.ops.iter().enumerate() {
            let e = classify_op(op).memory;
            if !mentions(&e.direct_references, local)
                && !e.direct_writes.iter().any(|w| is_local(&w.base, local))
            {
                continue;
            }
            match op {
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Local { id, offset }),
                    src,
                    width,
                } if *id == local => {
                    let bytes = width_bytes(*width);
                    if offset
                        .checked_add(bytes)
                        .is_none_or(|end| end > slot.storage_size)
                    {
                        return Err(observation);
                    }
                    stores.push((bi, oi, src, *width));
                }
                MirOp::CopyBytes {
                    source: MirAddr::Direct(MirMem::Local { id, offset: 0 }),
                    size,
                    source_volatile: false,
                    destination_volatile: false,
                    ..
                } if *id == local && *size == slot.storage_size && copy.is_none() => {
                    copy = Some((bi, oi))
                }
                _ => return Err(observation),
            }
        }
    }
    let (bi, oi) = copy.ok_or(observation)?;
    let block = &r.blocks[bi];
    if stores.is_empty()
        || oi + 1 != block.ops.len()
        || !matches!(block.terminator, MirTerminator::Return)
    {
        return Err(observation);
    }
    let cfg = MirCfg::from_routine(r).map_err(|_| observation)?;
    let region = constructor_region(
        r,
        &cfg,
        bi,
        oi,
        stores.iter().map(|(b, i, _, _)| (*b, *i)).collect(),
    )
    .ok_or(observation)?;
    let defs = MirTempUseDefIndex::from_routine(r);
    let widths = temp_widths::collect_routine_temp_widths(r);
    for (_, _, value, width) in &stores {
        for lane in 0..width_bytes(*width) {
            copy_values::captured_lane(value, lane as u8, width_bytes(*width), &defs, &widths)
                .ok_or(observation)?;
        }
    }
    copy_values::captured_bytes(r, &cfg, &defs, &widths, bi, oi, local, 0, slot.storage_size)
        .ok_or(observation)?;
    for op in region {
        if matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if *id == local)
        {
            continue;
        }
        // Evaluations stay in place. Only inert scalar preparation may separate
        // the constructor's first store from its already-captured transfer.
        if !matches!(
            op,
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Local { .. } | MirMem::Param { .. }),
                ..
            } | MirOp::LoadImm { .. }
                | MirOp::Move { .. }
                | MirOp::Extend { .. }
                | MirOp::Truncate { .. }
        ) {
            return Err(observation);
        }
        let e = classify_op(op).memory;
        if e.direct_reads
            .iter()
            .any(|a| !range_in_bounds(program, r, &a.base, a.bytes))
        {
            return Err(observation);
        }
        if e.indirect_reads
            || e.indirect_writes
            || e.opaque
            || e.has_unknown_effects
            || !e.address_references.is_empty()
        {
            return Err(observation);
        }
    }
    let MirOp::CopyBytes {
        destination: addr, ..
    } = &block.ops[oi]
    else {
        unreachable!()
    };
    let param = destination_param(r, block, oi, addr).ok_or(destination)?;
    let Some(first_param) = r.frame.params.first() else {
        return Err(destination);
    };
    if first_param.base != MirStorageBase::Param(param)
        || first_param.storage_size != 2
        || first_param.offset != 0
        || first_param.init.is_some()
    {
        return Err(destination);
    }
    // Entry provenance is useful only if the incoming pointer cannot have been
    // changed by the body. Direct callees need an existing exact write summary;
    // recursive/unknown/indirect callees do not pass that proof.
    for b in &r.blocks {
        for (i, op) in b.ops.iter().enumerate() {
            match op {
                MirOp::Call { target, .. } => {
                    let Some(summary) = summaries.for_target(target) else {
                        return Err(destination);
                    };
                    let MirKnownMemoryWrites::Exact(writes) = summary.writes() else {
                        return Err(destination);
                    };
                    if writes.iter().any(|w| matches!(w, MirMem::Absolute(_))) {
                        return Err(destination);
                    }
                    let MirCallTarget::Routine(id) = target else {
                        return Err(destination);
                    };
                    if !callee_writes_in_bounds(program, *id, &mut BTreeSet::new(), &mut 128) {
                        return Err(destination);
                    }
                }
                MirOp::CopyBytes {
                    size,
                    source_volatile: false,
                    destination_volatile: false,
                    ..
                } if *size == slot.storage_size
                    && i + 1 == b.ops.len()
                    && matches!(b.terminator, MirTerminator::Return)
                    && matches!(op, MirOp::CopyBytes { destination: a, .. }
                            if destination_param(r, b, i, a) == Some(param)) => {}
                _ => {
                    let e = classify_op(op).memory;
                    if e.indirect_writes
                        || e.may_write_any
                        || e.opaque
                        || e.has_unknown_effects
                        || e.structured_writes != MirMemoryEffect::None
                        || e.direct_writes.iter().any(|w| {
                            matches!(w.base, MirMem::Param { id, .. } if id == param)
                                || matches!(w.base, MirMem::Absolute(_))
                        })
                        || e.direct_writes
                            .iter()
                            .any(|w| !range_in_bounds(program, r, &w.base, w.bytes))
                    {
                        return Err(destination);
                    }
                }
            }
        }
    }
    let mut protected = BTreeMap::<RoutineId, Vec<MirMem>>::new();
    protected
        .entry(r.id)
        .or_default()
        .extend((0..slot.storage_size).map(|offset| MirMem::Local { id: local, offset }));
    protected
        .entry(r.id)
        .or_default()
        .extend((0..2).map(|offset| MirMem::Param { id: param, offset }));
    let mut callers = 0;
    for caller in &program.routines {
        for b in &caller.blocks {
            for (i, op) in b.ops.iter().enumerate() {
                let MirOp::Call {
                    target: MirCallTarget::Routine(target),
                    args,
                    ..
                } = op
                else {
                    continue;
                };
                if *target != r.id {
                    continue;
                }
                callers += 1;
                if caller.id == r.id {
                    return Err(destination);
                }
                let arg = args.first().ok_or(destination)?;
                if arg.width != MirWidth::Word {
                    return Err(destination);
                }
                let Origin::Address(MirMem::Local { id, offset: 0 }) =
                    origin(caller, b, i, &arg.value, 16).ok_or(destination)?
                else {
                    return Err(destination);
                };
                let home = caller
                    .frame
                    .locals
                    .iter()
                    .find(|s| s.base == MirStorageBase::Local(id))
                    .ok_or(destination)?;
                if home.purpose != MirStoragePurpose::AggregateCapture || home.offset != 0
                    || home.init.is_some() || home.storage_size != slot.storage_size
                    || caller.frame.locals.iter().any(|s| matches!(s.base, MirStorageBase::LocalAlias { target, .. } if target == id))
                { return Err(destination); }
                protected
                    .entry(caller.id)
                    .or_default()
                    .extend((0..home.storage_size).map(|offset| MirMem::Local { id, offset }));
            }
        }
    }
    if callers == 0 {
        return Err(destination);
    }
    Ok(Candidate {
        routine: r.id,
        local,
        bytes: slot.storage_size,
        stored_bytes: stores
            .iter()
            .map(|(_, _, _, w)| usize::from(width_bytes(*w)))
            .sum(),
        protected,
    })
}

fn is_local(mem: &MirMem, id: LocalId) -> bool {
    matches!(mem, MirMem::Local { id: other, .. } if *other == id)
}
fn mentions(mems: &[MirMem], id: LocalId) -> bool {
    mems.iter().any(|m| is_local(m, id))
}
fn width_bytes(width: MirWidth) -> u16 {
    if width == MirWidth::Byte { 1 } else { 2 }
}

fn range_in_bounds(p: &MirProgram, r: &MirRoutine, mem: &MirMem, bytes: u16) -> bool {
    let (offset, size) = match mem {
        MirMem::Local { id, offset } => (
            *offset,
            r.frame
                .locals
                .iter()
                .find(|s| s.base == MirStorageBase::Local(*id) && s.offset == 0)
                .map(|s| s.storage_size),
        ),
        MirMem::Param { id, offset } => (
            *offset,
            r.frame
                .params
                .iter()
                .find(|s| s.base == MirStorageBase::Param(*id) && s.offset == 0)
                .map(|s| s.storage_size),
        ),
        MirMem::Global { id, offset } => (
            *offset,
            p.globals
                .iter()
                .find(|g| {
                    g.id == *id && matches!(g.backing, MirGlobalBacking::Ordinary { offset: 0 })
                })
                .map(|g| g.storage_size),
        ),
        MirMem::FixedZeroPage(zp) => return u16::from(zp.0) + bytes <= 0x100,
        _ => return false,
    };
    offset
        .checked_add(bytes)
        .zip(size)
        .is_some_and(|(end, size)| end <= size)
}

// Exact callee-write summaries describe identities, not their bounds. Check
// those in each owning routine before using them to protect an incoming pointer.
fn callee_writes_in_bounds(
    p: &MirProgram,
    id: RoutineId,
    seen: &mut BTreeSet<RoutineId>,
    budget: &mut usize,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    let Some(r) = p.routines.iter().find(|r| r.id == id) else {
        return false;
    };
    for op in r.blocks.iter().flat_map(|b| &b.ops) {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        match op {
            MirOp::Call {
                target: MirCallTarget::Routine(next),
                ..
            } => {
                if !callee_writes_in_bounds(p, *next, seen, budget) {
                    return false;
                }
            }
            MirOp::Call { .. }
            | MirOp::MachineBlock { .. }
            | MirOp::RuntimeHelper { .. }
            | MirOp::Barrier { .. } => return false,
            _ => {
                let e = classify_op(op).memory;
                if e.indirect_writes
                    || e.may_write_any
                    || e.opaque
                    || e.has_unknown_effects
                    || e.structured_writes != MirMemoryEffect::None
                    || e.direct_writes
                        .iter()
                        .any(|w| !range_in_bounds(p, r, &w.base, w.bytes))
                {
                    return false;
                }
            }
        }
    }
    seen.remove(&id);
    true
}

// Tiny-fill selection leaves its stores in an unconditional predecessor.
// Follow the same bounded, nonbranching chains as captured-copy forwarding;
// do not merge the CFG or cross joins, backedges, or block arguments.
fn constructor_region<'a>(
    r: &'a MirRoutine,
    cfg: &MirCfg,
    mut bi: usize,
    mut end: usize,
    mut required: BTreeSet<(usize, usize)>,
) -> Option<Vec<&'a MirOp>> {
    let mut region = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        if seen.len() == 4 || !seen.insert(bi) {
            return None;
        }
        let b = &r.blocks[bi];
        for i in (0..end).rev() {
            if region.len() == 64 {
                return None;
            }
            region.push(&b.ops[i]);
            required.remove(&(bi, i));
            if required.is_empty() {
                return Some(region);
            }
        }
        if !b.params.is_empty() || cfg.entry() == Some(b.id) {
            return None;
        }
        let pred = cfg.predecessors(b.id);
        if pred.len() != 1 {
            return None;
        }
        let next = r.blocks.iter().position(|p| pred.contains(&p.id))?;
        if !matches!(&r.blocks[next].terminator, MirTerminator::Jump(e) if e.target == b.id && e.args.is_empty())
        {
            return None;
        }
        bi = next;
        end = r.blocks[bi].ops.len();
    }
}

fn frame_exposed(p: &MirProgram, r: &MirRoutine) -> bool {
    private_cells::frame_exposed_in_data(p, r.id)
        || r.frame
            .locals
            .iter()
            .any(|s| matches!(s.base, MirStorageBase::LocalAlias { .. }))
        || r.blocks.iter().any(|b| {
            b.ops
                .iter()
                .map(|op| classify_op(op).memory)
                .chain(std::iter::once(classify_terminator(&b.terminator).memory))
                .any(|e| {
                    e.address_references
                        .iter()
                        .any(|m| matches!(m, MirMem::Local { .. } | MirMem::Param { .. }))
                })
        })
}

enum Origin {
    Address(MirMem),
    Parameter(ParamId),
}

fn origin(
    r: &MirRoutine,
    b: &MirBlock,
    before: usize,
    value: &MirValue,
    depth: usize,
) -> Option<Origin> {
    if depth == 0 {
        return None;
    }
    if let MirValue::Word { lo, hi } = value {
        if let (
            MirValue::StorageAddrByte { mem: a, byte: 0 },
            MirValue::StorageAddrByte { mem: z, byte: 1 },
        ) = (lo.as_ref(), hi.as_ref())
        {
            return (a == z).then(|| Origin::Address(a.clone()));
        }
    }
    let MirValue::Def(MirDef::VTemp(temp)) = value else {
        return None;
    };
    let defs = MirTempUseDefIndex::from_routine(r);
    let lo = defs.unique_definition(MirTempLane {
        temp: *temp,
        byte: 0,
    })?;
    let hi = defs.unique_definition(MirTempLane {
        temp: *temp,
        byte: 1,
    })?;
    if lo.site != hi.site {
        return None;
    }
    let MirSite::Op { block, op_index } = lo.site else {
        return None;
    };
    if block != b.id || op_index >= before {
        return None;
    }
    match &b.ops[op_index] {
        MirOp::LeaAddr {
            target,
            width: MirWidth::Word,
            ..
        } => Some(Origin::Address(target.clone())),
        MirOp::Load {
            src: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
            width: MirWidth::Word,
            ..
        } => Some(Origin::Parameter(*id)),
        MirOp::Move {
            src,
            width: MirWidth::Word,
            ..
        } => origin(r, b, op_index, src, depth - 1),
        _ => None,
    }
}

fn destination_param(r: &MirRoutine, b: &MirBlock, i: usize, a: &MirAddr) -> Option<ParamId> {
    match a {
        MirAddr::Deref { ptr, offset: 0 } => match origin(r, b, i, ptr, 16)? {
            Origin::Parameter(id) => Some(id),
            _ => None,
        },
        MirAddr::PointerCell {
            ptr: MirMem::Param { id, offset: 0 },
            offset: 0,
        } => Some(*id),
        _ => None,
    }
}

fn escaped_routines(p: &MirProgram) -> BTreeSet<RoutineId> {
    let mut escaped = BTreeSet::new();
    for s in &p.statics {
        visit_data_image_routines(&s.image, &mut escaped);
    }
    for g in &p.globals {
        if let Some(i) = &g.init {
            visit_global_init_routines(i, &mut escaped);
        }
    }
    for h in &p.runtime_helpers {
        if let MirRuntimeHelperTarget::Routine(id) = h.target {
            escaped.insert(id);
        }
    }
    for m in &p.machine_blocks {
        for item in &m.items {
            match item {
                MirMachineItem::Relocation {
                    target: MirInlineAsmTarget::Routine(id),
                    ..
                } => {
                    escaped.insert(*id);
                }
                MirMachineItem::Name(_)
                | MirMachineItem::AddressByte { .. }
                | MirMachineItem::AddressExpr {
                    atom: MirMachineAtom::Name(_),
                    ..
                } => return p.routines.iter().map(|r| r.id).collect(),
                _ => {}
            }
        }
    }
    for r in &p.routines {
        for s in r.frame.locals.iter().chain(&r.frame.params) {
            if let Some(i) = &s.init {
                visit_storage_init_routines(i, &mut escaped);
            }
        }
        for b in &r.blocks {
            visit_terminator_routines(&b.terminator, &mut escaped);
            for op in &b.ops {
                visit_op_routines(op, &mut escaped);
            }
        }
    }
    escaped
}

pub(super) fn improve(
    forwarded: &mut MirProgram,
    emitted: &mut MirProgram,
    discovery: Discovery,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
) -> MirPeepholeStats {
    let Discovery {
        candidates,
        mut stats,
    } = discovery;
    let mut selected = forwarded.clone();
    let mut affected = BTreeSet::new();
    let mut protected = BTreeMap::<RoutineId, Vec<MirMem>>::new();
    let mut applied = Vec::new();
    for c in candidates {
        let r = selected
            .routines
            .iter_mut()
            .find(|r| r.id == c.routine)
            .unwrap();
        // The copy must actually have been forwarded, and no other surviving
        // operation may reference the home. Recheck after all copy rewrites.
        if r.blocks.iter().any(|b| {
            mentions(&classify_terminator(&b.terminator).memory.direct_references, c.local)
                || b.ops.iter().any(|op| {
                    let e = classify_op(op).memory;
                    (mentions(&e.direct_references, c.local) || e.direct_writes.iter().any(|w| is_local(&w.base, c.local)))
                        && !matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if *id == c.local)
                })
        }) { continue; }
        for b in &mut r.blocks {
            b.ops.retain(|op| !matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if *id == c.local));
        }
        affected.insert(c.routine);
        for (&id, mems) in &c.protected {
            protected
                .entry(id)
                .or_default()
                .extend(mems.iter().cloned());
        }
        applied.push(c);
    }
    if applied.is_empty() {
        return stats;
    }
    let proposed = crate::mir6502::verify_program(&selected, MirPhase::PreMaterialization)
        .ok()
        .and_then(|()| {
            crate::mir6502::materialize_resolved_program_without_copy_costing(
                selected.clone(),
                config,
                origin,
                runtime,
                false,
            )
            .ok()
        });
    let accepted = proposed.as_ref().is_some_and(|new| {
        Cost::measure(emitted, origin)
            .zip(Cost::measure(new, origin))
            .is_some_and(|(old, new)| {
                new.bytes < old.bytes && affected.iter().all(|&id| new.does_not_grow(&old, id))
            })
            && occupied(new).is_subset(&occupied(emitted))
            && private_cells::physical_cells_unaliased(emitted, origin, &protected)
            && private_cells::physical_cells_unaliased(new, origin, &protected)
    });
    for c in &applied {
        if accepted {
            stats.record_many(
                c.routine,
                "constructor-stored-bytes-removed",
                c.stored_bytes,
            );
            stats.record_many(
                c.routine,
                "constructor-home-bytes-retained",
                usize::from(c.bytes),
            );
        } else {
            stats.record(c.routine, "constructor-storage-declined-cost-or-alias");
        }
    }
    if accepted {
        *forwarded = selected;
        *emitted = proposed.unwrap();
    }
    stats
}

fn occupied(p: &MirProgram) -> BTreeSet<u8> {
    p.routines
        .iter()
        .flat_map(|r| {
            r.frame.fixed_zero_page.iter().map(|s| s.0).chain(
                r.frame
                    .zero_page_allocations
                    .iter()
                    .flat_map(|a| (0..a.size).map(move |n| a.start.0.wrapping_add(n))),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = "TYPE Choice=VARIANT [EMPTY FOUND [CARD address]] Choice result Choice FUNC Make(CARD n) IF n=0 THEN RETURN(Choice.EMPTY) FI RETURN(Choice.FOUND(n)) PROC Main() result=Make(1234) RETURN";

    fn lower(source: &str, optimized: bool) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let raw = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let nir = if optimized {
            crate::nir::optimize_program(&raw).unwrap()
        } else {
            raw
        };
        let mut p = crate::mir6502::lower_program(&nir).unwrap();
        super::super::tiny_fills::select(&mut p);
        p
    }

    #[test]
    fn constructor_storage_proves_terminal_captures_and_saves_code() {
        for optimized in [false, true] {
            let p = lower(SOURCE, optimized);
            let d = discover(&p, &Mir6502Config::default());
            assert_eq!(d.candidates.len(), 2, "{:?}", d.stats.aggregate_counts());
            let mut off = Mir6502Config::default();
            off.enable_constructor_store_elision = false;
            let baseline =
                crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
            let after = crate::mir6502::materialize_program_with_origin(
                p,
                &Mir6502Config::default(),
                0x3000,
            )
            .unwrap();
            let old = Cost::measure(&baseline, 0x3000).unwrap();
            let new = Cost::measure(&after, 0x3000).unwrap();
            assert!(new.bytes < old.bytes, "{} -> {}", old.bytes, new.bytes);
            for r in &baseline.routines {
                let next = after.routines.iter().find(|n| n.id == r.id).unwrap();
                assert_eq!(
                    r.frame.locals, next.frame.locals,
                    "store slice retains allocations"
                );
            }
        }
    }

    #[test]
    fn constructor_storage_rejects_unsafe_destinations_and_observations() {
        for reason in [
            "unknown caller",
            "parameter write",
            "frame exposure",
            "volatile copy",
            "overflow",
            "call",
            "read",
            "observable entry",
        ] {
            let mut p = lower(SOURCE, false);
            let ri = p.routines.iter().position(|r| r.name == "Make").unwrap();
            let rid = p.routines[ri].id;
            let original = discover(&p, &Mir6502Config::default());
            let c = original.candidates.iter().find(|c| p.routines[ri].blocks.iter().any(|b|
                matches!(b.ops.last(), Some(MirOp::CopyBytes { .. }))
                && b.ops.iter().any(|op| matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if *id == c.local)))).unwrap().clone();
            if reason == "unknown caller" {
                for op in p
                    .routines
                    .iter_mut()
                    .flat_map(|r| &mut r.blocks)
                    .flat_map(|b| &mut b.ops)
                {
                    if let MirOp::Call {
                        target: MirCallTarget::Routine(id),
                        args,
                        ..
                    } = op
                    {
                        if *id == rid {
                            args[0].value = MirValue::ConstU16(0x9000);
                        }
                    }
                }
            } else {
                let r = &mut p.routines[ri];
                let param = match r.frame.params[0].base {
                    MirStorageBase::Param(id) => id,
                    _ => unreachable!(),
                };
                let b = r.blocks.iter_mut().find(|b| matches!(b.ops.last(), Some(MirOp::CopyBytes { .. })) && b.ops.iter().any(|op| matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if *id == c.local))).unwrap();
                let first = b
                    .ops
                    .iter()
                    .position(|op| matches!(op, MirOp::Store { .. }))
                    .unwrap();
                match reason {
                    "parameter write" => b.ops.insert(
                        first,
                        MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Param {
                                id: param,
                                offset: 0,
                            }),
                            src: MirValue::ConstU16(0x9000),
                            width: MirWidth::Word,
                        },
                    ),
                    "frame exposure" => b.ops.insert(
                        first,
                        MirOp::LeaAddr {
                            dst: MirDef::VTemp(MirTempId(900)),
                            target: MirMem::Local {
                                id: c.local,
                                offset: 0,
                            },
                            width: MirWidth::Word,
                        },
                    ),
                    "volatile copy" => {
                        let MirOp::CopyBytes {
                            destination_volatile,
                            ..
                        } = b.ops.last_mut().unwrap()
                        else {
                            unreachable!()
                        };
                        *destination_volatile = true;
                    }
                    "overflow" => {
                        let MirOp::Store {
                            dst: MirAddr::Direct(MirMem::Local { offset, .. }),
                            ..
                        } = &mut b.ops[first]
                        else {
                            unreachable!()
                        };
                        *offset = u16::MAX;
                    }
                    "call" => b.ops.insert(
                        first + 1,
                        MirOp::Barrier {
                            effects: MirEffects::default(),
                        },
                    ),
                    "read" => b.ops.insert(
                        first + 1,
                        MirOp::Load {
                            dst: MirDef::VTemp(MirTempId(900)),
                            src: MirAddr::Direct(MirMem::Local {
                                id: c.local,
                                offset: 0,
                            }),
                            width: MirWidth::Byte,
                        },
                    ),
                    "observable entry" => r.abi = MirRoutineAbi::ActionObservable,
                    _ => unreachable!(),
                }
            }
            assert!(
                !discover(&p, &Mir6502Config::default())
                    .candidates
                    .iter()
                    .any(|n| n.local == c.local && n.routine == rid),
                "{reason}"
            );
        }
    }

    #[test]
    fn constructor_storage_protects_incoming_pointer_across_known_calls() {
        let source = SOURCE
            .replace(
                "Choice FUNC Make",
                "CARD FUNC Identity(CARD n) RETURN(n) Choice FUNC Make",
            )
            .replace("IF n=0", "n=Identity(n) IF n=0");
        let original = lower(&source, false);
        assert_eq!(
            discover(&original, &Mir6502Config::default())
                .candidates
                .len(),
            2
        );
        for unknown in [false, true] {
            let mut p = original.clone();
            let r = p
                .routines
                .iter_mut()
                .find(|r| r.name == "Identity")
                .unwrap();
            let param = match r.frame.params[0].base {
                MirStorageBase::Param(id) => id,
                _ => unreachable!(),
            };
            r.blocks[0].ops.insert(
                0,
                if unknown {
                    MirOp::Barrier {
                        effects: MirEffects {
                            opaque: true,
                            ..MirEffects::default()
                        },
                    }
                } else {
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Param {
                            id: param,
                            offset: 2,
                        }),
                        src: MirValue::ConstU8(99),
                        width: MirWidth::Byte,
                    }
                },
            );
            assert!(
                discover(&p, &Mir6502Config::default())
                    .candidates
                    .is_empty(),
                "unknown={unknown}"
            );
        }
    }

    #[test]
    fn constructor_storage_rejects_routine_escape_and_physical_aliases() {
        let mut p = lower(SOURCE, false);
        let r = p.routines.iter().find(|r| r.name == "Make").unwrap();
        let id = r.id;
        let local = discover(&p, &Mir6502Config::default()).candidates[0].local;
        let mut off = Mir6502Config::default();
        off.enable_constructor_store_elision = false;
        let baseline =
            crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
        let address = super::super::MaterializeLayout::new(&baseline, 0x3000)
            .mem_address(
                id,
                &MirMem::Local {
                    id: local,
                    offset: 0,
                },
            )
            .unwrap();
        let mut alias = p
            .globals
            .iter()
            .find(|g| g.storage_size > 0)
            .unwrap()
            .clone();
        alias.id = crate::nir::SymbolId(900);
        alias.name = "observer".into();
        alias.backing = MirGlobalBacking::Absolute(address);
        alias.init = None;
        p.globals.push(alias);
        let before =
            crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
        let after = crate::mir6502::materialize_program_with_origin(
            p.clone(),
            &Mir6502Config::default(),
            0x3000,
        )
        .unwrap();
        assert!(
            before == after,
            "physical aliases decline the store transaction"
        );
        p.routines.last_mut().unwrap().blocks[0].ops.insert(
            0,
            MirOp::Move {
                dst: MirDef::VTemp(MirTempId(900)),
                src: MirValue::RoutineAddr(id),
                width: MirWidth::Word,
            },
        );
        assert!(
            discover(&p, &Mir6502Config::default())
                .candidates
                .is_empty()
        );
    }
}
