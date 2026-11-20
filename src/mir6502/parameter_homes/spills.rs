//! Byte spills spanning CFG edges and ordinary leaf calls. This is part of the
//! existing placement trial, after core coloring, before late decisions replay.
use super::*;
mod interference;
mod contexts;
mod hazards;
use contexts::Contexts;
use hazards::Hazards;
use interference::Byte;
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

#[cfg(test)]
fn discover(program: &MirProgram, multiple: bool) -> Vec<Placement> {
    discover_at(program, multiple, 0x3000)
}

pub(in crate::mir6502) fn discover_at(program: &MirProgram, multiple: bool, origin: u16) -> Vec<Placement> {
    // A known missing inventory must not invalidate unrelated source-home
    // candidates in the shared batch. Decodable symbolic operands may wait for layout.
    let Ok(observations) = physical::numeric_observations(program, None) else {
        return Vec::new();
    };
    let mut contexts = Contexts::new(program);
    contexts.reserved.extend((0xe0..=0xef).filter(|&byte| observations.iter().any(|&(start, bytes)|
        u16::from(byte).wrapping_sub(start) < bytes)));
    let ownership = StorageOwnership::new(program);
    let mut calls = ResidenceCalls::at_origin(program, MirCalleeEntryStage::ActualEntry, origin);
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
        if !contexts.frames.contains_key(&r.id) { continue; }
        let mut used = super::source_zero_page(program, r);
        used.extend(r.frame.fixed_zero_page.iter().map(|s| s.0));
        for a in &r.frame.zero_page_allocations {
            used.extend((0..a.size).map(|i| a.start.0.wrapping_add(i)));
        }
        let call_count = r.blocks.iter().flat_map(|b| &b.ops).filter(|op| matches!(op, MirOp::Call { .. })).count();
        if call_count == 0 || (!multiple && call_count > 1) { continue; }
        let hazards = Hazards::new(r, &mut calls, None);
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
            let lifetimes = &contexts.frames[&r.id];
            if ownership.role(r.id, &mem) != StorageRole::Temporary
                || !remapping::spill_uses_zero_offset_only(r, *spill)
                || lifetimes.temps.live_in(entry).is_none_or(|s| s.contains(home))
                || !r
                    .blocks
                    .iter()
                    .any(|b| lifetimes.temps.live_out(b.id).is_some_and(|s| s.contains(home)))
                || !byte_accesses_only(r, *spill)
            {
                continue;
            }
            let key = Byte::Home(home);
            let Some(range) = lifetimes.get(key) else { continue };
            let Some(byte) = (0xe0..=0xef).find(|b| !used.contains(b)
                && hazards.allows(range, *b) && contexts.fits(r.id, key, *b)) else { continue };
            contexts.bind(r.id, key, byte);
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

pub(in crate::mir6502) fn place_at(
    program: &mut MirProgram,
    multiple: bool,
    origin: u16,
    trace: &super::super::trace::Trace,
) -> Vec<Placement> {
    let placements = discover_at(program, multiple, origin);
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
pub(in crate::mir6502) fn verify_at(
    homes: &[Placement],
    old: &Executable,
    new: &Executable,
    source: &MirProgram,
    fallback: &Executable,
    origin: u16,
) -> Result<(), Vec<MirDiagnostic>> {
    let observations = physical::resolve_sources(&[
        (source, &fallback.memory),
        (&fallback.program, &fallback.memory),
        (source, &old.memory),
        (source, &new.memory),
        (&old.program, &old.memory),
        (&new.program, &new.memory),
    ])?;
    let contexts = Contexts::new(&new.program);
    let mut calls = ResidenceCalls::at_origin(&new.program, MirCalleeEntryStage::ActualEntry, origin);
    let mut hazards = BTreeMap::new();
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
            || !contexts.source_allows(&new.memory, p.fixed.0)
            || observations.iter().any(|&(start, bytes)| {
                [old_addr, address]
                    .iter()
                    .any(|a| a.wrapping_sub(start) < bytes)
            })
        {
            return Err(bad("explicit observation overlaps old or proposed home"));
        }
        let key = Byte::Home(MirHomeByte::VirtualZeroPage { slot: p.slot, offset: 0 });
        let frame = contexts.frames.get(&r.id).ok_or_else(|| bad("missing final frame"))?;
        let range = frame.get(key).ok_or_else(|| bad("missing final spill lifetime"))?;
        let hazards = hazards.entry(r.id).or_insert_with(|| Hazards::new(r, &mut calls, Some(&new.memory)));
        if !contexts.fits(r.id, key, p.fixed.0) || !hazards.allows(range, p.fixed.0) {
            return Err(bad("live byte overlaps another occupant or physical access"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
