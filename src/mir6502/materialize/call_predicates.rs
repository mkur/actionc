//! Final code/home/image and physical-placement gate for predicate arguments.
use super::address_values::Cost;
use super::*;
use crate::mir6502::ir::MirRoutine;
use crate::mir6502::analysis::call_predicates::{
    ArgumentSnapshot, physical_source, predicate_call,
};
use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::rewrite::context::PreHomeRewriteContext;
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::mir6502) fn materialize_costed(
    original: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let layout = MaterializeLayout::new(&original, origin);
    let mut selected = original.clone();
    let mut proofs = BTreeMap::<RoutineId, Vec<(ArgumentSnapshot, MirOp)>>::new();
    for routine in &mut selected.routines {
        let snapshot = PreHomeAnalysisSnapshot::new(routine, MirRoutineGeneration::initial())
            .map_err(|e| {
                vec![MirDiagnostic::routine(
                    &routine.name,
                    format!("argument snapshot CFG: {e:?}"),
                )]
            })?;
        let context = PreHomeRewriteContext::new(&snapshot);
        let mut facts = Vec::new();
        for b in &routine.blocks {
            for (i, op) in b.ops.iter().enumerate() {
                let site = MirSite::Op {
                    block: b.id,
                    op_index: i,
                };
                if let Some((_, predicate)) = predicate_call(routine, &context, site) {
                    for a in 0..predicate {
                        if let Ok(fact) =
                            context.argument_snapshot(routine, context.point(site), a, &layout)
                        {
                            facts.push((fact, op.clone()));
                        }
                    }
                }
            }
        }
        if facts.is_empty() {
            continue;
        }
        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point(routine, |r, c| {
                crate::mir6502::rewrite::call_predicates::discover_argument_snapshots(r, c, &layout)
            })
            .map_err(|e| {
                vec![MirDiagnostic::routine(
                    &routine.name,
                    format!("argument snapshot transaction: {e:?}"),
                )]
            })?;
        if result.applied > 0 {
            proofs.insert(routine.id, facts);
        }
    }
    let mut off = config.clone();
    off.enable_call_predicate_arguments = false;
    off.enable_call_predicate_abi = false;
    if proofs.is_empty() {
        return crate::mir6502::materialize_resolved_program_without_call_costing(
            original, &off, origin, runtime, report,
        );
    }
    let baseline = crate::mir6502::materialize_resolved_program_without_call_costing(
        original.clone(),
        &off,
        origin,
        runtime,
        false,
    )?;
    let mut trial = crate::mir6502::materialize_resolved_program_without_call_costing(
        selected.clone(),
        config,
        origin,
        runtime,
        false,
    )?;
    let baseline_cost = Cost::measure(&baseline, origin);
    let trial_cost = Cost::measure(&trial, origin);
    let placed = MaterializeLayout::new(&trial, origin);
    let mut accepted = BTreeSet::new();
    for (id, facts) in &proofs {
        let r = trial.routines.iter().find(|r| r.id == *id).unwrap();
        if facts
            .iter()
            .all(|(fact, call)| physical_source(r, &placed, &fact.source, fact.width, call))
            && baseline_cost
                .as_ref()
                .and_then(|c| c.routine(*id))
                .zip(trial_cost.as_ref().and_then(|c| c.routine(*id)))
                .is_some_and(|(old, new)| new <= old)
        {
            accepted.insert(*id);
        }
    }
    if accepted.len() != proofs.len() && !accepted.is_empty() {
        for r in &mut selected.routines {
            if proofs.contains_key(&r.id) && !accepted.contains(&r.id) {
                *r = original
                    .routines
                    .iter()
                    .find(|old| old.id == r.id)
                    .unwrap()
                    .clone();
            }
        }
        trial = crate::mir6502::materialize_resolved_program_without_call_costing(
            selected.clone(),
            config,
            origin,
            runtime,
            false,
        )?;
    }
    // Reconfirm after rollback: placement and relaxed branches may have moved.
    let placed = MaterializeLayout::new(&trial, origin);
    let final_cost = Cost::measure(&trial, origin);
    if !baseline_cost
        .as_ref()
        .zip(final_cost.as_ref())
        .is_some_and(|(old, new)| new.bytes <= old.bytes)
        || accepted.iter().any(|id| {
            let r = trial.routines.iter().find(|r| r.id == *id).unwrap();
            !proofs[id]
                .iter()
                .all(|(fact, call)| physical_source(r, &placed, &fact.source, fact.width, call))
                || !baseline_cost
                    .as_ref()
                    .and_then(|c| c.routine(*id))
                    .zip(final_cost.as_ref().and_then(|c| c.routine(*id)))
                    .is_some_and(|(old, new)| new <= old)
        })
    {
        accepted.clear();
    }
    if report {
        let mut stats = MirPeepholeStats::default();
        for id in proofs.keys() {
            stats.record(
                *id,
                if accepted.contains(id) {
                    "call-predicate-applied"
                } else {
                    "call-predicate-declined"
                },
            );
        }
        maybe_report_peepholes(&original, &stats, config);
        if stats::reporting_enabled(config) {
            return crate::mir6502::materialize_resolved_program_without_call_costing(
                if accepted.is_empty() {
                    original
                } else {
                    selected
                },
                if accepted.is_empty() { &off } else { config },
                origin,
                runtime,
                true,
            );
        }
    }
    Ok(if accepted.is_empty() { baseline } else { trial })
}

