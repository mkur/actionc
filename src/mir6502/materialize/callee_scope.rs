//! The bounded core tail owns its callee assumptions and its rollback input.
use super::*;
use crate::mir6502::analysis::known_callees::MirCalleeContractChange;
use crate::mir6502::ir::{MirRoutine, MirStorageSlot};
use crate::mir6502::trace::Trace;

pub(super) struct Scope {
    published: Option<(MirProgram, MirKnownCalleeSummaries)>,
}

impl Scope {
    fn new() -> Self {
        Self { published: None }
    }

    pub(super) fn refresh(
        &mut self,
        program: &MirProgram,
        trace: &Trace,
        boundary: &'static str,
    ) -> Option<MirKnownCalleeSummaries> {
        if let Some((source, facts)) = &self.published {
            if source == program {
                return Some(facts.clone());
            }
            if !same_domain(source, program) {
                decline(
                    trace,
                    boundary,
                    MirCalleeContractChange {
                        routine: None,
                        fact: "source-domain",
                    },
                );
                return None;
            }
        }
        trace.count("core_callee_summary_builds", 1);
        let next = MirKnownCalleeSummaries::analyze_for_rewriting(program);
        if let Some((_, previous)) = &self.published {
            trace.count("core_callee_contract_checks", 1);
            if let Err(change) = previous.contracts_preserved_by(program, &next) {
                trace.event("core-callee-contract-context", || serde_json::json!({
                    "routine": change.routine.and_then(|id| program.routines.iter().find(|r| r.id == id)).map(|r| &r.name),
                    "before": change.routine.and_then(|id| previous.get(id)).map(|s| format!("{s:?}")),
                    "after": change.routine.and_then(|id| next.get(id)).map(|s| format!("{s:?}")),
                }));
                decline(trace, boundary, change);
                return None;
            }
        }
        self.published = Some((program.clone(), next.clone()));
        Some(next)
    }
}

fn decline(trace: &Trace, boundary: &'static str, change: MirCalleeContractChange) {
    trace.count(
        match change.fact {
            "source-domain" | "fixed-backing" | "publication-policy" => {
                "core_callee_domain_declines"
            }
            "expanded-reads" | "unknown-reads" => "core_callee_read_declines",
            "expanded-writes" | "unknown-writes" => "core_callee_write_declines",
            _ => "core_callee_machine_declines",
        },
        1,
    );
    trace.event("core-callee-contract-declined", || {
        serde_json::json!({
            "boundary": boundary, "routine": change.routine.map(|r| r.0), "fact": change.fact,
        })
    });
}

// Logical footprint containment is meaningful only within the same source/ABI
// domain. Private coloring retains its existing proof; allocated ZP effects are
// checked again by their physical bytes. No emitted address is certified here.
fn same_domain(before: &MirProgram, after: &MirProgram) -> bool {
    if before.globals != after.globals
        || before.statics != after.statics
        || before.machine_blocks != after.machine_blocks
        || before.runtime_helpers != after.runtime_helpers
        || before.routines.len() != after.routines.len()
    {
        return false;
    }
    before.routines.iter().all(|a| {
        after
            .routines
            .iter()
            .find(|b| b.id == a.id)
            .is_some_and(|b| {
                a.abi == b.abi
                    && a.scalar_signature == b.scalar_signature
                    && a.direct_result_abi == b.direct_result_abi
                    && a.effects == b.effects
                    && a.frame.direct_parameter_abi == b.frame.direct_parameter_abi
                    && a.frame.parameter_homes == b.frame.parameter_homes
                    && a.frame.local_homes == b.frame.local_homes
                    && a.frame.parameter_handoff == b.frame.parameter_handoff
                    && a.frame.locals == b.frame.locals
                    && same_parameters(&a.frame.params, &b.frame.params)
                    && same_calls(a, b)
            })
    })
}

