//! Physical `action65816.native.v1` layouts, derived from typed NIR facts.
pub mod generated;
pub mod stack;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectPageRule {
    CurrentExecutionDomain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrqMaskRule {
    Preserve,
}

/// Additional obligations beyond the MIR's existing E/M/X mode state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryContract {
    pub decimal_clear: bool,
    pub data_bank: u8,
    pub direct_page: DirectPageRule,
    pub irq_mask: IrqMaskRule,
}

pub const BOUNDARY: BoundaryContract = BoundaryContract {
    decimal_clear: BOUNDARY_P_D == 0,
    data_bank: BOUNDARY_DBR as u8,
    direct_page: DirectPageRule::CurrentExecutionDomain,
    irq_mask: IrqMaskRule::Preserve,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FarTransfer {
    Jsl,
    StackRtl,
}

impl FarTransfer {
    pub const fn peak_bytes(self) -> ByteSize {
        ByteSize::new(match self {
            Self::Jsl => CALL_DIRECT_TRANSFER_PEAK_BYTES,
            Self::StackRtl => CALL_INDIRECT_TRANSFER_PEAK_BYTES,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCallContract {
    pub boundary: BoundaryContract,
    pub argument_bytes: ByteSize,
    pub transfer: FarTransfer,
    pub caller_cleanup_bytes: ByteSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedSignature {
    pub signature: crate::nir::SignatureId,
    pub reason: AbiError,
}

/// A planning contract, not evidence that emitted code implements the ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramContract {
    pub version: u32,
    pub boundary: BoundaryContract,
    /// Checked before aggregate ABI expansion can turn an unsupported public
    /// aggregate interface into otherwise supported physical pointer arguments.
    pub unsupported_signatures: Vec<UnsupportedSignature>,
}

pub(super) fn program_contract(program: &crate::nir::NirProgram) -> ProgramContract {
    let mut unsupported = std::collections::BTreeMap::new();
    for signature in program.routines.iter().flat_map(|routine| {
        std::iter::once(&routine.signature).chain(routine.blocks.iter().flat_map(|block| {
            block.ops.iter().filter_map(|op| match op {
                crate::nir::NirOp::Call { signature, .. } => signature.as_ref(),
                _ => None,
            })
        }))
    }) {
        if let Err(reason) = call_layout(signature) {
            unsupported.entry(signature.id).or_insert(reason);
        }
    }
    ProgramContract {
        version: ABI_VERSION,
        boundary: BOUNDARY,
        unsupported_signatures: unsupported
            .into_iter()
            .map(|(signature, reason)| UnsupportedSignature { signature, reason })
            .collect(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuspendedMemory {
    pub hardware_stack_contents: bool,
    pub saved_frame_bytes: ByteSize,
    pub direct_page_bytes: ByteSize,
    pub direct_page_alignment: ByteSize,
    pub call_clobbered_scratch_bytes: ByteSize,
    pub separate_interrupt_domain: bool,
}

pub const SUSPENDED_MEMORY: SuspendedMemory = SuspendedMemory {
    hardware_stack_contents: true,
    saved_frame_bytes: ByteSize::new(SAVED_FRAME_SIZE),
    direct_page_bytes: ByteSize::new(DP_SIZE),
    direct_page_alignment: ByteSize::new(DP_ALIGNMENT),
    call_clobbered_scratch_bytes: ByteSize::new(DP_SCRATCH_SIZE),
    separate_interrupt_domain: true,
};

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
