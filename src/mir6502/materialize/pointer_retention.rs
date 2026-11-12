//! Costed complete-base retention after all earlier home/placement decisions.
use super::*;
mod promotion;
pub(in crate::mir6502) mod checked_ranges;
use crate::mir6502::MirRoutine;
use crate::mir6502::analysis::effects::{MirFlagSet, MirHomeByte};
use crate::mir6502::analysis::record_bases::{
    self, MirPhysicalMemory,
    retention::{self, CallWrites},
};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::rewrite::context::MirExitStateChange;
use crate::mir6502::rewrite::context::{MirProof, PostHomeRewriteContext};
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
    let mut memory = summary.physical_memory;
    // Includes typed routines, machine payloads and embedded runtime storage
    // initializers. No optimizer lookup by a source/runtime display name.
    memory.protect_execution_ranges(
        summary
            .source_ranges
            .iter()
            .map(|r| (r.start, r.end))
            .collect(),
    );
    Some(memory)
}

fn has_setup(r: &MirRoutine) -> bool {
    r.blocks.iter().flat_map(|b| &b.ops).any(|op| {
        matches!(
            op,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAA | 0xAC | 0xAE))),
                ..
            }
        )
    })
}

#[cfg(test)]
fn apply(
    baseline: &MirProgram,
    proofs: &MirProgram,
    map: &MirPhysicalMemory,
    cfg: bool,
    promote: bool,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    apply_with_bounded_calls(baseline, proofs, map, cfg, promote, false)
}

