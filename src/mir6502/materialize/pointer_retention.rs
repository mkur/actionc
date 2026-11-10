//! Costed complete-base retention after all earlier home/placement decisions.
use super::*;
use crate::mir6502::analysis::record_bases::{
    self, MirPhysicalMemory,
    retention::{self, CallWrites},
};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::rewrite::context::MirExitStateChange;
use crate::mir6502::rewrite::plan::{MirChangeSet, MirPostHomeRewritePlan};
use crate::mir6502::rewrite::posthome::estimated_6502_cost;

fn memory(program: &MirProgram, references: &MirProgram, origin: u16) -> Option<MirPhysicalMemory> {
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = crate::mir6502::emit::emit_program_with_physical_references(
        program,
        origin,
        &mut emitter,
        true,
        references,
    )
    .ok()?;
    emitter.finish_with_relocations().ok()?;
    Some(summary.physical_memory)
}

fn apply(
    baseline: &MirProgram,
    proofs: &MirProgram,
    map: &MirPhysicalMemory,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    let calls = CallWrites::analyze(proofs, map);
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(proofs);
    let mut proposed = baseline.clone();
    let mut stats = MirPeepholeStats::default();
    let mut affected = BTreeSet::new();
    for r in &mut proposed.routines {
        if !r.blocks.iter().any(|b| {
            b.ops
                .windows(4)
                .any(|w| record_bases::setup_pair(w).is_some())
        }) {
            continue;
        }
        let rounds = r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() + 1;
        let result = MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .run_fixed_point_with_known_callees(r, &known, |r, c| {
                for b in &r.blocks {
                    for (index, window) in b.ops.windows(4).enumerate() {
                        if record_bases::setup_pair(window).is_none() {
                            continue;
                        }
                        let site = MirSite::Op {
                            block: b.id,
                            op_index: index,
                        };
                        let Some(proof) = retention::prove(r, c.generation(), site, map, &calls)
                        else {
                            stats.record(r.id, retention::blocker(r, site, map, &calls));
                            continue;
                        };
                        if retention::prove(r, c.generation(), site, map, &calls).as_ref()
                            != Some(&proof)
                        {
                            continue;
                        }
                        // The complete-pair equality proof authorizes these removed
                        // home writes. Keep the exact high load to preserve A/N/Z.
                        let replacement = vec![window[2].clone()];
                        let old = estimated_6502_cost(window);
                        let new = estimated_6502_cost(&replacement);
                        return vec![MirPostHomeRewritePlan {
                            generation: c.generation(),
                            block: b.id,
                            range: index..index + 4,
                            replacement,
                            removed_homes: vec![],
                            exit_state_change: MirExitStateChange::default(),
                            change_set: MirChangeSet::posthome_operation_change(),
                            stat: "pointer-retention-setup",
                            observations: vec![],
                            family_priority: 0,
                            estimated_byte_saving: old.0.saturating_sub(new.0),
                            estimated_cycle_saving: old.1.saturating_sub(new.1),
                        }];
                    }
                }
                vec![]
            })
            .ok()?;
        if result.applied > 0 {
            affected.insert(r.id);
        }
        record_prehome_rewrite_result(r.id, result, &mut stats);
    }
    Some((proposed, stats, affected))
}

pub(in crate::mir6502) fn select_costed(
    baseline: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    if !baseline.routines.iter().flat_map(|r| &r.blocks).any(|b| {
        b.ops
            .windows(4)
            .any(|w| record_bases::setup_pair(w).is_some())
    }) {
        return Ok(baseline);
    }
    let Some(map) = memory(&baseline, &baseline, origin) else {
        return Ok(baseline);
    };
    let Some((proposed, mut stats, affected)) = apply(&baseline, &baseline, &map) else {
        return Ok(baseline);
    };
    let old_callees = MirKnownCalleeSummaries::analyze_for_rewriting(&baseline);
    let new_callees = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
    let exits = baseline.routines.iter().all(|r| {
        !old_callees.accumulator_summary_is_observable(r.id)
            || old_callees.get(r.id).and_then(|s| s.accumulator())
                == new_callees.get(r.id).and_then(|s| s.accumulator())
    });
    let replay = !affected.is_empty()
        && memory(&proposed, &baseline, origin)
            .and_then(|map| apply(&baseline, &proposed, &map))
            .is_some_and(|(p, _, _)| p == proposed);
    let accepted = exits
        && replay
        && crate::mir6502::verify_program(&proposed, MirPhase::PreEmission).is_ok()
        && baseline
            .routines
            .iter()
            .zip(&proposed.routines)
            .all(|(a, b)| a.frame == b.frame)
        && match (
            address_values::Cost::measure(&baseline, origin),
            address_values::Cost::measure(&proposed, origin),
        ) {
            (Some(old), Some(new)) => {
                new.bytes < old.bytes && affected.iter().all(|id| new.does_not_grow(&old, *id))
            }
            _ => false,
        };
    if !accepted && !affected.is_empty() {
        stats = MirPeepholeStats::default();
        for id in affected {
            stats.record(id, "pointer-retention-declined-final-proof-or-cost");
        }
    }
    if report {
        maybe_report_peepholes(if accepted { &proposed } else { &baseline }, &stats, config);
    }
    Ok(if accepted { proposed } else { baseline })
}

#[cfg(test)]
#[path = "pointer_retention_tests.rs"]
mod tests;
