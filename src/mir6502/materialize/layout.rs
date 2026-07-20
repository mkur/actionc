use crate::mir6502::ir::{
    MirGlobalBacking, MirGlobalInit, MirMem, MirProgram, MirSpillId, MirStorageBase,
    MirStorageInit, MirStorageSlot, MirWidth, RoutineId,
};
use crate::nir::{LocalId, ParamId, SymbolId};

pub(super) struct MaterializeLayout {
    origin: u16,
    globals: Vec<(SymbolId, MirGlobalBacking, bool)>,
    statics: Vec<(SymbolId, u16, u16)>,
    routine_storage: Vec<(RoutineId, MaterializeRoutineStorage)>,
}

#[derive(Debug, Default, Clone)]
struct MaterializeRoutineStorage {
    params: Vec<(ParamId, u16, u16, MirWidth)>,
    locals: Vec<(LocalId, u16, u16, MirWidth)>,
    spills: Vec<(MirSpillId, u16, u16)>,
    descriptor_params: Vec<ParamId>,
    descriptor_locals: Vec<LocalId>,
}

impl MaterializeLayout {
    pub(super) fn new(program: &MirProgram, origin: u16) -> Self {
        let global_bytes = program
            .globals
            .iter()
            .filter_map(|global| match global.backing {
                MirGlobalBacking::Ordinary { offset } => Some(offset.saturating_add(
                    global.init.as_ref().map_or(global.storage_size, |init| {
                        global_init_object_size(init, global.storage_size)
                    }),
                )),
                MirGlobalBacking::Absolute(_) | MirGlobalBacking::Alias { .. } => None,
            })
            .max()
            .unwrap_or(0);
        let mut static_base = origin.saturating_add(global_bytes);
        let statics = program
            .statics
            .iter()
            .map(|static_data| {
                let start = static_base;
                static_base = static_base.saturating_add(static_data.bytes.len() as u16);
                (static_data.id, start, static_data.bytes.len() as u16)
            })
            .collect();
        let mut cursor = static_base;
        let mut routine_storage = Vec::new();
        for routine in &program.routines {
            let mut storage = MaterializeRoutineStorage::default();
            for param in &routine.frame.params {
                place_materialize_slot(&mut storage, param, &mut cursor);
            }
            for local in &routine.frame.locals {
                place_materialize_slot(&mut storage, local, &mut cursor);
            }
            for spill in &routine.frame.spills {
                let address = cursor;
                let size = 1;
                cursor = cursor.saturating_add(size);
                storage.spills.push((*spill, address, size));
            }
            routine_storage.push((routine.id, storage));
        }
        Self {
            origin,
            globals: program
                .globals
                .iter()
                .map(|global| {
                    (
                        global.id,
                        global.backing.clone(),
                        matches!(global.init, Some(MirGlobalInit::Descriptor { .. })),
                    )
                })
                .collect(),
            statics,
            routine_storage,
        }
    }

    pub(super) fn mem_address(&self, routine_id: RoutineId, mem: &MirMem) -> Option<u16> {
        match mem {
            MirMem::Absolute(address) => Some(*address),
            MirMem::Global { id, offset } => self.global_address(*id).map(|addr| addr + *offset),
            MirMem::Static { id, offset } => self.static_address(*id).map(|addr| addr + *offset),
            MirMem::Local { id, offset } => self
                .routine_storage(routine_id)
                .and_then(|storage| storage.local_address(*id, *offset)),
            MirMem::Param { id, offset } => self
                .routine_storage(routine_id)
                .and_then(|storage| storage.param_address(*id, *offset)),
            MirMem::Spill { id, offset } => self
                .routine_storage(routine_id)
                .and_then(|storage| storage.spill_address(*id, *offset)),
            MirMem::ZeroPage(_) | MirMem::FixedZeroPage(_) => None,
        }
    }

    pub(super) fn global_address(&self, id: SymbolId) -> Option<u16> {
        for (global_id, backing, _) in &self.globals {
            if *global_id == id {
                return match backing {
                    MirGlobalBacking::Ordinary { offset } => {
                        Some(self.origin.saturating_add(*offset))
                    }
                    MirGlobalBacking::Absolute(address) => Some(*address),
                    MirGlobalBacking::Alias { target, offset } => self
                        .global_address(*target)
                        .map(|address| address.saturating_add(*offset)),
                };
            }
        }
        None
    }

