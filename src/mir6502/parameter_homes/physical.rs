//! Final-layout obligations for the native pointee exception. Existing residence
//! rules still govern scratch and ABI traffic; numeric source observations must
//! not silently disappear when an inline object moves to a resident home.
use super::*;
use crate::mir6502::analysis::{
    effects::{classify_op, classify_terminator, writes_ordinary_pointee},
    known_callees::{fixed_storage_address, machine_storage_observations},
    record_bases::{MirPhysicalMemory, SourceBinding, SourceObject, SourceStorage},
};

fn needs_proof(p: &MirProgram, r: &MirRoutine) -> bool {
    !r.frame.resident_bytes().is_empty() && has_pointee_writes(p, r)
}

pub(super) fn has_pointee_writes(p: &MirProgram, r: &MirRoutine) -> bool {
    let own = r.blocks
        .iter()
        .flat_map(|b| &b.ops)
        .any(writes_ordinary_pointee);
    if own { return true; }
    // A wrapper inherits the same layout obligation as its buffer call.
    let mut pending = vec![r.id];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) { continue; }
        let Some(r) = p.routines.iter().find(|r| r.id == id) else { continue; };
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            if super::super::runtime_contracts::library::may_write_buffer(op) { return true; }
            if let MirOp::Call { target: MirCallTarget::Routine(id), .. } = op { pending.push(*id); }
        }
    }
    false
}

/// A known missing obligation rejects discovery, before it can poison the
/// combined trial. Concrete overlaps still require both authoritative layouts.
pub(super) fn observations_complete(p: &MirProgram) -> bool {
    unresolved_observation(p).is_none()
}

#[derive(Debug, serde::Serialize)]
pub(super) struct UnresolvedObservation {
    routine: String,
    routine_id: u32,
    block: String,
    machine_block: u32,
    stage: &'static str,
    operand: Option<String>,
}

impl UnresolvedObservation {
    fn diagnostics(self) -> Vec<MirDiagnostic> {
        vec![MirDiagnostic {
            routine: Some(self.routine),
            block: Some(self.block),
            message: format!(
                "source-object physical observation: unresolved explicit operand in machine block m{} (owner r{}, stage {}, operand {:?})",
                self.machine_block, self.routine_id, self.stage, self.operand
            ),
        }]
    }
}

pub(super) fn unresolved_observation(p: &MirProgram) -> Option<UnresolvedObservation> {
    p.routines
        .iter()
        .any(|r| has_pointee_writes(p, r))
        .then(|| numeric_observations(p, None).err())
        .flatten()
}

fn records<'a>(
    p: &MirProgram,
    m: &'a MirPhysicalMemory,
) -> Result<Vec<&'a SourceStorage>, Vec<MirDiagnostic>> {
    if !p.routines.iter().any(|r| needs_proof(p, r)) {
        return Ok(vec![]);
    }
    let inventory = m
        .source_storage()
        .ok_or_else(|| error("missing source-storage inventory"))?;
    let mut result = Vec::new();
    for r in p.routines.iter().filter(|r| needs_proof(p, r)) {
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
    let observations = numeric_observations(p, Some(memory)).map_err(UnresolvedObservation::diagnostics)?;
    for object in objects {
        check(object, memory, &observations)?;
    }
    Ok(())
}

