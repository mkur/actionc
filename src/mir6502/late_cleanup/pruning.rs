//! Opt-in, address-stable pruning of unused ordinary private spill declarations.
use super::*;
use crate::mir6502::{
    analysis::effects::classify_terminator,
    materialize::{MaterializeLayout, unused_spill_candidates},
};

type Removed = BTreeMap<RoutineId, BTreeSet<MirSpillId>>;

#[derive(Debug, PartialEq, Eq)]
enum Rejection {
    NoUnusedSpills,
    ReferencedOrExposed,
    CoreAllocation,
    InvalidChange,
    Verification,
    CalleeContract,
    ObservableAllocation,
    CandidateEmission,
    PhysicalAllocation,
    EarlierDecision(Family),
    Cost,
}

fn count(removed: &Removed) -> usize {
    removed.values().map(BTreeSet::len).sum()
}

/// Unresolved assembly names have no closed storage-reference inventory.
/// Typed relocations are handled per home by `StorageOwnership`.
fn unresolved_machine_storage(program: &MirProgram, routine: &MirRoutine) -> bool {
    // Machine storage names resolve in their owning routine (or globals).
    // An opaque name in this frame has no closed typed exposure inventory.
    routine.blocks.iter().flat_map(|b| &b.ops).any(|op| {
        let MirOp::MachineBlock { id, .. } = op else {
            return false;
        };
        program
            .machine_blocks
            .iter()
            .find(|b| b.id == *id)
            .is_none_or(|b| {
                b.items.iter().any(|item| {
                    matches!(
                        item,
                        MirMachineItem::Name(_)
                            | MirMachineItem::AddressByte { .. }
                            | MirMachineItem::AddressExpr {
                                atom: MirMachineAtom::Name(_),
                                ..
                            }
                    )
                })
            })
    })
}

fn discover(program: &MirProgram, references: &[&MirProgram]) -> (Removed, usize) {
    let mut removed = Removed::new();
    let mut candidates = 0;
    // Historical address-taking, typed machine references and source backing
    // retain their ownership even after the executable use has disappeared.
    let ownership: Vec<_> = std::iter::once(program)
        .chain(references.iter().copied())
        .map(|p| (p, StorageOwnership::new(p)))
        .collect();
    for r in &program.routines {
        let ids = unused_spill_candidates(r);
        candidates += ids.len();
        let mut used = BTreeSet::new();
        for b in &r.blocks {
            used.extend(classify_terminator(&b.terminator).projected_spill_reads);
            for op in &b.ops {
                let e = classify_op(op);
                used.extend(e.projected_spill_reads);
                used.extend(e.projected_spill_writes);
                for mem in e
                    .memory
                    .direct_references
                    .iter()
                    .chain(&e.memory.address_references)
                {
                    if let MirMem::Spill { id, .. } = mem {
                        used.insert(*id);
                    }
                }
            }
        }
        let unresolved = ownership.iter().any(|(p, _)| {
            p.routines
                .iter()
                .find(|old| old.id == r.id)
                .is_some_and(|old| unresolved_machine_storage(p, old))
        });
        if !unresolved {
            let ids = ids
                .into_iter()
                .filter(|id| {
                    !used.contains(id)
                        && ownership.iter().all(|(_, owner)| {
                            owner.role(r.id, &MirMem::Spill { id: *id, offset: 0 })
                                == StorageRole::Temporary
                        })
                })
                .collect::<BTreeSet<_>>();
            if !ids.is_empty() {
                removed.insert(r.id, ids);
            }
        }
    }
    (removed, candidates)
}

fn erase(program: &MirProgram, removed: &Removed) -> MirProgram {
    let mut proposed = program.clone();
    for r in &mut proposed.routines {
        if let Some(ids) = removed.get(&r.id) {
            r.frame.spills.retain(|id| !ids.contains(id));
        }
    }
    proposed
}

fn only_declarations(before: &MirProgram, after: &MirProgram, removed: &Removed) -> bool {
    !removed.is_empty()
        && removed.iter().all(|(id, spills)| {
            before
                .routines
                .iter()
                .find(|r| r.id == *id)
                .is_some_and(|r| {
                    !spills.is_empty() && spills.iter().all(|s| r.frame.spills.contains(s))
                })
        })
        && erase(before, removed) == *after
}