/// Each diamond is committed as a whole routine candidate. Every commit gets a
/// new generation and rebuilds the immutable CFG/use-def/liveness snapshot.
pub(super) fn carry_predicate_into_abi(
    routine: &mut MirRoutine,
    layout: &MaterializeLayout,
    stats: &mut MirPeepholeStats,
) -> Result<(), Vec<MirDiagnostic>> {
    let mut generation = MirRoutineGeneration::initial();
    loop {
        let snapshot = PreHomeAnalysisSnapshot::new(routine, generation).map_err(|e| {
            vec![MirDiagnostic::routine(
                &routine.name,
                format!("predicate ABI CFG: {e:?}"),
            )]
        })?;
        let context = PreHomeRewriteContext::new(&snapshot);
        let plan = routine.blocks.iter().find_map(|b| {
            b.ops.iter().enumerate().find_map(|(i, _)| {
                crate::mir6502::rewrite::predicate_abi::discover_predicate_abi(
                    routine,
                    &context,
                    context.point(MirSite::Op {
                        block: b.id,
                        op_index: i,
                    }),
                    layout,
                )
            })
        });
        let Some(plan) = plan else { return Ok(()) };
        if !crate::mir6502::rewrite::predicate_abi::apply_predicate_abi(routine, generation, plan) {
            return Err(vec![MirDiagnostic::routine(
                &routine.name,
                "predicate ABI transaction rejected",
            )]);
        }
        generation = generation.next();
        stats.record(routine.id, "call-predicate-abi-home");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn call_predicate_abi_selection_removes_all_argument_spills() {
        let p = crate::mir6502::analysis::call_predicates::tests::program();
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            let mut on = Mir6502Config::optimized();
            on.enable_small_leaf_inlining = false;
            let trial = crate::mir6502::materialize_program_with_origin_and_runtime(
                p.clone(),
                &on,
                0x3000,
                runtime,
            )
            .unwrap();
            let r = trial.routines.iter().find(|r| r.name == "Forward").unwrap();
            assert_eq!(r.frame.spills.len(), 0);
            assert_eq!(
                r.frame.params.iter().map(|p| p.storage_size).sum::<u16>(),
                5
            );
            let other = trial
                .routines
                .iter()
                .find(|r| r.name == "PredicateFirst")
                .unwrap();
            assert_eq!(
                other
                    .frame
                    .locals
                    .iter()
                    .map(|p| p.storage_size)
                    .sum::<u16>(),
                1
            );
        }
    }
    #[test]
    fn call_predicate_selection_removes_word_spills_with_bounded_final_cost() {
        let p = crate::mir6502::analysis::call_predicates::tests::program();
        let mut on = Mir6502Config::optimized();
        on.enable_small_leaf_inlining = false;
        on.enable_call_predicate_abi = false;
        let mut off = on.clone();
        off.enable_call_predicate_arguments = false;
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            let baseline = crate::mir6502::materialize_program_with_origin_and_runtime(
                p.clone(),
                &off,
                0x3000,
                runtime,
            )
            .unwrap();
            let trial = crate::mir6502::materialize_program_with_origin_and_runtime(
                p.clone(),
                &on,
                0x3000,
                runtime,
            )
            .unwrap();
            let r = trial.routines.iter().find(|r| r.name == "Forward").unwrap();
            assert_eq!(r.frame.spills.len(), 1);
            assert_eq!(
                r.frame.params.iter().map(|p| p.storage_size).sum::<u16>(),
                5
            );
            let old = Cost::measure(&baseline, 0x3000).unwrap();
            let new = Cost::measure(&trial, 0x3000).unwrap();
            assert!(new.routine(r.id).unwrap() < old.routine(r.id).unwrap());
            assert!(new.bytes <= old.bytes);
        }
    }
}