fn apply_with_bounded_calls(
    baseline: &MirProgram,
    proofs: &MirProgram,
    map: &MirPhysicalMemory,
    cfg: bool,
    promote: bool,
    bounded_calls: bool,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    let calls = CallWrites::analyze(proofs, map);
    let calls = if bounded_calls && !promote {
        calls.with_bounded_machine_calls(proofs)
    } else {
        calls
    };
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(proofs);
    let mut proposed = baseline.clone();
    let mut stats = MirPeepholeStats::default();
    let mut affected = BTreeSet::new();
    for r in &mut proposed.routines {
        if !has_setup(r) {
            continue;
        }
        if promote {
            if let Some(selected) = promotion::try_promote(baseline, r, map, &calls, &known) {
                *r = selected;
                affected.insert(r.id);
                stats.record(r.id, "pointer-retention-promoted-home");
            }
            continue;
        }
        if cfg {
            let mut generation = crate::mir6502::analysis::sites::MirRoutineGeneration::initial();
            loop {
                let snapshot=crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot::new_with_known_callees(r,generation,&known).ok()?;
                let context = PostHomeRewriteContext::new(&snapshot);
                let Some(facts) = retention::ResidentFacts::analyze(r, map, &calls) else {
                    break;
                };
                let plan = indexed_store_plan(r, &context, &facts, map, &calls);
                drop(facts);
                drop(snapshot);
                let Some(plan) = plan else {
                    break;
                };
                if !plan.apply(r, generation) {
                    return None;
                }
                generation = generation.next();
                affected.insert(r.id);
                stats.record(r.id, "pointer-retention-indexed-store");
            }
        }
        if cfg && let Some(seeded) = seed_loop(r, map, &calls, &known) {
            *r = seeded;
            affected.insert(r.id);
            stats.record(r.id, "pointer-retention-entry-setup");
        }
        let rounds = r.blocks.iter().map(|b| b.ops.len()).sum::<usize>() + 1;
        let mut retained = affected.contains(&r.id);
        let result = MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .run_fixed_point_with_known_callees(r, &known, |r, c| {
                let facts = cfg
                    .then(|| retention::ResidentFacts::analyze(r, map, &calls))
                    .flatten();
                if let Some(facts) = &facts {
                    for b in &r.blocks {
                        for index in 0..b.ops.len() {
                            let site = MirSite::Op {
                                block: b.id,
                                op_index: index,
                            };
                            let Some(setup) =
                                facts.redundant_setup(r, c.generation(), site, map, &calls)
                            else {
                                continue;
                            };
                            if setup.proof.site != site || setup.proof.generation != c.generation()
                            {
                                continue;
                            }
                            let end = c.point(MirSite::Op {
                                block: b.id,
                                op_index: index + setup.len - 1,
                            });
                            let replacement = if matches!(
                                c.register_dead_after(MirReg::A, end),
                                MirProof::Proven(())
                            ) && matches!(
                                c.flags_dead_after(
                                    MirFlagSet {
                                        n: true,
                                        z: true,
                                        ..Default::default()
                                    },
                                    end
                                ),
                                MirProof::Proven(())
                            ) {
                                vec![]
                            } else {
                                setup.loads
                            };
                            let old = estimated_6502_cost(&b.ops[index..index + setup.len]);
                            let new = estimated_6502_cost(&replacement);
                            retained = true;
                            return vec![MirPostHomeRewritePlan {
                                witness: None,
                                generation: c.generation(),
                                block: b.id,
                                range: index..index + setup.len,
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
                }
                for b in &r.blocks {
                    for (index, window) in b.ops.windows(4).enumerate() {
                        if record_bases::setup_pair(window).is_none() {
                            continue;
                        }
                        let site = MirSite::Op {
                            block: b.id,
                            op_index: index,
                        };
                        let proof = if let Some(facts) = &facts {
                            facts.prove(r, c.generation(), site, map, &calls)
                        } else {
                            retention::prove(r, c.generation(), site, map, &calls)
                        };
                        let Some(proof) = proof else {
                            stats.record(r.id, retention::blocker(r, site, map, &calls));
                            continue;
                        };
                        let replay = if let Some(facts) = &facts {
                            facts.prove(r, c.generation(), site, map, &calls)
                        } else {
                            retention::prove(r, c.generation(), site, map, &calls)
                        };
                        if replay.as_ref() != Some(&proof) {
                            continue;
                        }
                        // The complete-pair equality proof authorizes these removed
                        // home writes. Keep the exact high load to preserve A/N/Z.
                        let end = c.point(MirSite::Op {
                            block: b.id,
                            op_index: index + 3,
                        });
                        let replacement = if cfg
                            && matches!(c.register_dead_after(MirReg::A, end), MirProof::Proven(()))
                            && matches!(
                                c.flags_dead_after(
                                    MirFlagSet {
                                        n: true,
                                        z: true,
                                        ..Default::default()
                                    },
                                    end
                                ),
                                MirProof::Proven(())
                            ) {
                            vec![]
                        } else {
                            vec![window[2].clone()]
                        };
                        let old = estimated_6502_cost(window);
                        let new = estimated_6502_cost(&replacement);
                        retained = true;
                        return vec![MirPostHomeRewritePlan {
                            witness: None,
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
                if retained && let Some(plan) = result_reload_plan(r, c) {
                    return vec![plan];
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

fn result_reload_plan(
    r: &MirRoutine,
    c: &PostHomeRewriteContext<'_, '_>,
) -> Option<MirPostHomeRewritePlan> {
    for b in &r.blocks {
        for (index, op) in b.ops.iter().enumerate() {
            let MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem @ MirMem::FixedZeroPage(MirFixedZpSlot(0xA0..=0xA1))),
                width: MirWidth::Byte,
            } = op
            else {
                continue;
            };
            let point = c.point(MirSite::Op {
                block: b.id,
                op_index: index,
            });
            if !matches!(c.accumulator_value_at(point),MirProof::Proven(ref value) if *value==crate::mir6502::analysis::machine_values::MirMachineValue::DirectMem(mem.clone()))
            {
                continue;
            }
            // Typed compiler ABI result slots are RAM. Raw absolute/named
            // readbacks never match; N/Z changes need the normal deadness proof.
            if let Some(plan) = crate::mir6502::rewrite::posthome::structural_plan(
                r,
                c,
                b.id,
                index..index + 1,
                vec![],
                MirExitStateChange {
                    flags: MirFlagSet {
                        n: true,
                        z: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                "pointer-retention-result-reload",
                0,
            ) {
                return Some(plan);
            }
        }
    }
    None
}

fn seed_loop(
    r: &MirRoutine,
    map: &MirPhysicalMemory,
    calls: &CallWrites,
    known: &MirKnownCalleeSummaries,
) -> Option<MirRoutine> {
    use crate::mir6502::analysis::{
        posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration,
    };
    let snapshot =
        PostHomeAnalysisSnapshot::new_with_known_callees(r, MirRoutineGeneration::initial(), known)
            .ok()?;
    let c = PostHomeRewriteContext::new(&snapshot);
    for region in retention::entered_loops(r, map, calls) {
        let b = r.blocks.iter().find(|b| b.id == region.body)?;
        let Some(window) = b.ops.get(..4) else {
            continue;
        };
        let Some(pair) = record_bases::setup_pair(window) else {
            continue;
        };
        let end = c.point(MirSite::Op {
            block: b.id,
            op_index: 3,
        });
        let entry = c.point(MirSite::Terminator {
            block: region.preheader,
        });
        if ![entry, end].into_iter().all(|point| {
            matches!(
                c.register_dead_after(MirReg::A, point),
                MirProof::Proven(())
            ) && matches!(
                c.flags_dead_after(
                    MirFlagSet {
                        n: true,
                        z: true,
                        ..Default::default()
                    },
                    point
                ),
                MirProof::Proven(())
            )
        }) {
            continue;
        }
        if !matches!(
            c.pointer_pair_dead_after(
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                    lo: MirFixedZpSlot(pair)
                }),
                entry
            ),
            MirProof::Proven(())
        ) {
            continue;
        }
        let mut trial = r.clone();
        trial
            .blocks
            .iter_mut()
            .find(|b| b.id == region.preheader)?
            .ops
            .extend_from_slice(window);
        let facts = retention::ResidentFacts::analyze(&trial, map, calls)?;
        if facts
            .prove(
                &trial,
                MirRoutineGeneration::initial(),
                MirSite::Op {
                    block: b.id,
                    op_index: 0,
                },
                map,
                calls,
            )
            .is_some()
        {
            return Some(trial);
        }
    }
    None
}

/// A representation change: old pair = base+index, new pair = base and Y=index.
/// Generic removed-definition checks cannot certify that effective-address
/// equivalence. This proof owns the exact window and all changed exit state;
/// the whole transaction independently replays it against final placement.
struct IndexedStorePlan {
    generation: crate::mir6502::analysis::sites::MirRoutineGeneration,
    block: MirBlockId,
    index: usize,
    original: Vec<MirOp>,
    replacement: Vec<MirOp>,
}
impl IndexedStorePlan {
    fn apply(
        self,
        r: &mut MirRoutine,
        generation: crate::mir6502::analysis::sites::MirRoutineGeneration,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        let Some(b) = r.blocks.iter_mut().find(|b| b.id == self.block) else {
            return false;
        };
        let end = self.index + self.original.len();
        if b.ops.get(self.index..end) != Some(self.original.as_slice()) {
            return false;
        }
        let replacement_len = self.replacement.len();
        let removed_len = self.original.len();
        b.ops.splice(self.index..end, self.replacement);
        if let MirTerminator::Branch {
            cond: MirCond::FusedCompare { producer, .. },
            ..
        } = &mut b.terminator
        {
            if producer.block == b.id && producer.op_index >= end {
                producer.op_index = producer.op_index - removed_len + replacement_len;
            }
        }
        true
    }
}

fn indexed_store_plan(
    r: &MirRoutine,
    c: &PostHomeRewriteContext<'_, '_>,
    facts: &retention::ResidentFacts<'_>,
    map: &MirPhysicalMemory,
    calls: &CallWrites,
) -> Option<IndexedStorePlan> {
    for b in &r.blocks {
        for (index, window) in b.ops.windows(3).enumerate() {
            let [
                MirOp::MaterializeIndexedAddress {
                    consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo }),
                    base,
                    index: MirValue::Def(MirDef::Reg(MirReg::A)),
                    scale: 1,
                },
                reload,
                MirOp::StoreIndirect {
                    consumer,
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    offset: 0,
                },
            ] = window
            else {
                continue;
            };
            if *consumer != MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo: *lo })
                || !matches!(
                    reload,
                    MirOp::Load {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirAddr::Direct(_),
                        width: MirWidth::Byte
                    }
                )
            {
                continue;
            }
            let site = MirSite::Op {
                block: b.id,
                op_index: index,
            };
            if !facts.matches_pair(r, site, lo.0, base, map, calls) {
                continue;
            }
            let MirOp::Load {
                src: MirAddr::Direct(source),
                ..
            } = reload
            else {
                continue;
            };
            if map
                .address(r.id, source)
                .is_none_or(|a| a == u16::from(lo.0) || a == u16::from(lo.0) + 1)
            {
                continue;
            }
            let end = c.point(MirSite::Op {
                block: b.id,
                op_index: index + 2,
            });
            let mut exit = MirExitStateChange::default();
            exit.registers.y = true;
            exit.flags = MirFlagSet::all();
            exit.homes.insert(MirHomeByte::FixedZeroPage(*lo));
            exit.homes
                .insert(MirHomeByte::FixedZeroPage(MirFixedZpSlot(lo.0 + 1)));
            if !matches!(
                c.exit_state_change_is_unobservable(&exit, end),
                MirProof::Proven(())
            ) {
                continue;
            }
            let mut start = index;
            let index_in_y = if index > 0
                && let MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } = &b.ops[index - 1]
            {
                start = index - 1;
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::Y),
                    src: MirAddr::Direct(mem.clone()),
                    width: MirWidth::Byte,
                }
            } else {
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::Y),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                }
            };
            let replacement = vec![
                index_in_y,
                reload.clone(),
                MirOp::Store {
                    dst: MirAddr::FixedIndirectIndexedY { zp: *lo },
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ];
            return Some(IndexedStorePlan {
                generation: c.generation(),
                block: b.id,
                index: start,
                original: b.ops[start..index + 3].to_vec(),
                replacement,
            });
        }
    }
    None
}

pub(in crate::mir6502) fn select_costed(
    baseline: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    select(baseline, config, origin, report, false)
}

pub(in crate::mir6502) fn select_promotion_costed(
    baseline: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    select(baseline, config, origin, report, true)
}

fn select(
    baseline: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    report: bool,
    promote: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    if !baseline.routines.iter().any(has_setup) {
        return Ok(baseline);
    }
    let Some(map) = memory(&baseline, &baseline, origin) else {
        return Ok(baseline);
    };
    let bounded_calls =
        config.enable_pointer_call_retention && config.enable_bounded_call_pointer_retention;
    let Some((proposed, mut stats, affected)) = apply_with_bounded_calls(
        &baseline,
        &baseline,
        &map,
        config.enable_pointer_loop_retention,
        promote,
        bounded_calls,
    ) else {
        return Ok(baseline);
    };
    if affected.is_empty() {
        if report {
            maybe_report_peepholes(&baseline, &stats, config);
        }
        return Ok(baseline);
    }
    let old_callees = MirKnownCalleeSummaries::analyze_for_rewriting(&baseline);
    let new_callees = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
    let exits = baseline.routines.iter().all(|r| {
        !old_callees.accumulator_summary_is_observable(r.id)
            || old_callees.get(r.id).and_then(|s| s.accumulator())
                == new_callees.get(r.id).and_then(|s| s.accumulator())
    });
    let replay = !affected.is_empty()
        && memory(&proposed, &baseline, origin)
            .and_then(|map| {
                apply_with_bounded_calls(
                    &baseline,
                    &proposed,
                    &map,
                    config.enable_pointer_loop_retention,
                    promote,
                    bounded_calls,
                )
            })
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
