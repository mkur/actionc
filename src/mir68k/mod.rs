//! Portable NIR-to-MIR canary for the Motorola 68000.
//!
//! The canary owns 68k access strategy and big-endian data projection. It does
//! not share an instruction or register model with either 6502-family backend.

mod lower;

use crate::backend::{BackendLoweringError, NirBackend, VerifiedNir};
use crate::nir::{
    BlockId, NirBinaryOp, NirCastKind, NirCompareOp, NirDataAddressTarget, NirRuntimeTarget,
    NirStorageId, NirUnaryOp, ParamId, RuntimeSymbolId, SignatureId, SymbolId, TempId,
};
use crate::target::{AddressSpaceId, AddressValue, ByteOffset, ByteSize, Endian, TargetId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kProgram {
    pub endian: Endian,
    pub architectural_address_bits: u8,
    pub data_pointer_width: ByteSize,
    pub code_pointer_width: ByteSize,
    pub data: Vec<Mir68kData>,
    pub runtime_bindings: Vec<Mir68kRuntimeBinding>,
    pub routines: Vec<Mir68kRoutine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kData {
    pub name: String,
    pub bytes: Vec<u8>,
    pub alignment: ByteSize,
    pub relocations: Vec<Mir68kRelocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kRelocation {
    pub offset: ByteOffset,
    pub width: ByteSize,
    pub address_space: AddressSpaceId,
    pub target: Mir68kRelocationTarget,
    pub addend: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir68kRelocationTarget {
    Data(NirStorageId),
    Code(u32),
    Absolute(AddressValue),
    ImageEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kRuntimeBinding {
    pub symbol: RuntimeSymbolId,
    pub target: Option<NirRuntimeTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kRoutine {
    pub name: String,
    pub blocks: Vec<Mir68kBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kBlock {
    pub id: BlockId,
    pub ops: Vec<Mir68kOp>,
    pub terminator: Mir68kTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir68kOp {
    Load {
        dest: TempId,
        width: ByteSize,
        address: Mir68kAddress,
        access: Mir68kAccess,
        volatile: bool,
    },
    Store {
        address: Mir68kAddress,
        value: Mir68kValue,
        width: ByteSize,
        access: Mir68kAccess,
        volatile: bool,
    },
    AddressOf {
        dest: TempId,
        address: Mir68kAddress,
        width: ByteSize,
    },
    Copy {
        destination: Mir68kAddress,
        source: Mir68kAddress,
        bytes: ByteSize,
        overlap_safe: bool,
    },
    Unary {
        dest: TempId,
        width: ByteSize,
        operation: NirUnaryOp,
        value: Mir68kValue,
    },
    Cast {
        dest: TempId,
        from: ByteSize,
        to: ByteSize,
        kind: NirCastKind,
        value: Mir68kValue,
    },
    PointerOffset {
        dest: TempId,
        width: ByteSize,
        base: Mir68kValue,
        offset: Mir68kValue,
        subtract: bool,
    },
    Binary {
        dest: TempId,
        width: ByteSize,
        operation: NirBinaryOp,
        left: Mir68kValue,
        right: Mir68kValue,
    },
    Compare {
        dest: TempId,
        width: ByteSize,
        operation: NirCompareOp,
        left: Mir68kValue,
        right: Mir68kValue,
    },
    Call {
        target: Mir68kCallTarget,
        signature: Option<SignatureId>,
        args: Vec<Mir68kValue>,
        result: Option<(TempId, ByteSize)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kAddress {
    pub base: Mir68kAddressBase,
    pub displacement: ByteOffset,
    pub index: Option<Mir68kIndex>,
    pub mode: Mir68kAddressMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir68kAddressBase {
    Param(ParamId),
    Local(crate::nir::LocalId),
    Global(SymbolId),
    Absolute(AddressValue),
    Pointer(Mir68kValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kIndex {
    pub value: Mir68kValue,
    pub stride: ByteSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir68kAddressMode {
    FrameOrStatic,
    Absolute,
    AddressIndirect,
    Indexed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir68kAccess {
    Byte,
    NativeAlignedWord,
    BytewisePackedOddWord { endian: Endian },
    Bytes(ByteSize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir68kValue {
    U8(u8),
    U16(u16),
    Null(ByteSize),
    Address(AddressValue, ByteSize),
    StaticAddress(SymbolId, ByteSize),
    Temp(TempId, ByteSize),
    Param(ParamId),
    GlobalAddress(SymbolId, ByteSize),
    RoutineAddress(u32, ByteSize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir68kCallTarget {
    Direct(u32),
    Builtin(String),
    Runtime(RuntimeSymbolId),
    Indirect(Mir68kValue, ByteSize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir68kTerminator {
    Fallthrough,
    Goto(BlockId),
    Branch {
        condition: Mir68kValue,
        then_block: BlockId,
        else_block: BlockId,
    },
    Return(Option<Mir68kValue>),
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kDiagnostic {
    pub routine: Option<String>,
    pub block: Option<String>,
    pub message: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Mir68kBackend;

impl NirBackend for Mir68kBackend {
    type Output = Mir68kProgram;
    type Diagnostic = Mir68kDiagnostic;

    fn supports_target(&self, target: TargetId) -> bool {
        target == TargetId::Motorola68000
    }

    fn lower(&self, input: VerifiedNir<'_>) -> Result<Self::Output, Vec<Self::Diagnostic>> {
        lower::lower_program(input)
    }
}

pub fn lower_program(
    program: &crate::nir::NirProgram,
) -> Result<Mir68kProgram, BackendLoweringError<Mir68kDiagnostic>> {
    crate::backend::lower_program(&Mir68kBackend, program)
}

pub fn lower_verified(
    input: VerifiedNir<'_>,
) -> Result<Mir68kProgram, BackendLoweringError<Mir68kDiagnostic>> {
    crate::backend::lower_verified(&Mir68kBackend, input)
}

fn relocation_target(target: NirDataAddressTarget) -> Mir68kRelocationTarget {
    match target {
        NirDataAddressTarget::Storage(storage) => Mir68kRelocationTarget::Data(storage),
        NirDataAddressTarget::Routine(routine) => Mir68kRelocationTarget::Code(routine),
        NirDataAddressTarget::Absolute(address) => Mir68kRelocationTarget::Absolute(address),
    }
}
