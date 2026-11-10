//! Diagnostics and rewrites for direct byte copies through the accumulator.
use super::{layout::MaterializeLayout, stats::MirPeepholeStats};
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::known_callees::MirKnownCalleeSummaries;
use crate::mir6502::analysis::machine_values::{MirMachineMemoryMap, MirMachineValue};
use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
use crate::mir6502::analysis::record_bases::MirPhysicalMemory;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::ir::*;
use crate::mir6502::rewrite::context::{MirExitStateChange, MirProof, PostHomeRewriteContext};
use crate::mir6502::rewrite::driver::MirPostHomeRewriteDriver;
use crate::mir6502::rewrite::plan::MirPostHomeRewritePlan;
use crate::mir6502::rewrite::posthome::structural_plan;
use std::collections::BTreeSet;

/// Final transaction: home selection/inlining and all previous savings survive
/// a failed proof, verifier or exact emission-cost comparison.
pub(in crate::mir6502) fn select_costed(
    baseline: MirProgram,
    config: &crate::mir6502::Mir6502Config,
    origin: u16,
    report: bool,
) -> Result<MirProgram, Vec<crate::mir6502::MirDiagnostic>> {
    if !baseline.routines.iter().any(has_candidate) {
        return Ok(baseline);
    }
    let Some(physical) = physical_memory(&baseline, &baseline, origin) else {
        return Ok(baseline);
    };
    let Some((proposed, mut stats, affected)) = apply(&baseline, origin, &physical) else {
        return Ok(baseline);
    };
    if affected.is_empty() {
        return Ok(baseline);
    }
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(&baseline);
    let new_callees = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
    // Earlier caller rewrites can already rely on a callee's incidental A
    // result. Preserve those published exit values as well as the public ABI.
    let exits_preserved = baseline.routines.iter().all(|r| {
        !callees.accumulator_summary_is_observable(r.id)
            || callees.get(r.id).and_then(|s| s.accumulator())
                == new_callees.get(r.id).and_then(|s| s.accumulator())
    });
    // Deferred data can move when code shrinks. Re-run all proofs from the
    // unchanged baseline using the independently emitted proposed placement.
    let replayed = physical_memory(&proposed, &baseline, origin)
        .and_then(|map| apply(&baseline, origin, &map))
        .is_some_and(|(replay, _, _)| replay == proposed);
    let accepted = exits_preserved
        && replayed
        && crate::mir6502::verify_program(&proposed, MirPhase::PreEmission).is_ok()
        && proposed
            .routines
            .iter()
            .zip(&baseline.routines)
            .all(|(a, b)| a.frame == b.frame)
        && match (
            super::address_values::Cost::measure(&baseline, origin),
            super::address_values::Cost::measure(&proposed, origin),
        ) {
            (Some(old), Some(new)) => {
                new.bytes < old.bytes && affected.iter().all(|id| new.does_not_grow(&old, *id))
            }
            _ => false,
        };
    if !accepted {
        stats = MirPeepholeStats::default();
        for id in affected {
            stats.record(id, "register-copy-declined-final-proof-or-cost");
            if !exits_preserved {
                stats.record(id, "register-copy-declined-callee-exit");
            }
            if !replayed {
                stats.record(id, "register-copy-declined-placement-replay");
            }
        }
    }
    if report {
        super::stats::maybe_report_peepholes(
            if accepted { &proposed } else { &baseline },
            &stats,
            config,
        );
    }
    Ok(if accepted { proposed } else { baseline })
}

fn has_candidate(routine: &MirRoutine) -> bool {
    routine.blocks.iter().any(|b| {
        b.ops.windows(2).any(|pair| {
            matches!(
                &pair[0],
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirValue::Def(MirDef::Reg(MirReg::X | MirReg::Y)),
                    width: MirWidth::Byte
                } | MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(_),
                    width: MirWidth::Byte
                }
            ) && matches!(
                &pair[1],
                MirOp::Store {
                    dst: MirAddr::Direct(_),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte
                }
            )
        })
    })
}

