//! Portable NIR-to-MIR canary for the Motorola 68000.
//!
//! The canary owns 68k access strategy and big-endian data projection. It does
//! not share an instruction or register model with either 6502-family backend.

mod lower;

use crate::backend::{BackendLoweringError, NirBackend, VerifiedNir};
use crate::nir::{
    BlockId, NirBinaryOp, NirCallConvention, NirCastKind, NirCompareOp, NirDataAddressTarget,
    NirRuntimeTarget, NirStorageId, NirUnaryOp, ParamId, RoutineId, RuntimeSymbolId, SignatureId,
    SymbolId, TempId,
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
    pub id: RoutineId,
    pub name: String,
    pub convention: NirCallConvention,
    pub frame: Mir68kFramePlan,
    pub prologue: Mir68kProloguePlan,
    pub epilogue: Mir68kEpiloguePlan,
    pub blocks: Vec<Mir68kBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mir68kFrameObjectId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mir68kActivationId(pub u64);

/// Abstract identity of a frame object in one live invocation. The eventual
/// emitter resolves this pair to a machine address; the lexical object ID
/// alone is intentionally insufficient for native reentrant storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mir68kActivationAddress {
    pub activation: Mir68kActivationId,
    pub object: Mir68kFrameObjectId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir68kFrameObjectOwner {
    Param(ParamId),
    Local(crate::nir::LocalId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kFrameObject {
    pub id: Mir68kFrameObjectId,
    pub owner: Mir68kFrameObjectOwner,
    pub size: ByteSize,
    pub alignment: ByteSize,
    /// Signed displacement from A6 to the first byte of the object.
    pub frame_offset: i32,
    pub mutable: bool,
    pub addressable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir68kAbiHome {
    /// Offset within the caller-provided argument area.
    StackArgument {
        offset: ByteOffset,
        size: ByteSize,
    },
    DataRegister(u8),
    AddressRegister(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kParameterPlan {
    pub param: ParamId,
    pub incoming: Mir68kAbiHome,
    /// Mutated and address-required parameters are copied into this object.
    pub frame_object: Option<Mir68kFrameObjectId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mir68kOutgoingArea {
    pub frame_offset: i32,
    pub size: ByteSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kFramePlan {
    pub objects: Vec<Mir68kFrameObject>,
    pub parameters: Vec<Mir68kParameterPlan>,
    pub automatic_bytes: ByteSize,
    pub saved_register_bytes: ByteSize,
    pub spill_bytes: ByteSize,
    pub outgoing: Mir68kOutgoingArea,
    /// Total amount subtracted by LINK. Always even for an original 68000.
    pub extent: ByteSize,
}

impl Mir68kFramePlan {
    pub fn address_in(
        &self,
        activation: Mir68kActivationId,
        object: Mir68kFrameObjectId,
    ) -> Option<Mir68kActivationAddress> {
        self.objects
            .iter()
            .any(|candidate| candidate.id == object)
            .then_some(Mir68kActivationAddress { activation, object })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir68kRegister {
    D(u8),
    A(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mir68kParameterCopy {
    pub source: Mir68kAbiHome,
    pub destination: Mir68kFrameObjectId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kProloguePlan {
    pub frame_pointer: Mir68kRegister,
    pub link_bytes: ByteSize,
    pub saved_registers: Vec<Mir68kRegister>,
    pub parameter_copies: Vec<Mir68kParameterCopy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kEpiloguePlan {
    pub frame_pointer: Mir68kRegister,
    pub unlink_bytes: ByteSize,
    pub restored_registers: Vec<Mir68kRegister>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir68kCallActivation {
    Fresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kCallPlan {
    pub convention: NirCallConvention,
    pub arguments: Vec<Mir68kAbiHome>,
    pub result: Option<Mir68kAbiHome>,
    pub outgoing_bytes: ByteSize,
    pub activation: Mir68kCallActivation,
    /// The outgoing area is preallocated in the caller's frame, so a call is
    /// stack-neutral when it returns.
    pub net_stack_delta: i32,
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
        plan: Mir68kCallPlan,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kAddress {
    pub base: Mir68kAddressBase,
    /// Proven base alignment. `None` means the backend must assume byte
    /// alignment, as for an arbitrary pointer value.
    pub base_alignment: Option<ByteSize>,
    pub displacement: ByteOffset,
    pub index: Option<Mir68kIndex>,
    pub mode: Mir68kAddressMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir68kAddressBase {
    Static(NirStorageId),
    AutomaticFrame(Mir68kFrameObjectId),
    Parameter(ParamId),
    External(Mir68kExternalAddress),
    Indirect(Mir68kValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir68kExternalAddress {
    Absolute(AddressValue),
    Global(SymbolId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir68kIndex {
    pub value: Mir68kValue,
    pub stride: ByteSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir68kAddressMode {
    Static,
    AutomaticFrame,
    Parameter,
    External,
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
    Return {
        value: Option<Mir68kValue>,
        restore_frame_bytes: ByteSize,
    },
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
        NirDataAddressTarget::Routine(routine) => Mir68kRelocationTarget::Code(routine.0),
        NirDataAddressTarget::Absolute(address) => Mir68kRelocationTarget::Absolute(address),
    }
}
