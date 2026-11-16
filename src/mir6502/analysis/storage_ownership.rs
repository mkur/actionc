//! Ownership of compiler homes, independent of their eventual address.
//!
//! This is eligibility, not deadness: a caller must prove the lifetime of the
//! particular definition and retain explicit reads, ABI uses and machine effects.
use super::{
    effects::{MirHomeByte, classify_op, classify_terminator},
    record_bases::MirPhysicalMemory,
};
use crate::mir6502::ir::*;
use std::collections::{BTreeMap, BTreeSet};

mod source_objects;
pub(in crate::mir6502) use source_objects::ObjectAddressControl;

pub(in crate::mir6502) const POINTER_SCRATCH_LO: u8 = 0xAC;
pub(in crate::mir6502) const POINTER_SCRATCH_HI: u8 = 0xAD;
pub(in crate::mir6502) const POINTER_INDEX_SCRATCH_LO: u8 = 0xAE;
pub(in crate::mir6502) const POINTER_INDEX_SCRATCH_HI: u8 = 0xAF;
pub(in crate::mir6502) const DEST_POINTER_SCRATCH_LO: u8 = 0xAA;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum StorageRole {
    Temporary,
    PointerScratch,
    ProgramOrAbi,
}

pub(in crate::mir6502) struct StorageOwnership<'a> {
    program: &'a MirProgram,
    pointer_scratch: BTreeMap<RoutineId, BTreeSet<MirFixedZpSlot>>,
    exposed: BTreeMap<RoutineId, Vec<MirMem>>,
}

impl<'a> StorageOwnership<'a> {
    pub fn new(program: &'a MirProgram) -> Self {
        let mut pointer_scratch = BTreeMap::new();
        let mut exposed = BTreeMap::new();
        for routine in &program.routines {
            let mut scratch = BTreeSet::new();
            let mut incoming = BTreeSet::new();
            let mut addresses = Vec::new();
            for block in &routine.blocks {
                let mut defined = BTreeSet::new();
                addresses.extend(
                    classify_terminator(&block.terminator)
                        .memory
                        .address_references,
                );
                for op in &block.ops {
                    if let MirOp::MachineBlock { id, .. } = op
                        && let Some(machine) = program.machine_blocks.iter().find(|m| m.id == *id)
                    {
                        for item in &machine.items {
                            if let MirMachineItem::Relocation {
                                target: MirInlineAsmTarget::Memory(mem),
                                ..
                            } = item
                            {
                                addresses.push(mem.clone());
                            }
                        }
                    }
                    let effects = classify_op(op);
                    addresses.extend(effects.memory.address_references);
                    for home in effects
                        .homes
                        .reads
                        .iter()
                        .chain(&effects.addresses.pair_reads)
                    {
                        if let MirHomeByte::FixedZeroPage(slot) = home
                            && !defined.contains(slot)
                        {
                            incoming.insert(*slot);
                        }
                    }
                    for home in effects
                        .addresses
                        .pair_reads
                        .iter()
                        .chain(&effects.addresses.pair_writes)
                    {
                        if let MirHomeByte::FixedZeroPage(slot) = home
                            && (DEST_POINTER_SCRATCH_LO..=POINTER_INDEX_SCRATCH_HI)
                                .contains(&slot.0)
                            && routine.frame.fixed_zero_page.contains(slot)
                        {
                            // A typed pointer consumer and a materializer reservation
                            // establish a role. A numerical address alone does not.
                            scratch.insert(*slot);
                        }
                    }
                    // A call's may-clobber does not establish a private value.
                    if matches!(
                        effects.kind,
                        super::effects::MirOpKind::Call
                            | super::effects::MirOpKind::RuntimeHelper
                            | super::effects::MirOpKind::MachineBlock
                            | super::effects::MirOpKind::Barrier
                    ) {
                        defined.clear();
                    } else {
                        for home in effects
                            .homes
                            .definite_writes
                            .iter()
                            .chain(&effects.addresses.pair_writes)
                        {
                            if let MirHomeByte::FixedZeroPage(slot) = home {
                                defined.insert(*slot);
                            }
                        }
                    }
                }
            }
            // This first fixed-scratch consumer requires local initialization
            // in each block. Incoming and cross-block pair values keep their
            // public/ABI treatment until a dominance-based ownership proof.
            scratch.retain(|slot| !incoming.contains(slot));
            pointer_scratch.insert(routine.id, scratch);
            exposed.insert(routine.id, addresses);
        }
        Self {
            program,
            pointer_scratch,
            exposed,
        }
    }

    pub fn role(&self, owner: RoutineId, mem: &MirMem) -> StorageRole {
        let Some(routine) = self.program.routines.iter().find(|r| r.id == owner) else {
            return StorageRole::ProgramOrAbi;
        };
        let exposed = self.exposed[&owner]
            .iter()
            .any(|other| same_home(other, mem));
        if exposed {
            return StorageRole::ProgramOrAbi;
        }
        match mem {
            MirMem::Spill { id, .. }
                if routine.frame.spills.contains(id)
                    && !routine
                        .frame
                        .params
                        .iter()
                        .chain(&routine.frame.locals)
                        .any(|s| s.base == MirStorageBase::Spill(*id)) =>
            {
                StorageRole::Temporary
            }
            MirMem::ZeroPage(slot) if routine.frame.virtual_zero_page.contains(slot) => {
                StorageRole::Temporary
            }
            MirMem::FixedZeroPage(slot) if self.pointer_scratch[&owner].contains(slot) => {
                StorageRole::PointerScratch
            }
            _ => StorageRole::ProgramOrAbi,
        }
    }

    /// Before layout this is only a candidate filter. The terminal transaction
    /// must repeat it with both baseline and candidate final physical maps.
    pub fn private_store(
        &self,
        owner: RoutineId,
        op: &MirOp,
        memory: Option<&MirPhysicalMemory>,
    ) -> Option<MirMem> {
        let MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::Def(MirDef::Reg(_)),
            width: MirWidth::Byte,
        } = op
        else {
            return None;
        };
        if self.role(owner, mem) == StorageRole::ProgramOrAbi {
            return None;
        }
        if let Some(memory) = memory {
            let address = memory.address(owner, mem)?;
            if !memory.source_storage_disjoint(address) {
                return None;
            }
        }
        Some(mem.clone())
    }
}

fn same_home(a: &MirMem, b: &MirMem) -> bool {
    match (a, b) {
        (MirMem::Spill { id: a, .. }, MirMem::Spill { id: b, .. }) => a == b,
        _ => a == b,
    }
}
