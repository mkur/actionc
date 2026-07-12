use super::layout::MaterializeLayout;
use crate::codegen::runtime_zp;
use crate::mir6502::ir::{
    MirDef, MirFixedZpSlot, MirMem, MirTempId, MirValue, MirWidth, RoutineId,
};
use std::collections::BTreeMap;

pub(super) fn split_def(def: MirDef) -> Option<(MirDef, MirDef)> {
    match def {
        MirDef::VTemp(id) => Some((
            MirDef::VTempByte { id, byte: 0 },
            MirDef::VTempByte { id, byte: 1 },
        )),
        MirDef::VTempByte { .. } | MirDef::Reg(_) => None,
    }
}

pub(super) fn split_value(value: MirValue, _layout: &MaterializeLayout) -> (MirValue, MirValue) {
    match value {
        MirValue::ConstU8(value) => (MirValue::ConstU8(value), MirValue::ConstU8(0)),
        MirValue::ConstU16(value) => (
            MirValue::ConstU8((value & 0x00FF) as u8),
            MirValue::ConstU8((value >> 8) as u8),
        ),
        MirValue::Def(MirDef::VTemp(id)) => (
            MirValue::Def(MirDef::VTempByte { id, byte: 0 }),
            MirValue::Def(MirDef::VTempByte { id, byte: 1 }),
        ),
        MirValue::Word { lo, hi } => (*lo, *hi),
        MirValue::StaticAddr(id) => split_storage_address(MirMem::Static { id, offset: 0 }),
        MirValue::GlobalAddr(id) => split_storage_address(MirMem::Global { id, offset: 0 }),
        MirValue::RoutineAddr(id) => split_routine_address(id),
        MirValue::PointerCell(mem) => (
            MirValue::PointerCell(mem.clone()),
            MirValue::PointerCell(offset_mem(&mem, 1)),
        ),
        other => (other.clone(), other),
    }
}

pub(super) fn split_value_with_temp_widths(
    value: MirValue,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
) -> (MirValue, MirValue) {
    match &value {
        MirValue::Def(MirDef::VTemp(id)) if temp_widths.get(id) == Some(&MirWidth::Byte) => {
            (value, MirValue::ConstU8(0))
        }
        _ => split_value(value, layout),
    }
}

pub(super) fn split_value_with_storage_widths(
    value: MirValue,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
) -> (MirValue, MirValue) {
    match &value {
        MirValue::PointerCell(mem) if layout.is_byte_scalar_storage(routine_id, mem) => {
            (value, MirValue::ConstU8(0))
        }
        _ => split_value_with_temp_widths(value, layout, temp_widths),
    }
}

pub(super) fn split_value_as_word(
    value: MirValue,
    layout: &MaterializeLayout,
) -> (MirValue, MirValue) {
    match value {
        MirValue::Word { lo, hi } => (*lo, *hi),
        _ => split_value(value, layout),
    }
}

pub(super) fn split_address(address: u16) -> (MirValue, MirValue) {
    (
        MirValue::ConstU8((address & 0x00FF) as u8),
        MirValue::ConstU8((address >> 8) as u8),
    )
}

fn split_storage_address(mem: MirMem) -> (MirValue, MirValue) {
    (
        MirValue::StorageAddrByte {
            mem: mem.clone(),
            byte: 0,
        },
        MirValue::StorageAddrByte { mem, byte: 1 },
    )
}

pub(super) fn split_routine_address(id: RoutineId) -> (MirValue, MirValue) {
    (
        MirValue::RoutineAddrByte { id, byte: 0 },
        MirValue::RoutineAddrByte { id, byte: 1 },
    )
}

pub(super) fn offset_mem(mem: &MirMem, delta: u16) -> MirMem {
    match mem {
        MirMem::Absolute(address) => MirMem::Absolute(address.saturating_add(delta)),
        MirMem::Static { id, offset } => MirMem::Static {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::Global { id, offset } => MirMem::Global {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::Local { id, offset } => MirMem::Local {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::Param { id, offset } => MirMem::Param {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::Spill { id, offset } => MirMem::Spill {
            id: *id,
            offset: offset.saturating_add(delta),
        },
        MirMem::ZeroPage(slot) => MirMem::ZeroPage(*slot),
        MirMem::FixedZeroPage(slot) => {
            MirMem::FixedZeroPage(MirFixedZpSlot(slot.0.saturating_add(delta as u8)))
        }
    }
}

pub(super) fn return_slot_mem(offset: u16) -> MirMem {
    MirMem::FixedZeroPage(MirFixedZpSlot(
        runtime_zp::ARGS.address().wrapping_add(offset as u8),
    ))
}
