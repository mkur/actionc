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

#[cfg(test)]
pub(in crate::mir6502) fn verify(routine: &MirRoutine, calls: Option<&mut ResidenceCalls<'_>>) -> Vec<MirDiagnostic> {
    verify_with_address_control(routine, calls, None)
}

pub(in crate::mir6502) fn verify_with_address_control(routine: &MirRoutine,
    mut calls: Option<&mut ResidenceCalls<'_>>, addresses: Option<&ObjectAddressControl<'_>>,
) -> Vec<MirDiagnostic> {
    let fixed = calls.as_ref().map(|facts| facts.fixed_storage());
    let resident = routine.frame.resident_bytes();
    let owned: BTreeSet<_> = resident.iter().map(|(_, slot)| MirHomeByte::FixedZeroPage(*slot)).collect();
    if owned.is_empty() { return vec![]; }
    let private: BTreeSet<_> = resident.iter().filter(|(mem, _)| addresses.is_some_and(|facts|
        facts.owns_routine(routine) && facts.compiler_controls_address(routine.id, mem)))
        .map(|(_,slot)| MirHomeByte::FixedZeroPage(*slot)).collect();
    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return vec![MirDiagnostic::routine(&routine.name, "source-object residence requires a closed CFG")];
    };
    let liveness = super::logical::analyze(routine, &cfg, |mem, offset| {
        match routine.frame.object_lane(mem, offset) {
            Some(MirParamLane::ZeroPage(slot)) => Some(MirHomeByte::FixedZeroPage(slot)),
            _ => None,
        }
    });
    let mut clobbers = Vec::new();
    let terminal = super::handoff::terminal_site(routine);
    for block in &routine.blocks {
        for (op_index, op) in block.ops.iter().enumerate() {
            let site = MirSite::Op { block: block.id, op_index };
            if let MirOp::SArgsCapture { .. } = op {
                let Some(capture) = calls.as_ref().and_then(|facts| facts.sargs_capture(routine, op)) else {
                    return vec![MirDiagnostic::routine(&routine.name, "unqualified SArgs capture for resident homes")];
                };
                // The capture initializes every parameter. Other resident objects
                // must be disjoint from its scratch and incoming argument bytes.
                if resident.iter().any(|(mem, slot)| !matches!(mem, MirMem::Param { .. })
                    && (capture.scratch.contains(&MirMem::FixedZeroPage(*slot))
                        || capture.inputs.contains(&MirArgHome::FixedZeroPage(*slot)))) {
                    return vec![MirDiagnostic::routine(&routine.name, "resident local overlaps SArgs capture")];
                }
                continue;
            }
            let consumes = terminal == Some(site);
            let call_dead = matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. }) && owned.iter()
                .all(|home| matches!(liveness.live_after(*home, site), Ok(false)));
            let invocation = if matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. }) && !consumes {
                let Some(facts) = calls.as_mut() else {
                    return vec![MirDiagnostic::routine(&routine.name, "missing residence call facts")];
                };
                if call_dead {
                    match facts.inputs(op) {
                        Ok(inputs) if resident.iter().any(|(_, slot)| inputs.contains(&slot.0)) =>
                            return vec![MirDiagnostic::routine(&routine.name, "resident home overlaps a callee entry input")],
                        Err(reason) => return vec![MirDiagnostic::routine(&routine.name, reason)],
                        _ => {}
                    }
                    None
                } else {
                    match facts.home_writes_at(routine, site, op) {
                        Ok(writes) => Some(writes),
                        Err(reason) => return vec![MirDiagnostic::routine(&routine.name, reason)],
                    }
                }
            } else { None };
            if matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) && !consumes && !call_dead && invocation.is_none() {
                return vec![MirDiagnostic::routine(
                    &routine.name,
                    "source-object residence acquired an unsupported expanded barrier",
                )];
            }
            let e = classify_op(op);
            let ordinary_pointee = !private.is_empty() && super::super::analysis::effects::writes_ordinary_pointee(op);
            if (e.memory.opaque || (e.memory.indirect_writes && !ordinary_pointee)) && !consumes && !call_dead && invocation.is_none() {
                return vec![MirDiagnostic::routine(
                    &routine.name,
                    "source-object residence acquired unsupported expanded memory effects",
                )];
            }
            let mut writes: BTreeSet<_> = e
                .homes
                .writes
                .union(&e.addresses.pair_writes)
                .copied()
                .collect();
            if ordinary_pointee { writes.extend(owned.difference(&private)); }
            for range in &e.memory.direct_writes {
                for offset in 0..range.bytes {
                    if let Some(slot) = physical_slot(&range.base, offset) {
                        writes.insert(MirHomeByte::FixedZeroPage(slot));
                    }
                }
            }
            if consumes || call_dead {
                // Inputs are read before invocation effects. Unknown writes
                // need no footprint when every protected value dies here.
                writes.extend(&owned);
            }
            if let Some(invocation) = invocation {
                writes.extend(owned.iter().filter(|home| match home {
                    MirHomeByte::FixedZeroPage(slot) => {
                        let fixed = fixed.as_deref().expect("invocation has call facts");
                        !(if private.contains(home) { invocation.preserves_private_fixed_byte(*slot, fixed) }
                            else { invocation.preserves_fixed_byte(*slot, fixed) })
                    },
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
    }
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