fn physical_memory(
    program: &MirProgram,
    references: &MirProgram,
    origin: u16,
) -> Option<MirPhysicalMemory> {
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
    origin: u16,
    physical: &MirPhysicalMemory,
) -> Option<(MirProgram, MirPeepholeStats, BTreeSet<RoutineId>)> {
    let layout = MaterializeLayout::new(baseline, origin);
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(baseline);
    // Explicit source/ABI reads can expose a separately named scratch alias.
    // Private homes keep the existing reaching-definition alias policy.
    let mut observable_reads = Vec::new();
    for r in &baseline.routines {
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            for read in classify_op(op).memory.direct_reads {
                if matches!(read.base, MirMem::ZeroPage(_) | MirMem::Spill { .. }) {
                    continue;
                }
                observable_reads.push((physical.address(r.id, &read.base), read.bytes));
            }
        }
    }
    let mut proposed = baseline.clone();
    let mut stats = MirPeepholeStats::default();
    let mut affected = BTreeSet::new();
    for routine in &mut proposed.routines {
        if !has_candidate(routine) {
            continue;
        }
        let mut copied = false;
        // Plans can depend on a load that another plan removes. Nonoverlapping
        // ranges alone do not make those machine-state assumptions independent.
        // Apply one plan per snapshot. Every plan strictly removes an operation,
        // so the input operation count plus one bounds the complete fixed point.
        let rounds = routine
            .blocks
            .iter()
            .map(|b| b.ops.len())
            .sum::<usize>()
            .saturating_add(1);
        let result = MirPostHomeRewriteDriver::with_max_rounds(rounds)
            .run_fixed_point_with_memory_map(
                routine,
                &callees,
                |r| {
                    MirMachineMemoryMap::from_routine(r, |mem| {
                        if layout.mem_has_absolute_backing(mem) {
                            None
                        } else {
                            physical.address(r.id, mem)
                        }
                    })
                },
                |r, c| {
                    let mut plans =
                        super::peepholes::discover_register_direct_store_folds(r, c, true);
                    plans.extend(discover_load_stores(r, c, &layout, physical));
                    if !plans.is_empty() {
                        copied = true;
                    }
                    if !copied {
                        return vec![];
                    }
                    plans.extend(
                        super::peepholes::discover_dead_private_scratch_stores(r, c)
                            .into_iter()
                            .filter(|p| {
                                let Some(block) = r.blocks.iter().find(|b| b.id == p.block) else {
                                    return false;
                                };
                                let MirOp::Store {
                                    dst: MirAddr::Direct(home),
                                    ..
                                } = &block.ops[p.range.start]
                                else {
                                    return false;
                                };
                                let Some(address) = physical.address(r.id, home) else {
                                    return false;
                                };
                                !observable_reads.iter().any(|(start, bytes)| {
                                    start.is_none_or(|start| address.wrapping_sub(start) < *bytes)
                                })
                            }),
                    );
                    plans.into_iter().take(1).collect()
                },
            )
            .ok()?;
        if result.applied > 0 {
            affected.insert(routine.id);
        }
        super::record_prehome_rewrite_result(routine.id, result, &mut stats);
    }
    Some((proposed, stats, affected))
}

