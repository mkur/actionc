use super::facts::{BlockId, LocalId, NirStorageId, NirType, NirValue, ParamId, SymbolId, TempId};
use crate::asm6502::{InlineAsmRelocationKind, InlineAsmSymbolUse};
use crate::source::Span;
use crate::target::{AddressSpaceId, AddressValue, ByteOffset, ByteSize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirProgram {
    pub target_layout: crate::target::TargetLayout,
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
    ProgramEndWord {
        mutable: bool,
        section: String,
    },
    RoutineAddress {
        routine: u32,
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
    pub bytes: Vec<u8>,
    pub relocations: Vec<NirDataRelocation>,
}

impl NirDataImage {
    pub fn literal(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            relocations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirDataRelocation {
    pub offset: ByteOffset,
    pub kind: NirDataRelocationKind,
    pub target: NirDataRelocationTarget,
    pub addend: i32,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirDataRelocationKind {
    Low8,
    High8,
    Word16,
}

impl NirDataRelocationKind {
    pub fn width(self) -> ByteSize {
        match self {
            Self::Low8 | Self::High8 => ByteSize::new(1),
            Self::Word16 => ByteSize::new(2),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirDataRelocationTarget {
    Storage(NirStorageId),
    Routine(u32),
    Absolute(AddressValue),
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
        id: u32,
        /// Readable backend/link name; `id` is the executable identity.
        name: String,
    },
    Builtin(String),
    Indirect {
        target: NirValue,
        ty: NirType,
    },
    Runtime {
        name: String,
        address: Option<AddressValue>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirRuntimeHelperTarget {
    Absolute(AddressValue),
    Routine(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirCallEffects {
    pub memory: NirMemoryEffects,
    pub may_call_os: bool,
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
    ZeroPage,
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
    pub params: Vec<NirType>,
    pub variadic: Option<NirType>,
    pub result: Option<NirType>,
    pub kind: String,
    pub abi: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirLocal {
    pub id: LocalId,
    pub name: String,
    pub kind: String,
    pub purpose: NirLocalPurpose,
    pub storage: NirStorageClass,
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
    /// The last source procedure selected as the Action! program entry.
    ProgramEntry,
    CurrentLocationEntry,
    /// Signature-only declaration that runtime binding must resolve before
    /// emission.
    ExternalInterface,
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
    pub name: String,
    pub params: Vec<NirParam>,
    pub locals: Vec<NirLocal>,
    pub temps: Vec<NirTemp>,
    pub notes: Vec<NirRoutineNote>,
    pub blocks: Vec<NirBlock>,
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
    RuntimeHelperOverride {
        slot: AddressValue,
        target: NirRuntimeHelperTarget,
    },
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
    MachineBlock {
        items: Vec<NirMachineItem>,
        effects: NirMachineEffects,
    },
    InlineAsm {
        code: NirInlineAsm,
        effects: NirMachineEffects,
    },
    Unsupported {
        note: String,
    },
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
pub struct NirInlineAsm {
    pub bytes: Vec<u8>,
    pub relocations: Vec<NirInlineAsmRelocation>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirInlineAsmRelocation {
    pub offset: ByteOffset,
    pub kind: InlineAsmRelocationKind,
    pub target: NirInlineAsmTarget,
    pub addend: i32,
    pub requires_zero_page: bool,
    pub symbol_use: InlineAsmSymbolUse,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NirInlineAsmTarget {
    Storage(NirStorageId),
    Routine(u32),
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
        kind: InlineAsmRelocationKind,
        target: NirInlineAsmTarget,
        addend: i32,
        requires_zero_page: bool,
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
    pub may_call_os: bool,
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
