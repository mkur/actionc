//! Project logical parameter lifetimes onto their selected bytes. A physical
//! scratch/result write is a possible clobber, never a definition of the
//! parameter. This distinction prevents a clobber from killing its own proof.
use super::*;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    effects::{MirHomeByte, MirMemoryEffects, classify_op, classify_terminator},
    home_liveness::{MirHomeBlockTransfers, MirHomeLiveness, MirHomeTransfer},
    sites::MirSite,
};
use std::collections::BTreeMap;

#[cfg(test)]
pub(in crate::mir6502) fn verify(routine: &MirRoutine, calls: Option<&mut ResidenceCalls<'_>>) -> Vec<MirDiagnostic> {
    verify_with_address_control(routine, calls, None)
}

pub(in crate::mir6502) fn verify_with_address_control(routine: &MirRoutine,
    mut calls: Option<&mut ResidenceCalls<'_>>, addresses: Option<&ObjectAddressControl<'_>>,
) -> Vec<MirDiagnostic> {
    let resident = routine.frame.resident_bytes();
    let owned: BTreeSet<_> = resident.iter().map(|(_, slot)| MirHomeByte::FixedZeroPage(*slot)).collect();
    if owned.is_empty() { return vec![]; }
    let private = addresses.is_some_and(|facts| facts.owns_routine(routine)
        && resident.iter().all(|(mem, _)| facts.compiler_controls_address(routine.id, mem)));
    let transfer = |memory: &MirMemoryEffects, reads: &BTreeSet<MirHomeByte>, defines| {
        project(routine, &owned, memory, reads, defines)
    };
    let mut transfers = BTreeMap::new();
    let mut clobbers = Vec::new();
    let terminal = super::handoff::terminal_site(routine);
    for block in &routine.blocks {
        let mut ops = Vec::new();
        for (op_index, op) in block.ops.iter().enumerate() {
            let site = MirSite::Op { block: block.id, op_index };
            let consumes = terminal == Some(site);
            let invocation = if matches!(op, MirOp::Call { .. }) && !consumes {
                match calls.as_mut().ok_or("missing residence call facts").and_then(|c| c.writes(op)) {
                    Ok(writes) => Some(writes),
                    Err(reason) => return vec![MirDiagnostic::routine(&routine.name, reason)],
                }
            } else { None };
            if matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::SArgsCapture { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) && !consumes && invocation.is_none() {
                return vec![MirDiagnostic::routine(
                    &routine.name,
                    "source-object residence acquired an unsupported expanded barrier",
                )];
            }
            let e = classify_op(op);
            let ordinary_pointee = private && super::super::analysis::effects::writes_ordinary_pointee(op);
            if (e.memory.opaque || (e.memory.indirect_writes && !ordinary_pointee)) && !consumes && invocation.is_none() {
                return vec![MirDiagnostic::routine(
                    &routine.name,
                    "source-object residence acquired unsupported expanded memory effects",
                )];
            }
            let reads = e
                .homes
                .reads
                .union(&e.addresses.pair_reads)
                .copied()
                .collect();
            // Callee effects cannot define caller source objects, even if a
            // callee-local ID happens to equal a caller's parameter ID.
            ops.push(transfer(&e.memory, &reads, !matches!(op, MirOp::Call { .. })));
            let mut writes: BTreeSet<_> = e
                .homes
                .writes
                .union(&e.addresses.pair_writes)
                .copied()
                .collect();
            for range in &e.memory.direct_writes {
                for offset in 0..range.bytes {
                    if let Some(slot) = physical_slot(&range.base, offset) {
                        writes.insert(MirHomeByte::FixedZeroPage(slot));
                    }
                }
            }
            if consumes {
                // Inputs are read before invocation effects. Unknown writes
                // need no footprint when every protected value dies here.
                writes.extend(&owned);
            }
            if let Some(invocation) = invocation {
                writes.extend(owned.iter().filter(|home| match home {
                    MirHomeByte::FixedZeroPage(slot) => !invocation.preserves_fixed_byte(*slot),
                    _ => true,
                }));
            }
            for home in writes.intersection(&owned) {
                clobbers.push((
                    MirSite::Op {
                        block: block.id,
                        op_index,
                    },
                    *home,
                ));
            }
        }
        let e = classify_terminator(&block.terminator);
        transfers.insert(
            block.id,
            MirHomeBlockTransfers {
                ops,
                terminator: transfer(&e.memory, &BTreeSet::new(), true),
            },
        );
    }
    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return vec![MirDiagnostic::routine(
            &routine.name,
            "source-object residence requires a closed CFG",
        )];
    };
    let liveness = MirHomeLiveness::analyze_transfers(routine, &cfg, transfers);
    clobbers
        .into_iter()
        .filter_map(|(site, home)| {
            (!matches!(liveness.live_after(home, site), Ok(false))).then(|| {
                MirDiagnostic::routine(
                    &routine.name,
                    format!("physical write clobbers live source-object home {home:?} at {site:?}"),
                )
            })
        })
        .collect()
}

/// Shared logical definition projection. Physical writes never initialize a
/// source object; physical reads still contribute its entry dependence.
fn project(routine: &MirRoutine, owned: &BTreeSet<MirHomeByte>,
    memory: &MirMemoryEffects, extra_reads: &BTreeSet<MirHomeByte>, defines: bool,
) -> MirHomeTransfer {
    super::logical::project(routine, memory, extra_reads, defines, &|mem, offset| {
        match routine.frame.object_lane(mem, offset) {
            Some(MirParamLane::ZeroPage(slot)) if owned.contains(&MirHomeByte::FixedZeroPage(slot)) =>
                Some(MirHomeByte::FixedZeroPage(slot)),
            _ => None,
        }
    })
}

pub(super) fn entry_dependencies(routine: &MirRoutine) -> Option<BTreeSet<MirHomeByte>> {
    let cfg = MirCfg::from_routine(routine).ok()?;
    let entry = cfg.entry()?;
    let owned = routine.frame.resident_bytes().iter()
        .map(|(_, s)| MirHomeByte::FixedZeroPage(*s)).collect();
    let transfers = routine.blocks.iter().map(|b| {
        let ops = b.ops.iter().map(|op| {
            let e = classify_op(op);
            project(routine, &owned, &e.memory,
                &e.homes.reads.union(&e.addresses.pair_reads).copied().collect(),
                !matches!(op, MirOp::Call { .. }))
        }).collect();
        let e = classify_terminator(&b.terminator);
        (b.id, MirHomeBlockTransfers { ops,
            terminator: project(routine, &owned, &e.memory, &BTreeSet::new(), true) })
    }).collect();
    let live = MirHomeLiveness::analyze_transfers(routine, &cfg, transfers);
    Some(live.live_in(entry)?.iter().collect())
}

use super::logical::physical_slot;
