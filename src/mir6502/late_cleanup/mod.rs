//! Terminal store/producer cleanup and preservation of selected late decisions.
pub(super) mod preservation;
mod producers;
mod scoped;
use super::parameter_homes::InvocationHomes;
mod pruning;
mod register_reuse;

use super::{
    Mir6502Config, MirDiagnostic,
    analysis::{
        address_ranges::MirAddressRanges,
        effects::classify_op,
        known_callees::{MirKnownCalleeSummaries, MirKnownMemoryReads, MirKnownMemoryWrites},
        record_bases::MirPhysicalMemory,
        storage_ownership::{StorageOwnership, StorageRole},
    },
    emit,
    ir::*,
    materialize::peepholes,
    rewrite::driver::MirPostHomeRewriteDriver,
    trace::Trace,
};
use preservation::{Family, Preservation};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, PartialEq, Eq)]
enum Declined {
    NoDeadStores,
    NoDeadDefinitions,
    UntrackedDecision,
    Analysis,
    ObservableAlias,
    InvalidChange,
    Verification,
    CalleeContract,
    CandidateEmission,
    DeletionReplay,
    EarlierDecision(Family),
    Cost,
}

struct Measurement {
    storage_layout: Option<emit::MirStorageLayoutProof>,
    memory: MirPhysicalMemory,
    image_bytes: usize,
    instruction_bytes: BTreeMap<RoutineId, usize>,
}

/// One converged emission supplies every placement/cost consumer. There are no
/// separate physical-map or load-image probes hidden inside the transaction.
fn measure(
    program: &MirProgram,
    references: &[&MirProgram],
    origin: u16,
    runtime: crate::runtime::Runtime,
    trace: &Trace,
) -> Result<Measurement, Vec<MirDiagnostic>> {
    let t = trace.child("emission-request", "late-cleanup-cost-and-layout");
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = emit::emit_program_with_reference_programs_traced(
        program,
        origin,
        &mut emitter,
        true,
        references,
        &t,
    )?;
    let emission = emitter.finish_with_relocations().map_err(|errors| {
        errors
            .into_iter()
            .map(|e| MirDiagnostic {
                routine: None,
                block: None,
                message: e.message,
            })
            .collect::<Vec<_>>()
    })?;
    let mut memory = summary.physical_memory.clone();
    memory.protect_execution_ranges(
        summary
            .source_ranges
            .iter()
            .map(|r| (r.start, r.end))
            .collect(),
    );
    let mut instruction_bytes = BTreeMap::new();
    for (id, _, range) in &summary.block_ranges {
        *instruction_bytes.entry(*id).or_default() += range.len();
    }
    let storage_layout = summary.storage_layout.clone();
    let output = super::codegen_output(emission, origin, summary, runtime, program);
    Ok(Measurement {
        storage_layout,
        memory,
        instruction_bytes,
        image_bytes: crate::codegen::format_load_file(&output).len(),
    })
}

/// Both terminal transactions accept only measured packaged savings without
/// increasing any existing routine's instruction footprint.
fn improves_cost(program: &MirProgram, old: &Measurement, new: &Measurement) -> bool {
    new.image_bytes < old.image_bytes
        && program.routines.iter().all(|r| {
            new.instruction_bytes.get(&r.id).copied().unwrap_or(0)
                <= old.instruction_bytes.get(&r.id).copied().unwrap_or(0)
        })
}

fn observable_reads(program: &MirProgram, memory: &MirPhysicalMemory) -> Vec<(Option<u16>, u16)> {
    read_aliases(program, memory, false)
}

#[derive(Debug)]
struct ReadObservation {
    owner: RoutineId,
    site: Option<super::analysis::sites::MirSite>,
    // None is an explicit physical region; a missing site is a machine summary.
    mem: Option<MirMem>,
    address: Option<u16>,
    bytes: u16,
}

fn read_aliases(program: &MirProgram, memory: &MirPhysicalMemory, scratch_lifetimes: bool) -> Vec<(Option<u16>, u16)> {
    read_observations(program, memory, scratch_lifetimes).into_iter()
        .map(|r| (r.address, r.bytes)).collect()
}