    pub(super) fn global_allows_idempotent_store_removal(&self, id: SymbolId) -> bool {
        for (global_id, backing, _) in &self.globals {
            if *global_id == id {
                return match backing {
                    MirGlobalBacking::Ordinary { .. } => true,
                    MirGlobalBacking::Absolute(address) => *address < 0x0100,
                    MirGlobalBacking::Alias { target, .. } => {
                        self.global_allows_idempotent_store_removal(*target)
                    }
                };
            }
        }
        false
    }

    pub(super) fn mem_allows_deferred_direct_read(&self, mem: &MirMem) -> bool {
        match mem {
            MirMem::Local { .. } | MirMem::Param { .. } | MirMem::Static { .. } => true,
            MirMem::Global { id, .. } => self.global_has_ordinary_backing(*id),
            MirMem::Absolute(_)
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
            | MirMem::FixedZeroPage(_) => false,
        }
    }

    fn global_has_ordinary_backing(&self, id: SymbolId) -> bool {
        self.globals
            .iter()
            .find_map(|(global_id, backing, _)| {
                (*global_id == id).then(|| match backing {
                    MirGlobalBacking::Ordinary { .. } => true,
                    MirGlobalBacking::Alias { target, .. } => {
                        self.global_has_ordinary_backing(*target)
                    }
                    MirGlobalBacking::Absolute(_) => false,
                })
            })
            .unwrap_or(false)
    }

    pub(super) fn is_descriptor_storage(&self, routine_id: RoutineId, mem: &MirMem) -> bool {
        match mem {
            MirMem::Global { id, offset } if *offset == 0 => self
                .globals
                .iter()
                .find_map(|(global_id, _, descriptor)| (*global_id == *id).then_some(*descriptor))
                .unwrap_or(false),
            MirMem::Local { id, offset } if *offset == 0 => self
                .routine_storage(routine_id)
                .is_some_and(|storage| storage.is_descriptor_local(*id)),
            MirMem::Param { id, offset } if *offset == 0 => self
                .routine_storage(routine_id)
                .is_some_and(|storage| storage.is_descriptor_param(*id)),
            _ => false,
        }
    }

    pub(super) fn static_address(&self, id: SymbolId) -> Option<u16> {
        self.statics
            .iter()
            .find(|(static_id, _, _)| *static_id == id)
            .map(|(_, address, _)| *address)
    }

    pub(super) fn is_synthetic_byte_storage_high(
        &self,
        routine_id: RoutineId,
        mem: &MirMem,
    ) -> bool {
        match mem {
            MirMem::Local { id, offset } if *offset == 1 => self
                .routine_storage(routine_id)
                .is_some_and(|storage| storage.is_byte_scalar_local(*id)),
            MirMem::Param { id, offset } if *offset == 1 => self
                .routine_storage(routine_id)
                .is_some_and(|storage| storage.is_byte_scalar_param(*id)),
            _ => false,
        }
    }

    pub(super) fn is_byte_scalar_storage(&self, routine_id: RoutineId, mem: &MirMem) -> bool {
        match mem {
            MirMem::Local { id, offset } if *offset == 0 => self
                .routine_storage(routine_id)
                .is_some_and(|storage| storage.is_byte_scalar_local(*id)),
            MirMem::Param { id, offset } if *offset == 0 => self
                .routine_storage(routine_id)
                .is_some_and(|storage| storage.is_byte_scalar_param(*id)),
            _ => false,
        }
    }

    fn routine_storage(&self, routine_id: RoutineId) -> Option<&MaterializeRoutineStorage> {
        self.routine_storage
            .iter()
            .find_map(|(id, storage)| (*id == routine_id).then_some(storage))
    }
}

fn global_init_object_size(init: &MirGlobalInit, storage_size: u16) -> u16 {
    match init {
        MirGlobalInit::Bytes {
            bytes, zero_fill, ..
        } => (bytes.len() as u16)
            .saturating_add(*zero_fill)
            .max(storage_size),
        MirGlobalInit::ZeroFill { bytes, .. } => (*bytes).max(storage_size),
        MirGlobalInit::ProgramEndWord { .. } => 2.max(storage_size),
        MirGlobalInit::Descriptor {
            backing,
            descriptor_size,
            ..
        } => (backing.bytes.len() as u16)
            .saturating_add(backing.zero_fill)
            .saturating_add(*descriptor_size)
            .max(storage_size),
        MirGlobalInit::RoutineAddress {
            descriptor_size, ..
        } => (*descriptor_size).max(storage_size),
    }
}

