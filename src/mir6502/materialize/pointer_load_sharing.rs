//! Share two ordinary pointer reads within one fully restored machine window.
use super::*;
use crate::mir6502::analysis::{
    pointer_load_sharing as facts, record_bases::MirPhysicalMemory, sites::MirSite,
};
use crate::mir6502::rewrite::{
    context::{MirExitStateChange, PostHomeRewriteContext},
    plan::MirPostHomeRewritePlan,
    posthome::structural_plan,
};

fn discover(
    r: &super::super::ir::MirRoutine,
    c: &PostHomeRewriteContext<'_, '_>,
    memory: &MirPhysicalMemory,
    ids: &BTreeMap<MirBlockId, Vec<usize>>,
    seen: &mut BTreeSet<(MirSite, &'static str)>,
) -> Vec<MirPostHomeRewritePlan> {
    for b in &r.blocks {
        for (i, ops) in b.ops.windows(facts::WINDOW).enumerate() {
            if facts::shape(ops).is_err() {
                continue;
            }
            let key = MirSite::Op {
                block: b.id,
                op_index: ids[&b.id][i],
            };
            seen.insert((key, "pointer-load-sharing-candidate"));
            let proof = match c.pointer_load_sharing(
                c.point(MirSite::Op {
                    block: b.id,
                    op_index: i,
                }),
                memory,
            ) {
                Ok(proof) => proof,
                Err(reason) => {
                    let stat = match reason {
                        facts::Blocker::Storage => "pointer-load-sharing-blocked-storage",
                        facts::Blocker::Placement => "pointer-load-sharing-blocked-placement",
                        facts::Blocker::Overlap => "pointer-load-sharing-blocked-overlap",
                        _ => "pointer-load-sharing-blocked-shape",
                    };
                    seen.insert((key, stat));
                    continue;
                }
            };
            if let Some(mut plan) = structural_plan(
                r,
                c,
                b.id,
                i..i + facts::WINDOW,
                facts::replacement(ops).unwrap(),
                MirExitStateChange::default(),
                "pointer-load-sharing-selected",
                42,
            ) {
                plan.pointer_load_sharing = Some(proof);
                return vec![plan];
            }
            seen.insert((key, "pointer-load-sharing-blocked-region-proof"));
        }
    }
    vec![]
}

fn has_candidate(p: &MirProgram) -> bool {
    p.routines.iter().flat_map(|r| &r.blocks).any(|b| {
        b.ops
            .windows(facts::WINDOW)
            .any(|ops| facts::shape(ops).is_ok())
    })
}

fn apply(
    baseline: &MirProgram,
    memory: &MirPhysicalMemory,
    eligible: Option<&BTreeSet<RoutineId>>,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(baseline);
    let mut proposed = baseline.clone();
    let mut stats = MirPeepholeStats::default();
    let mut affected = BTreeSet::new();
    for r in &mut proposed.routines {
        if eligible.is_some_and(|ids| !ids.contains(&r.id))
            || !r.blocks.iter().any(|b| {
                b.ops
                    .windows(facts::WINDOW)
                    .any(|ops| facts::shape(ops).is_ok())
            })
        {
            continue;
        }
        let mut ids = r
            .blocks
            .iter()
            .map(|b| (b.id, (0..b.ops.len()).collect::<Vec<_>>()))
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        let rounds = r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() + 1;
        let result = MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .run_fixed_point_with_physical_memory(r, &callees, memory, |r, c| {
                let plans = discover(r, c, memory, &ids, &mut seen);
                // Keep the original first-load identity across index shifts so a
                // previously blocked neighbor is not counted again after motion.
                for p in &plans {
                    let positions = ids.get_mut(&p.block).unwrap();
                    let old = positions[p.range.clone()].to_vec();
                    positions.splice(
                        p.range.clone(),
                        [0, 7, 1, 2, 3, 9, 4, 5, 10].map(|i| old[i]),
                    );
                }
                plans
            })
            .ok()?;
        for (_, stat) in seen {
            stats.record(r.id, stat);
        }
        if result.applied > 0 {
            affected.insert(r.id);
        }
        record_prehome_rewrite_result(r.id, result, &mut stats);
    }
    Some((proposed, stats, affected))
}

fn exit_facts_preserved(
    old: &MirKnownCalleeSummaries,
    new: &MirKnownCalleeSummaries,
    id: RoutineId,
) -> bool {
    let Some(a) = old.get(id) else {
        return true;
    };
    let Some(b) = new.get(id) else {
        return false;
    };
    (!old.accumulator_summary_is_observable(id) || a.accumulator() == b.accumulator())
        && [MirReg::X, MirReg::Y]
            .iter()
            .all(|&r| !a.preserves_register(r) || b.preserves_register(r))
}
fn image_size(p: &MirProgram, origin: u16, runtime: crate::runtime::Runtime) -> Option<usize> {
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = super::super::emit::emit_program(p, origin, &mut emitter).ok()?;
    let output = super::super::codegen_output(
        emitter.finish_with_relocations().ok()?,
        origin,
        summary,
        runtime,
        p,
    );
    Some(crate::codegen::format_load_file(&output).len())
}

pub(in crate::mir6502) fn select_costed(
    baseline: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    if !has_candidate(&baseline) {
        return Ok(baseline);
    }
    let Some(memory) = register_copies::physical_memory(&baseline, &baseline, origin) else {
        return Ok(baseline);
    };
    let Some((mut proposed, mut stats, affected)) = apply(&baseline, &memory, None) else {
        return Ok(baseline);
    };
    if affected.is_empty() {
        maybe_report_peepholes(&baseline, &stats, config);
        return Ok(baseline);
    }
    let old = address_values::Cost::measure(&baseline, origin);
    let cost = address_values::Cost::measure(&proposed, origin);
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(&baseline);
    let new_callees = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
    let mut accepted = affected
        .iter()
        .copied()
        .filter(|&id| {
            old.as_ref()
                .zip(cost.as_ref())
                .is_some_and(|(a, b)| b.does_not_grow(a, id))
                && exit_facts_preserved(&callees, &new_callees, id)
        })
        .collect::<BTreeSet<_>>();
    for r in &mut proposed.routines {
        if affected.contains(&r.id) && !accepted.contains(&r.id) {
            *r = baseline
                .routines
                .iter()
                .find(|old| old.id == r.id)
                .unwrap()
                .clone();
        }
    }
    if !accepted.is_empty() {
        let replay = register_copies::physical_memory(&proposed, &baseline, origin)
            .and_then(|m| apply(&baseline, &m, Some(&accepted)))
            .is_some_and(|(p, _, _)| p == proposed);
        let final_callees = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
        let good = replay
            && super::super::verify_program(&proposed, MirPhase::PreEmission).is_ok()
            && baseline
                .routines
                .iter()
                .zip(&proposed.routines)
                .all(|(a, b)| {
                    a.frame == b.frame && exit_facts_preserved(&callees, &final_callees, a.id)
                })
            && old
                .as_ref()
                .zip(address_values::Cost::measure(&proposed, origin))
                .is_some_and(|(a, b)| {
                    b.bytes <= a.bytes && accepted.iter().all(|&id| b.does_not_grow(a, id))
                })
            && image_size(&baseline, origin, runtime)
                .zip(image_size(&proposed, origin, runtime))
                .is_some_and(|(a, b)| b < a);
        if !good {
            accepted.clear();
        }
    }
    for id in affected {
        let stat = if accepted.contains(&id) {
            "pointer-load-sharing-applied"
        } else if !exit_facts_preserved(&callees, &new_callees, id) {
            "pointer-load-sharing-declined-callee-exit"
        } else {
            "pointer-load-sharing-declined-final-proof-or-cost"
        };
        stats.record_many(
            id,
            stat,
            stats.count_for(id, "pointer-load-sharing-selected"),
        );
    }
    let result = if accepted.is_empty() {
        baseline
    } else {
        proposed
    };
    maybe_report_peepholes(&result, &stats, config);
    Ok(result)
}

#[cfg(test)]
#[path = "pointer_load_sharing_tests.rs"]
mod tests;
