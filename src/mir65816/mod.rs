//! Portable NIR-to-MIR canary for the WDC 65816.
//!
//! This deliberately stops before register allocation or emission. Its job is
//! to prove that a separate 65816 backend can consume verifier-clean NIR
//! without reaching back into Semantic IR or borrowing MIR6502 concepts.

pub mod abi;
mod data;
mod lower;

use crate::backend::{BackendLoweringError, NirBackend, VerifiedNir};
use crate::nir::{
    BlockId, NirBinaryOp, NirCallConvention, NirCastKind, NirCompareOp, NirDataAddressTarget,
    NirRuntimeTarget, NirStorageId, NirUnaryOp, ParamId, RoutineId, RuntimeSymbolId, SignatureId,
    SymbolId, TempId,
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
    pub native_abi: Option<abi::ProgramContract>,
    pub task_switch_state: Mir65816TaskSwitchState,
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
    pub id: Mir65816DataId,
    pub placement: Mir65816DataPlacement,
    /// Complete storage extent, including the zero-filled tail.
    pub size: ByteSize,
    /// Zero-initialized bytes following `bytes`; no host allocation is needed.
    pub zero_fill: ByteSize,
    pub mutable: bool,
    pub ty: Option<crate::nir::NirType>,
    pub array: Option<crate::nir::NirArrayGlobalFact>,
    /// NIR section hint, independent of the display name and storage identity.
    pub section: Option<String>,
    pub name: String,
    pub bytes: Vec<u8>,
    pub alignment: ByteSize,
    pub relocations: Vec<Mir65816Relocation>,
}

