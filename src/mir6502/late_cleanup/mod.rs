//! Terminal store-only cleanup and preservation of selected late decisions.
pub(super) mod preservation;
mod readers;

use super::{
    Mir6502Config, MirDiagnostic,
    analysis::{
        address_ranges::MirAddressRanges,
        effects::classify_op,
        known_callees::{MirKnownCalleeSummaries, MirKnownMemoryReads, MirKnownMemoryWrites},
        record_bases::MirPhysicalMemory,
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
    let output = super::codegen_output(emission, origin, summary, runtime, program);
    Ok(Measurement {
        memory,
        instruction_bytes,
        image_bytes: crate::codegen::format_load_file(&output).len(),
    })
}

fn observable_reads(program: &MirProgram, memory: &MirPhysicalMemory) -> Vec<(Option<u16>, u16)> {
    let mut reads = Vec::new();
    for r in &program.routines {
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            for read in classify_op(op).memory.direct_reads {
                if !matches!(read.base, MirMem::Spill { .. } | MirMem::ZeroPage(_)) {
                    reads.push((memory.address(r.id, &read.base), read.bytes));
                }
            }
        }
    }
    // Inspectable machine routines may expose absolute reads behind a call.
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(program);
    for r in &program.routines {
        if let Some(summary) = callees.get(r.id)
            && let MirKnownMemoryReads::Exact(cells) = summary.reads()
        {
            for cell in cells {
                if matches!(cell, MirMem::Absolute(_) | MirMem::FixedZeroPage(_)) {
                    reads.push((memory.address(r.id, cell), 1));
                }
            }
        }
    }
    reads
}

fn apply(
    baseline: &MirProgram,
    memory: Option<&MirPhysicalMemory>,
    default_sargs: bool,
    origin: u16,
    trace: &Trace,
) -> Result<MirProgram, Declined> {
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(baseline);
    let reads = memory
        .map(|m| observable_reads(baseline, m))
        .unwrap_or_default();
    let mut proposed = baseline.clone();
    for r in &mut proposed.routines {
        if !r
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| peepholes::store_direct_private_scratch_byte(op).is_some())
        {
            continue;
        }
        let rounds = r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() + 1;
        MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .with_trace(trace)
            .run_fixed_point_with_known_callees(r, &known, |r, c| {
                peepholes::discover_dead_private_scratch_stores(r, c)
                    .into_iter()
                    .filter(|plan| {
                        let Some(memory) = memory else {
                            return true;
                        };
                        let block = r.blocks.iter().find(|b| b.id == plan.block).unwrap();
                        let home = peepholes::store_direct_private_scratch_byte(
                            &block.ops[plan.range.start],
                        )
                        .unwrap();
                        let Some(address) = memory.address(r.id, &home) else {
                            return false;
                        };
                        let reason = if reads.iter().any(|(start, bytes)| {
                            start.is_none_or(|start| address.wrapping_sub(start) < *bytes)
                        }) {
                            Some("physical-read-alias")
                        } else if address < 0x100
                            && !(readers::Readers {
                                program: baseline,
                                memory,
                                known: &known,
                                default_sargs,
                                origin,
                            })
                            .definition_unobserved(
                                r,
                                plan.block,
                                plan.range.end,
                                address,
                            )
                        {
                            Some("zero-page-reader-before-overwrite")
                        } else {
                            None
                        };
                        if let Some(reason) = reason {
                            trace.event("late-cleanup-store-retained", || serde_json::json!({
                                "routine": r.id.0, "block": plan.block.0, "op": plan.range.start,
                                "address": address, "reason": reason,
                            }));
                        }
                        reason.is_none()
                    })
                    .collect()
            })
            .map_err(|_| Declined::Analysis)?;
    }
    Ok(proposed)
}

/// Check the complete allowed delta, including compare-producer index repair.
fn only_private_store_deletions(before: &MirProgram, after: &MirProgram) -> bool {
    if before.routines.len() != after.routines.len() {
        return false;
    }
    let mut expected = before.clone();
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
                } else if peepholes::store_direct_private_scratch_byte(op).is_none() {
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

pub(super) fn select(
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
        let reject = |reason: Declined| {
            trace.event("late-cleanup-result", || serde_json::json!({"accepted": false, "reason": format!("{reason:?}")}));
        };
        if !preservation.covers(&baseline) {
            reject(Declined::UntrackedDecision);
            return Ok(baseline);
        }
        // No cost emission for programs whose definitions are all still live.
        match apply(&baseline, None, preservation.default_sargs(), origin, &trace.child("late-cleanup-analysis", "logical-discovery")) {
            Ok(p) if p == baseline => { reject(Declined::NoDeadStores); return Ok(baseline); }
            Err(reason) => { reject(reason); return Ok(baseline); }
            _ => {}
        }
        let references = preservation.references(&baseline);
        let old = measure(&baseline, &references, origin, runtime, trace)?;
        let proposed = match apply(&baseline, Some(&old.memory), preservation.default_sargs(), origin, &trace.child("late-cleanup-analysis", "physical-deletion")) {
            Ok(p) if p != baseline => p,
            Ok(_) => { reject(Declined::ObservableAlias); return Ok(baseline); }
            Err(reason) => { reject(reason); return Ok(baseline); }
        };
        trace.snapshot("proposed", &proposed);
        let affected: BTreeSet<_> = baseline.routines.iter().zip(&proposed.routines)
            .filter_map(|(a, b)| (a != b).then_some(a.id)).collect();
        let checked = (|| {
            if !only_private_store_deletions(&baseline, &proposed) { return Err(Declined::InvalidChange); }
            if let Err(diagnostics) = super::verify_program(&proposed, MirPhase::PreEmission) {
                trace.event("late-cleanup-verification", || serde_json::json!({"diagnostics": format!("{diagnostics:?}")}));
                return Err(Declined::Verification);
            }
            if !callee_contracts_preserved(&baseline, &proposed) { return Err(Declined::CalleeContract); }
            let new = measure(&proposed, &references, origin, runtime, trace).map_err(|_| Declined::CandidateEmission)?;
            if apply(&baseline, Some(&new.memory), preservation.default_sargs(), origin, &trace.child("late-cleanup-analysis", "final-placement-deletion-replay"))? != proposed { return Err(Declined::DeletionReplay); }
            preservation.validate(&new.memory, ranges, config, origin, trace).map_err(Declined::EarlierDecision)?;
            if new.image_bytes >= old.image_bytes || baseline.routines.iter().any(|r|
                new.instruction_bytes.get(&r.id).copied().unwrap_or(0) > old.instruction_bytes.get(&r.id).copied().unwrap_or(0)) {
                return Err(Declined::Cost);
            }
            Ok(new)
        })();
        trace.decision("late-private-store-cleanup", affected.iter().copied(), |_| checked.is_ok());
        match checked {
            Ok(new) => {
                trace.event("late-cleanup-result", || serde_json::json!({
                    "removed_stores": baseline.routines.iter().flat_map(|r| &r.blocks).map(|b| b.ops.len()).sum::<usize>() - proposed.routines.iter().flat_map(|r| &r.blocks).map(|b| b.ops.len()).sum::<usize>(),
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
