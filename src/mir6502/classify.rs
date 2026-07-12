use crate::nir::{NirPlace, NirPlaceKind, NirValue};

use super::ir::MirMem;
use crate::nir::SymbolId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MirPlaceShape {
    DirectMemory(MirMem),
    AbsoluteMemory(u16),
    PointerDeref {
        addr: NirValue,
        offset: u16,
    },
    IndexedElement {
        base_addr: NirValue,
        index: NirValue,
        elem_size: u16,
    },
    RecordField {
        base: Box<NirPlace>,
        offset: u16,
    },
    Unsupported(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MirValueShape {
    ConstByte(u8),
    ConstWord(u16),
    Temp(crate::nir::TempId),
    StaticAddress(SymbolId),
    GlobalAddress(SymbolId),
    ParamValue(crate::nir::ParamId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MirAddressShape {
    Direct(MirMem),
    Absolute(u16),
    Static(SymbolId),
    Global(SymbolId),
    Unsupported(&'static str),
}

pub(super) fn classify_place(place: &NirPlace) -> MirPlaceShape {
    match &place.kind {
        NirPlaceKind::Param { id, .. } => {
            MirPlaceShape::DirectMemory(MirMem::Param { id: *id, offset: 0 })
        }
        NirPlaceKind::Local { id, .. } => {
            MirPlaceShape::DirectMemory(MirMem::Local { id: *id, offset: 0 })
        }
        NirPlaceKind::Global { id, .. } => {
            MirPlaceShape::DirectMemory(MirMem::Global { id: *id, offset: 0 })
        }
        NirPlaceKind::Absolute(address) => MirPlaceShape::AbsoluteMemory(*address),
        NirPlaceKind::Deref { addr } => MirPlaceShape::PointerDeref {
            addr: addr.clone(),
            offset: 0,
        },
        NirPlaceKind::Index {
            base_addr,
            index,
            elem_size,
            ..
        } => MirPlaceShape::IndexedElement {
            base_addr: base_addr.clone(),
            index: index.clone(),
            elem_size: *elem_size,
        },
        NirPlaceKind::Field { base, offset, .. } => MirPlaceShape::RecordField {
            base: base.clone(),
            offset: *offset,
        },
        NirPlaceKind::Symbol(_) | NirPlaceKind::UnresolvedName(_) => {
            MirPlaceShape::Unsupported("unresolved storage")
        }
    }
}

pub(super) fn classify_value(value: &NirValue) -> MirValueShape {
    match value {
        NirValue::ConstU8(value) => MirValueShape::ConstByte(*value),
        NirValue::ConstU16(value) => MirValueShape::ConstWord(*value),
        NirValue::Temp { id, .. } => MirValueShape::Temp(*id),
        NirValue::StaticAddr { id, .. } => MirValueShape::StaticAddress(*id),
        NirValue::GlobalAddr(id) => MirValueShape::GlobalAddress(*id),
        NirValue::Param(id) => MirValueShape::ParamValue(*id),
    }
}

pub(super) fn classify_address(place: &NirPlace) -> MirAddressShape {
    match classify_place(place) {
        MirPlaceShape::DirectMemory(mem @ MirMem::Global { id, .. }) => {
            let _ = mem;
            MirAddressShape::Global(id)
        }
        MirPlaceShape::DirectMemory(mem @ MirMem::Static { id, .. }) => {
            let _ = mem;
            MirAddressShape::Static(id)
        }
        MirPlaceShape::DirectMemory(mem) => MirAddressShape::Direct(mem),
        MirPlaceShape::AbsoluteMemory(address) => MirAddressShape::Absolute(address),
        MirPlaceShape::PointerDeref { .. } => MirAddressShape::Unsupported("pointer deref address"),
        MirPlaceShape::IndexedElement { .. } => {
            MirAddressShape::Unsupported("indexed element address")
        }
        MirPlaceShape::RecordField { .. } => MirAddressShape::Unsupported("record field address"),
        MirPlaceShape::Unsupported(reason) => MirAddressShape::Unsupported(reason),
    }
}