/// Descriptor cells, their elements, and initializer templates are distinct objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mir65816DataId {
    Global(SymbolId),
    Static(SymbolId),
    ArrayBacking(SymbolId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816DataPlacement {
    Allocate,
    Absolute(AddressValue),
    Alias {
        target: Mir65816DataId,
        offset: ByteOffset,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Relocation {
    pub offset: ByteOffset,
    /// Numeric byte significance, selected after applying the signed addend.
    /// `None` writes the complete value at `width` in target byte order.
    pub byte_index: Option<u8>,
    pub width: ByteSize,
    pub address_space: AddressSpaceId,
    pub target: Mir65816RelocationTarget,
    pub addend: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816RelocationTarget {
    Data(NirStorageId),
    Code(RoutineId),
    ArrayBacking(SymbolId),
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
    pub id: RoutineId,
    pub name: String,
    pub convention: NirCallConvention,
    pub result_home: Option<Mir65816AbiHome>,
    pub frame: Mir65816FramePlan,
    pub prologue: Mir65816ProloguePlan,
    pub epilogue: Mir65816EpiloguePlan,
    pub blocks: Vec<Mir65816Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mir65816FrameObjectId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mir65816ActivationId(pub u64);

/// Abstract identity of an automatic object in one live invocation. A task
/// switch or recursive call changes the activation identity even though the
/// lexical frame-object ID remains stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mir65816ActivationAddress {
    pub activation: Mir65816ActivationId,
    pub object: Mir65816FrameObjectId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816FrameObjectOwner {
    Param(ParamId),
    Local(crate::nir::LocalId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816FrameObject {
    pub id: Mir65816FrameObjectId,
    pub owner: Mir65816FrameObjectOwner,
    pub size: ByteSize,
    pub alignment: ByteSize,
    /// Displacement from S after the routine prologue has reserved the frame.
    pub stack_offset: ByteOffset,
    pub mutable: bool,
    pub addressable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816AbiHome {
    /// Offset within the caller-provided argument area.
    StackArgument {
        offset: ByteOffset,
        size: ByteSize,
        alignment: ByteSize,
    },
    Accumulator,
    AccumulatorAndX,
    NativeResult(abi::ResultLocation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816ParameterPlan {
    pub param: ParamId,
    pub incoming: Mir65816AbiHome,
    pub frame_object: Option<Mir65816FrameObjectId>,
    /// Native incoming address relative to S after reserving the fixed frame.
    pub body_stack_offset: Option<ByteOffset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816FrameStrategy {
    /// The first implementation keeps the complete activation in bank zero
    /// and requires every planned access to fit the 8-bit `d,S` displacement.
    HardwareStackRelative,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816FramePlan {
    pub strategy: Mir65816FrameStrategy,
    pub bank: u8,
    pub objects: Vec<Mir65816FrameObject>,
    pub parameters: Vec<Mir65816ParameterPlan>,
    pub automatic_bytes: ByteSize,
    pub saved_state_bytes: ByteSize,
    pub spill_bytes: ByteSize,
    pub outgoing: Mir65816OutgoingArea,
    pub outgoing_bytes: ByteSize,
    pub incoming_bytes: ByteSize,
    pub incoming_extent: ByteSize,
    pub extent: ByteSize,
    /// Includes fixed objects and known call transfers, excluding unknown
    /// allocated spills and instruction-local pushes.
    pub minimum_stack_peak: ByteSize,
    pub allocation_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816OutgoingArea {
    FixedInFrame { offset: ByteOffset },
    PerCallBelowFrame,
}

impl Mir65816FramePlan {
    pub fn address_in(
        &self,
        activation: Mir65816ActivationId,
        object: Mir65816FrameObjectId,
    ) -> Option<Mir65816ActivationAddress> {
        self.objects
            .iter()
            .any(|candidate| candidate.id == object)
            .then_some(Mir65816ActivationAddress { activation, object })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816RegisterWidth {
    Bits8,
    Bits16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mir65816ModeState {
    pub native_mode: bool,
    pub accumulator: Mir65816RegisterWidth,
    pub index: Mir65816RegisterWidth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816CallForm {
    NearJsr,
    FarJsl,
    FarStackRtl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816ReturnForm {
    NearRts,
    FarRtl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816CallActivation {
    Fresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816CallPlan {
    pub convention: NirCallConvention,
    pub native: Option<abi::NativeCallContract>,
    pub arguments: Vec<Mir65816AbiHome>,
    pub result: Option<Mir65816AbiHome>,
    pub outgoing_bytes: ByteSize,
    pub code_pointer_width: ByteSize,
    pub call_form: Mir65816CallForm,
    pub mode_before: Mir65816ModeState,
    pub mode_after: Mir65816ModeState,
    pub activation: Mir65816CallActivation,
    pub net_stack_delta: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816ProloguePlan {
    pub required_mode: Mir65816ModeState,
    pub reserve_bytes: ByteSize,
    pub parameter_copies: Vec<(Mir65816AbiHome, Mir65816FrameObjectId)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816EpiloguePlan {
    pub restored_mode: Mir65816ModeState,
    pub release_bytes: ByteSize,
    pub return_form: Mir65816ReturnForm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816SavedState {
    Accumulator,
    X,
    Y,
    StackPointer,
    DirectPage,
    DataBank,
    ProgramBank,
    ProgramCounter,
    ProcessorStatus,
}

const TASK_STATE: [Mir65816SavedState; 9] = [
    Mir65816SavedState::Accumulator,
    Mir65816SavedState::X,
    Mir65816SavedState::Y,
    Mir65816SavedState::StackPointer,
    Mir65816SavedState::DirectPage,
    Mir65816SavedState::DataBank,
    Mir65816SavedState::ProgramBank,
    Mir65816SavedState::ProgramCounter,
    Mir65816SavedState::ProcessorStatus,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816TaskSwitchState {
    pub required: Vec<Mir65816SavedState>,
    pub native_memory: Option<abi::SuspendedMemory>,
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
        signed: bool,
        operation: NirBinaryOp,
        left: Mir65816Value,
        right: Mir65816Value,
    },
    Compare {
        dest: TempId,
        width: ByteSize,
        signed: bool,
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
        plan: Mir65816CallPlan,
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
    Static(NirStorageId),
    AutomaticFrame(Mir65816FrameObjectId),
    Parameter(ParamId),
    External(Mir65816ExternalAddress),
    Indirect(Mir65816Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir65816ExternalAddress {
    Absolute(AddressValue),
    Global(SymbolId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mir65816Index {
    pub value: Mir65816Value,
    pub stride: ByteSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mir65816AddressMode {
    Static,
    AutomaticFrame,
    Parameter,
    External,
    LongIndirect,
    LongIndexed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mir65816Value {
    U8(u8),
    U16(u16),
    U24(u32),
    U32(u32),
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
    Return {
        value: Option<Mir65816Value>,
        release_frame_bytes: ByteSize,
        form: Mir65816ReturnForm,
        restored_mode: Mir65816ModeState,
    },
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

    fn lower(&self, input: VerifiedNir<'_>) -> Result<Self::Output, Vec<Self::Diagnostic>> {
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

/// Recheck mutable MIR plans before subsequent target passes consume them.
/// Final instruction allocation and its concrete stack accesses are a later gate.
pub fn verify_program(program: &Mir65816Program) -> Result<(), Vec<Mir65816Diagnostic>> {
    let mut errors = Vec::new();
    let native = program.call_convention == Mir65816CallConvention::Native;
    let expected_width = ByteSize::new(if native {
        abi::generated::SCALAR_CODE_POINTER_SIZE
    } else {
        2
    });
    if program.endian != Endian::Little
        || program.architectural_address_bits != 24
        || program.code_pointer_width != expected_width
        || program.data_pointer_width != expected_width
        || program.target
            != if native {
                TargetId::Wdc65816Native
            } else {
                TargetId::Wdc65816Small
            }
        || (native
            && program.native_abi.as_ref().is_none_or(|abi| {
                abi.version != abi::generated::ABI_VERSION || abi.boundary != abi::BOUNDARY
            }))
        || (!native && program.native_abi.is_some())
        || program.task_switch_state.native_memory != native.then_some(abi::SUSPENDED_MEMORY)
        || program.task_switch_state.required != TASK_STATE
    {
        errors.push(Mir65816Diagnostic {
            routine: None,
            block: None,
            message: "65816 program does not match its ABI/model state contract".into(),
        });
    }
    let mut routine_ids = std::collections::BTreeSet::new();
    for routine in &program.routines {
        if !routine_ids.insert(routine.id) {
            errors.push(Mir65816Diagnostic {
                routine: Some(routine.name.clone()),
                block: None,
                message: "65816 duplicate routine identity".into(),
            });
        }
        if let Err(message) =
            lower::verify_routine_plan(routine, expected_width, program.call_convention)
        {
            errors.push(Mir65816Diagnostic {
                routine: Some(routine.name.clone()),
                block: None,
                message,
            });
        }
        if native {
            for block in &routine.blocks {
                for op in &block.ops {
                    if let Mir65816Op::Call {
                        target: Mir65816CallTarget::Direct(id),
                        plan,
                        ..
                    } = op
                    {
                        let callee = program.routines.iter().find(|r| r.id.0 == *id);
                        if callee.is_none_or(|callee| {
                            plan.arguments
                                != callee
                                    .frame
                                    .parameters
                                    .iter()
                                    .map(|p| p.incoming)
                                    .collect::<Vec<_>>()
                                || plan.result != callee.result_home
                        }) {
                            errors.push(Mir65816Diagnostic {
                                routine: Some(routine.name.clone()),
                                block: Some(format!("b{}", block.id.0)),
                                message: "65816 direct caller/callee ABI homes disagree".into(),
                            });
                        }
                    }
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn relocation_target(target: NirDataAddressTarget) -> Mir65816RelocationTarget {
    match target {
        NirDataAddressTarget::Storage(storage) => Mir65816RelocationTarget::Data(storage),
        NirDataAddressTarget::Routine(routine) => Mir65816RelocationTarget::Code(routine),
        NirDataAddressTarget::Absolute(address) => Mir65816RelocationTarget::Absolute(address),
    }
}