fn read_observations(
    program: &MirProgram,
    memory: &MirPhysicalMemory,
    scratch_lifetimes: bool,
) -> Vec<ReadObservation> {
    let ownership = StorageOwnership::new(program);
    let mut reads = Vec::new();
    for r in &program.routines {
        for (site, op) in r.blocks.iter().flat_map(|b| b.ops.iter().enumerate().map(move |(op_index, op)|
            (super::analysis::sites::MirSite::Op { block: b.id, op_index }, op))) {
            let effects = classify_op(op);
            if scratch_lifetimes {
                for home in effects.addresses.pair_reads {
                    if let super::analysis::effects::MirHomeByte::FixedZeroPage(slot) = home
                        && ownership.role(r.id, &MirMem::FixedZeroPage(slot))
                            != StorageRole::PointerScratch
                    {
                        reads.push(ReadObservation { owner: r.id, site: Some(site), mem: Some(MirMem::FixedZeroPage(slot)), address: Some(u16::from(slot.0)), bytes: 1 });
                    }
                }
                match effects.memory.structured_reads {
                    MirMemoryEffect::Regions(regions) => {
                        for region in regions {
                            if matches!(
                                region.kind,
                                MirMemoryRegionKind::ZeroPage | MirMemoryRegionKind::AbsoluteRange
                            ) {
                                reads.push(ReadObservation { owner: r.id, site: Some(site), mem: None, address: Some(region.offset), bytes: region.size });
                            } else if region.kind == MirMemoryRegionKind::Stack {
                                reads.push(ReadObservation { owner: r.id, site: Some(site), mem: None, address: Some(0x100), bytes: 0x100 });
                            }
                        }
                    }
                    // Unknown source-memory effects do not expose non-escaping
                    // compiler homes. Preserve the shared home-analysis policy.
                    MirMemoryEffect::None | MirMemoryEffect::Unknown | MirMemoryEffect::All => {}
                }
            }
            for read in effects.memory.direct_reads {
                let pointer_scratch_read = scratch_lifetimes
                    && matches!(read.base, MirMem::FixedZeroPage(slot) if (0..read.bytes).all(|offset| {
                        u16::from(slot.0).checked_add(offset).filter(|&a| a < 0x100).is_some_and(|a| {
                            ownership.role(r.id, &MirMem::FixedZeroPage(MirFixedZpSlot(a as u8))) == StorageRole::PointerScratch
                                && memory.source_storage_disjoint(a)
                        })
                    }));
                if !matches!(read.base, MirMem::Spill { .. } | MirMem::ZeroPage(_))
                    && !pointer_scratch_read
                {
                    reads.push(ReadObservation { owner: r.id, site: Some(site), address: memory.address(r.id, &read.base), mem: Some(read.base), bytes: read.bytes });
                }
            }
        }
    }
    // Inspectable machine routines may expose absolute reads behind a call.
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(program);
    for r in &program.routines {
        if (!scratch_lifetimes
            || r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(op, MirOp::MachineBlock { .. })))
            && let Some(summary) = callees.get(r.id)
            && let MirKnownMemoryReads::Exact(cells) = summary.reads()
        {
            for cell in cells {
                if matches!(cell, MirMem::Absolute(_) | MirMem::FixedZeroPage(_)) {
                    reads.push(ReadObservation { owner: r.id, site: None, address: memory.address(r.id, cell), mem: Some(cell.clone()), bytes: 1 });
                }
            }
        }
    }
    reads
}

#[cfg(test)]
fn apply(baseline: &MirProgram, memory: Option<&MirPhysicalMemory>, remove_producers: bool, trace: &Trace) -> Result<MirProgram, Declined> {
    apply_with_observations(baseline, memory, remove_producers, trace, None)
}

