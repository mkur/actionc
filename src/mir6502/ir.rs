use crate::asm6502::InlineAsmRelocationKind;
use crate::nir::{LocalId, ParamId, SymbolId};
use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirPhase {
    PreMaterialization,
    PostHome,
    PostMaterialization,
    PreEmission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoutineId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirBlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirTempId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirSpillId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirZpSlot(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirFixedZpSlot(pub u8);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirLabel(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirProgram {
    pub statics: Vec<MirStatic>,
    pub globals: Vec<MirGlobal>,
    pub routines: Vec<MirRoutine>,
    pub machine_blocks: Vec<MirMachineBlock>,
    pub runtime_helpers: Vec<MirRuntimeHelperDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirStatic {
    pub id: SymbolId,
    pub name: String,
    pub ty: String,
    pub image: MirDataImage,
    pub display: String,
    pub alignment: u16,
    pub mutable: bool,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MirDataImage {
    pub bytes: Vec<u8>,
    pub relocations: Vec<MirDataRelocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDataRelocation {
    pub offset: u16,
    pub kind: MirDataRelocationKind,
    pub target: MirDataRelocationTarget,
    pub addend: i32,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirDataRelocationKind {
    Low8,
    High8,
    Word16,
}

impl MirDataRelocationKind {
    pub fn width(self) -> u16 {
        match self {
            Self::Low8 | Self::High8 => 1,
            Self::Word16 => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirDataRelocationTarget {
    Global(SymbolId),
    Local { routine: RoutineId, id: LocalId },
    Param { routine: RoutineId, id: ParamId },
    Routine(RoutineId),
    Absolute(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirGlobal {
    pub id: SymbolId,
    pub name: String,
    pub kind: String,
    pub width: Option<MirWidth>,
    pub storage_size: u16,
    pub backing: MirGlobalBacking,
    pub init: Option<MirGlobalInit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirArrayGlobalFact {
    pub elem_size: u16,
    pub length: Option<u16>,
    pub pointer_backed: bool,
    pub address_initializer: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirGlobalBacking {
    Ordinary { offset: u16 },
    Absolute(u16),
    Alias { target: SymbolId, offset: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirGlobalInit {
    Bytes {
        image: MirDataImage,
        zero_fill: u16,
        mutable: bool,
        section: String,
        array: Option<MirArrayGlobalFact>,
    },
    Descriptor {
        backing: MirDataBacking,
        descriptor_size: u16,
        size_word: Option<u16>,
        mutable: bool,
        section: String,
    },
    ZeroFill {
        bytes: u16,
        mutable: bool,
        section: String,
        array: Option<MirArrayGlobalFact>,
    },
    ProgramEndWord {
        mutable: bool,
        section: String,
    },
    RoutineAddress {
        routine: RoutineId,
        descriptor_size: u16,
        size_word: Option<u16>,
        mutable: bool,
        section: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDataBacking {
    pub owner: SymbolId,
    pub image: MirDataImage,
    pub zero_fill: u16,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirRuntimeHelperDecl {
    pub helper: MirRuntimeHelper,
    pub target: MirRuntimeHelperTarget,
    pub abi: MirCallAbi,
    pub effects: MirEffects,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirMachineBlock {
    pub id: MirMachineBlockId,
    pub items: Vec<MirMachineItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirMachineItem {
    Byte(u8),
    Word(u16),
    StringLiteral(String),
    CharLiteral(char),
    Name(String),
    AddressExpr {
        selector: Option<MirMachineByteSelector>,
        explicit_address: bool,
        atom: MirMachineAtom,
        offset: i32,
        text: String,
    },
    AddressByte {
        high: bool,
        name: String,
    },
    Relocation {
        kind: InlineAsmRelocationKind,
        target: MirInlineAsmTarget,
        addend: i32,
        requires_zero_page: bool,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirInlineAsmTarget {
    Memory(MirMem),
    Routine(RoutineId),
    Absolute(u16),
    InlineOffset(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirMachineAtom {
    Number(u16),
    Name(String),
    Current,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirMachineByteSelector {
    Low,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirRuntimeHelperTarget {
    KnownAbsolute(u16),
    Routine(RoutineId),
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirRoutine {
    pub id: RoutineId,
    pub name: String,
    pub abi: MirRoutineAbi,
    pub frame: MirFrame,
    pub temps: Vec<MirTemp>,
    pub blocks: Vec<MirBlock>,
    pub effects: MirEffects,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirRoutineAbi {
    /// Ordinary Action ABI entry whose private parameter storage is not part of
    /// an externally observable routine boundary.
    Action,
    /// Ordinary Action ABI plus the stable executable-entry designation.
    ProgramEntry,
    /// Program entry whose physical Action ABI boundary also remains
    /// observable, for example because it was declared at `=*`.
    ProgramEntryObservable,
    /// Action ABI entry whose physical parameter storage remains observable,
    /// for example a system-address or current-location routine.
    ActionObservable,
    /// A signature-only interface that the selected runtime must resolve.
    ExternalInterface,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MirFrame {
    pub params: Vec<MirStorageSlot>,
    pub locals: Vec<MirStorageSlot>,
    pub spills: Vec<MirSpillId>,
    pub virtual_zero_page: Vec<MirZpSlot>,
    pub fixed_zero_page: Vec<MirFixedZpSlot>,
    pub zero_page_allocations: Vec<MirZpAllocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirZpAllocation {
    pub slot: MirZpSlot,
    pub start: MirFixedZpSlot,
    pub size: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirTemp {
    pub id: MirTempId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirStorageId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirStorageSlot {
    pub id: MirStorageId,
    pub name: Option<String>,
    pub storage: MirStorageClass,
    /// Bytes reserved for this object. This is independent of the width of an
    /// individual 6502 transfer and may be larger than two bytes.
    pub storage_size: u16,
    /// Width of the slot when it participates in the byte/word scalar lane.
    /// Address-only objects such as inline arrays, records, and native REAL
    /// storage do not have a scalar width.
    pub scalar_width: Option<MirWidth>,
    pub base: MirStorageBase,
    pub offset: u16,
    pub mutable: bool,
    pub init: Option<MirStorageInit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirStorageClass {
    Scalar,
    Array,
    Record,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirStorageInit {
    Bytes {
        image: MirDataImage,
        zero_fill: u16,
        mutable: bool,
        section: String,
    },
    Descriptor {
        backing: MirStorageBacking,
        descriptor_size: u16,
        size_word: Option<u16>,
        mutable: bool,
        section: String,
    },
    RoutineAddress {
        routine: RoutineId,
        descriptor_size: u16,
        size_word: Option<u16>,
        mutable: bool,
        section: String,
    },
    ZeroFill {
        bytes: u16,
        mutable: bool,
        section: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirStorageBacking {
    pub image: MirDataImage,
    pub zero_fill: u16,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirStorageBase {
    Param(ParamId),
    /// Parameter metadata retained for ABI placement and signature reporting
    /// after its unobservable physical storage has been elided.
    ParamAbiOnly(ParamId),
    Local(LocalId),
    LocalAlias {
        id: LocalId,
        target: LocalId,
    },
    Spill(MirSpillId),
    Global(SymbolId),
    Static(SymbolId),
    Absolute(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirBlock {
    pub id: MirBlockId,
    pub label: String,
    pub params: Vec<MirBlockParam>,
    pub ops: Vec<MirOp>,
    pub terminator: MirTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirBlockParam {
    pub dest: MirTempId,
    pub width: MirWidth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirOp {
    LoadImm {
        dst: MirDef,
        value: u16,
        width: MirWidth,
    },
    Load {
        dst: MirDef,
        src: MirAddr,
        width: MirWidth,
    },
    Store {
        dst: MirAddr,
        src: MirValue,
        width: MirWidth,
    },
    Move {
        dst: MirDef,
        src: MirValue,
        width: MirWidth,
    },
    LeaAddr {
        dst: MirDef,
        target: MirMem,
        width: MirWidth,
    },
    Extend {
        dst: MirDef,
        src: MirValue,
        from_width: MirWidth,
        to_width: MirWidth,
        signed: bool,
    },
    Truncate {
        dst: MirDef,
        src: MirValue,
        from_width: MirWidth,
        to_width: MirWidth,
    },
    Unary {
        op: MirUnaryOp,
        dst: MirDef,
        src: MirValue,
        width: MirWidth,
    },
    Binary {
        op: MirBinaryOp,
        dst: MirDef,
        left: MirValue,
        right: MirValue,
        width: MirWidth,
        carry_in: Option<MirCarryIn>,
        carry_out: MirCarryOut,
    },
    UpdateMem {
        op: MirUpdateOp,
        mem: MirMem,
        width: MirWidth,
    },
    /// Native byte read/modify/write through an absolute base indexed by X.
    /// This is a post-home 6502 operation; X is an implicit input.
    UpdateIndexedMem {
        op: MirUpdateOp,
        base: MirMem,
    },
    AddByteToWordMem {
        mem: MirMem,
        value: MirValue,
    },
    SubByteFromWordMem {
        mem: MirMem,
        value: MirValue,
    },
    /// Add or subtract a byte loaded through `source` from the pointer value
    /// held by `source`, writing the word result directly to `dst`.
    ///
    /// This is a post-home 6502 carry-chain operation. Selection must keep the
    /// destination word disjoint from the source pointer pair.
    OffsetPointerByIndirectByte {
        op: MirBinaryOp,
        dst: MirMem,
        source: MirAddressConsumer,
        offset: u16,
    },
    /// Copy a word between two prepared pointer pairs.
    ///
    /// Emission reads both source bytes before either destination write, then
    /// preserves low-to-high read and write order. This keeps overlapping
    /// transfers well-defined without transient logical homes.
    CopyIndirectWord {
        source: MirAddressConsumer,
        destination: MirAddressConsumer,
        source_offset: u16,
        destination_offset: u16,
    },
    /// Copy a directly addressable word through a prepared destination pointer.
    ///
    /// Emission reads both source lanes before either destination write, uses
    /// balanced stack staging plus X, and writes the destination low lane
    /// before the high lane.
    CopyDirectWordToIndirect {
        source: MirMem,
        destination: MirAddressConsumer,
        destination_offset: u16,
    },
    /// Copy a bounded indirect byte range into consecutive fixed-ZP homes.
    ///
    /// Emission reads and stack-stages the complete source range before the
    /// first destination write, then restores the bytes into the fixed homes
    /// in source order. The stack is balanced on exit.
    CopyIndirectBytesToFixedZp {
        source: MirAddressConsumer,
        source_offset: u16,
        destinations: Vec<MirFixedZpSlot>,
    },
    /// Subtract an ordinary direct word from an absolute-backed word and store
    /// the result through a prepared pointer.
    ///
    /// The source retains its structured storage identity through rewrite
    /// validation and is resolved to its fixed address only during emission.
    /// Both source lanes and both RHS lanes are read before the first indirect
    /// destination write. Carry/borrow flows from low to high.
    AbsoluteWordSubToIndirect {
        source: MirMem,
        rhs: MirMem,
        destination: MirAddressConsumer,
        destination_offset: u16,
    },
    Compare {
        dst: MirCondDest,
        op: MirCompareOp,
        left: MirValue,
        right: MirValue,
        width: MirWidth,
        signed: bool,
    },
    /// Native byte comparison through two independently materialized pointer
    /// pairs. Both operands use the same constant Y offset.
    CompareIndirectBytes {
        dst: MirCondDest,
        op: MirCompareOp,
        left: MirAddressConsumer,
        right: MirAddressConsumer,
        offset: u16,
        signed: bool,
    },
    CompareIndirectWords {
        dst: MirCondDest,
        op: MirCompareOp,
        left: MirAddressConsumer,
        right: MirAddressConsumer,
        offset: u16,
        signed: bool,
    },
    /// Compare the two six-byte Atari packed-REAL operands staged in FR0
    /// ($D4..$D9) and FR1 ($E0..$E5).
    ///
    /// The operation leaves A equal to canonical Boolean 0 or 1 for `op`, so
    /// Z is set for false and clear for true. It is a target operation used
    /// when a native REAL comparison feeds a branch directly; this avoids
    /// materializing a Boolean expression for every packed byte.
    PackedRealCompare {
        op: MirCompareOp,
    },
    /// Copy one six-byte Atari packed-REAL value without exposing its lanes as
    /// six simultaneously live MIR temporaries.
    ///
    /// Emission uses a compact descending X-indexed loop for ordinary direct
    /// ranges, with a forward fallback only for a statically known leftward
    /// overlap. It stack-stages the complete source with two compact Y-indexed
    /// loops when either endpoint is indirect. When `negate` is set, emission
    /// toggles the packed sign bit only when the copied magnitude is nonzero,
    /// preserving canonical positive zero.
    PackedRealCopy {
        source: MirAddr,
        destination: MirAddr,
        source_offset: u16,
        destination_offset: u16,
        negate: bool,
    },
    Call {
        target: MirCallTarget,
        abi: MirCallAbi,
        args: Vec<MirCallArg>,
        result: Option<MirCallResult>,
        effects: MirEffects,
    },
    RuntimeHelper {
        helper: MirRuntimeHelper,
        args: Vec<MirArgHome>,
        result: Option<MirResultHome>,
        effects: MirEffects,
    },
    MaterializeAddress {
        consumer: MirAddressConsumer,
        value: MirValue,
    },
    MaterializeIndexedAddress {
        consumer: MirAddressConsumer,
        base: MirValue,
        index: MirValue,
        scale: u8,
    },
    AdvanceAddress {
        consumer: MirAddressConsumer,
        index: MirValue,
        scale: u8,
    },
    LoadIndirect {
        consumer: MirAddressConsumer,
        dst: MirDef,
        offset: u16,
    },
    StoreIndirect {
        consumer: MirAddressConsumer,
        src: MirValue,
        offset: u16,
    },
    IndirectByteCompound {
        op: MirBinaryOp,
        target: MirAddressConsumer,
        source: MirAddressConsumer,
        offset: u16,
    },
    /// Add a source word to a target word through two prepared pointer pairs.
    ///
    /// Emission reads both target/source lanes before either target write,
    /// carries from low to high, stages both result lanes in reserved fixed
    /// scratch, and writes the target low lane before the high lane.
    IndirectWordCompound {
        op: MirBinaryOp,
        target: MirAddressConsumer,
        source: MirAddressConsumer,
        offset: u16,
    },
    Barrier {
        effects: MirEffects,
    },
    MachineBlock {
        id: MirMachineBlockId,
        effects: MirEffects,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirDef {
    VTemp(MirTempId),
    VTempByte { id: MirTempId, byte: u8 },
    Reg(MirReg),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirValue {
    ConstU8(u8),
    ConstU16(u16),
    Def(MirDef),
    Word {
        lo: Box<MirValue>,
        hi: Box<MirValue>,
    },
    StaticAddr(SymbolId),
    GlobalAddr(SymbolId),
    RoutineAddr(RoutineId),
    RoutineAddrByte {
        id: RoutineId,
        byte: u8,
    },
    StorageAddrByte {
        mem: MirMem,
        byte: u8,
    },
    PointerCell(MirMem),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirMem {
    Absolute(u16),
    Static { id: SymbolId, offset: u16 },
    Global { id: SymbolId, offset: u16 },
    Local { id: LocalId, offset: u16 },
    Param { id: ParamId, offset: u16 },
    Spill { id: MirSpillId, offset: u16 },
    ZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirAddr {
    Direct(MirMem),
    Label(MirLabel),
    ZeroPageIndexedX {
        base: MirZpSlot,
    },
    AbsoluteIndexedX {
        base: MirMem,
    },
    AbsoluteIndexedY {
        base: MirMem,
    },
    IndirectIndexedY {
        zp: MirZpSlot,
    },
    FixedIndirectIndexedY {
        zp: MirFixedZpSlot,
    },
    ComputedIndex {
        base: MirValue,
        index: MirValue,
        elem_size: u16,
        offset: u16,
    },
    PointerCell {
        ptr: MirMem,
        offset: u16,
    },
    PointerIndex {
        ptr: MirMem,
        index: MirValue,
        elem_size: u16,
        offset: u16,
    },
    Deref {
        ptr: MirValue,
        offset: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirAddressConsumer {
    IndirectIndexedY(MirPointerPair),
    /// The pointer pair contains the unindexed base (with the scale carry
    /// folded into its high byte) and Y contains the scaled byte offset.
    /// This form is intentionally 6502-specific and is only valid for a
    /// scale-two indexed materialization followed by byte offsets 0 or 1.
    /// Load consumers may visit those two offsets in either order; emission
    /// adjusts Y with INY or DEY while retaining the scaled index. Store
    /// consumers remain monotone because a trailing DEY would expose flags not
    /// represented by the store operation.
    ScaledIndirectIndexedY(MirPointerPair),
}

impl MirAddressConsumer {
    pub(crate) fn pointer_pair(self) -> MirPointerPair {
        match self {
            Self::IndirectIndexedY(pair) | Self::ScaledIndirectIndexedY(pair) => pair,
        }
    }

    pub(crate) fn uses_scaled_y(self) -> bool {
        matches!(self, Self::ScaledIndirectIndexedY(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirPointerPair {
    Fixed { lo: MirFixedZpSlot },
    Virtual(MirZpSlot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirWidth {
    Byte,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirReg {
    A,
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirFlag {
    C,
    Z,
    N,
    V,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirUnaryOp {
    Neg,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirBinaryOp {
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
pub enum MirUpdateOp {
    Inc,
    Dec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirCompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirCondDest {
    Temp(MirTempId),
    Flags,
}

/// Carry input required by a 6502 arithmetic operation.
///
/// Before emission, `Add` must use `Clear` for the low lane or `FromPrevious`
/// for a carry chain. `Sub` must use `Set` for the low lane or `FromPrevious`
/// for a borrow chain. `None` is only valid for operations that do not consume
/// carry, such as logical byte operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirCarryIn {
    Clear,
    Set,
    FromPrevious,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirCarryOut {
    Ignore,
    Produce,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirCallTarget {
    Routine(RoutineId),
    Indirect {
        target: MirValue,
        width: MirWidth,
    },
    Builtin {
        name: String,
        address: Option<u16>,
    },
    Runtime {
        name: String,
        address: Option<u16>,
    },
    /// Fixed Atari OS floating-point package entry point. This is a target
    /// service, not an Action runtime helper or source-level routine.
    AtariFpp(MirAtariFppService),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirAtariFppService {
    IntegerToFloat,
    FloatToInteger,
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl MirAtariFppService {
    /// First byte of the Atari floating-point package's zero-page workspace.
    pub const WORKSPACE_START: u16 = 0x00D4;
    /// Size of the complete package-owned zero-page workspace ($D4-$FF).
    pub const WORKSPACE_SIZE: u16 = 0x002C;

    pub const fn address(self) -> u16 {
        match self {
            Self::IntegerToFloat => 0xD9AA,
            Self::FloatToInteger => 0xD9D2,
            Self::Subtract => 0xDA60,
            Self::Add => 0xDA66,
            Self::Multiply => 0xDADB,
            Self::Divide => 0xDB28,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::IntegerToFloat => "IFP",
            Self::FloatToInteger => "FPI",
            Self::Add => "FADD",
            Self::Subtract => "FSUB",
            Self::Multiply => "FMULT",
            Self::Divide => "FDIV",
        }
    }

    /// Audited portable effects for the core Atari FPP services used by
    /// native REAL lowering.
    ///
    /// Individual entry points use smaller subsets, but the complete package
    /// workspace is the stable contract across the original Atari math pack,
    /// AltirraOS, and compatible replacement ROMs. The routines do not make
    /// nested OS calls and return with the hardware stack balanced.
    pub fn effects(self) -> MirEffects {
        let _ = self;
        let clobbers = MirRegisterSet {
            a: true,
            x: true,
            y: true,
            flags: true,
            sp: false,
        };
        let workspace = || {
            MirMemoryEffect::Regions(vec![MirMemoryRegion {
                kind: MirMemoryRegionKind::ZeroPage,
                offset: Self::WORKSPACE_START,
                size: Self::WORKSPACE_SIZE,
            }])
        };
        MirEffects {
            memory_reads: workspace(),
            memory_writes: workspace(),
            clobbers,
            stack_depth_delta: Some(0),
            may_call_os: false,
            opaque: false,
            ..MirEffects::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirCallAbi {
    pub params: Vec<MirArgHome>,
    pub result: Option<MirResultHome>,
    pub clobbers: MirRegisterSet,
    pub preserves: MirRegisterSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirArgHome {
    Reg(MirReg),
    RegisterPair {
        lo: MirReg,
        hi: MirReg,
    },
    BytePair {
        lo: Box<MirArgHome>,
        hi: Box<MirArgHome>,
    },
    ZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
    Absolute(u16),
    StackFrame {
        base: u16,
        offset: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirCallArg {
    pub value: MirValue,
    pub width: MirWidth,
    pub home: MirArgHome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirCallResult {
    pub dst: MirDef,
    pub width: MirWidth,
    pub home: MirResultHome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirResultHome {
    Reg(MirReg),
    RegisterPair { lo: MirReg, hi: MirReg },
    ZeroPage(MirZpSlot),
    FixedZeroPage(MirFixedZpSlot),
    Absolute(u16),
    ReturnSlot { offset: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirRuntimeHelper {
    Mul,
    Div,
    Mod,
    Lsh,
    Rsh,
    SArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirMachineBlockId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirTerminator {
    Jump(MirEdge),
    Branch {
        cond: MirCond,
        then_edge: MirEdge,
        else_edge: MirEdge,
    },
    Return,
    Exit,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirEdge {
    pub target: MirBlockId,
    pub args: Vec<MirEdgeArg>,
}

impl MirEdge {
    pub fn plain(target: MirBlockId) -> Self {
        Self {
            target,
            args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirEdgeArg {
    pub value: MirValue,
    pub width: MirWidth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirCond {
    Deferred,
    BoolValue(MirValue),
    FlagTest(MirFlagTest),
    AnyFlagTest([MirFlagTest; 2]),
    FusedCompare {
        producer: MirOpRef,
        flag_test: MirFlagTest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirFlagTest {
    ZSet,
    ZClear,
    CSet,
    CClear,
    NSet,
    NClear,
    VSet,
    VClear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirOpRef {
    pub block: MirBlockId,
    pub op_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MirEffects {
    pub memory_reads: MirMemoryEffect,
    pub memory_writes: MirMemoryEffect,
    /// Machine registers and status flags whose incoming values are observed.
    ///
    /// Calls normally describe their inputs through the ABI. This field is
    /// primarily for structured inline machine code, where a block such as
    /// `STA target` consumes the incoming accumulator without changing it
    /// first.
    pub reads: MirRegisterSet,
    pub clobbers: MirRegisterSet,
    pub preserves: MirRegisterSet,
    pub stack_depth_delta: Option<i8>,
    pub may_call_os: bool,
    pub opaque: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MirMemoryEffect {
    #[default]
    None,
    Regions(Vec<MirMemoryRegion>),
    Unknown,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirMemoryRegion {
    pub kind: MirMemoryRegionKind,
    pub offset: u16,
    pub size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirMemoryRegionKind {
    Local(LocalId),
    Param(ParamId),
    Global(SymbolId),
    Static(SymbolId),
    AbsoluteRange,
    ZeroPage,
    Stack,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MirRegisterSet {
    pub a: bool,
    pub x: bool,
    pub y: bool,
    pub flags: bool,
    pub sp: bool,
}
