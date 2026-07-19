use super::facts::{BlockId, LocalId, NirType, NirValue, ParamId, SymbolId, TempId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirProgram {
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
    pub storage_size: u16,
    pub array: Option<NirArrayGlobalFact>,
    pub init: Option<NirGlobalInit>,
    pub backing: NirGlobalBacking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirArrayGlobalFact {
    pub elem_size: u16,
    pub length: Option<u16>,
    pub pointer_backed: bool,
    pub address_initializer: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirGlobalInit {
    Bytes {
        bytes: Vec<u8>,
        zero_fill: u16,
        mutable: bool,
        section: String,
    },
    Descriptor {
        backing: NirDataBacking,
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
        name: String,
        descriptor_size: u16,
        size_word: Option<u16>,
        mutable: bool,
        section: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirDataBacking {
    pub owner: SymbolId,
    pub bytes: Vec<u8>,
    pub zero_fill: u16,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirStorageInit {
    Bytes {
        bytes: Vec<u8>,
        zero_fill: u16,
        mutable: bool,
        section: String,
    },
    Descriptor {
        backing: NirStorageBacking,
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
pub struct NirStorageBacking {
    pub bytes: Vec<u8>,
    pub zero_fill: u16,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirGlobalBacking {
    Ordinary,
    Absolute(u16),
    Alias { target: String, offset: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirStaticData {
    pub id: SymbolId,
    pub name: String,
    pub ty: NirType,
    pub bytes: Vec<u8>,
    pub display: String,
    pub alignment: u16,
    pub mutable: bool,
    pub section: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirPlace {
    pub kind: NirPlaceKind,
    pub ty: Option<NirType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirPlaceKind {
    Symbol(String),
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
    Absolute(u16),
    UnresolvedName(String),
    Deref {
        addr: NirValue,
    },
    Index {
        base_addr: NirValue,
        index: NirValue,
        elem_ty: NirType,
        elem_size: u16,
    },
    Field {
        base: Box<NirPlace>,
        offset: u16,
        ty: NirType,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NirOperand {
    pub kind: NirOperandKind,
    pub ty: Option<NirType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirOperandKind {
    Missing,
    Raw(String),
    UnresolvedName(String),
    CurrentLocation,
    Literal { text: String, value: Option<u16> },
    Temp(TempId),
    Symbol(String),
    Place(Box<NirPlace>),
    AddressOf(Box<NirPlace>),
    AddressOfSymbol(String),
    Expr(String),
    Call(String),
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
    User(String),
    Builtin(String),
    Indirect { target: NirValue, ty: NirType },
    Runtime { name: String, address: Option<u16> },
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
    Known { regions: usize },
    Unknown,
    All,
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
    pub storage: NirStorageClass,
    pub ty: NirType,
    pub backing: NirLocalBacking,
    pub init: Option<NirStorageInit>,
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
    Absolute(u16),
    Alias {
        target: LocalId,
        target_name: String,
        offset: u16,
    },
    GlobalAlias {
        target: SymbolId,
        target_name: String,
        offset: u16,
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
    CurrentLocationEntry,
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
    pub op_index: usize,
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
    pub ops: Vec<NirOp>,
    pub terminator: NirTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NirOp {
    Define {
        name: String,
        value: String,
    },
    Set {
        address: NirOperand,
        value: NirOperand,
    },
    Declare {
        name: String,
        kind: String,
    },
    Assign {
        target: NirPlace,
        value: NirOperand,
    },
    CompoundAssign {
        target: NirPlace,
        op: String,
        value: NirOperand,
    },
    Load {
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
        op: NirCompareOp,
        left: NirValue,
        right: NirValue,
    },
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
    Unsupported {
        note: String,
    },
    Note {
        text: String,
    },
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
    Raw(String),
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
pub enum NirTerminator {
    Open,
    Fallthrough,
    Goto(String),
    Branch {
        condition: NirValue,
        then_label: String,
        else_label: String,
    },
    Return(Option<NirValue>),
    Exit,
    Unknown(String),
}
