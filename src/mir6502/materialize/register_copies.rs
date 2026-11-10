//! Diagnostics and rewrites for direct byte copies through the accumulator.
use super::{layout::MaterializeLayout, stats::MirPeepholeStats};
use crate::mir6502::analysis::effects::MirFlagSet;
use crate::mir6502::analysis::known_callees::MirKnownCalleeSummaries;
use crate::mir6502::analysis::machine_values::{MirMachineMemoryMap, MirMachineValue};
use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
use crate::mir6502::analysis::sites::{MirRoutineGeneration, MirSite};
use crate::mir6502::ir::*;
use crate::mir6502::rewrite::context::{MirProof, PostHomeRewriteContext};
use crate::mir6502::rewrite::driver::MirPostHomeRewriteDriver;
use std::collections::BTreeSet;

/// Final transaction: home selection/inlining and all previous savings survive
/// a failed proof, verifier or exact emission-cost comparison.
pub(in crate::mir6502) fn select_costed(
    baseline: MirProgram,
    config: &crate::mir6502::Mir6502Config,
    origin: u16,
    report: bool,
) -> Result<MirProgram, Vec<crate::mir6502::MirDiagnostic>> {
    let callees = MirKnownCalleeSummaries::analyze_for_rewriting(&baseline);
    let mut proposed = baseline.clone();
    let mut affected = BTreeSet::new();
    let mut stats = MirPeepholeStats::default();
    for routine in &mut proposed.routines {
        let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_known_callees(
            routine,
            &callees,
            |r, c| super::peepholes::discover_register_direct_store_folds(r, c, true),
        );
        let Ok(result) = result else {
            return Ok(baseline);
        };
        if result.applied > 0 {
            affected.insert(routine.id);
        }
        super::record_prehome_rewrite_result(routine.id, result, &mut stats);
    }
    if affected.is_empty() {
        return Ok(baseline);
    }
    let new_callees = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
    // Earlier caller rewrites can already rely on a callee's incidental A
    // result. Preserve those published exit values as well as the public ABI.
    let exits_preserved = baseline.routines.iter().all(|r| {
        !callees.accumulator_summary_is_observable(r.id)
            || callees.get(r.id).and_then(|s| s.accumulator())
                == new_callees.get(r.id).and_then(|s| s.accumulator())
    });
    let accepted = exits_preserved
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
        for id in affected {
            stats.record(id, "register-copy-declined-final-proof-or-cost");
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
