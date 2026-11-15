//! Final-layout obligations for the native pointee exception. Existing residence
//! rules still govern scratch and ABI traffic; numeric source observations must
//! not silently disappear when an inline object moves to a resident home.
use super::*;
use crate::mir6502::analysis::{
    effects::{classify_op, classify_terminator, writes_ordinary_pointee},
    known_callees::machine_storage_observations,
    record_bases::{MirPhysicalMemory, SourceBinding, SourceObject, SourceStorage},
};

fn needs_proof(r: &MirRoutine) -> bool {
    !r.frame.resident_bytes().is_empty()
        && r.blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(writes_ordinary_pointee)
}

fn records<'a>(
    p: &MirProgram,
    m: &'a MirPhysicalMemory,
) -> Result<Vec<&'a SourceStorage>, Vec<MirDiagnostic>> {
    if !p.routines.iter().any(needs_proof) {
        return Ok(vec![]);
    }
    let inventory = m
        .source_storage()
        .ok_or_else(|| error("missing source-storage inventory"))?;
    let mut result = Vec::new();
    for r in p.routines.iter().filter(|r| needs_proof(r)) {
        for (mem, _) in r.frame.resident_bytes() {
            let base = match mem {
                MirMem::Param { id, .. } => MirStorageBase::Param(id),
                MirMem::Local { id, .. } => MirStorageBase::Local(id),
                _ => return Err(error("invalid resident object")),
            };
            let record = inventory
                .iter()
                .find(|s| {
                    matches!(&s.object,
                SourceObject::Routine { owner, base: b, .. } if *owner == r.id && *b == base)
                        && s.offset == 0
                })
                .ok_or_else(|| error("missing resident placement"))?;
            if !result.contains(&record) {
                result.push(record);
            }
        }
    }
    Ok(result)
}

fn error(reason: &str) -> Vec<MirDiagnostic> {
    vec![MirDiagnostic {
        routine: None,
        block: None,
        message: format!("source-object physical observation: {reason}"),
    }]
}

pub(in crate::mir6502) fn verify(
    p: &MirProgram,
    memory: &MirPhysicalMemory,
) -> Result<(), Vec<MirDiagnostic>> {
    let objects = records(p, memory)?;
    if objects.is_empty() {
        return Ok(());
    }
    let observations =
        numeric_observations(p).ok_or_else(|| error("unresolved explicit operand"))?;
    for object in objects {
        check(object, memory, &observations)?;
    }
    Ok(())
}

pub(super) fn verify_relocation(
    old: &super::super::preparation::Executable,
    new: &super::super::preparation::Executable,
    references: &[&MirProgram],
) -> Result<(), Vec<MirDiagnostic>> {
    let objects = records(&new.program, &new.memory)?;
    if objects.is_empty() {
        return Ok(());
    }
    let sources: Vec<_> = references
        .iter()
        .copied()
        .chain([&old.program, &new.program])
        .collect();
    let addresses: Vec<_> = sources
        .iter()
        .map(|p| ObjectAddressControl::new(p))
        .collect();
    let mut observations = Vec::new();
    for p in sources {
        observations.extend(
            numeric_observations(p).ok_or_else(|| error("unresolved retained explicit operand"))?,
        );
    }
    for object in objects {
        let previous = old
            .memory
            .source_storage()
            .and_then(|s| {
                s.iter()
                    .find(|s| s.object == object.object && s.offset == object.offset)
            })
            .ok_or_else(|| error("missing previous placement"))?;
        if previous.binding == object.binding {
            continue;
        }
        let SourceObject::Routine {
            owner, ref base, ..
        } = object.object
        else {
            unreachable!()
        };
        let mem = match *base {
            MirStorageBase::Param(id) => MirMem::Param { id, offset: 0 },
            MirStorageBase::Local(id) => MirMem::Local { id, offset: 0 },
            _ => return Err(error("nonordinary resident object")),
        };
        if !addresses
            .iter()
            .all(|a| a.compiler_controls_address(owner, &mem))
        {
            return Err(error("retained source exposure"));
        }
        check(previous, &old.memory, &observations)?;
        check(object, &new.memory, &observations)?;
    }
    Ok(())
}

