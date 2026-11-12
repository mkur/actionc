//! Separately costed final family. Bounds never leak into generic retention.
use super::*;
use crate::mir6502::MirRoutineAbi;
use crate::mir6502::analysis::{address_ranges::MirAddressRanges, sites::MirRoutineGeneration};
use crate::mir6502::rewrite::{driver::apply_compound_region, plan::MirCompoundRegionPlan};

/// Cheap admission before cloning source MIR or running numeric analysis.
/// This tests only a target shape; it supplies no semantic range assumption.
pub(in crate::mir6502) fn has_candidate(program: &MirProgram) -> bool {
    program
        .routines
        .iter()
        .filter(|r| r.abi == MirRoutineAbi::Action)
        .any(|r| {
            r.blocks.iter().any(|b| {
                let Some(i) = b.ops.len().checked_sub(9) else {
                    return false;
                };
                i > 0
                    && matches!(b.terminator, MirTerminator::Return)
                    && matches!(
                        b.ops[i - 1],
                        MirOp::Call {
                            target: MirCallTarget::Routine(_),
                            ..
                        }
                    )
                    && record_bases::setup_pair(&b.ops[i..i + 4]).is_some()
                    && record_bases::setup_pair(&b.ops[i + 4..i + 8]).is_some()
                    && matches!(
                        b.ops[i + 8],
                        MirOp::IndirectByteCompound {
                            op: MirBinaryOp::Add,
                            offset: 0,
                            ..
                        }
                    )
            })
        })
}

fn apply_ranges(
    baseline: &MirProgram,
    proofs: &MirProgram,
    map: &MirPhysicalMemory,
    ranges: &MirAddressRanges<'_>,
) -> Option<(MirProgram, BTreeSet<RoutineId>)> {
    let calls = CallWrites::analyze(proofs, map)
        .with_bounded_machine_calls(proofs)
        .with_range_inputs(baseline, ranges);
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(proofs);
    let mut proposed = baseline.clone();
    let mut affected = BTreeSet::new();
    for r in &mut proposed.routines {
        // Only unchanged baseline generations can consume source receipts.
        let selected=r.blocks.iter().find_map(|b| {
            let index=b.ops.len().checked_sub(9)?;
            let MirOp::Call { target:MirCallTarget::Routine(id),.. }=b.ops.get(index.checked_sub(1)?)? else { return None; };
            ranges.homes_for_call(r,*id,MirRoutineGeneration::initial())?;
            let (point,_)=ranges.for_call(r,*id,MirRoutineGeneration::initial())?;
            if point.site!=(MirSite::Op {block:b.id,op_index:index-1}) { return None; }
            let snapshot=crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot::new_with_known_callees(r,MirRoutineGeneration::initial(),&known).ok()?;
            let c=PostHomeRewriteContext::new(&snapshot);
            c.bounded_compound_region(c.point(MirSite::Op {block:b.id,op_index:index}),map,&calls)
        });
        if let Some(proof) = selected {
            let plan = MirCompoundRegionPlan { proof };
            if !apply_compound_region(r, MirRoutineGeneration::initial(), &known, map, &plan) {
                return None;
            }
            affected.insert(r.id);
        }
    }
    Some((proposed, affected))
}

fn profitable(
    baseline: &MirProgram,
    proposed: &MirProgram,
    affected: &BTreeSet<RoutineId>,
    origin: u16,
) -> bool {
    match (
        address_values::Cost::measure(baseline, origin),
        address_values::Cost::measure(proposed, origin),
    ) {
        (Some(old), Some(new)) => {
            new.bytes < old.bytes && affected.iter().all(|id| new.does_not_grow(&old, *id))
        }
        _ => false,
    }
}

pub(in crate::mir6502) fn select_costed(
    baseline: MirProgram,
    ranges: &MirAddressRanges<'_>,
    origin: u16,
    config: &Mir6502Config,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let Some(map) = memory(&baseline, &baseline, origin) else {
        return Ok(baseline);
    };
    let Some((proposed, affected)) = apply_ranges(&baseline, &baseline, &map, ranges) else {
        return Ok(baseline);
    };
    if affected.is_empty() {
        return Ok(baseline);
    }
    let old = MirKnownCalleeSummaries::analyze_for_rewriting(&baseline);
    let new = MirKnownCalleeSummaries::analyze_for_rewriting(&proposed);
    let exits = baseline.routines.iter().all(|r| {
        let (Some(a), Some(b)) = (old.get(r.id), new.get(r.id)) else {
            return old.get(r.id).is_none();
        };
        (!old.accumulator_summary_is_observable(r.id) || a.accumulator() == b.accumulator())
            && (a.zn().is_none() || a.zn() == b.zn())
            && [MirReg::A, MirReg::X, MirReg::Y]
                .into_iter()
                .all(|v| !a.preserves_register(v) || b.preserves_register(v))
            && [0xAA, 0xAC, 0xAE].into_iter().all(|v| {
                !a.preserves_fixed_pair(MirFixedZpSlot(v))
                    || b.preserves_fixed_pair(MirFixedZpSlot(v))
            })
    });
    let replay = memory(&proposed, &baseline, origin)
        .and_then(|m| apply_ranges(&baseline, &proposed, &m, ranges))
        .is_some_and(|(p, a)| p == proposed && a == affected);
    let accepted = exits
        && replay
        && crate::mir6502::verify_program(&proposed, MirPhase::PreEmission).is_ok()
        && baseline
            .routines
            .iter()
            .zip(&proposed.routines)
            .all(|(a, b)| a.frame == b.frame)
        && profitable(&baseline, &proposed, &affected, origin);
    let mut stats = MirPeepholeStats::default();
    for id in affected {
        stats.record(
            id,
            if accepted {
                "pointer-retention-checked-compound"
            } else {
                "pointer-retention-checked-declined-final-proof-or-cost"
            },
        );
    }
    maybe_report_peepholes(if accepted { &proposed } else { &baseline }, &stats, config);
    Ok(if accepted { proposed } else { baseline })
}

#[cfg(test)]
mod qualification;