fn apply_with_observations(
    baseline: &MirProgram,
    memory: Option<&MirPhysicalMemory>,
    remove_producers: bool,
    trace: &Trace,
    facts: Option<&InvocationHomes<'_>>,
) -> Result<MirProgram, Declined> {
    if facts.is_some_and(|f| !f.is_current(baseline)) { return Err(Declined::Analysis); }
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(baseline);
    let ownership = StorageOwnership::new(baseline);
    let reads = memory
        .map(|m| read_observations(baseline, m, true))
        .unwrap_or_default();
    if let (Some(facts), Some(memory)) = (facts, memory) {
        scoped::trace_rejections(baseline, &ownership, facts, memory, trace);
    }
    let mut proposed = baseline.clone();
    for r in &mut proposed.routines {
        if !r.blocks.iter().flat_map(|b| &b.ops).any(|op| {
            scoped::private_store(&ownership, facts, r.id, op, memory).is_some()
                || (remove_producers && producers::pure_register(op, r.id, memory).is_some())
        }) {
            continue;
        }
        let rounds = r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() + 1;
        MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .with_trace(trace)
            .run_fixed_point_with_known_callees(r, &known, |r, c| {
                let mut plans =
                    peepholes::discover_dead_scratch_stores_matching(r, c, |op| scoped::private_store(&ownership, facts, r.id, op, memory).is_some())
                        .into_iter()
                        .filter(|plan| {
                            let Some(memory) = memory else {
                                return true;
                            };
                            let block = r.blocks.iter().find(|b| b.id == plan.block).unwrap();
                            let home = scoped::private_store(
                                &ownership, facts, r.id, &block.ops[plan.range.start], Some(memory),
                            )
                            .unwrap();
                            let Some(address) = memory.address(r.id, &home) else {
                                return false;
                            };
                            let reason =
                                if reads.iter().any(|read| scoped::blocks_read(read, r.id, address, memory, facts)) {
                                    Some("physical-read-alias")
                                } else {
                                    None
                                };
                            if let Some(reason) = reason {
                                trace.event("late-cleanup-store-retained", || serde_json::json!({
                                "routine": r.id.0, "block": plan.block.0, "op": plan.range.start,
                                "address": address, "reason": reason,
                                "readers": reads.iter().filter(|read| scoped::blocks_read(read, r.id, address, memory, facts))
                                    .map(|read| format!("{read:?}")).collect::<Vec<_>>(),
                            }));
                            }
                            reason.is_none()
                        })
                        .collect::<Vec<_>>();
                if remove_producers {
                    plans.extend(peepholes::discover_dead_register_writes_with_purity(
                        r,
                        c,
                        |op| producers::pure_register(op, r.id, memory),
                    ));
                }
                plans
            })
            .map_err(|_| Declined::Analysis)?;
    }
    Ok(proposed)
}

struct CleanupCandidate {
    program: MirProgram,
    deletions: MirProgram,
    reuse: register_reuse::ReuseStats,
}

fn apply_traced(
    baseline: &MirProgram,
    memory: Option<&MirPhysicalMemory>,
    remove_producers: bool,
    reuse_registers: bool,
    trace: &Trace,
    purpose: &'static str,
    facts: Option<&InvocationHomes<'_>>,
) -> Result<CleanupCandidate, Declined> {
    let t = trace.child("late-cleanup-analysis", purpose);
    let result = apply_with_observations(
        baseline,
        memory,
        remove_producers,
        &t,
        facts,
    ).and_then(|deletions| {
        let mut program = deletions.clone();
        let reuse = if reuse_registers {
            register_reuse::apply(&mut program, memory, &t)?
        } else { register_reuse::ReuseStats::default() };
        Ok(CleanupCandidate { program, deletions, reuse })
    });
    t.event("attempt-end", || {
        serde_json::json!({
            "stage": "late-cleanup-analysis",
            "status": if result.is_ok() { "returned" } else { "failed" },
            "returned_snapshot": null,
        })
    });
    result
}

/// Check the complete allowed delta, including compare-producer index repair.
#[cfg(test)]
fn only_cleanup_deletions(before: &MirProgram, after: &MirProgram, remove_producers: bool, memory: Option<&MirPhysicalMemory>) -> bool {
    only_cleanup_deletions_with_observations(before, after, remove_producers, memory, None)
}

