use crate::semantic::{ScalarType, ValueType, ValueTypeBase, ValueTypeKind};

use super::ir::{NirOperand, NirOperandKind, NirPlace, NirPlaceKind};

pub(super) struct NirFacts;

impl NirFacts {
    pub(super) fn type_from_value(value: &ValueType) -> NirType {
        NirType::from_value(value)
    }

    pub(super) fn condition_type() -> NirType {
        condition_type()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirType {
    pub kind: NirTypeKind,
    pub summary: String,
    pub width: Option<u16>,
    pub pointer: bool,
}

impl NirType {
    pub(super) fn from_value(value: &ValueType) -> Self {
        let kind = NirTypeKind::from_value(value);
        Self {
            kind,
            summary: type_summary(value),
            width: value.value_width_bytes(),
            pointer: value.pointer,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirTypeKind {
    Void,
    Bool,
    U8,
    I8,
    U16,
    I16,
    Ptr16 { pointee: Option<Box<NirTypeKind>> },
    Record { name: String, size: Option<u16> },
    Callable { kind: String },
    Error,
}

impl NirTypeKind {
    pub(super) fn from_value(value: &ValueType) -> Self {
        match value.kind() {
            ValueTypeKind::Scalar(scalar) => Self::from_scalar(scalar),
            ValueTypeKind::Pointer(pointer) => Self::Ptr16 {
                pointee: Some(Box::new(Self::from_value(&pointer.pointee))),
            },
            ValueTypeKind::CallablePointer(callable) => Self::Callable {
                kind: format!("{:?}", callable.kind),
            },
            ValueTypeKind::Record(name) => Self::Record { name, size: None },
            ValueTypeKind::Error => Self::Error,
        }
    }

    fn from_scalar(scalar: ScalarType) -> Self {
        match scalar {
            ScalarType::Byte | ScalarType::Char => Self::U8,
            ScalarType::Card => Self::U16,
            ScalarType::Int => Self::I16,
        }
    }

    pub(super) fn width(&self) -> Option<u16> {
        match self {
            Self::Void => Some(0),
            Self::Bool | Self::U8 | Self::I8 => Some(1),
            Self::U16 | Self::I16 | Self::Ptr16 { .. } | Self::Callable { .. } => Some(2),
            Self::Record { size, .. } => *size,
            Self::Error => None,
        }
    }

    pub(super) fn is_pointer(&self) -> bool {
        matches!(self, Self::Ptr16 { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TempId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(pub u32);

/// Stable identity for storage that can be named exactly by a direct NIR place.
///
/// Absolute addresses, dereferences, indexed places, and fields deliberately do
/// not have a `NirStorageId`: they may alias other storage and need a richer
/// region model before storage-value propagation can reason about them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirStorageId {
    Local(LocalId),
    Param(ParamId),
    Global(SymbolId),
}

pub fn direct_storage_id(place: &NirPlace) -> Option<NirStorageId> {
    match place.kind {
        NirPlaceKind::Local { id, .. } => Some(NirStorageId::Local(id)),
        NirPlaceKind::Param { id, .. } => Some(NirStorageId::Param(id)),
        NirPlaceKind::Global { id, .. } => Some(NirStorageId::Global(id)),
        NirPlaceKind::Symbol(_)
        | NirPlaceKind::Absolute(_)
        | NirPlaceKind::UnresolvedName(_)
        | NirPlaceKind::Deref { .. }
        | NirPlaceKind::Index { .. }
        | NirPlaceKind::Field { .. } => None,
    }
}

pub(super) fn root_storage_id(place: &NirPlace) -> Option<NirStorageId> {
    match &place.kind {
        NirPlaceKind::Field { base, .. } => root_storage_id(base),
        _ => direct_storage_id(place),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirValue {
    ConstU8(u8),
    ConstU16(u16),
    StaticAddr {
        id: SymbolId,
        name: String,
        ty: NirType,
    },
    Temp {
        id: TempId,
        ty: NirType,
    },
    Param(ParamId),
    GlobalAddr(SymbolId),
}

impl NirValue {
    pub(super) fn from_legacy_operand(operand: &NirOperand) -> Option<Self> {
        match &operand.kind {
            NirOperandKind::Literal {
                value: Some(value), ..
            } if operand.ty.as_ref().and_then(|ty| ty.kind.width()) == Some(1) => {
                u8::try_from(*value).ok().map(Self::ConstU8)
            }
            NirOperandKind::Literal {
                value: Some(value), ..
            } if operand.ty.as_ref().and_then(|ty| ty.kind.width()) == Some(2) => {
                Some(Self::ConstU16(*value))
            }
            NirOperandKind::Temp(id) => operand.ty.clone().map(|ty| Self::Temp { id: *id, ty }),
            _ => None,
        }
    }

    pub(super) fn temp(&self) -> Option<TempId> {
        match self {
            Self::Temp { id, .. } => Some(*id),
            Self::ConstU8(_)
            | Self::ConstU16(_)
            | Self::StaticAddr { .. }
            | Self::Param(_)
            | Self::GlobalAddr(_) => None,
        }
    }
}

pub(super) fn type_summary(ty: &ValueType) -> String {
    let base = match &ty.base {
        ValueTypeBase::Fund(fund) => format!("{fund:?}"),
        ValueTypeBase::Named(name) => name.clone(),
        ValueTypeBase::Callable(callable) => format!("{:?}", callable.kind),
        ValueTypeBase::Error => "error".to_string(),
    };
    if ty.pointer { format!("{base}*") } else { base }
}

pub(super) fn condition_type() -> NirType {
    NirType {
        kind: NirTypeKind::Bool,
        summary: "condition".to_string(),
        width: Some(1),
        pointer: false,
    }
}

pub(super) fn value_width(value: &NirValue) -> Option<u16> {
    match value {
        NirValue::ConstU8(_) => Some(1),
        NirValue::ConstU16(_) => Some(2),
        NirValue::StaticAddr { ty, .. } | NirValue::Temp { ty, .. } => ty.width,
        NirValue::Param(_) | NirValue::GlobalAddr(_) => None,
    }
}

pub(super) fn value_is_oversized_literal(value: &NirValue, width: u16) -> bool {
    let NirValue::ConstU16(value) = value else {
        return false;
    };
    match width {
        0 => true,
        1 => *value > 0x00FF,
        2 => false,
        _ => false,
    }
}
