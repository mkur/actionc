use crate::ast::RoutineKind;
use crate::semantic::{CallableType, ScalarType, ValueType, ValueTypeBase, ValueTypeKind};
use crate::target::{AddressSpaceId, ByteSize, TargetLayout};

use super::ir::{NirCallConvention, NirPlace, NirPlaceKind};

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
    pub width: Option<ByteSize>,
    pub pointer: bool,
}

impl NirType {
    pub(super) fn from_value(value: &ValueType) -> Self {
        Self::from_value_with_layout(value, TargetLayout::default())
    }

    pub(super) fn from_value_with_layout(value: &ValueType, layout: TargetLayout) -> Self {
        let kind = NirTypeKind::from_value(value);
        let width = kind.width(layout);
        Self {
            kind,
            summary: type_summary(value),
            width,
            pointer: value.pointer,
        }
    }

    pub(super) fn apply_target_layout(&mut self, layout: TargetLayout) {
        self.kind.apply_target_layout(layout);
        self.width = self.kind.width(layout);
        self.pointer = self.kind.is_pointer();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirTypeKind {
    Void,
    Bool,
    Integer(NirIntegerType),
    Real,
    Pointer {
        pointee: Option<Box<NirTypeKind>>,
        address_space: AddressSpaceId,
    },
    Record {
        name: String,
        size: Option<ByteSize>,
    },
    Callable {
        kind: String,
        signature: SignatureId,
        convention: NirCallConvention,
        address_space: AddressSpaceId,
    },
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirIntegerRole {
    Ordinary,
    Address,
    Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NirIntegerType {
    pub bits: u8,
    pub signed: bool,
    pub role: NirIntegerRole,
}

impl NirIntegerType {
    pub const U8: Self = Self::ordinary(8, false);
    pub const I8: Self = Self::ordinary(8, true);
    pub const U16: Self = Self::ordinary(16, false);
    pub const I16: Self = Self::ordinary(16, true);
    pub const U32: Self = Self::ordinary(32, false);
    pub const I32: Self = Self::ordinary(32, true);

    pub const fn ordinary(bits: u8, signed: bool) -> Self {
        Self {
            bits,
            signed,
            role: NirIntegerRole::Ordinary,
        }
    }

    pub const fn storage_width(self) -> ByteSize {
        ByteSize::new((self.bits as u32).div_ceil(8))
    }

    pub const fn mask(self) -> u64 {
        if self.bits >= 64 {
            u64::MAX
        } else if self.bits == 0 {
            0
        } else {
            (1u64 << self.bits) - 1
        }
    }
}

impl NirTypeKind {
    #[allow(non_upper_case_globals)]
    pub const U8: Self = Self::Integer(NirIntegerType::U8);
    #[allow(non_upper_case_globals)]
    pub const I8: Self = Self::Integer(NirIntegerType::I8);
    #[allow(non_upper_case_globals)]
    pub const U16: Self = Self::Integer(NirIntegerType::U16);
    #[allow(non_upper_case_globals)]
    pub const I16: Self = Self::Integer(NirIntegerType::I16);

    pub(super) fn from_value(value: &ValueType) -> Self {
        match value.kind() {
            ValueTypeKind::Scalar(scalar) => Self::from_scalar(scalar),
            ValueTypeKind::Real => Self::Real,
            ValueTypeKind::Pointer(pointer) => Self::Pointer {
                pointee: Some(Box::new(Self::from_value(&pointer.pointee))),
                address_space: TargetLayout::DATA_ADDRESS_SPACE,
            },
            ValueTypeKind::CallablePointer(callable) => Self::Callable {
                kind: format!("{:?}", callable.kind),
                signature: signature_id(&callable, NirCallConvention::TargetPublic),
                convention: NirCallConvention::TargetPublic,
                address_space: TargetLayout::CODE_ADDRESS_SPACE,
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
            ScalarType::Long => Self::Integer(NirIntegerType::I32),
            ScalarType::ULong => Self::Integer(NirIntegerType::U32),
        }
    }

    pub(super) fn width(&self, layout: TargetLayout) -> Option<ByteSize> {
        match self {
            Self::Void => Some(ByteSize::ZERO),
            Self::Bool => Some(ByteSize::new(1)),
            Self::Integer(integer) => Some(integer.storage_width()),
            Self::Pointer { address_space, .. }
                if *address_space == layout.data_pointer.address_space =>
            {
                Some(layout.data_pointer.size_bytes)
            }
            Self::Callable { address_space, .. }
                if *address_space == layout.code_pointer.address_space =>
            {
                Some(layout.code_pointer.size_bytes)
            }
            Self::Pointer { .. } | Self::Callable { .. } => None,
            Self::Real => Some(ByteSize::new(6)),
            Self::Record { size, .. } => *size,
            Self::Error => None,
        }
    }

    pub(super) fn is_pointer(&self) -> bool {
        matches!(self, Self::Pointer { .. })
    }

    pub(super) fn is_address(&self) -> bool {
        matches!(self, Self::Pointer { .. } | Self::Callable { .. })
    }

    pub fn integer(&self) -> Option<NirIntegerType> {
        match self {
            Self::Integer(integer) => Some(*integer),
            _ => None,
        }
    }

    fn apply_target_layout(&mut self, layout: TargetLayout) {
        match self {
            Self::Pointer {
                pointee,
                address_space,
            } => {
                *address_space = layout.data_pointer.address_space;
                if let Some(pointee) = pointee {
                    pointee.apply_target_layout(layout);
                }
            }
            Self::Callable { address_space, .. } => {
                *address_space = layout.code_pointer.address_space;
            }
            _ => {}
        }
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

/// Stable identity for a routine within one verified NIR program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoutineId(pub u32);

impl std::fmt::Display for RoutineId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Stable structural identity for an Action! callable signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignatureId(pub u32);

/// Stable identity for a compiler/runtime service. The readable name is debug
/// metadata; calls and late bindings use this identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeSymbolId(pub u32);

pub fn runtime_symbol_id(name: &str) -> RuntimeSymbolId {
    let mut hash = 2_166_136_261u32;
    for byte in name.bytes() {
        hash ^= u32::from(byte.to_ascii_uppercase());
        hash = hash.wrapping_mul(16_777_619);
    }
    RuntimeSymbolId(hash)
}

pub(super) fn signature_id(callable: &CallableType, convention: NirCallConvention) -> SignatureId {
    fn byte(hash: &mut u32, value: u8) {
        *hash ^= u32::from(value);
        *hash = hash.wrapping_mul(16_777_619);
    }
    fn text(hash: &mut u32, value: &str) {
        for value in value.bytes() {
            byte(hash, value.to_ascii_uppercase());
        }
        byte(hash, 0xFF);
    }
    fn value_type(hash: &mut u32, value: &ValueType) {
        byte(hash, u8::from(value.pointer));
        match &value.base {
            ValueTypeBase::Fund(fund) => {
                byte(hash, 1);
                text(hash, &format!("{fund:?}"));
            }
            ValueTypeBase::Real => byte(hash, 2),
            ValueTypeBase::Named(name) => {
                byte(hash, 3);
                text(hash, name);
            }
            ValueTypeBase::Callable(callable) => {
                byte(hash, 4);
                callable_type(hash, callable);
            }
            ValueTypeBase::Error => byte(hash, 5),
        }
    }
    fn callable_type(hash: &mut u32, callable: &CallableType) {
        match callable.kind {
            RoutineKind::Proc => byte(hash, 1),
            RoutineKind::Func { return_type } => {
                byte(hash, 2);
                text(hash, &format!("{return_type:?}"));
            }
        }
        for param in &callable.params {
            byte(hash, 0x10);
            value_type(hash, param);
        }
        if let Some(variadic) = &callable.variadic {
            byte(hash, 0x20);
            value_type(hash, variadic);
        }
        if let Some(result) = &callable.return_type {
            byte(hash, 0x30);
            value_type(hash, result);
        }
        byte(hash, 0);
    }

    let mut hash = 2_166_136_261;
    byte(&mut hash, convention.identity_byte());
    if let NirCallConvention::External(id) = convention {
        for byte_value in id.0.to_le_bytes() {
            byte(&mut hash, byte_value);
        }
    }
    callable_type(&mut hash, callable);
    SignatureId(hash)
}

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
        NirPlaceKind::Absolute(_)
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
    IntegerConst {
        bits: u64,
        ty: NirIntegerType,
    },
    Null {
        ty: NirType,
    },
    AddressConst {
        address: crate::target::AddressValue,
        ty: NirType,
    },
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
    RoutineAddr {
        id: RoutineId,
        name: String,
        ty: NirType,
    },
}

impl NirValue {
    #[allow(non_snake_case)]
    pub fn ConstU8(value: u8) -> Self {
        Self::IntegerConst {
            bits: u64::from(value),
            ty: NirIntegerType::U8,
        }
    }

    #[allow(non_snake_case)]
    pub fn ConstU16(value: u16) -> Self {
        Self::IntegerConst {
            bits: u64::from(value),
            ty: NirIntegerType::U16,
        }
    }

    pub fn integer_const(bits: u64, ty: NirIntegerType) -> Self {
        Self::IntegerConst {
            bits: bits & ty.mask(),
            ty,
        }
    }

    pub fn as_integer_const(&self) -> Option<(u64, NirIntegerType)> {
        match self {
            Self::IntegerConst { bits, ty } => Some((*bits, *ty)),
            _ => None,
        }
    }

    pub(super) fn temp(&self) -> Option<TempId> {
        match self {
            Self::Temp { id, .. } => Some(*id),
            Self::IntegerConst { .. }
            | Self::Null { .. }
            | Self::AddressConst { .. }
            | Self::StaticAddr { .. }
            | Self::Param(_)
            | Self::GlobalAddr(_)
            | Self::RoutineAddr { .. } => None,
        }
    }
}

pub(super) fn type_summary(ty: &ValueType) -> String {
    let base = match &ty.base {
        ValueTypeBase::Fund(fund) => format!("{fund:?}"),
        ValueTypeBase::Real => "REAL".to_string(),
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
        width: Some(ByteSize::new(1)),
        pointer: false,
    }
}

pub(super) fn value_width(value: &NirValue) -> Option<ByteSize> {
    match value {
        NirValue::IntegerConst { ty, .. } => Some(ty.storage_width()),
        NirValue::Null { ty }
        | NirValue::AddressConst { ty, .. }
        | NirValue::StaticAddr { ty, .. }
        | NirValue::Temp { ty, .. }
        | NirValue::RoutineAddr { ty, .. } => ty.width,
        NirValue::Param(_) | NirValue::GlobalAddr(_) => None,
    }
}

pub(super) fn value_is_oversized_literal(value: &NirValue, width: ByteSize) -> bool {
    let NirValue::IntegerConst { bits, .. } = value else {
        return false;
    };
    let width_bits = width.get().saturating_mul(8);
    width_bits == 0 || (width_bits < 64 && *bits > (1u64 << width_bits) - 1)
}
