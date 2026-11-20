//! Source locals retain their identity and initialization semantics when bound.
use super::*;
use crate::nir::LocalId;
#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;

pub(super) fn eligible(addresses: &ObjectAddressControl<'_>, routine: &MirRoutine, slot: &MirStorageSlot) -> bool {
    slot.purpose != MirStoragePurpose::Own && slot.init.is_none() && ordinary(addresses, routine, slot)
}

pub(super) fn ordinary(addresses: &ObjectAddressControl<'_>, routine: &MirRoutine, slot: &MirStorageSlot) -> bool {
    let MirStorageBase::Local(id) = slot.base else {
        return false;
    };
    matches!(slot.purpose, MirStoragePurpose::Storage | MirStoragePurpose::Own)
        && slot.storage == MirStorageClass::Scalar
        && slot.offset == 0
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

#[cfg(test)]
pub(super) fn verify_bindings(routine: &MirRoutine) -> Vec<MirDiagnostic> {
    verify_bindings_with_sharing(routine, &super::spills::shared_frame_bytes(routine))
}

pub(super) fn verify_bindings_with_sharing(routine: &MirRoutine, shared: &BTreeSet<u8>) -> Vec<MirDiagnostic> {
    let Some(bindings) = &routine.frame.local_homes else {
        return vec![];
    };
    let mut errors = Vec::new();
    let mut fail = |message| errors.push(MirDiagnostic::routine(&routine.name, message));
    let mut ids = BTreeSet::new();
    let occupied: BTreeSet<_> = routine
        .frame
        .resident_bytes()
        .into_iter()
        .filter_map(|(m, s)| matches!(m, MirMem::Param { .. }).then_some(s.0))
        .collect();
    let mut local_users = std::collections::BTreeMap::<u8, Vec<LocalId>>::new();
    let logical = std::cell::LazyCell::new(|| super::logical::LogicalHomes::new(routine));
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
        let Some(start) = binding.home.slot() else {
            continue;
        };
        if slot.offset != 0
            || ((slot.init.is_some() || slot.purpose == MirStoragePurpose::Own) && !matches!(binding.home, MirLocalHome::PersistentZeroPage(_)))
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
            let users = local_users.entry(byte as u8).or_default();
            if occupied.contains(&(byte as u8)) || users.iter().any(|other|
                logical.as_ref().is_none_or(|facts| facts.locals_interfere(binding.local, *other))) {
                fail("live source-object zero-page bindings overlap");
            }
            users.push(binding.local);
            if !routine
                .frame
                .fixed_zero_page
                .contains(&MirFixedZpSlot(byte as u8))
            {
                fail("local zero-page binding is not reserved");
            }
            if routine.frame.zero_page_allocations.iter().any(|a| {
                byte >= u16::from(a.start.0) && byte < u16::from(a.start.0) + u16::from(a.size)
            }) && !shared.contains(&(byte as u8)) {
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
        if !matches!(routine.abi, MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry)
            || !routine.frame.locals.iter().any(|s| {
                s.base == MirStorageBase::Local(binding.local) && ordinary(addresses, routine, s)
                    && ((s.init.is_none() && s.purpose != MirStoragePurpose::Own) || matches!(binding.home, MirLocalHome::PersistentZeroPage(_)))
            })
        {
            errors.push(MirDiagnostic::routine(
                &routine.name,
                "exposed, aliased or nonordinary local cannot change its home",
            ));
        }
        if !matches!(binding.home, MirLocalHome::PersistentZeroPage(_)) && entry_reads
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

/// Entry dependence uses the same logical projection as call-site residence.
pub(super) fn entry_dependencies(routine: &MirRoutine) -> Option<BTreeSet<LocalId>> {
    Some(super::logical::LogicalHomes::new(routine)?.entry_dependencies()?.into_iter()
        .filter_map(|home| match home {
            super::logical::ObjectByte::Local(id, _) => Some(id),
            _ => None,
        }).collect())
}
