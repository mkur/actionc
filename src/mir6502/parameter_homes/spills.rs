//! Byte spills spanning CFG edges and ordinary leaf calls. This is part of the
//! existing placement trial, after core coloring, before late decisions replay.
use super::*;
use crate::mir6502::{
    analysis::{
        cfg::MirCfg,
        effects::{MirHomeByte, classify_op, classify_terminator, writes_ordinary_pointee},
        home_liveness::MirHomeLiveness,
        storage_ownership::{StorageOwnership, StorageRole},
    },
    materialize::spills as remapping,
    preparation::Executable,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(in crate::mir6502) struct Placement {
    owner: RoutineId,
    spill: MirSpillId,
    slot: MirZpSlot,
    fixed: MirFixedZpSlot,
}

/// A spill introduced after source-home selection must respect live values in
/// callers, including callers above an intervening routine with no homes.
fn live_caller_bytes(program: &MirProgram) -> BTreeMap<RoutineId, BTreeSet<u8>> {
    use crate::mir6502::analysis::sites::MirSite;
    let mut children = BTreeMap::<_, BTreeSet<_>>::new();
    let mut protected = BTreeMap::<_, BTreeSet<_>>::new();
    for r in &program.routines {
        let resident = r.frame.resident_bytes();
        let live = std::cell::LazyCell::new(|| logical::LogicalHomes::new(r));
        for block in &r.blocks {
            for (op_index, op) in block.ops.iter().enumerate() {
                let MirOp::Call { target: MirCallTarget::Routine(target), .. } = op else { continue };
                children.entry(r.id).or_default().insert(*target);
                let site = MirSite::Op { block: block.id, op_index };
                for (object, byte) in &resident {
                    if live.as_ref().and_then(|l| l.live_after(object, site)) != Some(false) {
                        protected.entry(*target).or_default().insert(byte.0);
                    }
                }
            }
        }
    }
    let mut pending: BTreeSet<_> = protected.keys().copied().collect();
    while let Some(caller) = pending.pop_first() {
        let inherited = protected[&caller].clone();
        for &target in children.get(&caller).into_iter().flatten() {
            let bytes = protected.entry(target).or_default();
            let old = bytes.len();
            bytes.extend(&inherited);
            if bytes.len() != old { pending.insert(target); }
        }
    }
    protected
}

/// The first slice reserves each byte for the entire invocation and requires
/// every call to preserve it. No sharing, save/restore or recursive summaries.
pub(in crate::mir6502) fn discover(program: &MirProgram, multiple: bool) -> Vec<Placement> {
    // A known missing inventory must not invalidate unrelated source-home
    // candidates in the shared batch. Decodable symbolic operands may wait for layout.
    if physical::numeric_observations(program, None).is_err() {
        return Vec::new();
    }
    let protected = live_caller_bytes(program);
    let ownership = StorageOwnership::new(program);
    let mut calls = ResidenceCalls::new(program, MirCalleeEntryStage::ActualEntry);
    let fixed = calls.fixed_storage();
    let mut result = Vec::new();
    for r in &program.routines {
        if r.abi != MirRoutineAbi::Action || r.frame.spills.is_empty() {
            continue;
        }
        let Ok(cfg) = MirCfg::from_routine(r) else {
            continue;
        };
        let Some(entry) = cfg.entry() else {
            continue;
        };
        let live = MirHomeLiveness::analyze(r, &cfg);
        let mut used = super::source_zero_page(program, r);
        used.extend(protected.get(&r.id).into_iter().flatten().copied());
        used.extend(r.frame.fixed_zero_page.iter().map(|s| s.0));
        used.extend(r.frame.resident_bytes().iter().map(|(_, s)| s.0));
        for a in &r.frame.zero_page_allocations {
            used.extend((0..a.size).map(|i| a.start.0.wrapping_add(i)));
        }
        let mut call_count = 0;
        let safe = r.blocks.iter().flat_map(|b| &b.ops).all(|op| {
            if matches!(op, MirOp::Call { .. }) {
                call_count += 1;
                let Ok(writes) = calls.writes(op) else {
                    return false;
                };
                used.extend(
                    (0..=255u8).filter(|s| !writes.preserves_fixed_byte(MirFixedZpSlot(*s), &fixed)),
                );
                return true;
            }
            let e = classify_op(op);
            !matches!(
                op,
                MirOp::RuntimeHelper { .. }
                    | MirOp::SArgsCapture { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) && !e.memory.opaque
                && (!e.memory.indirect_writes || writes_ordinary_pointee(op))
        });
        if !safe || call_count == 0 || (!multiple && call_count > 1) {
            continue;
        }
        let mut next = r
            .frame
            .virtual_zero_page
            .iter()
            .map(|s| s.0)
            .max()
            .map_or(0, |s| s + 1);
        for spill in &r.frame.spills {
            let mem = MirMem::Spill {
                id: *spill,
                offset: 0,
            };
            let home = MirHomeByte::Spill {
                id: *spill,
                offset: 0,
            };
            if ownership.role(r.id, &mem) != StorageRole::Temporary
                || !remapping::spill_uses_zero_offset_only(r, *spill)
                || live.live_in(entry).is_none_or(|s| s.contains(home))
                || !r
                    .blocks
                    .iter()
                    .any(|b| live.live_out(b.id).is_some_and(|s| s.contains(home)))
                || !byte_accesses_only(r, *spill)
            {
                continue;
            }
            let Some(byte) = (0xe0..=0xef).find(|b| !used.contains(b)) else {
                break;
            };
            used.insert(byte);
            result.push(Placement {
                owner: r.id,
                spill: *spill,
                slot: MirZpSlot(next),
                fixed: MirFixedZpSlot(byte),
            });
            next += 1;
        }
    }
    result
}

fn byte_accesses_only(r: &MirRoutine, spill: MirSpillId) -> bool {
    r.blocks.iter().all(|b| {
        b.ops
            .iter()
            .map(|op| classify_op(op).memory)
            .chain(std::iter::once(classify_terminator(&b.terminator).memory))
            .all(|m| {
                m.direct_reads.iter().chain(&m.direct_writes).all(|range| {
                    !matches!(range.base, MirMem::Spill { id, .. } if id == spill)
                        || range.bytes == 1
                })
            })
    })
}

pub(in crate::mir6502) fn place(
    program: &mut MirProgram,
    multiple: bool,
    trace: &super::super::trace::Trace,
) -> Vec<Placement> {
    let placements = discover(program, multiple);
    for r in &mut program.routines {
        let homes: Vec<_> = placements.iter().filter(|p| p.owner == r.id).collect();
        if homes.is_empty() {
            continue;
        }
        let remap: BTreeMap<_, _> = homes.iter().map(|p| (p.spill, p.slot)).collect();
        for p in homes {
            r.frame.virtual_zero_page.push(p.slot);
            r.frame.zero_page_allocations.push(MirZpAllocation {
                slot: p.slot,
                start: p.fixed,
                size: 1,
            });
            trace.event("cross-call-spill-home", || {
                serde_json::json!({
                    "routine": r.name, "spill": p.spill.0, "slot": p.slot.0, "address": p.fixed.0,
                })
            });
        }
        for b in &mut r.blocks {
            for op in &mut b.ops {
                remapping::remap_op_spills_to_zero_page(op, &remap);
            }
            remapping::remap_terminator_spills_to_zero_page(&mut b.terminator, &remap);
        }
        remapping::prune_unused_spills_traced(r, trace);
    }
    placements
}

/// Recheck against the final candidate, including late scratch selection and
/// callee changes. A physical write never defines the protected logical value.
pub(in crate::mir6502) fn verify(
    homes: &[Placement],
    old: &Executable,
    new: &Executable,
    source: &MirProgram,
    fallback: &Executable,
) -> Result<(), Vec<MirDiagnostic>> {
    let observations = physical::resolve_sources(&[
        (source, &fallback.memory),
        (&fallback.program, &fallback.memory),
        (source, &old.memory),
        (source, &new.memory),
        (&old.program, &old.memory),
        (&new.program, &new.memory),
    ])?;
    let mut calls = ResidenceCalls::new(&new.program, MirCalleeEntryStage::ActualEntry);
    let fixed = calls.fixed_storage();
    for p in homes {
        let bad = |reason| {
            vec![MirDiagnostic {
                routine: None,
                block: None,
                message: format!("cross-call spill residence: {reason}"),
            }]
        };
        let r = new
            .program
            .routines
            .iter()
            .find(|r| r.id == p.owner)
            .ok_or_else(|| bad("missing caller"))?;
        if !r
            .frame
            .zero_page_allocations
            .iter()
            .any(|a| a.slot == p.slot && a.start == p.fixed && a.size == 1)
        {
            return Err(bad("changed allocation"));
        }
        let old_addr = old
            .memory
            .address(
                p.owner,
                &MirMem::Spill {
                    id: p.spill,
                    offset: 0,
                },
            )
            .ok_or_else(|| bad("missing original address"))?;
        let address = u16::from(p.fixed.0);
        // Core coloring may differ from the accepted executable. Protect every
        // old inline spill of this caller instead of guessing an ID correspondence.
        let previous = fallback
            .program
            .routines
            .iter()
            .find(|r| r.id == p.owner)
            .ok_or_else(|| bad("missing previous caller"))?;
        for spill in &previous.frame.spills {
            let byte = fallback
                .memory
                .address(
                    p.owner,
                    &MirMem::Spill {
                        id: *spill,
                        offset: 0,
                    },
                )
                .ok_or_else(|| bad("missing previous spill address"))?;
            if !fallback.memory.source_storage_disjoint(byte)
                || observations
                    .iter()
                    .any(|&(start, bytes)| byte.wrapping_sub(start) < bytes)
            {
                return Err(bad("explicit observation overlaps previous inline storage"));
            }
        }
        if !old.memory.source_storage_disjoint(old_addr)
            || !new.memory.source_storage_disjoint(address)
            || observations.iter().any(|&(start, bytes)| {
                [old_addr, address]
                    .iter()
                    .any(|a| a.wrapping_sub(start) < bytes)
            })
        {
            return Err(bad("explicit observation overlaps old or proposed home"));
        }
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            if matches!(op, MirOp::Call { .. }) {
                if !calls
                    .writes(op)
                    .is_ok_and(|w| w.preserves_fixed_byte(p.fixed, &fixed))
                {
                    return Err(bad("callee no longer preserves home"));
                }
                continue;
            }
            let e = classify_op(op);
            if matches!(
                op,
                MirOp::RuntimeHelper { .. }
                    | MirOp::SArgsCapture { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) || e.memory.opaque
                || (e.memory.indirect_writes && !writes_ordinary_pointee(op))
            {
                return Err(bad("unsupported expanded effects"));
            }
            for range in &e.memory.direct_writes {
                if range.base == MirMem::ZeroPage(p.slot) && range.bytes == 1 {
                    continue;
                }
                let start = new
                    .memory
                    .address(r.id, &range.base)
                    .ok_or_else(|| bad("unresolved write"))?;
                if address.wrapping_sub(start) < range.bytes {
                    return Err(bad("physical write overlaps home"));
                }
            }
            if e.addresses
                .pair_writes
                .contains(&MirHomeByte::FixedZeroPage(p.fixed))
            {
                return Err(bad("pointer scratch overlaps home"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
