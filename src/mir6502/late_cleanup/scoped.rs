//! Contextual eligibility, separate from the existing definition-deadness proof.
use super::*;
use crate::mir6502::{analysis::record_bases::SourceObject, parameter_homes::InvocationHomes};

pub(super) fn private_store(
    ownership: &StorageOwnership<'_>, facts: Option<&InvocationHomes<'_>>,
    owner: RoutineId, op: &MirOp, memory: Option<&MirPhysicalMemory>,
) -> Option<MirMem> {
    if let Some(home) = ownership.private_store(owner, op, memory) { return Some(home); }
    let facts = facts?;
    let memory = memory?;
    let home = ownership.private_store(owner, op, None)?;
    if !matches!(home, MirMem::ZeroPage(_)) { return None; }
    let address = memory.address(owner, &home)?;
    let objects = memory.source_storage()?;
    objects.iter().all(|object| !object.contains(address) || (
        matches!(object.object, SourceObject::Routine { owner: other, .. } if other != owner)
        && facts.certifies_storage(object)
    )).then_some(home)
}

pub(super) fn blocks_read(
    read: &ReadObservation, owner: RoutineId, address: u16,
    memory: &MirPhysicalMemory, facts: Option<&InvocationHomes<'_>>,
) -> bool {
    if read.address.is_some_and(|a| address.wrapping_sub(a) >= read.bytes) { return false; }
    let Some(facts) = facts else { return true; };
    if read.owner == owner { return true; }
    let (Some(start), Some(site), Some(mem)) = (read.address, read.site, &read.mem) else { return true; };
    let delta = address.wrapping_sub(start);
    let byte = match mem {
        MirMem::Param { id, offset } => offset.checked_add(delta).map(|offset| MirMem::Param { id: *id, offset }),
        MirMem::Local { id, offset } => offset.checked_add(delta).map(|offset| MirMem::Local { id: *id, offset }),
        MirMem::FixedZeroPage(s) => u16::from(s.0).checked_add(delta).filter(|a| *a < 256)
            .map(|a| MirMem::FixedZeroPage(MirFixedZpSlot(a as u8))),
        _ => None,
    };
    let Some(base) = byte.and_then(|b| facts.read_object(read.owner, site, &b)) else { return true; };
    !memory.source_storage().is_some_and(|objects| objects.iter().any(|object| {
        object.contains(address) && facts.certifies_storage(object)
            && matches!(&object.object, SourceObject::Routine { owner: other, base: b, .. }
                if *other == read.owner && *b == base)
    }))
}

pub(super) fn trace_rejections(
    program: &MirProgram, ownership: &StorageOwnership<'_>, facts: &InvocationHomes<'_>,
    memory: &MirPhysicalMemory, trace: &Trace,
) {
    if !trace.enabled() { return; }
    for r in &program.routines {
        for b in &r.blocks {
            for (index, op) in b.ops.iter().enumerate() {
                let Some(home @ MirMem::ZeroPage(_)) = ownership.private_store(r.id, op, None) else { continue; };
                if private_store(ownership, Some(facts), r.id, op, Some(memory)).is_some() { continue; }
                trace.event("late-cleanup-store-ineligible", || serde_json::json!({
                    "routine": r.id.0, "block": b.id.0, "op": index, "home": format!("{home:?}"),
                    "address": memory.address(r.id, &home),
                    "reason": "source-overlap",
                    "objects": memory.address(r.id, &home).and_then(|a| memory.source_storage()
                        .map(|objects| objects.iter().filter(|s| s.contains(a)).map(|s| serde_json::json!({
                            "object": format!("{s:?}"), "reason":
                                if matches!(s.object, SourceObject::Routine { owner, .. } if owner == r.id) {
                                    Some("same-routine-source")
                                } else { facts.storage_rejection(s) }
                        })).collect::<Vec<_>>())),
                }));
            }
        }
    }
}
