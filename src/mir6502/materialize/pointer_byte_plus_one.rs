//! Adjacent fixed-pair fusion after all previous final selectors have finished.
use super::*;
use crate::mir6502::analysis::{
    effects::MirFlagSet, pointer_byte_plus_one as facts, sites::MirSite,
};
use crate::mir6502::rewrite::{
    context::{MirExitStateChange, PostHomeRewriteContext},
    plan::MirPostHomeRewritePlan,
    posthome::structural_plan,
};

fn discover(
    r: &super::super::ir::MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    seen: &mut BTreeSet<(MirSite, &'static str)>,
) -> Vec<MirPostHomeRewritePlan> {
    for b in &r.blocks {
        for i in 0..b.ops.len().saturating_sub(1) {
            if !matches!(b.ops[i], MirOp::OffsetPointerByIndirectByte { .. }) {
                continue;
            }
            let site = MirSite::Op {
                block: b.id,
                op_index: i,
            };
            let point = context.point(site);
            match facts::shape(&b.ops[i..i + 2]) {
                Err(facts::Blocker::Shape) => {
                    seen.insert((site, "pointer-byte-plus-one-blocked-shape"));
                    continue;
                }
                _ => {
                    seen.insert((site, "pointer-byte-plus-one-candidate"));
                }
            }
            let proof = match context.pointer_byte_plus_one(point) {
                Ok(p) => p,
                Err(reason) => {
                    let stat = match reason {
                        facts::Blocker::Placement => "pointer-byte-plus-one-blocked-placement",
                        facts::Blocker::Source => "pointer-byte-plus-one-blocked-source-definition",
                        facts::Blocker::MachineState => {
                            "pointer-byte-plus-one-blocked-machine-state"
                        }
                        facts::Blocker::Definition => {
                            "pointer-byte-plus-one-blocked-observed-definition"
                        }
                        _ => "pointer-byte-plus-one-blocked-shape",
                    };
                    seen.insert((site, stat));
                    continue;
                }
            };
            let mut change = MirExitStateChange {
                flags: MirFlagSet::all(),
                ..MirExitStateChange::default()
            };
            change.registers.a = true;
            change.registers.x = true;
            if let Some(plan) = structural_plan(
                r,
                context,
                b.id,
                i..i + 2,
                vec![proof.shape.selected()],
                change,
                "pointer-byte-plus-one-selected",
                41,
            ) {
                // Stop at the first legal plan; unvisited sites are counted only
                // in their own generation after any preceding index shift.
                return vec![plan];
            }
            seen.insert((site, "pointer-byte-plus-one-blocked-region-proof"));
        }
    }
    vec![]
}

fn apply(
    baseline: &MirProgram,
    callees: &MirKnownCalleeSummaries,
    eligible: Option<&BTreeSet<RoutineId>>,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    let mut proposed = baseline.clone();
    let mut stats = MirPeepholeStats::default();
    let mut affected = BTreeSet::new();
    for r in &mut proposed.routines {
        if eligible.is_some_and(|ids| !ids.contains(&r.id))
            || !r
                .blocks
                .iter()
                .any(|b| b.ops.windows(2).any(|ops| facts::shape(ops).is_ok()))
        {
            continue;
        }
        let mut seen = BTreeSet::new();
        let rounds = r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() + 1;
        let result = MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .run_fixed_point_with_known_callees(r, callees, |r, c| discover(r, c, &mut seen))
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
    if !baseline
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .any(|b| b.ops.windows(2).any(|ops| facts::shape(ops).is_ok()))
    {
        return Ok(baseline);
    }
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(&baseline);
    let Some((mut proposed, mut stats, affected)) = apply(&baseline, &callees, None) else {
        return Ok(baseline);
    };
    if affected.is_empty() {
        maybe_report_peepholes(&baseline, &stats, config);
        return Ok(baseline);
    }
    let old = address_values::Cost::measure(&baseline, origin);
    let cost = address_values::Cost::measure(&proposed, origin);
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
                .find(|a| a.id == r.id)
                .unwrap()
                .clone();
        }
    }
    if !accepted.is_empty() {
        // These operands are literal fixed physical pairs. Reconstruct the
        // typed proof after selection; code/data relocation cannot change
        // their disjointness and no frame or allocation may change.
        let replay =
            apply(&baseline, &callees, Some(&accepted)).is_some_and(|(p, _, _)| p == proposed);
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
            "pointer-byte-plus-one-applied"
        } else if !exit_facts_preserved(&callees, &new_callees, id) {
            "pointer-byte-plus-one-declined-callee-exit"
        } else {
            "pointer-byte-plus-one-declined-final-proof-or-cost"
        };
        stats.record_many(
            id,
            stat,
            stats.count_for(id, "pointer-byte-plus-one-selected"),
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
#[path = "pointer_byte_plus_one_selection_tests.rs"]
mod tests;
