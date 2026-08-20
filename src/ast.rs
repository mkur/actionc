use crate::asm6502::InlineAsmProgram;
use crate::lexer::NumberLiteral;
use crate::source::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// Retained as declaration regions for compatibility with the existing
    /// semantic and code-generation pipeline. Named files always have one.
    pub modules: Vec<Module>,
    pub source_kind: SourceUnitKind,
}

impl Program {
    pub fn legacy(modules: Vec<Module>) -> Self {
        Self {
            modules,
            source_kind: SourceUnitKind::Legacy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceUnitKind {
    Legacy,
    Named(NamedModuleDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedModuleDecl {
    pub path: ModulePath,
    pub imports: Vec<ImportDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub path: ModulePath,
    /// The explicit or default alias. Open imports have no alias.
    pub alias: Option<String>,
    pub open: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePath {
    pub components: Vec<String>,
    pub canonical_components: Vec<String>,
    pub span: Span,
}

/// A source-level symbol reference which may be qualified through an imported
/// module alias. Keeping components separate prevents later compiler stages
/// from rediscovering namespace meaning by splitting display strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    pub components: Vec<String>,
    text: String,
}

impl QualifiedName {
    pub fn simple(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            components: vec![name.clone()],
            text: name,
        }
    }

    pub fn new(components: Vec<String>) -> Self {
        debug_assert!(!components.is_empty());
        let text = components.join(".");
        Self { components, text }
    }

    pub fn display_name(&self) -> &str {
        &self.text
    }

    pub fn simple_name(&self) -> Option<&str> {
        (self.components.len() == 1).then(|| self.components[0].as_str())
    }

    pub fn module_member(&self) -> Option<(&str, &str)> {
        (self.components.len() == 2)
            .then(|| (self.components[0].as_str(), self.components[1].as_str()))
    }
}

impl std::ops::Deref for QualifiedName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

impl std::fmt::Display for QualifiedName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.text.fmt(formatter)
    }
}

impl PartialEq<str> for QualifiedName {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for QualifiedName {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl From<String> for QualifiedName {
    fn from(name: String) -> Self {
        Self::simple(name)
    }
}

impl From<&str> for QualifiedName {
    fn from(name: &str) -> Self {
        Self::simple(name)
    }
}

impl ModulePath {
    pub fn new(components: Vec<String>, span: Span) -> Self {
        let canonical_components = components
            .iter()
            .map(|component| component.to_ascii_lowercase())
            .collect();
        Self {
            components,
            canonical_components,
            span,
        }
    }

    pub fn display_name(&self) -> String {
        self.components.join(".")
    }

    pub fn canonical_name(&self) -> String {
        self.canonical_components.join(".")
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Private,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Define(DefineDecl),
    Include(IncludeDirective),
    Set(SetDirective),
    Declaration(Decl),
    Routine(Routine),
    Statement(Stmt),
    Unsupported { span: Span, note: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefineDecl {
    pub entries: Vec<DefineEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefineEntry {
    pub name: String,
    pub value: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeDirective {
    pub path: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetDirective {
    pub address: Expr,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decl {
    Var(VarDecl),
    Const(ConstDecl),
    Type(TypeDecl),
    Record(RecordDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstDecl {
    pub visibility: Visibility,
    pub declared_type: Option<FundType>,
    pub entries: Vec<ConstEntry>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstEntry {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarDecl {
    pub visibility: Visibility,
    pub qualifiers: VarQualifiers,
    pub ty: TypeRef,
    pub storage: VarStorage,
    pub entries: Vec<DeclEntry>,
    pub span: Span,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VarQualifiers {
    pub is_volatile: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclEntry {
    pub name: String,
    pub size: Option<Expr>,
    pub initializer: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDecl {
    pub visibility: Visibility,
    pub name: String,
    pub fields: Vec<VarDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDecl {
    pub visibility: Visibility,
    pub name: String,
    pub fields: Vec<VarDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub base: TypeBase,
    pub pointer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeBase {
    Fund(FundType),
    Named(QualifiedName),
    Callable(RoutineKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarStorage {
    Plain,
    Array,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    pub visibility: Visibility,
    pub kind: RoutineKind,
    pub name: String,
    pub system_address: Option<Expr>,
    pub params: Vec<VarDecl>,
    pub locals: Vec<Decl>,
    pub body: Vec<Stmt>,
    pub annotations: Vec<ActioncAnnotation>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActioncAnnotation {
    ReturnsAEqualsA0,
    DebugProfileCompat,
    Preserves {
        registers: AnnotationRegisterSet,
        zero_page: AnnotationZeroPageRanges,
    },
    Clobbers {
        registers: AnnotationRegisterSet,
        zero_page: AnnotationZeroPageRanges,
    },
    Writes {
        addresses: AnnotationAddressRanges,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationRegisterSet {
    pub a: bool,
    pub x: bool,
    pub y: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationRegister {
    A,
    X,
    Y,
}

impl AnnotationRegisterSet {
    pub fn iter(self) -> impl Iterator<Item = AnnotationRegister> {
        [
            self.a.then_some(AnnotationRegister::A),
            self.x.then_some(AnnotationRegister::X),
            self.y.then_some(AnnotationRegister::Y),
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AnnotationZeroPageRanges {
    pub ranges: [Option<AnnotationZeroPageRange>; 8],
    pub symbols: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AnnotationAddressRanges {
    pub ranges: [Option<AnnotationAddressRange>; 8],
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationZeroPageRange {
    pub start: u8,
    pub end: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationAddressRange {
    pub start: u16,
    pub end: u16,
}

impl AnnotationZeroPageRanges {
    pub fn push(&mut self, range: AnnotationZeroPageRange) -> bool {
        if let Some(slot) = self.ranges.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(range);
            true
        } else {
            false
        }
    }
}

impl AnnotationAddressRanges {
    pub fn push(&mut self, range: AnnotationAddressRange) -> bool {
        if let Some(slot) = self.ranges.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(range);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineKind {
    Proc,
    Func { return_type: FundType },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundType {
    Byte,
    Card,
    Char,
    Int,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Define(DefineDecl),
    Return(Option<Expr>),
    Exit {
        span: Span,
    },
    Assign {
        target: Expr,
        value: Expr,
        span: Span,
    },
    CompoundAssign {
        target: Expr,
        op: BinaryOp,
        value: Expr,
        span: Span,
    },
    Call {
        expr: Expr,
        span: Span,
    },
    MachineBlock {
        items: Vec<MachineItem>,
        text: String,
        span: Span,
    },
    InlineAsm {
        program: InlineAsmProgram,
        span: Span,
    },
    If {
        branches: Vec<IfBranch>,
        else_body: Vec<Stmt>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    DoUntil {
        body: Vec<Stmt>,
        condition: Option<Expr>,
        span: Span,
    },
    For {
        target: Expr,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
        span: Span,
    },
    Unsupported {
        span: Span,
        note: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfBranch {
    pub condition: Expr,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineItem {
    Number(NumberLiteral),
    StringLiteral(String),
    CharLiteral(char),
    Name(QualifiedName),
    AddressExpr(MachineAddressExpr),
    AddressByte {
        selector: AddressByteSelector,
        name: QualifiedName,
    },
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineAddressExpr {
    pub selector: Option<AddressByteSelector>,
    pub explicit_address: bool,
    pub atom: MachineAddressAtom,
    pub offset: i32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineAddressAtom {
    Number(NumberLiteral),
    Name(QualifiedName),
    Current,
}

pub fn machine_address_symbolic_offset(text: &str) -> Option<(bool, &str)> {
    let (index, negative) = text.char_indices().rev().find_map(|(index, ch)| match ch {
        '+' if index > 0 => Some((index, false)),
        '-' if index > 0 => Some((index, true)),
        _ => None,
    })?;
    let name = &text[index + 1..];
    if machine_address_offset_name(name) {
        Some((negative, name))
    } else {
        None
    }
}

fn machine_address_offset_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('A'..='Z' | 'a'..='z' | '_'))
        && chars.all(|ch| matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressByteSelector {
    Low,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    Missing,
    Raw,
    InitializerList(Vec<InitializerElement>),
    CurrentLocation,
    Number(NumberLiteral),
    String(String),
    Char(char),
    Name(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Cast {
        ty: TypeRef,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Field {
        base: Box<Expr>,
        field: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitializerElement {
    pub kind: InitializerElementKind,
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitializerElementKind {
    Literal {
        value: InitializerLiteral,
        negative: bool,
    },
    Address {
        selector: Option<AddressByteSelector>,
        target: QualifiedName,
        addend: i32,
    },
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitializerLiteral {
    Number(NumberLiteral),
    Char(char),
    True,
    False,
    Nil,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Neg,
    AddressOf,
    Deref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
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
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