pub(super) fn verify_relocation(
    old: &super::super::preparation::Executable,
    new: &super::super::preparation::Executable,
    old_input: &MirProgram,
    new_input: &MirProgram,
) -> Result<(), Vec<MirDiagnostic>> {
    let objects = records(&new.program, &new.memory)?;
    if objects.is_empty() {
        return Ok(());
    }
    // Retained inputs are resolved only in the executable produced from them.
    let sources = [(old_input, &old.memory), (new_input, &new.memory),
        (&old.program, &old.memory), (&new.program, &new.memory)];
    let addresses: Vec<_> = sources.iter().map(|(p, _)| ObjectAddressControl::new(p)).collect();
    let observations = resolve_sources(&sources)?;
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

pub(super) fn resolve_sources(sources: &[(&MirProgram, &MirPhysicalMemory)]) -> Result<Vec<(u16, u16)>, Vec<MirDiagnostic>> {
    let mut observations = Vec::new();
    for (p, memory) in sources {
        observations.extend(numeric_observations(p, Some(memory)).map_err(UnresolvedObservation::diagnostics)?);
    }
    Ok(observations)
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
pub(super) fn numeric_observations(p: &MirProgram, memory: Option<&MirPhysicalMemory>) -> Result<Vec<(u16, u16)>, UnresolvedObservation> {
    let mut out: Vec<_> = crate::mir6502::analysis::known_callees::numeric_buffer_observations(p)
        .into_iter().map(|(_, address, size, _)| (address, size)).collect();
    for r in &p.routines {
        let values = r.blocks.iter().flat_map(|b| &b.ops)
            .any(|op| super::super::runtime_contracts::library_invocation(p, op).is_some())
            .then(|| super::super::runtime_contracts::buffers::argument_values(p, r)).flatten();
        for b in &r.blocks {
            for memory in b
                .ops
                .iter()
                .map(|op| classify_op(op).memory)
                .chain(std::iter::once(classify_terminator(&b.terminator).memory))
            {
                for range in memory.direct_reads.iter().chain(&memory.direct_writes) {
                    if let Some(a) = numeric_base(p, &range.base) {
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
            for (op_index, op) in b.ops.iter().enumerate() {
                if let Some(contract) = super::super::runtime_contracts::library_invocation(p, op) {
                    for buffer in contract.buffers {
                        let (base, bytes) = buffer.project(p, op, values.as_ref().map(|v|
                            (v, crate::mir6502::analysis::sites::MirSite::Op { block: b.id, op_index })));
                        if let Some(base) = base && bytes != 0 { out.push((base, bytes)); }
                    }
                }
                if let MirOp::Load {
                    src: MirAddr::AbsoluteIndexedX { base } | MirAddr::AbsoluteIndexedY { base },
                    width,
                    ..
                }
                | MirOp::Store {
                    dst: MirAddr::AbsoluteIndexedX { base } | MirAddr::AbsoluteIndexedY { base },
                    width,
                    ..
                } = op
                    && let Some(a) = numeric_base(p, base)
                {
                    out.push((a, if *width == MirWidth::Word { 257 } else { 256 }));
                }
                if let MirOp::MachineBlock { id, .. } = op {
                    let unresolved = || UnresolvedObservation {
                        routine: r.name.clone(),
                        routine_id: r.id.0,
                        block: b.label.clone(),
                        machine_block: id.0,
                        stage: "decode", operand: None,
                    };
                    let machine = p
                        .machine_blocks
                        .iter()
                        .find(|m| m.id == *id)
                        .ok_or_else(unresolved)?;
                    let inventory =
                        machine_storage_observations(machine, r, p).ok_or_else(unresolved)?;
                    for access in inventory.reads.iter().chain(&inventory.writes) {
                        if let Some(ranges) = access.resolve(p, r.id, memory) {
                            out.extend(ranges.iter().map(|range|
                                (fixed_storage_address(p, &range.base).expect("resolved physical address"), range.bytes)));
                        } else if memory.is_some() {
                            return Err(UnresolvedObservation { stage: "layout",
                                operand: Some(format!("{access:?}")), ..unresolved() });
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn numeric_base(p: &MirProgram, mem: &MirMem) -> Option<u16> {
    match mem {
        MirMem::Absolute(_) | MirMem::Global { .. } => fixed_storage_address(p, mem),
        // Generated scratch traffic is checked by residence. Only the explicit
        // machine inventory above records FixedZeroPage as a source observer.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn symbolic_observation_retained_sources_use_their_own_layouts_and_protect_old_and_new_homes() {
        let source = "PROC POINTER ptr\nPROC Probe=*()\nASM\n jmp (ptr)\nENDASM\nPROC Main() RETURN";
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model = crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern()).unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let p = crate::mir6502::lower_program(&nir).unwrap();
        let id = p.globals.iter().find(|g| g.name == "ptr").unwrap().id;
        let map = |address| {
            let mut m = MirPhysicalMemory::default();
            m.protect_source_storage(Some(vec![SourceStorage { object: SourceObject::Global(id),
                binding: None, offset: 0, address, bytes: 2 }]));
            m
        };
        let old = map(0x30ff);
        let new = map(0x4100);
        // A later cleanup may remove the operation; its retained input still observes it.
        let mut removed = p.clone();
        for r in &mut removed.routines { for b in &mut r.blocks {
            b.ops.retain(|op| !matches!(op, MirOp::MachineBlock { .. }));
        }}
        assert!(numeric_observations(&p, None).unwrap().is_empty()); // Decodable pending.
        assert!(numeric_observations(&removed, Some(&new)).unwrap().is_empty());
        let observed = resolve_sources(&[(&p, &old), (&p, &new), (&removed, &new)]).unwrap();
        assert_eq!(observed, [(0x30ff, 1), (0x3000, 1), (0x4100, 1), (0x4101, 1)]);
        for address in [0x30ff, 0x3000, 0x4100, 0x4101, 0xe4] {
            let object = SourceStorage { object: SourceObject::Routine { owner: RoutineId(99),
                slot: MirStorageId(0), base: MirStorageBase::Local(crate::nir::LocalId(0)) },
                binding: None, offset: 0, address, bytes: 1 };
            let mut m = MirPhysicalMemory::default();
            m.protect_source_storage(Some(vec![object.clone()]));
            assert_eq!(check(&object, &m, &observed).is_ok(), address == 0xe4);
        }
        let missing = MirPhysicalMemory::default();
        let errors = resolve_sources(&[(&p, &old), (&p, &missing)]).unwrap_err();
        assert!(errors[0].message.contains("stage layout"));
        assert!(errors[0].message.contains("PointerFetch"));
    }

    #[test]
    fn fixed_observation_alias_fetches_protect_both_lanes_without_observing_scratch() {
        let source = "BYTE POINTER ptr=$E4 BYTE POINTER link=ptr+2\nPROC Probe=*()\nASM\n lda (link),y\n rts\nENDASM\nPROC Main() RETURN";
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model = crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern()).unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let mut p = crate::mir6502::lower_program(&nir).unwrap();
        let ptr = p.globals.iter().find(|g| g.name == "ptr").unwrap().id;
        let link = p.globals.iter_mut().find(|g| g.name == "link").unwrap();
        link.backing = MirGlobalBacking::Alias { target: ptr, offset: 2 };
        p.routines.last_mut().unwrap().blocks[0].ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xac))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte,
        });
        let observations = numeric_observations(&p, None).unwrap();
        assert!(observations.contains(&(0xe6, 1)));
        assert!(observations.contains(&(0xe7, 1)));
        assert!(!observations.contains(&(0xac, 1)));
        for address in [0xe6, 0xe7, 0x4000] {
            let record = SourceStorage {
                object: SourceObject::Routine { owner: RoutineId(99), slot: MirStorageId(0),
                    base: MirStorageBase::Local(crate::nir::LocalId(0)) },
                binding: None, offset: 0, address, bytes: 1,
            };
            let mut memory = MirPhysicalMemory::default();
            memory.protect_source_storage(Some(vec![record.clone()]));
            assert_eq!(check(&record, &memory, &observations).is_ok(), address == 0x4000);
        }
    }

    #[test]
    fn object_storage_physical_ranges_protect_old_and_new_lanes() {
        let old = SourceStorage {
            object: SourceObject::Routine {
                owner: RoutineId(0),
                slot: MirStorageId(0),
                base: MirStorageBase::Local(crate::nir::LocalId(0)),
            },
            binding: None,
            offset: 0,
            address: 0x4000,
            bytes: 2,
        };
        let mut new = old.clone();
        new.address = 0xe4;
        new.binding = Some(SourceBinding::Local(MirLocalHome::ZeroPage(
            MirFixedZpSlot(0xe4),
        )));
        for record in [old, new] {
            let mut memory = MirPhysicalMemory::default();
            memory.protect_source_storage(Some(vec![record.clone()]));
            check(&record, &memory, &[(0x6000, 256)]).unwrap();
            for lane in 0..2 {
                assert!(check(&record, &memory, &[(record.address + lane, 1)]).is_err());
            }
            assert!(check(&record, &memory, &[(record.address.wrapping_sub(255), 257)]).is_err());
        }
    }
}
