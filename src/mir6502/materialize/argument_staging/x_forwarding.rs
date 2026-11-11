//! General retained-store forwarding after all preceding final selectors.
use super::*;
use crate::mir6502::analysis::argument_x_forwarding as facts;
use crate::mir6502::rewrite::plan::MirPostHomeWitness;
use std::collections::BTreeMap;

fn discover(
    r: &MirRoutine,
    c: &PostHomeRewriteContext<'_, '_>,
    m: &MirPhysicalMemory,
    ids: &BTreeMap<MirBlockId, Vec<usize>>,
    seen: &mut BTreeSet<(MirSite, &'static str)>,
) -> Vec<MirPostHomeRewritePlan> {
    for b in &r.blocks {
        for i in 0..b.ops.len() {
            let point = c.point(MirSite::Op {
                block: b.id,
                op_index: i,
            });
            if facts::shape(r, point.site).is_err() {
                continue;
            }
            let key = MirSite::Op {
                block: b.id,
                op_index: ids[&b.id][i],
            };
            seen.insert((key, "argument-x-forwarding-candidate"));
            let proof = match c.argument_x_forwarding(point, m) {
                Ok(p) => p,
                Err(e) => {
                    let stat = match e {
                        facts::Blocker::Storage => "argument-x-forwarding-blocked-storage",
                        facts::Blocker::Placement => "argument-x-forwarding-blocked-placement",
                        facts::Blocker::Alias => "argument-x-forwarding-blocked-alias",
                        facts::Blocker::IncomingX => "argument-x-forwarding-blocked-incoming-x",
                        facts::Blocker::XRead => "argument-x-forwarding-blocked-x-read",
                        facts::Blocker::XClobber => "argument-x-forwarding-blocked-x-clobber",
                        facts::Blocker::InsertedNz => "argument-x-forwarding-blocked-inserted-nz",
                        facts::Blocker::RemovedNz => "argument-x-forwarding-blocked-removed-nz",
                        facts::Blocker::CallFlags => "argument-x-forwarding-blocked-call-flags",
                        facts::Blocker::EntryFlagsUnknown => "callee-entry-flags-blocked-unknown",
                        facts::Blocker::EntryFlagsLive(flags) => {
                            if flags.n {
                                seen.insert((key, "callee-entry-flags-blocked-live-n"));
                            }
                            if flags.z {
                                seen.insert((key, "callee-entry-flags-blocked-live-z"));
                            }
                            "callee-entry-flags-blocked-live"
                        }
                        facts::Blocker::Gap => "argument-x-forwarding-blocked-gap",
                        facts::Blocker::Call => "argument-x-forwarding-blocked-call",
                        _ => "argument-x-forwarding-blocked-shape",
                    };
                    seen.insert((key, stat));
                    continue;
                }
            };
            let exit = MirExitStateChange {
                flags: MirFlagSet {
                    n: true,
                    z: true,
                    ..MirFlagSet::default()
                },
                ..MirExitStateChange::default()
            };
            if let Some(mut p) = structural_plan(
                r,
                c,
                b.id,
                proof.shape.range(),
                facts::replacement(r, &proof.shape).unwrap(),
                exit,
                "argument-x-forwarding-selected",
                43,
            ) {
                if proof.entry_flags.is_some() {
                    seen.insert((key, "callee-entry-flags-selected"));
                }
                p.witness = Some(MirPostHomeWitness::ArgumentXForwarding(proof));
                return vec![p];
            }
            seen.insert((key, "argument-x-forwarding-blocked-region-proof"));
        }
    }
    vec![]
}
fn has_candidate(p: &MirProgram) -> bool {
    p.routines.iter().any(|r| {
        r.blocks.iter().any(|b| {
            (0..b.ops.len()).any(|i| {
                facts::shape(
                    r,
                    MirSite::Op {
                        block: b.id,
                        op_index: i,
                    },
                )
                .is_ok()
            })
        })
    })
}
fn prepare_summaries(
    p: &MirProgram,
    precise: bool,
) -> (MirKnownCalleeSummaries, BTreeSet<RoutineId>) {
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(p);
    let mut targets = BTreeSet::new();
    if precise {
        for r in &p.routines {
            for b in &r.blocks {
                for i in 0..b.ops.len() {
                    let Ok(s) = facts::shape(
                        r,
                        MirSite::Op {
                            block: b.id,
                            op_index: i,
                        },
                    ) else {
                        continue;
                    };
                    let MirSite::Op { op_index: call, .. } = s.call else {
                        continue;
                    };
                    if b.ops[s.range().end..call].iter().any(|o| {
                        let f = crate::mir6502::analysis::effects::classify_op(o)
                            .machine
                            .flag_writes;
                        f.n && f.z
                    }) {
                        continue;
                    }
                    if let MirOp::Call {
                        target: MirCallTarget::Routine(id),
                        ..
                    } = &b.ops[call]
                    {
                        if !summaries.scalar_entry_ignores_flags(*id) {
                            targets.insert(*id);
                        }
                    }
                }
            }
        }
    }
    (
        if precise {
            summaries.with_precise_entry_flags(p, &targets)
        } else {
            summaries
        },
        targets,
    )
}
#[cfg(test)]
fn apply(
    baseline: &MirProgram,
    m: &MirPhysicalMemory,
    eligible: Option<&BTreeSet<RoutineId>>,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    apply_with_summaries(baseline, m, eligible, &prepare_summaries(baseline, false).0)
}
fn apply_with_summaries(
    baseline: &MirProgram,
    m: &MirPhysicalMemory,
    eligible: Option<&BTreeSet<RoutineId>>,
    summaries: &MirKnownCalleeSummaries,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    let mut p = baseline.clone();
    let mut stats = MirPeepholeStats::default();
    let mut affected = BTreeSet::new();
    for r in &mut p.routines {
        if eligible.is_some_and(|set| !set.contains(&r.id))
            || !r.blocks.iter().any(|b| {
                (0..b.ops.len()).any(|i| {
                    facts::shape(
                        r,
                        MirSite::Op {
                            block: b.id,
                            op_index: i,
                        },
                    )
                    .is_ok()
                })
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
        let rounds = r
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter(|op| {
                matches!(
                    op,
                    MirOp::Load {
                        dst: MirDef::Reg(MirReg::X),
                        src: MirAddr::Direct(_),
                        width: MirWidth::Byte
                    }
                )
            })
            .count()
            + 1;
        let result = MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .run_fixed_point_with_physical_memory(r, summaries, m, |r, c| {
                let plans = discover(r, c, m, &ids, &mut seen);
                for plan in &plans {
                    let positions = ids.get_mut(&plan.block).unwrap();
                    let old = positions[plan.range.clone()].to_vec();
                    let mut new = vec![old[0], usize::MAX];
                    new.extend_from_slice(&old[1..old.len() - 1]);
                    positions.splice(plan.range.clone(), new);
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
    Some((p, stats, affected))
}
fn exits_preserved(
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
        && a.zn() == b.zn()
        && [MirReg::X, MirReg::Y]
            .iter()
            .all(|&r| !a.preserves_register(r) || b.preserves_register(r))
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
    let (summaries, entry_targets) =
        prepare_summaries(&baseline, config.enable_precise_callee_entry_flags);
    let Some(m) = register_copies::physical_memory(&baseline, &baseline, origin) else {
        return Ok(baseline);
    };
    let Some((mut proposed, mut stats, affected)) =
        apply_with_summaries(&baseline, &m, None, &summaries)
    else {
        return Ok(baseline);
    };
    if affected.is_empty() {
        maybe_report_peepholes(&baseline, &stats, config);
        return Ok(baseline);
    }
    let old = address_values::Cost::measure(&baseline, origin);
    let cost = address_values::Cost::measure(&proposed, origin);
    let old_exits = summaries;
    let new_exits = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
    let mut accepted = affected
        .iter()
        .copied()
        .filter(|&id| {
            old.as_ref()
                .zip(cost.as_ref())
                .is_some_and(|(a, b)| b.does_not_grow(a, id))
                && exits_preserved(&old_exits, &new_exits, id)
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
        let replay_summaries = old_exits
            .clone()
            .with_precise_entry_flags(&proposed, &entry_targets);
        let replay = register_copies::physical_memory(&proposed, &baseline, origin)
            .and_then(|m| apply_with_summaries(&baseline, &m, Some(&accepted), &replay_summaries))
            .is_some_and(|(p, _, _)| p == proposed);
        let exits = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
        let good = replay
            && (!config.enable_precise_callee_entry_flags
                || old_exits.entry_inputs_preserved_by(&proposed, &exits))
            && crate::mir6502::verify_program(&proposed, MirPhase::PreEmission).is_ok()
            && baseline
                .routines
                .iter()
                .zip(&proposed.routines)
                .all(|(a, b)| a.frame == b.frame && exits_preserved(&old_exits, &exits, a.id))
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
            "argument-x-forwarding-applied"
        } else if !exits_preserved(&old_exits, &new_exits, id) {
            "argument-x-forwarding-declined-callee-exit"
        } else {
            "argument-x-forwarding-declined-final-proof-or-cost"
        };
        stats.record_many(
            id,
            stat,
            stats.count_for(id, "argument-x-forwarding-selected"),
        );
    }
    if config.enable_precise_callee_entry_flags {
        for r in &baseline.routines {
            let count = stats.count_for(r.id, "callee-entry-flags-selected");
            if count > 0 {
                stats.record_many(
                    r.id,
                    if accepted.contains(&r.id) {
                        "callee-entry-flags-applied"
                    } else {
                        "callee-entry-flags-declined"
                    },
                    count,
                );
            }
        }
    }
    let p = if accepted.is_empty() {
        baseline
    } else {
        proposed
    };
    maybe_report_peepholes(&p, &stats, config);
    Ok(p)
}
#[cfg(test)]
mod tests;
#[cfg(test)]
mod entry_flags_tests;