fn only_cleanup_deletions_with_observations(
    before: &MirProgram,
    after: &MirProgram,
    remove_producers: bool,
    memory: Option<&MirPhysicalMemory>,
    facts: Option<&InvocationHomes<'_>>,
) -> bool {
    if facts.is_some_and(|f| !f.is_current(before)) { return false; }
    if before.routines.len() != after.routines.len() {
        return false;
    }
    let mut expected = before.clone();
    let ownership = StorageOwnership::new(before);
    for (a, b) in expected.routines.iter_mut().zip(&after.routines) {
        if a.blocks.len() != b.blocks.len() {
            return false;
        }
        for (old, new) in a.blocks.iter_mut().zip(&b.blocks) {
            if old == new {
                continue;
            }
            let mut retained = Vec::new();
            let mut indices = Vec::new();
            let mut next = 0;
            for op in &old.ops {
                indices.push(next);
                if new.ops.get(next) == Some(op) {
                    retained.push(op.clone());
                    next += 1;
                } else if scoped::private_store(&ownership, facts, a.id, op, memory).is_none()
                    && !(remove_producers && producers::pure_register(op, a.id, memory).is_some())
                {
                    return false;
                }
            }
            if next != new.ops.len() {
                return false;
            }
            old.ops = retained;
            if let MirTerminator::Branch {
                cond: MirCond::FusedCompare { producer, .. },
                ..
            } = &mut old.terminator
            {
                if producer.block != old.id {
                    return false;
                }
                let Some(index) = indices.get(producer.op_index) else {
                    return false;
                };
                producer.op_index = *index;
            }
        }
    }
    expected == *after
}

fn callee_contracts_preserved(before: &MirProgram, after: &MirProgram) -> bool {
    let ids = before.routines.iter().map(|r| r.id).collect();
    let old = MirKnownCalleeSummaries::analyze_for_rewriting(before)
        .with_precise_entry_flags(before, &ids);
    let new = MirKnownCalleeSummaries::analyze_for_rewriting(after);
    old.entry_inputs_preserved_by(after, &new)
        && before.routines.iter().all(|r| {
            let Some(a) = old.get(r.id) else {
                return true;
            };
            let Some(b) = new.get(r.id) else {
                return false;
            };
            // Exact effects are may-read/may-write sets: a subset preserves every
            // earlier exclusion, whereas a newly unknown or enlarged set does not.
            let writes = match (a.writes(), b.writes()) {
                (MirKnownMemoryWrites::Unknown, _) => true,
                (MirKnownMemoryWrites::Exact(a), MirKnownMemoryWrites::Exact(b)) => {
                    b.iter().all(|m| a.contains(m))
                }
                _ => false,
            };
            let reads = match (a.reads(), b.reads()) {
                (MirKnownMemoryReads::Unknown, _) => true,
                (MirKnownMemoryReads::Exact(a), MirKnownMemoryReads::Exact(b)) => {
                    b.iter().all(|m| a.contains(m))
                }
                _ => false,
            };
            writes
                && reads
                && (!old.accumulator_summary_is_observable(r.id)
                    || a.accumulator().is_none()
                    || a.accumulator() == b.accumulator())
                && (a.zn().is_none() || a.zn() == b.zn())
                && [MirReg::A, MirReg::X, MirReg::Y]
                    .into_iter()
                    .all(|reg| !a.preserves_register(reg) || b.preserves_register(reg))
                && (0..255).all(|slot| {
                    !a.preserves_fixed_pair(MirFixedZpSlot(slot))
                        || b.preserves_fixed_pair(MirFixedZpSlot(slot))
                })
        })
}

fn count_ops(program: &MirProgram) -> usize {
    program
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .map(|b| b.ops.len())
        .sum()
}
fn count_stores(program: &MirProgram) -> usize {
    program
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .filter(|op| matches!(op, MirOp::Store { .. }))
        .count()
}

pub(super) fn select(
    baseline: MirProgram,
    preservation: &Preservation,
    ranges: Option<&MirAddressRanges<'_>>,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    trace: &Trace,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    if !config.enable_late_spill_pruning || !preservation.covers(&baseline) {
        return select_definitions(
            baseline,
            preservation,
            ranges,
            config,
            origin,
            runtime,
            trace,
        );
    }
    // Keep the original proof references alive without treating them as storage.
    let references = preservation.references(&baseline);
    let cleaned = select_definitions(
        baseline.clone(),
        preservation,
        ranges,
        config,
        origin,
        runtime,
        trace,
    )?;
    pruning::select(
        cleaned,
        &references,
        preservation,
        ranges,
        config,
        origin,
        runtime,
        trace,
    )
}

