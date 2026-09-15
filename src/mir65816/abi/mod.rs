//! Physical `action65816.native.v1` layouts, derived from typed NIR facts.
pub mod generated;

use crate::nir::{NirCallConvention, NirCallableSignature, NirIntegerRole, NirType, NirTypeKind};
use crate::target::{ByteOffset, ByteSize, TargetLayout};
use generated::*;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarClass {
    Byte,
    Word,
    AddressOrSize,
    DataPointer,
    CodePointer,
    LongInteger,
}

/// Exact result bits. All unspecified registers/flags are caller-clobbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultLocation {
    A8ZeroExtended,
    A16,
    A16X8ZeroExtended,
    A16X16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarLayout {
    pub class: ScalarClass,
    pub size: ByteSize,
    pub alignment: ByteSize,
    pub result: ResultLocation,
}

impl ScalarClass {
    pub const fn layout(self) -> ScalarLayout {
        let (size, alignment, result) = match self {
            Self::Byte => (
                SCALAR_BYTE_SIZE,
                SCALAR_BYTE_ALIGNMENT,
                ResultLocation::A8ZeroExtended,
            ),
            Self::Word => (SCALAR_WORD_SIZE, SCALAR_WORD_ALIGNMENT, ResultLocation::A16),
            Self::AddressOrSize => (
                SCALAR_ADDRESS_OR_SIZE_SIZE,
                SCALAR_ADDRESS_OR_SIZE_ALIGNMENT,
                ResultLocation::A16X8ZeroExtended,
            ),
            Self::DataPointer => (
                SCALAR_DATA_POINTER_SIZE,
                SCALAR_DATA_POINTER_ALIGNMENT,
                ResultLocation::A16X8ZeroExtended,
            ),
            Self::CodePointer => (
                SCALAR_CODE_POINTER_SIZE,
                SCALAR_CODE_POINTER_ALIGNMENT,
                ResultLocation::A16X8ZeroExtended,
            ),
            Self::LongInteger => (
                SCALAR_LONG_INTEGER_SIZE,
                SCALAR_LONG_INTEGER_ALIGNMENT,
                ResultLocation::A16X16,
            ),
        };
        ScalarLayout {
            class: self,
            size: ByteSize::new(size),
            alignment: ByteSize::new(alignment),
            result,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiError {
    UnsupportedType,
    WidthMismatch {
        expected: ByteSize,
        actual: Option<ByteSize>,
    },
    ExternalConvention,
    VariadicSignature,
    ExtentOverflow,
}

impl fmt::Display for AbiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedType => {
                formatter.write_str("type is outside the native 65816 v1 scalar ABI")
            }
            Self::WidthMismatch { expected, actual } => write!(
                formatter,
                "native 65816 ABI width mismatch: expected {expected:?}, found {actual:?}"
            ),
            Self::ExternalConvention => {
                formatter.write_str("external calling convention requires a native 65816 adapter")
            }
            Self::VariadicSignature => {
                formatter.write_str("variadic calls are outside the native 65816 v1 ABI")
            }
            Self::ExtentOverflow => {
                formatter.write_str("native 65816 argument extent exceeds bank-zero stack capacity")
            }
        }
    }
}

impl std::error::Error for AbiError {}

pub fn classify(ty: &NirType) -> Result<ScalarLayout, AbiError> {
    let class = match &ty.kind {
        NirTypeKind::Bool => ScalarClass::Byte,
        NirTypeKind::Integer(integer) => match (integer.role, integer.bits, integer.signed) {
            (NirIntegerRole::Ordinary, 8, false) => ScalarClass::Byte,
            (NirIntegerRole::Ordinary, 16, _) => ScalarClass::Word,
            (NirIntegerRole::Ordinary, 32, _) => ScalarClass::LongInteger,
            (NirIntegerRole::Address | NirIntegerRole::Size, 24, false) => {
                ScalarClass::AddressOrSize
            }
            _ => return Err(AbiError::UnsupportedType),
        },
        NirTypeKind::Pointer { address_space, .. }
            if *address_space == TargetLayout::DATA_ADDRESS_SPACE =>
        {
            ScalarClass::DataPointer
        }
        NirTypeKind::Callable { address_space, .. }
            if *address_space == TargetLayout::CODE_ADDRESS_SPACE =>
        {
            ScalarClass::CodePointer
        }
        _ => return Err(AbiError::UnsupportedType),
    };
    let layout = class.layout();
    if ty.width != Some(layout.size) {
        return Err(AbiError::WidthMismatch {
            expected: layout.size,
            actual: ty.width,
        });
    }
    Ok(layout)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argument {
    pub offset: ByteOffset,
    pub scalar: ScalarLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallLayout {
    pub arguments: Vec<Argument>,
    pub payload_bytes: ByteSize,
    pub outgoing_bytes: ByteSize,
    pub result: Option<ResultLocation>,
}

pub fn call_layout(signature: &NirCallableSignature) -> Result<CallLayout, AbiError> {
    if matches!(signature.convention, NirCallConvention::External(_)) {
        return Err(AbiError::ExternalConvention);
    }
    if signature.variadic.is_some() {
        return Err(AbiError::VariadicSignature);
    }
    let result = signature
        .result
        .as_ref()
        .map(classify)
        .transpose()?
        .map(|scalar| scalar.result);
    let mut cursor = 0u32;
    let mut arguments = Vec::with_capacity(signature.params.len());
    for ty in &signature.params {
        let scalar = classify(ty)?;
        let offset = align_up(cursor, scalar.alignment.get()).ok_or(AbiError::ExtentOverflow)?;
        cursor = offset
            .checked_add(scalar.size.get())
            .ok_or(AbiError::ExtentOverflow)?;
        if cursor > u32::from(u16::MAX) - CALL_RETURN_ADDRESS_BYTES {
            return Err(AbiError::ExtentOverflow);
        }
        arguments.push(Argument {
            offset: ByteOffset::new(offset),
            scalar,
        });
    }
    // This also gives a no-argument call its required alignment byte.
    let outgoing = cursor | 1;
    if outgoing > u32::from(u16::MAX) - CALL_RETURN_ADDRESS_BYTES {
        return Err(AbiError::ExtentOverflow);
    }
    Ok(CallLayout {
        arguments,
        payload_bytes: ByteSize::new(cursor),
        outgoing_bytes: ByteSize::new(outgoing),
        result,
    })
}

pub(super) fn align_up(value: u32, alignment: u32) -> Option<u32> {
    if !alignment.is_power_of_two() {
        return None;
    }
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

#[cfg(test)]
mod tests;
