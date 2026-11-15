//! Declaration facts for ranges that remain ordinary under relocation.
//!
//! This is not an allocator. The budget deliberately overcounts initialized
//! objects and deferred data. It bounds all inline data without predicting
//! any object's address or depending on code size/branch relaxation.
use crate::mir6502::ir::{
    MirGlobalBacking, MirGlobalInit, MirMem, MirProgram, MirStorageBase, MirStorageClass,
    MirStorageInit, RoutineId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OrdinaryStorageBounds {
    bytes_upper_bound: usize,
    roots: Vec<(Option<RoutineId>, MirMem, u16)>,
}

impl OrdinaryStorageBounds {
    pub(super) fn new(program: &MirProgram) -> Self {
        let mut bounds = Self {
            bytes_upper_bound: 0,
            roots: Vec::new(),
        };
        for global in &program.globals {
            if !matches!(global.backing, MirGlobalBacking::Ordinary { .. }) {
                continue;
            }
            bounds.add(usize::from(global.storage_size));
            if let Some(init) = &global.init {
                bounds.add(global_payload_bound(init));
            }
            // Array backing can be deferred past code; descriptors change the
            // logical base. Neither is an inline scalar root for this proof.
            if global.width.is_some()
                && matches!(
                    global.init,
                    None | Some(MirGlobalInit::Bytes { array: None, .. })
                        | Some(MirGlobalInit::ZeroFill { array: None, .. })
                        | Some(MirGlobalInit::ProgramEndWord { .. })
                )
            {
                bounds.roots.push((
                    None,
                    MirMem::Global {
                        id: global.id,
                        offset: 0,
                    },
                    global.storage_size,
                ));
            }
        }
        for data in &program.statics {
            bounds.add(data.image.bytes.len());
        }
        for routine in &program.routines {
            for slot in routine.frame.params.iter().chain(&routine.frame.locals) {
                if let MirStorageBase::Param(id) = slot.base
                    && !routine.frame.inline_parameter(id)
                {
                    continue;
                }
                if let MirStorageBase::Local(id) = slot.base && !routine.frame.inline_local(id) {
                    continue;
                }
                // Aliases and ABI-only declarations do not allocate roots.
                let root = match slot.base {
                    MirStorageBase::Param(id) => MirMem::Param { id, offset: 0 },
                    MirStorageBase::Local(id) => MirMem::Local { id, offset: 0 },
                    MirStorageBase::Spill(_) => {
                        bounds.add(usize::from(slot.storage_size));
                        if let Some(init) = &slot.init {
                            bounds.add(slot_payload_bound(init));
                        }
                        continue;
                    }
                    _ => continue,
                };
                bounds.add(usize::from(slot.storage_size));
                if let Some(init) = &slot.init {
                    bounds.add(slot_payload_bound(init));
                }
                if slot.storage == MirStorageClass::Scalar
                    && slot.scalar_width.is_some()
                    && matches!(
                        slot.init,
                        None | Some(MirStorageInit::Bytes { .. })
                            | Some(MirStorageInit::ZeroFill { .. })
                    )
                {
                    bounds
                        .roots
                        .push((Some(routine.id), root, slot.storage_size));
                }
            }
            bounds.add(routine.frame.spills.len());
        }
        bounds
    }

    fn add(&mut self, bytes: usize) {
        self.bytes_upper_bound = self.bytes_upper_bound.saturating_add(bytes);
    }

    // The only declaration mutation admitted by the existing pruning guard.
    pub(super) fn remove_spills(&mut self, count: usize) {
        self.bytes_upper_bound = self.bytes_upper_bound.saturating_sub(count);
    }

    pub(super) fn range_is_outside_zero_page(
        &self,
        origin: u16,
        routine: RoutineId,
        mem: &MirMem,
        bytes: u16,
    ) -> bool {
        if origin < 0x0100
            || bytes == 0
            || self.bytes_upper_bound > 0x10000usize - usize::from(origin)
        {
            return false;
        }
        let (owner, root, offset) = match mem {
            MirMem::Global { id, offset } => (None, MirMem::Global { id: *id, offset: 0 }, *offset),
            MirMem::Local { id, offset } => {
                (Some(routine), MirMem::Local { id: *id, offset: 0 }, *offset)
            }
            MirMem::Param { id, offset } => {
                (Some(routine), MirMem::Param { id: *id, offset: 0 }, *offset)
            }
            _ => return false,
        };
        self.roots.iter().any(|(candidate_owner, candidate, size)| {
            *candidate_owner == owner
                && *candidate == root
                && u32::from(offset) + u32::from(bytes) <= u32::from(*size)
        })
    }
}

fn global_payload_bound(init: &MirGlobalInit) -> usize {
    match init {
        MirGlobalInit::Bytes {
            image, zero_fill, ..
        } => image.bytes.len().saturating_add(usize::from(*zero_fill)),
        MirGlobalInit::ZeroFill { bytes, .. } => usize::from(*bytes),
        MirGlobalInit::ProgramEndWord { .. } => 2,
        MirGlobalInit::Descriptor {
            backing,
            descriptor_size,
            ..
        } => backing
            .image
            .bytes
            .len()
            .saturating_add(usize::from(backing.zero_fill))
            .saturating_add(usize::from(*descriptor_size)),
        MirGlobalInit::RoutineAddress {
            descriptor_size, ..
        } => usize::from(*descriptor_size),
    }
}

fn slot_payload_bound(init: &MirStorageInit) -> usize {
    match init {
        MirStorageInit::Bytes {
            image, zero_fill, ..
        } => image.bytes.len().saturating_add(usize::from(*zero_fill)),
        MirStorageInit::ZeroFill { bytes, .. } => usize::from(*bytes),
        MirStorageInit::Descriptor {
            backing,
            descriptor_size,
            ..
        } => backing
            .image
            .bytes
            .len()
            .saturating_add(usize::from(backing.zero_fill))
            .saturating_add(usize::from(*descriptor_size)),
        MirStorageInit::RoutineAddress {
            descriptor_size, ..
        } => usize::from(*descriptor_size),
    }
}

#[cfg(test)]
mod local_binding_tests {
    use super::*;
    use crate::mir6502::ir::{MirLocalBinding, MirLocalHome, MirFixedZpSlot};
    #[test]
    fn bound_local_is_not_an_ordinary_root_or_inline_budget() {
        let ast = crate::parser::parse(&crate::lexer::tokenize("CARD FUNC Keep() CARD n n=1 RETURN(n)").unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let mut p = crate::mir6502::lower_program(&crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model))).unwrap();
        let r = &p.routines[0];
        let MirStorageBase::Local(local) = r.frame.locals[0].base else { panic!() };
        let mem = MirMem::Local { id: local, offset:0 };
        let old = OrdinaryStorageBounds::new(&p);
        assert!(old.range_is_outside_zero_page(0x3000, r.id, &mem, 2));
        p.routines[0].frame.local_homes = Some(vec![MirLocalBinding { local, home:MirLocalHome::ZeroPage(MirFixedZpSlot(0xe4)) }]);
        let new = OrdinaryStorageBounds::new(&p);
        assert!(!new.range_is_outside_zero_page(0x3000, p.routines[0].id, &mem, 2));
        assert_eq!(old.bytes_upper_bound - new.bytes_upper_bound, 2);
    }
}