impl MaterializeRoutineStorage {
    fn param_address(&self, id: ParamId, offset: u16) -> Option<u16> {
        find_materialize_slot(&self.params, id, offset)
    }

    fn is_byte_scalar_param(&self, id: ParamId) -> bool {
        self.params.iter().any(|(candidate, _, size, width)| {
            *candidate == id && *size == 1 && *width == MirWidth::Byte
        })
    }

    fn is_descriptor_param(&self, id: ParamId) -> bool {
        self.descriptor_params.contains(&id)
    }

    fn local_address(&self, id: LocalId, offset: u16) -> Option<u16> {
        find_materialize_slot(&self.locals, id, offset)
    }

    fn local_base_address(&self, id: LocalId) -> Option<u16> {
        self.locals
            .iter()
            .find_map(|(candidate, address, _, _)| (*candidate == id).then_some(*address))
    }

    fn is_byte_scalar_local(&self, id: LocalId) -> bool {
        self.locals.iter().any(|(candidate, _, size, width)| {
            *candidate == id && *size == 1 && *width == MirWidth::Byte
        })
    }

    fn is_descriptor_local(&self, id: LocalId) -> bool {
        self.descriptor_locals.contains(&id)
    }

    fn spill_address(&self, id: MirSpillId, offset: u16) -> Option<u16> {
        find_spill_slot(&self.spills, id, offset)
    }
}

fn place_materialize_slot(
    storage: &mut MaterializeRoutineStorage,
    slot: &MirStorageSlot,
    cursor: &mut u16,
) {
    if let MirStorageBase::LocalAlias { id, target } = slot.base {
        if let Some(address) = storage.local_base_address(target) {
            storage.locals.push((
                id,
                address.saturating_add(slot.offset),
                storage_slot_logical_size(slot),
                slot.width,
            ));
        }
        return;
    }
    let address = *cursor;
    let size = storage_slot_size(slot);
    *cursor = cursor.saturating_add(size);
    match slot.base {
        MirStorageBase::Param(id) => {
            if matches!(slot.init, Some(MirStorageInit::Descriptor { .. })) {
                storage.descriptor_params.push(id);
            }
            storage.params.push((id, address, size, slot.width));
        }
        MirStorageBase::Local(id) => {
            if matches!(slot.init, Some(MirStorageInit::Descriptor { .. })) {
                storage.descriptor_locals.push(id);
            }
            storage.locals.push((id, address, size, slot.width));
        }
        MirStorageBase::Spill(id) => storage.spills.push((id, address, size)),
        MirStorageBase::LocalAlias { .. }
        | MirStorageBase::Absolute(_)
        | MirStorageBase::Global(_)
        | MirStorageBase::Static(_) => {}
    }
}

fn find_materialize_slot<T: Copy + PartialEq>(
    slots: &[(T, u16, u16, MirWidth)],
    id: T,
    offset: u16,
) -> Option<u16> {
    slots.iter().find_map(|(candidate, address, size, _)| {
        (*candidate == id && offset < *size).then_some(address.saturating_add(offset))
    })
}

fn find_spill_slot<T: Copy + PartialEq>(
    slots: &[(T, u16, u16)],
    id: T,
    offset: u16,
) -> Option<u16> {
    slots.iter().find_map(|(candidate, address, size)| {
        (*candidate == id && offset < *size).then_some(address.saturating_add(offset))
    })
}

fn storage_slot_size(slot: &MirStorageSlot) -> u16 {
    let width = match slot.width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    };
    let storage_size = slot.offset.saturating_add(width);
    slot.init.as_ref().map_or(storage_size, |init| {
        storage_init_object_size(init, storage_size)
    })
}

fn storage_slot_logical_size(slot: &MirStorageSlot) -> u16 {
    match slot.width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
    }
}

fn storage_init_object_size(init: &MirStorageInit, storage_size: u16) -> u16 {
    match init {
        MirStorageInit::Bytes {
            bytes, zero_fill, ..
        } => (bytes.len() as u16)
            .saturating_add(*zero_fill)
            .max(storage_size),
        MirStorageInit::ZeroFill { bytes, .. } => (*bytes).max(storage_size),
        MirStorageInit::Descriptor {
            backing,
            descriptor_size,
            ..
        } => (backing.bytes.len() as u16)
            .saturating_add(backing.zero_fill)
            .saturating_add(*descriptor_size)
            .max(storage_size),
        MirStorageInit::RoutineAddress {
            descriptor_size, ..
        } => (*descriptor_size).max(storage_size),
    }
}
