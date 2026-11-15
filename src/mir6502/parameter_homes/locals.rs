//! Source locals retain their identity and initialization semantics when bound.
use super::*;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    effects::{MirHomeByte, classify_op, classify_terminator},
    home_liveness::{MirHomeBlockTransfers, MirHomeLiveness, MirHomeTransfer},
};
use crate::nir::LocalId;
#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;

pub(super) fn eligible(addresses: &ObjectAddressControl<'_>, routine: &MirRoutine, slot: &MirStorageSlot) -> bool {
    let MirStorageBase::Local(id) = slot.base else {
        return false;
    };
    slot.purpose == MirStoragePurpose::Storage
        && slot.storage == MirStorageClass::Scalar
        && slot.offset == 0
        && slot.init.is_none()
        && matches!(
            (slot.scalar_width, slot.storage_size),
            (Some(MirWidth::Byte), 1) | (Some(MirWidth::Word), 2)
        )
        && !routine
            .frame
            .locals
            .iter()
            .any(|s| matches!(s.base, MirStorageBase::LocalAlias { target, .. } if target == id))
        && addresses.compiler_controls_address(routine.id, &MirMem::Local { id, offset: 0 })
}

pub(super) fn verify_bindings(routine: &MirRoutine) -> Vec<MirDiagnostic> {
    let Some(bindings) = &routine.frame.local_homes else {
        return vec![];
    };
    let mut errors = Vec::new();
    let mut fail = |message| errors.push(MirDiagnostic::routine(&routine.name, message));
    let mut ids = BTreeSet::new();
    let mut occupied: BTreeSet<_> = routine
        .frame
        .resident_bytes()
        .into_iter()
        .filter_map(|(m, s)| matches!(m, MirMem::Param { .. }).then_some(s.0))
        .collect();
    for binding in bindings {
        if !ids.insert(binding.local) {
            fail("duplicate local binding");
        }
        let Some(slot) = routine
            .frame
            .locals
            .iter()
            .find(|s| s.base == MirStorageBase::Local(binding.local))
        else {
            fail("local binding has no matching ordinary Local declaration");
            continue;
        };
        let MirLocalHome::ZeroPage(start) = binding.home else {
            continue;
        };
        if slot.offset != 0
            || slot.init.is_some()
            || slot.storage != MirStorageClass::Scalar
            || !matches!(
                (slot.scalar_width, slot.storage_size),
                (Some(MirWidth::Byte), 1) | (Some(MirWidth::Word), 2)
            )
            || u16::from(start.0) + slot.storage_size > 256
        {
            fail("local residence requires a complete scalar byte/word within zero page");
            continue;
        }
        for byte in u16::from(start.0)..u16::from(start.0) + slot.storage_size {
            if !occupied.insert(byte as u8) {
                fail("source-object zero-page bindings overlap");
            }
            if !routine
                .frame
                .fixed_zero_page
                .contains(&MirFixedZpSlot(byte as u8))
            {
                fail("local zero-page binding is not reserved");
            }
            if routine.frame.zero_page_allocations.iter().any(|a| {
                byte >= u16::from(a.start.0) && byte < u16::from(a.start.0) + u16::from(a.size)
            }) {
                fail("local zero-page binding overlaps a virtual allocation");
            }
        }
    }
    if routine
        .frame
        .locals
        .iter()
        .any(|s| matches!(s.base, MirStorageBase::Local(id) if !ids.contains(&id)))
    {
        fail("bound routine has a missing local binding");
    }
    errors
}

#[cfg(test)]
pub(super) fn verify_storage(program: &MirProgram, routine: &MirRoutine) -> Vec<MirDiagnostic> {
    verify_storage_with_address_control(program, routine, &ObjectAddressControl::new(program))
}