fn unobserved_allocations(program: &MirProgram, old: &Measurement, removed: &Removed) -> bool {
    let Some(layout) = &old.storage_layout else {
        return false;
    };
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(program);
    // Generic source-memory effects cannot expose a non-escaping compiler
    // allocation. Raw assembly still needs an inspectable physical footprint:
    // its numeric operands may not appear in the operation's typed references.
    for r in &program.routines {
        if r.blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| matches!(op, MirOp::MachineBlock { .. }))
            && !callees.get(r.id).is_some_and(|s| {
                matches!(s.reads(), MirKnownMemoryReads::Exact(_))
                    && matches!(s.writes(), MirKnownMemoryWrites::Exact(_))
            })
        {
            return false;
        }
    }
    let mut observations = observable_reads(program, &old.memory);
    for r in &program.routines {
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            let effects = classify_op(op).memory;
            for effect in [&effects.structured_reads, &effects.structured_writes] {
                if let MirMemoryEffect::Regions(regions) = effect {
                    for region in regions {
                        let mem = match region.kind {
                            MirMemoryRegionKind::Local(id) => MirMem::Local {
                                id,
                                offset: region.offset,
                            },
                            MirMemoryRegionKind::Param(id) => MirMem::Param {
                                id,
                                offset: region.offset,
                            },
                            MirMemoryRegionKind::Global(id) => MirMem::Global {
                                id,
                                offset: region.offset,
                            },
                            MirMemoryRegionKind::Static(id) => MirMem::Static {
                                id,
                                offset: region.offset,
                            },
                            MirMemoryRegionKind::AbsoluteRange | MirMemoryRegionKind::ZeroPage => {
                                MirMem::Absolute(region.offset)
                            }
                            MirMemoryRegionKind::Stack => {
                                MirMem::Absolute(0x100u16.wrapping_add(region.offset))
                            }
                        };
                        observations.push((old.memory.address(r.id, &mem), region.size));
                    }
                }
            }
            for w in effects.direct_writes {
                if !matches!(w.base, MirMem::Spill { .. } | MirMem::ZeroPage(_)) {
                    observations.push((old.memory.address(r.id, &w.base), w.bytes));
                }
            }
        }
    }
    for r in &program.routines {
        if let Some(summary) = callees.get(r.id)
            && let MirKnownMemoryWrites::Exact(cells) = summary.writes()
        {
            for cell in cells {
                if matches!(cell, MirMem::Absolute(_) | MirMem::FixedZeroPage(_)) {
                    observations.push((old.memory.address(r.id, cell), 1));
                }
            }
        }
    }
    removed.iter().all(|(&r, ids)| {
        ids.iter().all(|&id| {
            layout.spill_address(r, id).is_some_and(|address| {
                (0x400..0x8000).contains(&address)
                    && old.memory.execution_disjoint(address)
                    && old.memory.source_storage_disjoint(address)
                    && !observations.iter().any(|(start, bytes)| {
                        start.is_none_or(|start| address.wrapping_sub(start) < *bytes)
                    })
            })
        })
    })
}

pub(super) fn select(
    baseline: MirProgram,
    references: &[&MirProgram],
    preservation: &Preservation,
    ranges: Option<&MirAddressRanges<'_>>,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    trace: &Trace,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    trace.run("late-spill-pruning", |trace| {
        let before_snapshot = trace.snapshot("attempt-input", &baseline);
        let (removed, candidates) = discover(&baseline, references);
        let result = |reason: Rejection| {
            trace.event("late-spill-pruning-result", || {
                serde_json::json!({
                    "accepted": false, "reason": format!("{reason:?}"),
                    "candidate_spills": candidates, "unreferenced_spills": count(&removed),
                    "removed_spills": 0,
                })
            });
        };
        if removed.is_empty() {
            result(if candidates == 0 {
                Rejection::NoUnusedSpills
            } else {
                Rejection::ReferencedOrExposed
            });
            return Ok(baseline);
        }
        let proposed = erase(&baseline, &removed);
        let proposed_snapshot = trace.snapshot("proposed", &proposed);
        let old_layout = MaterializeLayout::new(&baseline, origin);
        let new_layout = MaterializeLayout::new(&proposed, origin);
        if !old_layout.preserves_surviving_storage(&new_layout, &removed) {
            if trace.layout_census_enabled() {
                trace.event("core-allocation-movement", || {
                    serde_json::json!({
                        "before_snapshot": before_snapshot,
                        "proposed_snapshot": proposed_snapshot,
                        "delta": old_layout.spill_pruning_movement(&new_layout, &removed),
                    })
                });
            }
            result(Rejection::CoreAllocation);
            return Ok(baseline);
        }
        let old = measure(&baseline, references, origin, runtime, trace)?;
        let checked = (|| {
            if !only_declarations(&baseline, &proposed, &removed) {
                return Err(Rejection::InvalidChange);
            }
            if super::super::verify_program(&proposed, MirPhase::PreEmission).is_err() {
                return Err(Rejection::Verification);
            }
            if !callee_contracts_preserved(&baseline, &proposed) {
                return Err(Rejection::CalleeContract);
            }
            if !unobserved_allocations(&baseline, &old, &removed) {
                return Err(Rejection::ObservableAllocation);
            }
            let new = measure(&proposed, references, origin, runtime, trace)
                .map_err(|_| Rejection::CandidateEmission)?;
            if !old
                .storage_layout
                .as_ref()
                .zip(new.storage_layout.as_ref())
                .is_some_and(|(a, b)| a.preserves_surviving_storage(b, &removed))
            {
                return Err(Rejection::PhysicalAllocation);
            }
            preservation
                .validate(&new.memory, ranges, config, origin, trace)
                .map_err(Rejection::EarlierDecision)?;
            if !improves_cost(&baseline, &old, &new) {
                return Err(Rejection::Cost);
            }
            Ok(new)
        })();
        trace.decision("late-spill-pruning", removed.keys().copied(), |_| {
            checked.is_ok()
        });
        match checked {
            Ok(new) => {
                trace.event("late-spill-pruning-result", || {
                    serde_json::json!({
                        "accepted": true, "candidate_spills": candidates,
                        "unreferenced_spills": count(&removed), "removed_spills": count(&removed),
                        "image_bytes_before": old.image_bytes, "image_bytes_after": new.image_bytes,
                        "instruction_bytes_before": old.instruction_bytes.values().sum::<usize>(),
                        "instruction_bytes_after": new.instruction_bytes.values().sum::<usize>(),
                    })
                });
                Ok(proposed)
            }
            Err(reason) => {
                result(reason);
                Ok(baseline)
            }
        }
    })
}

#[cfg(test)]
mod tests;