fn select_definitions(
    baseline: MirProgram,
    preservation: &Preservation,
    ranges: Option<&MirAddressRanges<'_>>,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    trace: &Trace,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    trace.run("late-private-store-cleanup", |trace| {
        trace.snapshot("attempt-input", &baseline);
        let remove_producers = config.enable_late_dead_register_cleanup;
        let reuse_registers = config.enable_late_register_reuse;
        let reject = |reason: Declined| {
            trace.event("late-cleanup-result", || serde_json::json!({"accepted": false, "reason": format!("{reason:?}")}));
        };
        if !preservation.covers(&baseline) {
            reject(Declined::UntrackedDecision);
            return Ok(baseline);
        }
        // No cost emission for programs whose definitions are all still live.
        match apply_traced(&baseline, None, remove_producers, reuse_registers, trace, "logical-discovery", None) {
            Ok(p) if p.program == baseline => { reject(if remove_producers { Declined::NoDeadDefinitions } else { Declined::NoDeadStores }); return Ok(baseline); }
            Err(reason) => { reject(reason); return Ok(baseline); }
            _ => {}
        }
        let references = preservation.references(&baseline);
        let facts = (config.enable_peepholes && config.enable_late_private_store_cleanup
            && config.enable_scoped_storage_observation
            && baseline.routines.iter().any(|r| !r.frame.resident_bytes().is_empty()))
            .then(|| {
                trace.count("storage_observation_snapshot_builds", 1);
                InvocationHomes::with_references(&baseline, &references)
            });
        let old = measure(&baseline, &references, origin, runtime, trace)?;
        let CleanupCandidate { program: proposed, deletions, reuse } = match apply_traced(&baseline, Some(&old.memory), remove_producers, reuse_registers, trace, "physical-deletion", facts.as_ref()) {
            Ok(p) if p.program != baseline => p,
            Ok(_) => { reject(Declined::ObservableAlias); return Ok(baseline); }
            Err(reason) => { reject(reason); return Ok(baseline); }
        };
        trace.snapshot("proposed", &proposed);
        let affected: BTreeSet<_> = baseline.routines.iter().zip(&proposed.routines)
            .filter_map(|(a, b)| (a != b).then_some(a.id)).collect();
        let checked = (|| {
            if !only_cleanup_deletions_with_observations(&baseline, &deletions, remove_producers, Some(&old.memory), facts.as_ref()) { return Err(Declined::InvalidChange); }
            if let Err(diagnostics) = super::verify_program(&proposed, MirPhase::PreEmission) {
                trace.event("late-cleanup-verification", || serde_json::json!({"diagnostics": format!("{diagnostics:?}")}));
                return Err(Declined::Verification);
            }
            if !callee_contracts_preserved(&baseline, &proposed) { return Err(Declined::CalleeContract); }
            let new = measure(&proposed, &references, origin, runtime, trace).map_err(|_| Declined::CandidateEmission)?;
            if apply_traced(&baseline, Some(&new.memory), remove_producers, reuse_registers, trace, "final-placement-deletion-replay", facts.as_ref())?.program != proposed { return Err(Declined::DeletionReplay); }
            preservation.validate(&new.memory, ranges, config, origin, trace).map_err(Declined::EarlierDecision)?;
            if !improves_cost(&baseline, &old, &new) {
                return Err(Declined::Cost);
            }
            Ok(new)
        })();
        trace.decision("late-private-store-cleanup", affected.iter().copied(), |_| checked.is_ok());
        match checked {
            Ok(new) => {
                trace.event("late-cleanup-result", || serde_json::json!({
                    "removed_stores": count_stores(&baseline) - count_stores(&deletions),
                    "removed_producers": count_ops(&baseline) - count_ops(&deletions) - (count_stores(&baseline) - count_stores(&deletions)),
                    "expanded_operations": reuse.expanded_operations,
                    "removed_reloads": reuse.removed_reloads,
                    "register_transfers": reuse.register_transfers,
                    "accepted": true, "image_bytes_before": old.image_bytes, "image_bytes_after": new.image_bytes,
                    "instruction_bytes_before": old.instruction_bytes.values().sum::<usize>(),
                    "instruction_bytes_after": new.instruction_bytes.values().sum::<usize>(),
                    "routines": affected.iter().map(|id| id.0).collect::<Vec<_>>(),
                }));
                Ok(proposed)
            }
            Err(reason) => { reject(reason); Ok(baseline) }
        }
    })
}

#[cfg(test)]
mod tests;