pub(super) fn verify_storage_with_address_control(program: &MirProgram, routine: &MirRoutine,
    addresses: &ObjectAddressControl<'_>) -> Vec<MirDiagnostic> {
    let Some(bindings) = &routine.frame.local_homes else {
        return vec![];
    };
    let owned = source_zero_page(program, routine);
    let entry_reads = entry_dependencies(routine);
    let mut errors = Vec::new();
    for binding in bindings {
        if binding.home == MirLocalHome::Inline {
            continue;
        }
        if routine.abi != MirRoutineAbi::Action
            || !routine.frame.locals.iter().any(|s| {
                s.base == MirStorageBase::Local(binding.local) && eligible(addresses, routine, s)
            })
        {
            errors.push(MirDiagnostic::routine(
                &routine.name,
                "exposed, aliased or nonordinary local cannot change its home",
            ));
        }
        if entry_reads
            .as_ref()
            .is_none_or(|reads| reads.contains(&binding.local))
        {
            errors.push(MirDiagnostic::routine(
                &routine.name,
                "resident local depends on its previous invocation value",
            ));
        }
        if routine.frame.resident_bytes().iter().any(|(mem, slot)| {
            matches!(mem, MirMem::Local { id, .. } if *id == binding.local)
                && owned.contains(&slot.0)
        }) {
            errors.push(MirDiagnostic::routine(
                &routine.name,
                "local binding overlaps source-owned storage",
            ));
        }
    }
    errors
}

/// Reuse backward CFG liveness with a logical-local role. Spill IDs here are
/// analysis keys only; no source local becomes a machine spill. A physical or
/// invocation write cannot initialize a source object.
pub(super) fn entry_dependencies(routine: &MirRoutine) -> Option<BTreeSet<LocalId>> {
    let cfg = MirCfg::from_routine(routine).ok()?;
    let entry = cfg.entry()?;
    let logical = |mem: &MirMem, delta: u16| match mem {
        MirMem::Local { id, offset } => Some(MirHomeByte::Spill {
            id: MirSpillId(id.0),
            offset: offset.checked_add(delta)?,
        }),
        _ => None,
    };
    let resident = routine.frame.resident_bytes();
    let transfer = |memory: crate::mir6502::analysis::effects::MirMemoryEffects,
                    physical_reads: BTreeSet<MirHomeByte>,
                    defines: bool| {
        let mut t = MirHomeTransfer::default();
        for range in &memory.direct_reads {
            for offset in 0..range.bytes {
                if let Some(home) = logical(&range.base, offset) {
                    t.reads.insert(home);
                }
                let address = match range.base {
                    MirMem::FixedZeroPage(s) => Some(u16::from(s.0).wrapping_add(offset)),
                    MirMem::Absolute(a) => Some(a.wrapping_add(offset)),
                    _ => None,
                };
                for (mem, slot) in &resident {
                    if address == Some(u16::from(slot.0))
                        && let Some(home) = logical(mem, 0)
                    {
                        t.reads.insert(home);
                    }
                }
            }
        }
        for (mem, slot) in &resident {
            if physical_reads.contains(&MirHomeByte::FixedZeroPage(*slot))
                && let Some(home) = logical(mem, 0)
            {
                t.reads.insert(home);
            }
        }
        if defines {
            for range in &memory.direct_writes {
                for offset in 0..range.bytes {
                    if let Some(home) = logical(&range.base, offset) {
                        t.writes.insert(home);
                    }
                }
            }
        }
        t
    };
    let transfers = routine
        .blocks
        .iter()
        .map(|b| {
            let ops = b
                .ops
                .iter()
                .map(|op| {
                    let e = classify_op(op);
                    transfer(
                        e.memory,
                        e.homes
                            .reads
                            .union(&e.addresses.pair_reads)
                            .copied()
                            .collect(),
                        !matches!(op, MirOp::Call { .. }),
                    )
                })
                .collect();
            let e = classify_terminator(&b.terminator);
            (
                b.id,
                MirHomeBlockTransfers {
                    ops,
                    terminator: transfer(e.memory, BTreeSet::new(), true),
                },
            )
        })
        .collect();
    let live = MirHomeLiveness::analyze_transfers(routine, &cfg, transfers);
    Some(
        live.live_in(entry)?
            .iter()
            .filter_map(|home| match home {
                MirHomeByte::Spill { id, .. } => Some(LocalId(id.0)),
                _ => None,
            })
            .collect(),
    )
}