fn same_parameters(before: &[MirStorageSlot], after: &[MirStorageSlot]) -> bool {
    before.len() == after.len() && before.iter().zip(after).all(|(a, b)| {
        let mut normalized = b.clone();
        // The existing write-only-home elider retires an unused capture while
        // retaining its formal ABI. It cannot create or move a live parameter.
        if matches!((&a.base, &b.base), (MirStorageBase::Param(x), MirStorageBase::ParamAbiOnly(y)) if x == y) {
            normalized.base = a.base.clone();
        }
        *a == normalized
    })
}

fn same_calls(before: &MirRoutine, after: &MirRoutine) -> bool {
    let calls = |r: &MirRoutine| {
        r.blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| match op {
                MirOp::Call {
                    target,
                    abi,
                    effects,
                    ..
                } => Some((target.clone(), abi.clone(), effects.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let a = calls(before);
    let mut b = calls(after);
    for call in a {
        let Some(i) = b.iter().position(|c| *c == call) else {
            return false;
        };
        b.swap_remove(i);
    }
    b.is_empty()
}

pub(super) fn run(
    program: &mut MirProgram,
    config: &Mir6502Config,
    origin: u16,
    stats: &mut MirPeepholeStats,
    homes: &mut BTreeMap<RoutineId, HomeFateTracker>,
    trace: &Trace,
) -> Result<(), Vec<MirDiagnostic>> {
    run_with(program, stats, homes, trace, |p, s, h, facts, t| {
        super::finish_core_callee_tail(p, config, origin, s, h, facts, t)
    })
}

fn run_with(
    program: &mut MirProgram,
    stats: &mut MirPeepholeStats,
    homes: &mut BTreeMap<RoutineId, HomeFateTracker>,
    trace: &Trace,
    mut apply: impl FnMut(
        &mut MirProgram,
        &mut MirPeepholeStats,
        &mut BTreeMap<RoutineId, HomeFateTracker>,
        Option<&mut Scope>,
        &Trace,
    ) -> Result<bool, Vec<MirDiagnostic>>,
) -> Result<(), Vec<MirDiagnostic>> {
    let baseline = program.clone();
    let old_stats = stats.clone();
    let old_homes = homes.clone();
    let checked = trace.child("core-callee-scope", "checked");
    checked.snapshot("attempt-input", program);
    stats.trace = checked.clone();
    let mut scope = Scope::new();
    let outcome = apply(program, stats, homes, Some(&mut scope), &checked);
    if matches!(outcome, Ok(true)) {
        stats.trace = old_stats.trace;
        checked.event(
            "core-callee-scope-selected",
            || serde_json::json!({"mode": "checked"}),
        );
        checked.finish_attempt("core-callee-scope", Some(program), "returned");
        return Ok(());
    }
    checked.event(
        "core-callee-scope-rejected",
        || serde_json::json!({"diagnostics": outcome.err().map(|d| format!("{d:?}"))}),
    );
    checked.finish_attempt("core-callee-scope", None, "rejected");
    trace.count("core_callee_scope_fallbacks", 1);
    *program = baseline;
    *stats = old_stats;
    *homes = old_homes;
    let fallback = trace.child("core-callee-scope", "conservative");
    fallback.snapshot("attempt-input", program);
    let stats_trace = stats.trace.clone();
    stats.trace = fallback.clone();
    // Mandatory lowering still runs, but no stale external contract can remove
    // a caller operation. This is a single retry of the bounded tail only.
    let result = apply(program, stats, homes, None, &fallback);
    stats.trace = stats_trace;
    fallback.finish_attempt(
        "core-callee-scope",
        result.as_ref().ok().map(|_| &*program),
        if result.is_ok() { "returned" } else { "failed" },
    );
    result?;
    fallback.event(
        "core-callee-scope-selected",
        || serde_json::json!({"mode": "conservative"}),
    );
    Ok(())
}

#[cfg(test)]
mod tests;
