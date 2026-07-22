use crate::nir::{LocalId, ParamId, SymbolId};

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
    pub bytes: Vec<u8>,
    pub display: String,
    pub alignment: u16,
    pub mutable: bool,
    pub section: String,
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
pub enum MirGlobalBacking {
    Ordinary { offset: u16 },
    Absolute(u16),
    Alias { target: SymbolId, offset: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirGlobalInit {
    Bytes {
        bytes: Vec<u8>,
        zero_fill: u16,
        mutable: bool,
        section: String,
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
    pub bytes: Vec<u8>,
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
    RuntimeSymbol(String),
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
    Action,
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
    pub width: MirWidth,
    pub base: MirStorageBase,
    pub offset: u16,
    pub mutable: bool,
    pub init: Option<MirStorageInit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirStorageInit {
    Bytes {
        bytes: Vec<u8>,
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
    pub bytes: Vec<u8>,
    pub zero_fill: u16,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirStorageBase {
    Param(ParamId),
    Local(LocalId),
    LocalAlias { id: LocalId, target: LocalId },
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
    AddByteToWordMem {
        mem: MirMem,
        value: MirValue,
    },
    SubByteFromWordMem {
        mem: MirMem,
        value: MirValue,
    },
    Compare {
        dst: MirCondDest,
        op: MirCompareOp,
        left: MirValue,
        right: MirValue,
        width: MirWidth,
        signed: bool,
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
    Indirect { target: MirValue, width: MirWidth },
    Builtin { name: String, address: Option<u16> },
    Runtime { name: String, address: Option<u16> },
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
