use super::values::offset_mem;
use crate::mir6502::ir::{MirAddr, MirDef, MirTempId, MirValue};

pub(super) fn replace_temp_value(
    value: MirValue,
    temp: MirTempId,
    replacement: &MirValue,
) -> MirValue {
    match value {
        MirValue::Def(MirDef::VTemp(id)) if id == temp => replacement.clone(),
        MirValue::Def(MirDef::VTempByte { id, byte }) if id == temp => {
            replacement_byte_value(replacement, byte)
        }
        MirValue::Word { lo, hi } => MirValue::Word {
            lo: Box::new(replace_temp_value(*lo, temp, replacement)),
            hi: Box::new(replace_temp_value(*hi, temp, replacement)),
        },
        other => other,
    }
}

pub(super) fn replace_temp_addr(addr: MirAddr, temp: MirTempId, replacement: &MirValue) -> MirAddr {
    match addr {
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset,
        } => MirAddr::ComputedIndex {
            base: replace_temp_value(base, temp, replacement),
            index: replace_temp_value(index, temp, replacement),
            elem_size,
            offset,
        },
        MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset,
        } => MirAddr::PointerIndex {
            ptr,
            index: replace_temp_value(index, temp, replacement),
            elem_size,
            offset,
        },
        MirAddr::Deref { ptr, offset } => MirAddr::Deref {
            ptr: replace_temp_value(ptr, temp, replacement),
            offset,
        },
        other => other,
    }
}

fn replacement_byte_value(replacement: &MirValue, byte: u8) -> MirValue {
    match (replacement, byte) {
        (MirValue::ConstU8(value), 0) => MirValue::ConstU8(*value),
        (MirValue::ConstU8(_), _) => MirValue::ConstU8(0),
        (MirValue::ConstU16(value), 0) => MirValue::ConstU8(*value as u8),
        (MirValue::ConstU16(value), 1) => MirValue::ConstU8((value >> 8) as u8),
        (MirValue::ConstU16(_), _) => MirValue::ConstU8(0),
        (MirValue::Word { lo, .. }, 0) => lo.as_ref().clone(),
        (MirValue::Word { hi, .. }, 1) => hi.as_ref().clone(),
        (MirValue::Word { .. }, _) => MirValue::ConstU8(0),
        (MirValue::PointerCell(mem), 0) => MirValue::PointerCell(mem.clone()),
        (MirValue::PointerCell(mem), 1) => MirValue::PointerCell(offset_mem(mem, 1)),
        _ => replacement.clone(),
    }
}
