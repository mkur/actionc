use super::facts::{
    BlockId, LocalId, NirStorageId, NirType, NirValue, ParamId, RoutineId, RuntimeSymbolId,
    SignatureId, SymbolId, TempId, signature_id,
};
use crate::foreign::{ForeignRelocationEncoding, ForeignSymbolUse};
use crate::source::Span;
use crate::target::{AddressSpaceId, AddressValue, ByteOffset, ByteSize, TargetId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirProgram {
    pub target_layout: crate::target::TargetLayout,
    pub runtime_bindings: Vec<NirRuntimeBinding>,
    pub globals: Vec<NirGlobal>,
    pub statics: Vec<NirStaticData>,
    pub routines: Vec<NirRoutine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirGlobal {
    pub id: SymbolId,
    pub name: String,
    pub kind: String,
    pub ty: Option<NirType>,
    pub storage_size: ByteSize,
    pub array: Option<NirArrayGlobalFact>,
    pub init: Option<NirGlobalInit>,
    pub backing: NirGlobalBacking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirArrayGlobalFact {
    pub elem_size: ByteSize,
    pub length: Option<u16>,
    pub pointer_backed: bool,
    pub address_initializer: Option<AddressValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirGlobalInit {
    Bytes {
        image: NirDataImage,
        zero_fill: ByteSize,
        mutable: bool,
        section: String,
    },
    Descriptor {
        backing: NirDataBacking,
        descriptor_size: ByteSize,
        size_word: Option<u16>,
        mutable: bool,
        section: String,
    },
    ZeroFill {
        bytes: ByteSize,
        mutable: bool,
        section: String,
    },
    LinkValue {
        value: NirLinkValue,
        width: ByteSize,
        mutable: bool,
        section: String,
    },
    RoutineAddress {
        routine: RoutineId,
        descriptor_size: ByteSize,
        size_word: Option<u16>,
        mutable: bool,
        section: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirDataBacking {
    pub owner: SymbolId,
    pub image: NirDataImage,
    pub zero_fill: ByteSize,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirStorageInit {
    Bytes {
        image: NirDataImage,
        zero_fill: ByteSize,
        mutable: bool,
        section: String,
    },
    Descriptor {
        backing: NirStorageBacking,
        descriptor_size: ByteSize,
        size_word: Option<u16>,
        mutable: bool,
        section: String,
    },
    ZeroFill {
        bytes: ByteSize,
        mutable: bool,
        section: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirStorageBacking {
    pub image: NirDataImage,
    pub zero_fill: ByteSize,
    pub layout: NirObjectLayout,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirGlobalBacking {
    Ordinary,
    Absolute(AddressValue),
    Alias { target: SymbolId, offset: ByteOffset },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirStaticData {
    pub id: SymbolId,
    pub name: String,
    pub ty: NirType,
    pub image: NirDataImage,
    pub display: String,
    pub alignment: ByteSize,
    pub mutable: bool,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NirDataImage {
    /// Explicit source bytes plus zero placeholders for logical fragments.
    /// Typed values are never serialized into this template in NIR.
    pub bytes: Vec<u8>,
    pub fragments: Vec<NirDataFragment>,
}

impl NirDataImage {
    pub fn literal(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            fragments: Vec::new(),
        }
    }

    pub fn project_constants(&self, endian: crate::target::Endian) -> Option<Vec<u8>> {
        let mut bytes = self.bytes.clone();
        for fragment in &self.fragments {
            let NirDataFragment::Integer {
                offset,
                width,
                value,
            } = fragment
            else {
                continue;
            };
            let start = offset.as_usize()?;
            let width = width.as_usize()?;
            if width == 0 || width > std::mem::size_of::<u64>() {
                return None;
            }
            let destination = bytes.get_mut(start..start.checked_add(width)?)?;
            for (index, byte) in destination.iter_mut().enumerate() {
                let significance = match endian {
                    crate::target::Endian::Little => index,
                    crate::target::Endian::Big => width - index - 1,
                };
                *byte = (value >> (significance * 8)) as u8;
            }
        }
        Some(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirDataFragment {
    Integer {
        offset: ByteOffset,
        width: ByteSize,
        value: u64,
    },
    Address {
        offset: ByteOffset,
        encoding: NirDataAddressEncoding,
        target: NirDataAddressTarget,
        addend: i64,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirDataAddressEncoding {
    Pointer {
        address_space: AddressSpaceId,
        width: ByteSize,
    },
    /// Explicit numeric byte selection retained for compatibility source and
    /// tagged with the target whose address convention it names.
    TargetByte {
        target: crate::target::TargetId,
        byte_index: u8,
    },
}

impl NirDataAddressEncoding {
    pub fn width(self) -> ByteSize {
        match self {
            Self::Pointer { width, .. } => width,
            Self::TargetByte { .. } => ByteSize::ONE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirDataAddressTarget {
    Storage(NirStorageId),
    Routine(RoutineId),
    Absolute(AddressValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirLinkValue {
    ImageEndAddress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirPlace {
    pub kind: NirPlaceKind,
    pub ty: Option<NirType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirPlaceKind {
    Param {
        id: ParamId,
        name: String,
    },
    Local {
        id: LocalId,
        name: String,
    },
    Global {
        id: SymbolId,
        name: String,
    },
    Absolute(AddressValue),
    Deref {
        addr: NirValue,
    },
    Index {
        base_addr: NirValue,
        index: NirValue,
        elem_ty: NirType,
        elem_size: ByteSize,
    },
    Field {
        base: Box<NirPlace>,
        offset: ByteOffset,
        ty: NirType,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirUnaryOp {
    Plus,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lsh,
    Rsh,
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirCompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirCallee {
    User {
        id: RoutineId,
        /// Readable backend/link name; `id` is the executable identity.
        name: String,
    },
    Builtin(String),
    Indirect {
        target: NirValue,
        ty: NirType,
    },
    Runtime {
        symbol: RuntimeSymbolId,
        name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirRuntimeTarget {
    Absolute(AddressValue),
    Routine(RoutineId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirRuntimeBinding {
    pub symbol: RuntimeSymbolId,
    /// Debug/link metadata only; `symbol` is the executable identity.
    pub name: String,
    /// `None` is a declared service that the selected runtime must bind later.
    pub target: Option<NirRuntimeTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirCallEffects {
    pub memory: NirMemoryEffects,
    pub may_call_external: bool,
    pub opaque: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirMemoryEffects {
    pub reads: NirMemoryAccess,
    pub writes: NirMemoryAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirMemoryAccess {
    None,
    Regions(Vec<NirMemoryRegion>),
    Unknown,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirMemoryRegion {
    pub kind: NirMemoryRegionKind,
    pub offset: ByteOffset,
    pub size: ByteSize,
}

impl NirMemoryRegion {
    pub fn overlaps(&self, other: &Self) -> bool {
        self.kind == other.kind && ranges_overlap(self.offset, self.size, other.offset, other.size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirMemoryRegionKind {
    Storage(NirStorageId),
    Static(SymbolId),
    AbsoluteRange(AddressSpaceId),
}

fn ranges_overlap(
    left: ByteOffset,
    left_size: ByteSize,
    right: ByteOffset,
    right_size: ByteSize,
) -> bool {
    if left_size.is_zero() || right_size.is_zero() {
        return false;
    }
    let left = u64::from(left.get())..u64::from(left.get()) + u64::from(left_size.get());
    let right = u64::from(right.get())..u64::from(right.get()) + u64::from(right_size.get());
    left.start < right.end && right.start < left.end
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirCallResult {
    pub dest: TempId,
    pub ty: NirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirCallableSignature {
    pub id: SignatureId,
    pub params: Vec<NirType>,
    pub variadic: Option<NirType>,
    pub result: Option<NirType>,
    pub kind: String,
    pub convention: NirCallConvention,
}

impl Default for NirCallableSignature {
    fn default() -> Self {
        Self::empty_proc(NirCallConvention::TargetPublic)
    }
}

impl NirCallableSignature {
    pub fn empty_proc(convention: NirCallConvention) -> Self {
        Self {
            id: signature_id(&crate::semantic::CallableType::unknown_proc(), convention),
            params: Vec::new(),
            variadic: None,
            result: None,
            kind: "Proc".to_string(),
            convention,
        }
    }
}

/// Target-independent class of a callable boundary. Physical argument and
/// result placement remains a MIR decision for the selected target ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirCallConvention {
    TargetInternal,
    TargetPublic,
    Runtime,
    External(ExternalAbiId),
}

impl NirCallConvention {
    pub(crate) const fn identity_byte(self) -> u8 {
        match self {
            Self::TargetInternal => 1,
            Self::TargetPublic => 2,
            Self::Runtime => 3,
            Self::External(_) => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalAbiId(pub u32);

/// Invocation lifetime model selected before target-specific MIR lowering.
/// This says whether routine storage is shared or invocation-scoped; it does
/// not prescribe a stack frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirActivationModel {
    ClassicStatic,
    NativeReentrant,
}

/// Lifetime class of a parameter or routine-local storage view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirStorageDuration {
    Automatic,
    RoutineStatic,
    External,
}

/// Final target-selected extent and required base alignment of a storage
/// object. Offsets and physical homes remain MIR decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NirObjectLayout {
    pub size: ByteSize,
    pub alignment: ByteSize,
}

impl NirObjectLayout {
    pub const fn new(size: ByteSize, alignment: ByteSize) -> Self {
        Self { size, alignment }
    }

    pub const fn byte() -> Self {
        Self::new(ByteSize::ONE, ByteSize::ONE)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirLocal {
    pub id: LocalId,
    pub name: String,
    pub kind: String,
    pub purpose: NirLocalPurpose,
    pub storage: NirStorageClass,
    pub duration: NirStorageDuration,
    pub layout: NirObjectLayout,
    pub ty: NirType,
    pub backing: NirLocalBacking,
    pub init: Option<NirStorageInit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirLocalPurpose {
    Storage,
    RealTemporary,
}

/// Source-independent storage shape retained for NIR consumers.
///
/// `kind` remains printable/debug metadata; analyses must use this structured
/// classification instead of parsing that text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NirStorageClass {
    Scalar,
    Array,
    Record,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirLocalBacking {
    Ordinary,
    Absolute(AddressValue),
    Alias {
        target: LocalId,
        target_name: String,
        offset: ByteOffset,
    },
    GlobalAlias {
        target: SymbolId,
        target_name: String,
        offset: ByteOffset,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirParam {
    pub id: ParamId,
    pub name: String,
    pub storage: NirStorageClass,
    pub duration: NirStorageDuration,
    pub layout: NirObjectLayout,
    pub ty: NirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirRoutineNote {
    pub text: String,
    pub kind: NirRoutineNoteKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirRoutineNoteKind {
    Informational,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirTemp {
    pub id: TempId,
    pub ty: NirType,
    pub def: NirTempDef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirTempDef {
    pub block: BlockId,
    /// `None` denotes a block-entry parameter definition. Ordinary operation
    /// definitions carry their operation index.
    pub op_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirRoutine {
    pub id: RoutineId,
    pub signature: NirCallableSignature,
    pub convention: NirCallConvention,
    pub activation: NirActivationModel,
    pub entry: NirRoutineEntry,
    pub name: String,
    pub params: Vec<NirParam>,
    pub locals: Vec<NirLocal>,
    pub temps: Vec<NirTemp>,
    pub notes: Vec<NirRoutineNote>,
    pub blocks: Vec<NirBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NirRoutineEntry {
    pub program: bool,
    pub external: bool,
    pub placement: NirRoutinePlacement,
}

impl Default for NirRoutineEntry {
    fn default() -> Self {
        Self {
            program: false,
            external: false,
            placement: NirRoutinePlacement::Relocatable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirRoutinePlacement {
    Relocatable,
    CurrentLocation,
    Absolute(AddressValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirBlock {
    pub id: BlockId,
    pub label: String,
    pub params: Vec<NirBlockParam>,
    pub ops: Vec<NirOp>,
    pub terminator: NirTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirBlockParam {
    pub dest: TempId,
    pub ty: NirType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirOp {
    Load {
        dest: TempId,
        ty: NirType,
        place: NirPlace,
    },
    /// An observable source read that must execute exactly once and remain
    /// ordered with other memory effects.
    VolatileLoad {
        dest: TempId,
        ty: NirType,
        place: NirPlace,
    },
    AddrOf {
        dest: TempId,
        ty: NirType,
        place: NirPlace,
    },
    Store {
        place: NirPlace,
        src: NirValue,
        ty: NirType,
    },
    /// An observable source write that must execute exactly once and remain
    /// ordered with other memory effects.
    VolatileStore {
        place: NirPlace,
        src: NirValue,
        ty: NirType,
    },
    /// Copy a complete aggregate value between two addressable places.
    ///
    /// Both addresses have already been evaluated according to source order.
    /// The operation has overlap-safe value semantics.
    CopyBytes {
        destination: NirPlace,
        source: NirPlace,
        size: ByteSize,
        destination_volatile: bool,
        source_volatile: bool,
    },
    Unary {
        dest: TempId,
        ty: NirType,
        op: NirUnaryOp,
        src: NirValue,
    },
    Cast {
        dest: TempId,
        src: NirValue,
        from: NirType,
        to: NirType,
        kind: NirCastKind,
    },
    /// Address arithmetic kept distinct from fixed-width Action! integer
    /// arithmetic. The displacement is measured in bytes.
    PointerOffset {
        dest: TempId,
        ty: NirType,
        base: NirValue,
        offset: NirValue,
        subtract: bool,
    },
    Binary {
        dest: TempId,
        ty: NirType,
        op: NirBinaryOp,
        left: NirValue,
        right: NirValue,
    },
    Compare {
        dest: TempId,
        ty: NirType,
        operand_ty: NirType,
        op: NirCompareOp,
        left: NirValue,
        right: NirValue,
    },
    /// Address-based native REAL computation. REAL values never inhabit the
    /// byte/word temporary lane; only comparison results and integer
    /// conversion sources use ordinary scalar temps.
    Real(NirRealOp),
    Call {
        callee: NirCallee,
        args: Vec<NirValue>,
        result: Option<NirCallResult>,
        signature: Option<NirCallableSignature>,
        effects: NirCallEffects,
    },
    ForeignCode {
        code: NirForeignCode,
        effects: NirMachineEffects,
    },
    Unsupported {
        note: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirCastKind {
    Integer,
    Pointer,
    IntegerToPointer,
    PointerToInteger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirRealOp {
    Copy {
        destination: NirPlace,
        source: NirRealSource,
    },
    Unary {
        operation: NirUnaryOp,
        destination: NirPlace,
        operand: NirRealSource,
    },
    Binary {
        operation: NirBinaryOp,
        destination: NirPlace,
        left: NirRealSource,
        right: NirRealSource,
    },
    Compare {
        predicate: NirCompareOp,
        result: TempId,
        result_type: NirType,
        left: NirRealSource,
        right: NirRealSource,
    },
    IntegerToReal {
        destination: NirPlace,
        source: NirValue,
        source_type: NirType,
    },
    RealToInteger {
        result: TempId,
        result_type: NirType,
        source: NirPlace,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirRealSource {
    Place(NirPlace),
    Static { id: SymbolId, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirForeignCode {
    pub target: TargetId,
    pub kind: NirForeignCodeKind,
    pub payload: NirForeignCodePayload,
    pub source: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirForeignCodePayload {
    Structured(Vec<NirMachineItem>),
    Bytes {
        bytes: Vec<u8>,
        relocations: Vec<NirForeignRelocation>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirForeignCodeKind {
    LegacyMachineBlock,
    InlineAssembly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirForeignRelocation {
    pub offset: ByteOffset,
    pub encoding: ForeignRelocationEncoding,
    pub target: NirForeignCodeTarget,
    pub addend: i32,
    pub required_address_bits: Option<u8>,
    pub symbol_use: ForeignSymbolUse,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirForeignCodeTarget {
    Storage(NirStorageId),
    Routine(RoutineId),
    Absolute(AddressValue),
    InlineOffset(ByteOffset),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirMachineItem {
    Byte(u8),
    Word(u16),
    StringLiteral(String),
    CharLiteral(char),
    Name(String),
    AddressExpr {
        selector: Option<NirMachineByteSelector>,
        explicit_address: bool,
        atom: NirMachineAtom,
        offset: i32,
        text: String,
    },
    AddressByte {
        high: bool,
        name: String,
    },
    /// A resolved symbolic machine-block operand. Legacy unresolved names are
    /// retained only in the compatibility variants above.
    Relocation {
        encoding: ForeignRelocationEncoding,
        target: NirForeignCodeTarget,
        addend: i32,
        required_address_bits: Option<u8>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirMachineAtom {
    Number(u16),
    Name(String),
    Current,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirMachineByteSelector {
    Low,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirMachineEffects {
    pub memory: NirMemoryEffects,
    pub may_call_external: bool,
    pub opaque: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirEdge {
    pub target: BlockId,
    pub args: Vec<NirValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirTerminator {
    Open,
    Fallthrough,
    Goto(NirEdge),
    Branch {
        condition: NirValue,
        then_edge: NirEdge,
        else_edge: NirEdge,
    },
    Return(Option<NirValue>),
    Exit,
}

#[cfg(test)]
mod memory_region_tests {
    use super::*;

    fn local_region(id: u32, offset: u16, size: u16) -> NirMemoryRegion {
        NirMemoryRegion {
            kind: NirMemoryRegionKind::Storage(NirStorageId::Local(LocalId(id))),
            offset: ByteOffset::from(offset),
            size: ByteSize::from(size),
        }
    }

    #[test]
    fn overlap_requires_the_same_identity_and_intersecting_byte_ranges() {
        assert!(local_region(0, 0, 2).overlaps(&local_region(0, 1, 1)));
        assert!(!local_region(0, 0, 1).overlaps(&local_region(0, 1, 1)));
        assert!(!local_region(0, 0, 2).overlaps(&local_region(1, 0, 2)));
        assert!(!local_region(0, 1, 0).overlaps(&local_region(0, 0, 2)));
    }
}
