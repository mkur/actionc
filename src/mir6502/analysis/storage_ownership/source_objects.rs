//! Logical source-object exposure, independent of working-home placement.
//! Physical observations remain separate obligations against actual layout.
use super::*;

pub(in crate::mir6502) struct ObjectAddressControl<'a> {
    source: &'a MirProgram,
    exposed: BTreeMap<RoutineId, Vec<MirMem>>,
    machine_owners: BTreeSet<RoutineId>,
}

impl<'a> ObjectAddressControl<'a> {
    pub fn new(source: &'a MirProgram) -> Self {
        let mut exposed: BTreeMap<RoutineId, Vec<MirMem>> = BTreeMap::new();
        let mut machine_owners = BTreeSet::new();
        for r in &source.routines {
            let cells = exposed.entry(r.id).or_default();
            for slot in &r.frame.locals {
                if let MirStorageBase::LocalAlias { target, .. } = slot.base {
                    cells.push(MirMem::Local { id: target, offset: 0 });
                }
            }
            for block in &r.blocks {
                cells.extend(classify_terminator(&block.terminator).memory.address_references);
                for op in &block.ops {
                    cells.extend(classify_op(op).memory.address_references);
                    // Raw machine operands can name a source home without a
                    // typed address use. Keep the existing owning-body guard.
                    if matches!(op, MirOp::MachineBlock { .. }) {
                        machine_owners.insert(r.id);
                    }
                }
            }
        }
        let mut image = |image: &MirDataImage| {
            for relocation in &image.relocations {
                let (owner, mem) = match relocation.target {
                    MirDataRelocationTarget::Param { routine, id } =>
                        (routine, MirMem::Param { id, offset: 0 }),
                    MirDataRelocationTarget::Local { routine, id } =>
                        (routine, MirMem::Local { id, offset: 0 }),
                    _ => continue,
                };
                exposed.entry(owner).or_default().push(mem);
            }
        };
        for s in &source.statics { image(&s.image); }
        for g in &source.globals {
            match &g.init {
                Some(MirGlobalInit::Bytes { image: data, .. }) => image(data),
                Some(MirGlobalInit::Descriptor { backing, .. }) => image(&backing.image),
                _ => {}
            }
        }
        for s in source.routines.iter().flat_map(|r| r.frame.params.iter().chain(&r.frame.locals)) {
            match &s.init {
                Some(MirStorageInit::Bytes { image: data, .. }) => image(data),
                Some(MirStorageInit::Descriptor { backing, .. }) => image(&backing.image),
                _ => {}
            }
        }
        Self { source, exposed, machine_owners }
    }

    pub fn is_current(&self, source: &MirProgram) -> bool {
        std::ptr::eq(self.source, source)
    }

    pub fn owns_routine(&self, routine: &MirRoutine) -> bool {
        self.source.routines.iter().any(|r| std::ptr::eq(r, routine))
    }

    /// This logical permission says nothing about initialization, deadness,
    /// ABI/scratch clobbers, or explicit numeric aliases in final placement.
    pub fn compiler_controls_address(&self, owner: RoutineId, object: &MirMem) -> bool {
        let Some(r) = self.source.routines.iter().find(|r| r.id == owner) else { return false; };
        let (base, offset) = match object {
            MirMem::Param { id, offset } => (MirStorageBase::Param(*id), *offset),
            MirMem::Local { id, offset } => (MirStorageBase::Local(*id), *offset),
            _ => return false,
        };
        let mut slots = r.frame.params.iter().chain(&r.frame.locals).filter(|s| s.base == base);
        let Some(slot) = slots.next() else { return false; };
        slots.next().is_none() && slot.offset == 0 && offset < slot.storage_size
            && matches!(r.abi, MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry)
            && !self.machine_owners.contains(&owner)
            && !self.exposed.get(&owner).is_some_and(|cells| cells.iter().any(|m| same_object(m, object)))
    }
}

fn same_object(a: &MirMem, b: &MirMem) -> bool {
    match (a, b) {
        (MirMem::Param { id: a, .. }, MirMem::Param { id: b, .. }) => a == b,
        (MirMem::Local { id: a, .. }, MirMem::Local { id: b, .. }) => a == b,
        _ => false,
    }
}

#[cfg(test)]
#[path = "source_objects_tests.rs"]
mod tests;