fn check(
    object: &SourceStorage,
    memory: &MirPhysicalMemory,
    observations: &[(u16, u16)],
) -> Result<(), Vec<MirDiagnostic>> {
    let overlaps =
        |address: u16, bytes: u16| (0..bytes).any(|i| object.contains(address.wrapping_add(i)));
    if observations.iter().any(|&(a, n)| overlaps(a, n)) {
        return Err(error(
            "numeric observation overlaps an old or proposed home",
        ));
    }
    let Some(inventory) = memory.source_storage() else {
        return Err(error("missing source-storage inventory"));
    };
    if inventory.iter().any(|other| {
        // Different invocations may reuse compiler-chosen ZP homes. Explicit
        // aliases and fixed declarations have no such permission.
        let invocation_home = matches!(other.binding,
            Some(SourceBinding::Parameter(MirParamHome::ZeroPage(_)))
            | Some(SourceBinding::Local(MirLocalHome::ZeroPage(_))))
            && matches!((&object.object, &other.object),
                (SourceObject::Routine { owner: a, .. }, SourceObject::Routine { owner: b, .. }) if a != b);
        other.object != object.object && !invocation_home && overlaps(other.address, other.bytes)
    }) { return Err(error("source declaration aliases an old or proposed home")); }
    Ok(())
}

/// Preserve explicit source operands, declared regions and decoded assembly.
/// Generated FixedZeroPage operands are checked by residence, not mistaken for
/// source exposure. Unknown pointee footprints are deliberately not numeric
/// observations; undecodable explicit machine operands still prevent a proof.
fn numeric_observations(p: &MirProgram) -> Option<Vec<(u16, u16)>> {
    let mut out = Vec::new();
    for r in &p.routines {
        for b in &r.blocks {
            for memory in b
                .ops
                .iter()
                .map(|op| classify_op(op).memory)
                .chain(std::iter::once(classify_terminator(&b.terminator).memory))
            {
                for range in memory.direct_reads.iter().chain(&memory.direct_writes) {
                    if let MirMem::Absolute(a) = range.base {
                        out.push((a, range.bytes));
                    }
                }
                for mem in &memory.address_references {
                    if let MirMem::Absolute(a) = mem {
                        out.push((*a, 1));
                    }
                }
                for effect in [&memory.structured_reads, &memory.structured_writes] {
                    if let MirMemoryEffect::Regions(regions) = effect {
                        for region in regions {
                            match region.kind {
                                MirMemoryRegionKind::AbsoluteRange
                                | MirMemoryRegionKind::ZeroPage => {
                                    out.push((region.offset, region.size))
                                }
                                MirMemoryRegionKind::Stack => {
                                    out.push((0x100u16.wrapping_add(region.offset), region.size))
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            for op in &b.ops {
                if let MirOp::Load {
                    src:
                        MirAddr::AbsoluteIndexedX {
                            base: MirMem::Absolute(a),
                        }
                        | MirAddr::AbsoluteIndexedY {
                            base: MirMem::Absolute(a),
                        },
                    ..
                }
                | MirOp::Store {
                    dst:
                        MirAddr::AbsoluteIndexedX {
                            base: MirMem::Absolute(a),
                        }
                        | MirAddr::AbsoluteIndexedY {
                            base: MirMem::Absolute(a),
                        },
                    ..
                } = op
                {
                    out.push((*a, 256));
                }
                if let MirOp::MachineBlock { id, .. } = op {
                    let machine = p.machine_blocks.iter().find(|m| m.id == *id)?;
                    let inventory = machine_storage_observations(machine, r, p)?;
                    for access in inventory.reads.iter().chain(&inventory.writes) {
                        match access.base {
                            MirMem::Absolute(a) => out.push((a, access.bytes)),
                            MirMem::FixedZeroPage(s) => out.push((u16::from(s.0), access.bytes)),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    Some(out)
}