fn discover_load_stores(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
    physical: &MirPhysicalMemory,
) -> Vec<MirPostHomeRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, pair) in block.ops.windows(2).enumerate() {
            let [
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(src),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(dst),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ] = pair
            else {
                continue;
            };
            if !removable_read(src, layout) || !physical.ordinary(routine.id, src) {
                continue;
            }
            let point = context.point(MirSite::Op {
                block: block.id,
                op_index: index,
            });
            let value = MirMachineValue::DirectMem(src.clone());
            let Some(reg) = [MirReg::A,MirReg::X,MirReg::Y].into_iter().find(|reg|
                matches!(context.register_value_at(*reg,point),MirProof::Proven(ref v) if *v==value)) else { continue };
            let mut exit = MirExitStateChange::default();
            exit.registers.a =
                !matches!(context.accumulator_value_at(point),MirProof::Proven(ref v) if *v==value);
            if !matches!(context.zn_value_at(point),MirProof::Proven(ref v) if *v==value) {
                exit.flags = MirFlagSet {
                    n: true,
                    z: true,
                    ..Default::default()
                };
            }
            if let Some(plan) = structural_plan(
                routine,
                context,
                block.id,
                index..index + 2,
                vec![MirOp::Store {
                    dst: MirAddr::Direct(dst.clone()),
                    src: MirValue::Def(MirDef::Reg(reg)),
                    width: MirWidth::Byte,
                }],
                exit,
                "register-copy-load-store",
                0,
            ) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(super) fn memory_map(routine: &MirRoutine, layout: &MaterializeLayout) -> MirMachineMemoryMap {
    MirMachineMemoryMap::from_routine(routine, |mem| match mem {
        MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0)),
        MirMem::ZeroPage(slot) => routine
            .frame
            .zero_page_allocations
            .iter()
            .find_map(|a| (a.slot == *slot).then_some(u16::from(a.start.0))),
        _ if layout.mem_has_absolute_backing(mem) => None,
        _ => layout.mem_address(routine.id, mem),
    })
}

pub(super) fn removable_read(mem: &MirMem, layout: &MaterializeLayout) -> bool {
    !layout.mem_has_absolute_backing(mem) && layout.mem_allows_pure_read_reordering(mem)
}

/// A single final-program census. Counts are candidates, never additive savings.
pub(super) fn record_candidates(
    program: &MirProgram,
    layout: &MaterializeLayout,
    stats: &mut MirPeepholeStats,
) {
    let callees = MirKnownCalleeSummaries::analyze(program);
    for routine in &program.routines {
        let map = memory_map(routine, layout);
        let Ok(snapshot) = PostHomeAnalysisSnapshot::new_with_known_callees_and_memory_map(
            routine,
            MirRoutineGeneration::initial(),
            &callees,
            &map,
        ) else {
            continue;
        };
        let context = PostHomeRewriteContext::new(&snapshot);
        for block in &routine.blocks {
            for (index, pair) in block.ops.windows(2).enumerate() {
                if !matches!(
                    &pair[1],
                    MirOp::Store {
                        dst: MirAddr::Direct(_),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    }
                ) {
                    continue;
                }
                let point = context.point(MirSite::Op {
                    block: block.id,
                    op_index: index,
                });
                let (kind, available) = match &pair[0] {
                    MirOp::Move {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirValue::Def(MirDef::Reg(reg @ (MirReg::X | MirReg::Y))),
                        width: MirWidth::Byte,
                    } => ("register-copy-transfer-candidate", Some(*reg)),
                    MirOp::Load {
                        dst: MirDef::Reg(MirReg::A),
                        src: MirAddr::Direct(mem),
                        width: MirWidth::Byte,
                    } => {
                        if !removable_read(mem, layout) {
                            stats.record(routine.id, "register-copy-blocked-observable-read");
                            continue;
                        }
                        let value = MirMachineValue::DirectMem(mem.clone());
                        let available = [MirReg::A, MirReg::X, MirReg::Y].into_iter().find(|reg|
                            matches!(context.register_value_at(*reg, point), MirProof::Proven(ref v) if *v == value));
                        ("register-copy-load-candidate", available)
                    }
                    _ => continue,
                };
                stats.record(routine.id, kind);
                stats.record_site(
                    routine.id,
                    kind,
                    format!(
                        "block=b{} op=#{} available={available:?}",
                        block.id.0, index
                    ),
                );
                if available.is_none() {
                    stats.record(routine.id, "register-copy-blocked-value");
                    continue;
                }
                let end = context.point(MirSite::Op {
                    block: block.id,
                    op_index: index + 1,
                });
                if !context.register_dead_after(MirReg::A, end).is_proven() {
                    stats.record(routine.id, "register-copy-live-accumulator");
                }
                if !context
                    .flags_dead_after(
                        MirFlagSet {
                            n: true,
                            z: true,
                            ..Default::default()
                        },
                        end,
                    )
                    .is_proven()
                {
                    stats.record(routine.id, "register-copy-live-nz");
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "register_copies_tests.rs"]
mod tests;
