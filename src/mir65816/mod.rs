//! Portable NIR-to-MIR canary for the WDC 65816.
//!
//! This deliberately stops before register allocation or emission. Its job is
//! to prove that a separate 65816 backend can consume verifier-clean NIR
//! without reaching back into Semantic IR or borrowing MIR6502 concepts.

mod lower;

use crate::backend::{BackendLoweringError, NirBackend, VerifiedNir};
use crate::nir::{
    BlockId, NirBinaryOp, NirCastKind, NirCompareOp, NirDataAddressTarget, NirRuntimeTarget,
    NirStorageId, NirUnaryOp, ParamId, RuntimeSymbolId, SignatureId, SymbolId, TempId,
};
use crate::target::{AddressSpaceId, AddressValue, ByteOffset, ByteSize, Endian, TargetId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Program {
    pub target: TargetId,
    pub endian: Endian,
    pub architectural_address_bits: u8,
    pub data_pointer_width: ByteSize,
    pub code_pointer_width: ByteSize,
    pub call_convention: Mir65816CallConvention,
    pub data: Vec<Mir65816Data>,
    pub runtime_bindings: Vec<Mir65816RuntimeBinding>,
    pub routines: Vec<Mir65816Routine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816CallConvention {
    Native,
    Small,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Data {
    pub name: String,
    pub bytes: Vec<u8>,
    pub alignment: ByteSize,
    pub relocations: Vec<Mir65816Relocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Relocation {
    pub offset: ByteOffset,
    pub width: ByteSize,
    pub address_space: AddressSpaceId,
    pub target: Mir65816RelocationTarget,
    pub addend: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816RelocationTarget {
    Data(NirStorageId),
    Code(u32),
    Absolute(AddressValue),
    ImageEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816RuntimeBinding {
    pub symbol: RuntimeSymbolId,
    pub target: Option<NirRuntimeTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Routine {
    pub name: String,
    pub blocks: Vec<Mir65816Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Block {
    pub id: BlockId,
    pub ops: Vec<Mir65816Op>,
    pub terminator: Mir65816Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir65816Op {
    Load {
        dest: TempId,
        width: ByteSize,
        address: Mir65816Address,
        volatile: bool,
    },
    Store {
        address: Mir65816Address,
        value: Mir65816Value,
        width: ByteSize,
        volatile: bool,
    },
    AddressOf {
        dest: TempId,
        address: Mir65816Address,
        width: ByteSize,
    },
    Copy {
        destination: Mir65816Address,
        source: Mir65816Address,
        bytes: ByteSize,
        overlap_safe: bool,
    },
    Unary {
        dest: TempId,
        width: ByteSize,
        operation: NirUnaryOp,
        value: Mir65816Value,
    },
    Cast {
        dest: TempId,
        from: ByteSize,
        to: ByteSize,
        kind: NirCastKind,
        value: Mir65816Value,
    },
    PointerOffset {
        dest: TempId,
        width: ByteSize,
        base: Mir65816Value,
        offset: Mir65816Value,
        subtract: bool,
    },
    Binary {
        dest: TempId,
        width: ByteSize,
        operation: NirBinaryOp,
        left: Mir65816Value,
        right: Mir65816Value,
    },
    Compare {
        dest: TempId,
        width: ByteSize,
        operation: NirCompareOp,
        left: Mir65816Value,
        right: Mir65816Value,
    },
    Call {
        target: Mir65816CallTarget,
        signature: Option<SignatureId>,
        args: Vec<Mir65816Value>,
        result: Option<(TempId, ByteSize)>,
        convention: Mir65816CallConvention,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Address {
    pub base: Mir65816AddressBase,
    pub displacement: ByteOffset,
    pub index: Option<Mir65816Index>,
    pub mode: Mir65816AddressMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir65816AddressBase {
    Param(ParamId),
    Local(crate::nir::LocalId),
    Global(SymbolId),
    Absolute(AddressValue),
    Pointer(Mir65816Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Index {
    pub value: Mir65816Value,
    pub stride: ByteSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816AddressMode {
    FrameOrStatic,
    AbsoluteLong,
    LongIndirect,
    LongIndexed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir65816Value {
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
pub enum Mir65816CallTarget {
    Direct(u32),
    Builtin(String),
    Runtime(RuntimeSymbolId),
    Indirect(Mir65816Value, ByteSize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir65816Terminator {
    Fallthrough,
    Goto(BlockId),
    Branch {
        condition: Mir65816Value,
        then_block: BlockId,
        else_block: BlockId,
    },
    Return(Option<Mir65816Value>),
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Diagnostic {
    pub routine: Option<String>,
    pub block: Option<String>,
    pub message: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Mir65816Backend;

impl NirBackend for Mir65816Backend {
    type Output = Mir65816Program;
    type Diagnostic = Mir65816Diagnostic;

    fn supports_target(&self, target: TargetId) -> bool {
        matches!(target, TargetId::Wdc65816Native | TargetId::Wdc65816Small)
    }

    fn lower(
        &self,
        input: VerifiedNir<'_>,
    ) -> Result<Self::Output, Vec<Self::Diagnostic>> {
        lower::lower_program(input)
    }
}

pub fn lower_program(
    program: &crate::nir::NirProgram,
) -> Result<Mir65816Program, BackendLoweringError<Mir65816Diagnostic>> {
    crate::backend::lower_program(&Mir65816Backend, program)
}

pub fn lower_verified(
    input: VerifiedNir<'_>,
) -> Result<Mir65816Program, BackendLoweringError<Mir65816Diagnostic>> {
    crate::backend::lower_verified(&Mir65816Backend, input)
}

fn relocation_target(target: NirDataAddressTarget) -> Mir65816RelocationTarget {
    match target {
        NirDataAddressTarget::Storage(storage) => Mir65816RelocationTarget::Data(storage),
        NirDataAddressTarget::Routine(routine) => Mir65816RelocationTarget::Code(routine),
        NirDataAddressTarget::Absolute(address) => Mir65816RelocationTarget::Absolute(address),
    }
}
