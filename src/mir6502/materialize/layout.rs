use crate::mir6502::ir::{
    MirGlobalBacking, MirGlobalInit, MirMem, MirProgram, MirSpillId, MirStorageBase,
    MirStorageInit, MirStorageSlot, MirWidth, RoutineId,
};
use crate::nir::{LocalId, ParamId, SymbolId};

pub(in crate::mir6502) struct MaterializeLayout {
    origin: u16,
    globals: Vec<(SymbolId, MirGlobalBacking, bool)>,
    statics: Vec<(SymbolId, u16, u16)>,
    routine_storage: Vec<(RoutineId, MaterializeRoutineStorage)>,
}

#[derive(Debug, Default, Clone)]
struct MaterializeRoutineStorage {
    params: Vec<(ParamId, u16, u16, Option<MirWidth>)>,
    locals: Vec<(LocalId, u16, u16, Option<MirWidth>)>,
    spills: Vec<(MirSpillId, u16, u16)>,
    descriptor_params: Vec<ParamId>,
    descriptor_locals: Vec<LocalId>,
}

impl MaterializeLayout {
    pub(in crate::mir6502) fn new(program: &MirProgram, origin: u16) -> Self {
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
                static_base = static_base.saturating_add(static_data.image.bytes.len() as u16);
                (static_data.id, start, static_data.image.bytes.len() as u16)
            })
            .collect();
        let mut cursor = static_base;
        let mut routine_storage = Vec::new();
        for routine in &program.routines {
            let mut storage = MaterializeRoutineStorage::default();
            for param in &routine.frame.params {
                if matches!(param.base, MirStorageBase::ParamAbiOnly(_)) {
                    continue;
                }
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

    pub(in crate::mir6502) fn mem_address(
        &self,
        routine_id: RoutineId,
        mem: &MirMem,
    ) -> Option<u16> {
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

    pub(super) fn mem_has_absolute_backing(&self, mem: &MirMem) -> bool {
        match mem {
            MirMem::Absolute(_) => true,
            MirMem::Global { id, .. } => self.global_has_absolute_backing(*id),
            MirMem::Static { .. }
            | MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
            | MirMem::FixedZeroPage(_) => false,
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

    fn global_has_absolute_backing(&self, id: SymbolId) -> bool {
        self.globals
            .iter()
            .find(|(global_id, _, _)| *global_id == id)
            .is_some_and(|(_, backing, _)| match backing {
                MirGlobalBacking::Absolute(_) => true,
                MirGlobalBacking::Alias { target, .. } => self.global_has_absolute_backing(*target),
                MirGlobalBacking::Ordinary { .. } => false,
            })
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

    /// NMOS 6502 read/modify/write instructions perform observable bus writes,
    /// so only select indexed INC/DEC for ordinary compiler-owned storage.
    pub(super) fn mem_allows_direct_indexed_update(&self, mem: &MirMem) -> bool {
        match mem {
            MirMem::Local { .. } | MirMem::Param { .. } | MirMem::Static { .. } => true,
            MirMem::Global { id, .. } => self.global_has_ordinary_backing(*id),
            MirMem::Absolute(_)
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
            | MirMem::FixedZeroPage(_) => false,
        }
    }

    /// NMOS read/modify/write instructions are safe for compiler-owned scalar
    /// RAM. Source-declared zero-page absolute variables also denote RAM unless
    /// volatility inserted explicit barriers around the access.
    pub(super) fn mem_allows_direct_update(&self, mem: &MirMem) -> bool {
        match mem {
            MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Static { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_) => true,
            MirMem::Global { id, .. } => self.global_allows_idempotent_store_removal(*id),
            MirMem::Absolute(address) => *address < 0x0100,
            MirMem::FixedZeroPage(_) => false,
        }
    }

    /// Reordering the two byte stores of a word is only valid for
    /// compiler-owned, non-volatile storage. In particular, absolute-backed
    /// globals retain source order because their writes may be observable by
    /// hardware or the operating system.
    pub(super) fn mem_allows_word_lane_store_reordering(&self, mem: &MirMem) -> bool {
        match mem {
            MirMem::Local { .. } | MirMem::Param { .. } => true,
            MirMem::Global { id, .. } => self.global_has_ordinary_backing(*id),
            MirMem::Absolute(_)
            | MirMem::Static { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
            | MirMem::FixedZeroPage(_) => false,
        }
    }

    /// Reading compiler-owned storage or zero-page RAM out of source order has
    /// no externally observable effect. Higher absolute addresses remain
    /// excluded, including when reached through an absolute-backed global.
    pub(in crate::mir6502) fn mem_allows_pure_read_reordering(&self, mem: &MirMem) -> bool {
        match mem {
            MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Static { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
            | MirMem::FixedZeroPage(_) => true,
            MirMem::Global { id, .. } => self.global_allows_idempotent_store_removal(*id),
            MirMem::Absolute(address) => *address < 0x0100,
        }
    }

    /// Reversing a comparison changes which operand is read first.
    pub(super) fn mem_allows_compare_operand_reordering(&self, mem: &MirMem) -> bool {
        self.mem_allows_pure_read_reordering(mem)
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
            image, zero_fill, ..
        } => (image.bytes.len() as u16)
            .saturating_add(*zero_fill)
            .max(storage_size),
        MirGlobalInit::ZeroFill { bytes, .. } => (*bytes).max(storage_size),
        MirGlobalInit::ProgramEndWord { .. } => 2.max(storage_size),
        MirGlobalInit::Descriptor {
            backing,
            descriptor_size,
            ..
        } => (backing.image.bytes.len() as u16)
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
            *candidate == id && *size == 1 && *width == Some(MirWidth::Byte)
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
            *candidate == id && *size == 1 && *width == Some(MirWidth::Byte)
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
    if matches!(slot.base, MirStorageBase::ParamAbiOnly(_)) {
        return;
    }
    if let MirStorageBase::LocalAlias { id, target } = slot.base {
        if let Some(address) = storage.local_base_address(target) {
            storage.locals.push((
                id,
                address.saturating_add(slot.offset),
                storage_slot_logical_size(slot),
                slot.scalar_width,
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
            storage.params.push((id, address, size, slot.scalar_width));
        }
        MirStorageBase::Local(id) => {
            if matches!(slot.init, Some(MirStorageInit::Descriptor { .. })) {
                storage.descriptor_locals.push(id);
            }
            storage.locals.push((id, address, size, slot.scalar_width));
        }
        MirStorageBase::Spill(id) => storage.spills.push((id, address, size)),
        MirStorageBase::ParamAbiOnly(_)
        | MirStorageBase::LocalAlias { .. }
        | MirStorageBase::Absolute(_)
        | MirStorageBase::Global(_)
        | MirStorageBase::Static(_) => {}
    }
}

fn find_materialize_slot<T: Copy + PartialEq>(
    slots: &[(T, u16, u16, Option<MirWidth>)],
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
    slot.init.as_ref().map_or(slot.storage_size, |init| {
        storage_init_object_size(init, slot.storage_size)
    })
}

fn storage_slot_logical_size(slot: &MirStorageSlot) -> u16 {
    slot.init
        .as_ref()
        .and_then(|init| match init {
            MirStorageInit::Descriptor {
                descriptor_size, ..
            }
            | MirStorageInit::RoutineAddress {
                descriptor_size, ..
            } => Some(*descriptor_size),
            _ => None,
        })
        .unwrap_or(slot.storage_size)
}

fn storage_init_object_size(init: &MirStorageInit, storage_size: u16) -> u16 {
    match init {
        MirStorageInit::Bytes {
            image, zero_fill, ..
        } => (image.bytes.len() as u16)
            .saturating_add(*zero_fill)
            .max(storage_size),
        MirStorageInit::ZeroFill { bytes, .. } => (*bytes).max(storage_size),
        MirStorageInit::Descriptor {
            backing,
            descriptor_size,
            ..
        } => (backing.image.bytes.len() as u16)
            .saturating_add(backing.zero_fill)
            .saturating_add(*descriptor_size)
            .max(storage_size),
        MirStorageInit::RoutineAddress {
            descriptor_size, ..
        } => (*descriptor_size).max(storage_size),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirBlockId, MirEffects, MirFrame, MirRoutine, MirRoutineAbi, MirStorageClass,
        MirStorageId, MirTerminator,
    };

    #[test]
    fn address_only_slots_reserve_their_full_size_and_bound_aliases() {
        let real = MirStorageSlot {
            id: MirStorageId(0),
            name: Some("real".to_string()),
            storage: MirStorageClass::Scalar,
            storage_size: 6,
            scalar_width: None,
            base: MirStorageBase::Local(LocalId(0)),
            offset: 0,
            mutable: true,
            init: None,
        };
        let tail = MirStorageSlot {
            id: MirStorageId(4),
            name: Some("tail".to_string()),
            storage: MirStorageClass::Scalar,
            storage_size: 1,
            scalar_width: Some(MirWidth::Byte),
            base: MirStorageBase::Local(LocalId(4)),
            offset: 0,
            mutable: true,
            init: None,
        };
        let record = MirStorageSlot {
            id: MirStorageId(1),
            name: Some("record".to_string()),
            storage: MirStorageClass::Record,
            storage_size: 4,
            scalar_width: None,
            base: MirStorageBase::Local(LocalId(1)),
            offset: 0,
            mutable: true,
            init: Some(MirStorageInit::ZeroFill {
                bytes: 4,
                mutable: true,
                section: "local".to_string(),
            }),
        };
        let array = MirStorageSlot {
            id: MirStorageId(2),
            name: Some("array".to_string()),
            storage: MirStorageClass::Array,
            storage_size: 3,
            scalar_width: None,
            base: MirStorageBase::Local(LocalId(2)),
            offset: 0,
            mutable: true,
            init: Some(MirStorageInit::Bytes {
                image: crate::mir6502::MirDataImage {
                    bytes: vec![1, 2],
                    relocations: Vec::new(),
                },
                zero_fill: 1,
                mutable: true,
                section: "local".to_string(),
            }),
        };
        let word = MirStorageSlot {
            id: MirStorageId(3),
            name: Some("word".to_string()),
            storage: MirStorageClass::Scalar,
            storage_size: 2,
            scalar_width: Some(MirWidth::Word),
            base: MirStorageBase::Local(LocalId(3)),
            offset: 0,
            mutable: true,
            init: None,
        };
        let high_word_alias = MirStorageSlot {
            id: MirStorageId(5),
            name: Some("high_word".to_string()),
            storage: MirStorageClass::Scalar,
            storage_size: 2,
            scalar_width: Some(MirWidth::Word),
            base: MirStorageBase::LocalAlias {
                id: LocalId(5),
                target: LocalId(0),
            },
            offset: 4,
            mutable: true,
            init: None,
        };
        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![MirRoutine {
                id: RoutineId(0),
                name: "Main".to_string(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame {
                    locals: vec![real, record, array, word, tail, high_word_alias],
                    ..MirFrame::default()
                },
                temps: Vec::new(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                }],
                effects: MirEffects::default(),
            }],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };

        crate::mir6502::verify_program(&program, crate::mir6502::MirPhase::PreMaterialization)
            .expect("address-only local storage is verifier-clean");
        let layout = MaterializeLayout::new(&program, 0x2000);
        assert_eq!(
            layout.mem_address(
                RoutineId(0),
                &MirMem::Local {
                    id: LocalId(0),
                    offset: 5,
                },
            ),
            Some(0x2005)
        );
        assert_eq!(
            layout.mem_address(
                RoutineId(0),
                &MirMem::Local {
                    id: LocalId(0),
                    offset: 6,
                },
            ),
            None
        );
        assert_eq!(
            layout.mem_address(
                RoutineId(0),
                &MirMem::Local {
                    id: LocalId(1),
                    offset: 0,
                },
            ),
            Some(0x2006)
        );
        assert_eq!(
            layout.mem_address(
                RoutineId(0),
                &MirMem::Local {
                    id: LocalId(2),
                    offset: 2,
                },
            ),
            Some(0x200C)
        );
        assert_eq!(
            layout.mem_address(
                RoutineId(0),
                &MirMem::Local {
                    id: LocalId(3),
                    offset: 1,
                },
            ),
            Some(0x200E)
        );
        assert_eq!(
            layout.mem_address(
                RoutineId(0),
                &MirMem::Local {
                    id: LocalId(4),
                    offset: 0,
                },
            ),
            Some(0x200F)
        );
        assert_eq!(
            layout.mem_address(
                RoutineId(0),
                &MirMem::Local {
                    id: LocalId(5),
                    offset: 1,
                },
            ),
            Some(0x2005)
        );
        assert!(layout.is_byte_scalar_storage(
            RoutineId(0),
            &MirMem::Local {
                id: LocalId(4),
                offset: 0,
            }
        ));
        assert!(!layout.is_byte_scalar_storage(
            RoutineId(0),
            &MirMem::Local {
                id: LocalId(0),
                offset: 0,
            }
        ));
    }
}
