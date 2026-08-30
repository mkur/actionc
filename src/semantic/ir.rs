use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::asm6502::{
    InlineAsmMode, InlineAsmProgram, InlineAsmRelocationKind, InlineAsmRelocationTarget,
    InlineAsmSymbolUse,
};
use crate::ast::{
    ActioncAnnotation, AddressByteSelector, BinaryOp, ConstDecl, Decl, DefineDecl, Expr, ExprKind,
    FundType, IncludeDirective, InitializerElement, InitializerElementKind, InitializerLiteral,
    Item, LexicalBlockSyntaxId, MachineAddressAtom, MachineAddressExpr, MachineItem, Module,
    OrgDirective, Program, QualifiedName, RecordDecl, Routine, RoutineKind, SetDirective, Stmt,
    TypeBase, TypeDecl, TypeRef, UnaryOp, VarDecl, VarStorage,
};
use crate::atari_real::AtariReal;
use crate::includes::{LoadedCompilation, ModuleId};
use crate::lexer::{NumberLiteral, TokenKind, tokenize};
use crate::source::Span;

use super::{
    ArrayType, CallableType, ConstValue, ExprClass, FieldId, RecordFieldType, RecordType,
    ScalarSignedness, ScalarType, ScopeId, SemanticLayoutFacts, SemanticModel,
    SemanticNameResolution, StmtFlowFacts, SymbolClass, SymbolId, ValueType, ValueTypeBase,
    routine_control_flow_facts, subject::PlaceAccess,
};

pub fn lower_program(program: &Program, model: &SemanticModel) -> SemProgram {
    let mut builder = IrBuilder::new(model);
    builder.lower_program(program)
}

static LEGACY_SYS_INTERFACE: OnceLock<SemModule> = OnceLock::new();

fn legacy_sys_interface() -> &'static SemModule {
    LEGACY_SYS_INTERFACE.get_or_init(|| {
        crate::runtime_source::compile_embedded_module("sys.act")
            .expect("the embedded SYS interface must pass semantic analysis")
            .modules
            .into_iter()
            .find(|module| {
                module
                    .path
                    .as_ref()
                    .is_some_and(|path| path.canonical_name() == "sys")
            })
            .expect("the embedded SYS source must define MODULE SYS")
    })
}

pub fn lower_compilation(compilation: &LoadedCompilation, model: &SemanticModel) -> SemProgram {
    if matches!(
        compilation.root_module().program.source_kind,
        crate::ast::SourceUnitKind::Legacy
    ) {
        return IrBuilder::new(model).lower_legacy_compilation(compilation);
    }
    IrBuilder::new(model).lower_named_compilation(compilation)
}

pub fn format_program(program: &SemProgram) -> String {
    let mut out = SemIrFormatter::default();
    out.program(program);
    out.finish()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemProgram {
    pub modules: Vec<SemModule>,
    pub layout: SemanticLayoutFacts,
    /// Resolved root-source program placement. This compile-time fact is
    /// consumed before NIR lowering and never becomes an executable NIR op.
    pub origin: Option<SemOrigin>,
    /// Stable identity of the source root module. Legacy source regions share
    /// `None`; named compilations use the loader-assigned module identity.
    pub root_module: Option<ModuleId>,
    /// Stable identity of the last source PROC in the root program which emits
    /// code. Link selection and runtime composition must preserve this value
    /// instead of inferring an entry from transformed routine order.
    pub entry_routine: Option<SymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemOrigin {
    pub address: u16,
    pub span: Span,
}

impl SemProgram {
    pub(crate) fn program_entry_routine(&self) -> Option<&SemRoutine> {
        let entry = self.entry_routine?;
        Some(
            self.modules
                .iter()
                .flat_map(|module| &module.items)
                .filter_map(|item| match item {
                    SemItem::Routine(routine) if routine.symbol.id == entry => Some(routine),
                    _ => None,
                })
                .next()
                .expect("SemIR program entry must refer to a retained routine"),
        )
    }
}

fn source_program_entry(modules: &[SemModule], root_module: Option<ModuleId>) -> Option<SymbolId> {
    modules
        .iter()
        .filter(|module| module.id == root_module)
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            SemItem::Routine(routine)
                if !routine.is_external
                    && matches!(routine.signature.kind, RoutineKind::Proc)
                    && routine.system_address.as_ref().is_none_or(|address| {
                        matches!(address.kind, SemExprKind::CurrentLocation)
                    }) =>
            {
                Some(routine.symbol.id)
            }
            _ => None,
        })
        .last()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemModule {
    pub id: Option<ModuleId>,
    pub path: Option<crate::ast::ModulePath>,
    pub items: Vec<SemItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemItem {
    Define(SemDefine),
    Const(SemConst),
    Include(SemInclude),
    Set(SemSet),
    Declaration(SemDeclaration),
    Routine(SemRoutine),
    Statement(SemStmt),
    Unsupported { span: Span, note: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemSymbolRef {
    pub id: SymbolId,
    pub name: String,
    pub defining_module: Option<ModuleId>,
    pub canonical_qualified_key: String,
    pub qualified_name: String,
    /// Readable lexical path retained as source metadata, never executable identity.
    pub lexical_display_name: Option<String>,
    pub class: SymbolClass,
    pub ty: Option<ValueType>,
    pub is_volatile: bool,
    pub scope: ScopeId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemDefine {
    pub symbol: SemSymbolRef,
    pub value: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemConst {
    pub symbol: SemSymbolRef,
    pub value: ConstValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemInclude {
    pub path: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemSet {
    pub address: SemExpr,
    pub value: SemExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemDeclaration {
    pub symbol: SemSymbolRef,
    pub ty: SemType,
    pub storage: SemDeclarationStorage,
    pub initializer: Option<SemExpr>,
    /// Layout-resolved static data writes. Source initializer syntax remains in
    /// `initializer` for diagnostics and projection during migration, while
    /// this plan is authoritative for aggregate layout.
    pub static_initializer: Option<SemStaticInitializer>,
    pub span: Span,
    pub group_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemStaticInitializer {
    pub initialized_extent: u16,
    pub writes: Vec<SemStaticInitializerWrite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemStaticInitializerWrite {
    pub offset: u16,
    pub destination: ValueType,
    pub width: u16,
    pub value: SemStaticInitializerValue,
    pub span: Span,
    /// Readable aggregate path retained only for diagnostics and IR printing.
    pub display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemStaticInitializerValue {
    Literal {
        value: SemInitializerLiteral,
        negative: bool,
    },
    Address {
        selector: Option<AddressByteSelector>,
        target: SemSymbolRef,
        addend: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemDeclarationStorage {
    Scalar,
    Array {
        array_type: ArrayType,
        length: Option<SemExpr>,
        action_storage: VarStorage,
        origin: SemArrayOrigin,
    },
    Type {
        record_type: RecordType,
        fields: Vec<SemRecordField>,
    },
    Record {
        record_type: RecordType,
        fields: Vec<SemRecordField>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemRecordField {
    pub id: Option<FieldId>,
    pub owner: Option<SymbolId>,
    pub symbol: Option<SemSymbolRef>,
    pub name: String,
    pub ty: SemType,
    pub storage: SemDeclarationStorage,
    pub offset: Option<u16>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemType {
    pub value: ValueType,
    pub width: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemTypeFacts {
    pub width: Option<u16>,
    pub signedness: Option<ScalarSignedness>,
    pub is_pointer: bool,
    pub pointee: Option<ValueType>,
    pub pointee_width: Option<u16>,
    pub record_base: Option<String>,
    pub is_error: bool,
}

impl SemType {
    pub fn new(value: ValueType) -> Self {
        Self { value, width: None }
    }

    pub fn with_width(value: ValueType, width: u16) -> Self {
        Self {
            value,
            width: Some(width),
        }
    }
}

impl SemTypeFacts {
    pub fn from_value(value: &ValueType) -> Self {
        let scalar = value.as_scalar();
        let pointee = value.as_pointer().map(|pointer| *pointer.pointee);
        let pointee_width = pointee.as_ref().and_then(ValueType::value_width_bytes);

        Self {
            width: value.value_width_bytes(),
            signedness: scalar.map(ScalarType::signedness),
            is_pointer: value.is_pointer(),
            pointee,
            pointee_width,
            record_base: value.as_record_base_name().map(str::to_string),
            is_error: value.is_error(),
        }
    }
}

impl SemExpr {
    pub fn type_facts(&self) -> SemTypeFacts {
        SemTypeFacts::from_value(&self.ty)
    }
}

impl SemLValue {
    pub fn type_facts(&self) -> SemTypeFacts {
        SemTypeFacts::from_value(&self.ty)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemRoutine {
    pub symbol: SemSymbolRef,
    pub is_external: bool,
    pub signature: SemRoutineSignature,
    pub callable_type: CallableType,
    pub params: Vec<SemParam>,
    pub locals: Vec<SemDeclaration>,
    pub constants: Vec<SemConst>,
    pub body: Vec<SemStmt>,
    pub system_address: Option<SemExpr>,
    pub annotations: Vec<ActioncAnnotation>,
    pub effects: SemEffects,
    pub control_flow: SemControlFlow,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemControlFlow {
    pub always_returns: bool,
    pub may_fall_through: bool,
    pub contains_return: bool,
    pub contains_exit: bool,
    pub contains_loop: bool,
    pub max_loop_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemParam {
    pub symbol: SemSymbolRef,
    pub ty: SemType,
    pub storage: SemParamStorage,
    pub array_type: Option<ArrayType>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemInlineAsm {
    pub bytes: Vec<u8>,
    pub relocations: Vec<SemInlineAsmRelocation>,
    pub source: String,
    pub mode: InlineAsmMode,
    /// Compatibility emission keeps the already assembled machine items out of
    /// verifier-clean NIR while allowing the classic backend to share the same
    /// source feature.
    pub compatibility_items: Vec<MachineItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemInlineAsmRelocation {
    pub offset: u16,
    pub kind: InlineAsmRelocationKind,
    pub target: SemInlineAsmTarget,
    pub addend: i32,
    pub requires_zero_page: bool,
    pub symbol_use: InlineAsmSymbolUse,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemInlineAsmTarget {
    Symbol(SemSymbolRef),
    InlineOffset(u16),
    Absolute(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemParamStorage {
    Value,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemArrayOrigin {
    Global,
    Local,
    Parameter,
    RecordField,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemForStep {
    Up(u16),
    Down(u16),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemStmt {
    LexicalBlock {
        scope: SemLexicalScopeRef,
        declarations: Vec<SemDeclaration>,
        constants: Vec<SemConst>,
        body: Vec<SemStmt>,
        span: Span,
    },
    Define(SemDefine),
    Return {
        value: Option<SemExpr>,
        span: Span,
    },
    Exit {
        span: Span,
    },
    Assign {
        target: SemLValue,
        value: SemExpr,
        span: Span,
    },
    CompoundAssign {
        target: SemLValue,
        op: BinaryOp,
        value: SemExpr,
        span: Span,
    },
    Call {
        call: SemCall,
        span: Span,
    },
    MachineBlock {
        items: Vec<MachineItem>,
        /// Stable semantic targets for symbolic machine items. The original
        /// items remain only as a compatibility payload for the classic
        /// projection; target-aware lowering consumes these identities.
        resolved_symbols: Vec<SemMachineSymbolRef>,
        text: String,
        effects: SemEffects,
        span: Span,
    },
    InlineAsm {
        program: SemInlineAsm,
        effects: SemEffects,
        span: Span,
    },
    If {
        branches: Vec<SemIfBranch>,
        else_body: Vec<SemStmt>,
        span: Span,
    },
    While {
        condition: SemCondition,
        body: Vec<SemStmt>,
        span: Span,
    },
    DoUntil {
        body: Vec<SemStmt>,
        condition: Option<SemCondition>,
        span: Span,
    },
    For {
        target: SemLValue,
        start: SemExpr,
        end: SemExpr,
        step: Option<SemExpr>,
        step_control: SemForStep,
        body: Vec<SemStmt>,
        span: Span,
    },
    Unsupported {
        span: Span,
        note: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemLexicalScopeRef {
    pub syntax_id: LexicalBlockSyntaxId,
    pub scope: ScopeId,
    pub parent: ScopeId,
    pub depth: usize,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemMachineSymbolRef {
    pub item_index: usize,
    pub symbol: SemSymbolRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemIfBranch {
    pub condition: SemCondition,
    pub body: Vec<SemStmt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemStmtFlowAnnotation {
    pub index: usize,
    pub reachable: bool,
    pub facts: StmtFlowFacts,
}

pub fn statement_list_flow_facts(statements: &[SemStmt]) -> StmtFlowFacts {
    statement_list_flow_facts_at_depth(statements, 0)
}

pub fn statement_flow_facts(stmt: &SemStmt) -> StmtFlowFacts {
    stmt_flow_facts_at_depth(stmt, 0)
}

pub fn statement_list_flow_annotations(statements: &[SemStmt]) -> Vec<SemStmtFlowAnnotation> {
    statement_list_flow_annotations_at_depth(statements, 0)
}

fn statement_list_flow_annotations_at_depth(
    statements: &[SemStmt],
    loop_depth: usize,
) -> Vec<SemStmtFlowAnnotation> {
    let mut annotations = Vec::with_capacity(statements.len());
    let mut reachable = true;

    for (index, stmt) in statements.iter().enumerate() {
        let facts = stmt_flow_facts_at_depth(stmt, loop_depth);
        annotations.push(SemStmtFlowAnnotation {
            index,
            reachable,
            facts,
        });
        reachable &= facts.may_continue;
    }

    annotations
}

fn statement_list_flow_facts_at_depth(statements: &[SemStmt], loop_depth: usize) -> StmtFlowFacts {
    let mut facts = empty_continuing_flow_facts();
    let mut reachable = true;

    for stmt in statements {
        let stmt = stmt_flow_facts_at_depth(stmt, loop_depth);
        facts.may_return |= stmt.may_return;
        facts.may_exit_loop |= stmt.may_exit_loop;
        facts.contains_loop |= stmt.contains_loop;
        facts.max_loop_depth = facts.max_loop_depth.max(stmt.max_loop_depth);

        if reachable {
            facts.may_continue = stmt.may_continue;
            reachable = stmt.may_continue;
        }
    }

    facts.always_returns = !facts.may_continue && facts.may_return;
    facts
}

fn stmt_flow_facts_at_depth(stmt: &SemStmt, loop_depth: usize) -> StmtFlowFacts {
    match stmt {
        SemStmt::LexicalBlock { body, .. } => statement_list_flow_facts_at_depth(body, loop_depth),
        SemStmt::Return { .. } => StmtFlowFacts {
            may_continue: false,
            may_return: true,
            always_returns: true,
            may_exit_loop: false,
            contains_loop: false,
            max_loop_depth: loop_depth,
        },
        SemStmt::Exit { .. } => StmtFlowFacts {
            may_continue: false,
            may_return: false,
            always_returns: false,
            may_exit_loop: true,
            contains_loop: false,
            max_loop_depth: loop_depth,
        },
        SemStmt::If {
            branches,
            else_body,
            ..
        } => if_flow_facts(branches, else_body, loop_depth),
        SemStmt::While { body, .. } | SemStmt::For { body, .. } => {
            let body = statement_list_flow_facts_at_depth(body, loop_depth + 1);
            StmtFlowFacts {
                may_continue: true,
                may_return: body.may_return,
                always_returns: false,
                may_exit_loop: body.may_exit_loop,
                contains_loop: true,
                max_loop_depth: body.max_loop_depth.max(loop_depth + 1),
            }
        }
        SemStmt::DoUntil {
            body, condition, ..
        } => {
            let body = statement_list_flow_facts_at_depth(body, loop_depth + 1);
            let has_condition = condition.is_some();
            StmtFlowFacts {
                may_continue: body.may_exit_loop || (has_condition && body.may_continue),
                may_return: body.may_return,
                always_returns: body.always_returns && !body.may_exit_loop,
                may_exit_loop: body.may_exit_loop,
                contains_loop: true,
                max_loop_depth: body.max_loop_depth.max(loop_depth + 1),
            }
        }
        SemStmt::Define(_)
        | SemStmt::Assign { .. }
        | SemStmt::CompoundAssign { .. }
        | SemStmt::Call { .. }
        | SemStmt::MachineBlock { .. }
        | SemStmt::InlineAsm { .. }
        | SemStmt::Unsupported { .. } => empty_continuing_flow_facts(),
    }
}

fn if_flow_facts(
    branches: &[SemIfBranch],
    else_body: &[SemStmt],
    loop_depth: usize,
) -> StmtFlowFacts {
    let mut facts = StmtFlowFacts {
        may_continue: false,
        may_return: false,
        always_returns: false,
        may_exit_loop: false,
        contains_loop: false,
        max_loop_depth: loop_depth,
    };
    let mut else_reachable = true;

    for branch in branches {
        let condition_kind = branch.condition.kind;
        if condition_kind == SemConditionKind::ConstantFalse {
            continue;
        }

        let branch_facts = statement_list_flow_facts_at_depth(&branch.body, loop_depth);
        merge_flow_facts(&mut facts, branch_facts);

        if condition_kind == SemConditionKind::ConstantTrue {
            else_reachable = false;
            break;
        }
    }

    if else_reachable {
        if else_body.is_empty() {
            facts.may_continue = true;
        } else {
            let else_facts = statement_list_flow_facts_at_depth(else_body, loop_depth);
            merge_flow_facts(&mut facts, else_facts);
        }
    }

    facts.always_returns = !facts.may_continue && facts.may_return;
    facts
}

fn merge_flow_facts(target: &mut StmtFlowFacts, source: StmtFlowFacts) {
    target.may_continue |= source.may_continue;
    target.may_return |= source.may_return;
    target.may_exit_loop |= source.may_exit_loop;
    target.contains_loop |= source.contains_loop;
    target.max_loop_depth = target.max_loop_depth.max(source.max_loop_depth);
}

fn empty_continuing_flow_facts() -> StmtFlowFacts {
    StmtFlowFacts {
        may_continue: true,
        may_return: false,
        always_returns: false,
        may_exit_loop: false,
        contains_loop: false,
        max_loop_depth: 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemCondition {
    pub expr: SemExpr,
    pub kind: SemConditionKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemConditionKind {
    ConstantTrue,
    ConstantFalse,
    Compare,
    Logical,
    NonZeroValue,
    Error,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemExpr {
    pub kind: SemExprKind,
    pub ty: ValueType,
    pub class: SemExprClass,
    pub eval_order: Option<SemEvalOrderId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemExprKind {
    Missing,
    Raw(String),
    InitializerList(Vec<SemInitializerElement>),
    UnresolvedName(String),
    CurrentLocation,
    Literal(SemLiteral),
    Symbol(SemSymbolRef),
    LValue(Box<SemLValue>),
    AddressOf(Box<SemLValue>),
    AddressOfSymbol(SemSymbolRef),
    ImplicitAddressOf(SemImplicitAddressOf),
    ArrayDecay(SemArrayDecay),
    Cast {
        ty: ValueType,
        expr: Box<SemExpr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<SemExpr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<SemExpr>,
        right: Box<SemExpr>,
    },
    Call(SemCall),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemInitializerElement {
    pub kind: SemInitializerElementKind,
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemInitializerElementKind {
    Literal {
        value: SemInitializerLiteral,
        negative: bool,
    },
    Address {
        selector: Option<AddressByteSelector>,
        target: SemSymbolRef,
        addend: i32,
    },
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemInitializerLiteral {
    Number(NumberLiteral),
    Char(char),
    True,
    False,
    Nil,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemImplicitAddressOf {
    pub place: Box<SemLValue>,
    pub reason: SemImplicitAddressReason,
    pub pointer_type: ValueType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemImplicitAddressReason {
    RecordToPointer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemArrayDecay {
    pub array: Box<SemLValue>,
    pub element_type: ValueType,
    pub pointer_type: ValueType,
    pub origin: SemArrayOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemLiteral {
    Number(NumberLiteral),
    Real {
        source: NumberLiteral,
        value: AtariReal,
    },
    String(String),
    Char(char),
    Constant(ConstValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemExprClass {
    Unknown,
    Value,
    LValue,
    Callable,
    Condition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemEvalOrderId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemLValue {
    pub kind: SemLValueKind,
    pub ty: ValueType,
    pub access: PlaceAccess,
    pub is_volatile: bool,
    pub storage: Option<SemStorageRef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemLValueKind {
    Symbol(SemSymbolRef),
    UnresolvedName(String),
    Deref {
        pointer: Box<SemExpr>,
    },
    Index {
        base: Box<SemExpr>,
        index: Box<SemExpr>,
        element_type: ValueType,
        syntax: SemIndexSyntax,
    },
    Field {
        base: Box<SemLValue>,
        field: SemFieldRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemIndexSyntax {
    Call,
    Index,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemFieldRef {
    pub id: Option<FieldId>,
    pub owner: Option<SymbolId>,
    pub name: String,
    pub ty: ValueType,
    pub offset: Option<u16>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemCall {
    pub callee: SemCallable,
    pub callable_type: CallableType,
    pub args: Vec<SemExpr>,
    pub return_type: Option<ValueType>,
    pub effects: SemEffects,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemCallable {
    User(SemSymbolRef),
    Builtin(SemSymbolRef),
    Indirect {
        target: Box<SemExpr>,
        signature: SemRoutineSignature,
    },
    Runtime {
        name: String,
        address: Option<u16>,
        signature: SemRoutineSignature,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemRoutineSignature {
    pub kind: RoutineKind,
    pub params: Vec<ValueType>,
    pub return_type: Option<FundType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemStorageRef {
    pub symbol: Option<SemSymbolRef>,
    pub space: SemAddressSpace,
    pub address: Option<u16>,
    pub offset: u16,
    pub width: u16,
    pub signed: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemAddressSpace {
    Unknown,
    Absolute,
    ZeroPage,
    RuntimeZeroPage,
    InlineStatic,
    RoutineLocal,
    Parameter,
    IndirectIndexedY,
}

fn retain_referenced_external_interface_items(program: &mut SemProgram) {
    let external_routines = program
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            SemItem::Routine(routine) if routine.is_external => Some(routine.symbol.id),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let external_storage = program
        .modules
        .iter()
        // SYS is an always-available compiler interface. Its fixed volatile
        // storage belongs in executable SemIR only when source references it.
        .filter(|module| {
            module
                .path
                .as_ref()
                .is_some_and(|path| path.canonical_name() == "sys")
        })
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            SemItem::Declaration(declaration)
                if declaration.symbol.is_volatile
                    && declaration
                        .initializer
                        .as_ref()
                        .and_then(const_u16_sem_expr)
                        .is_some() =>
            {
                Some(declaration.symbol.id)
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    let external = external_routines
        .union(&external_storage)
        .copied()
        .collect::<HashSet<_>>();
    if external.is_empty() {
        return;
    }

    let mut referenced = HashSet::new();
    for module in &program.modules {
        for item in &module.items {
            collect_external_item_references(item, &external, &mut referenced);
        }
    }
    for module in &mut program.modules {
        module.items.retain(|item| {
            !matches!(item, SemItem::Routine(routine) if external_routines.contains(&routine.symbol.id) && !referenced.contains(&routine.symbol.id))
                && !matches!(item, SemItem::Declaration(declaration) if external_storage.contains(&declaration.symbol.id) && !referenced.contains(&declaration.symbol.id))
        });
    }
}

fn collect_external_item_references(
    item: &SemItem,
    external: &HashSet<SymbolId>,
    referenced: &mut HashSet<SymbolId>,
) {
    match item {
        SemItem::Set(set) => {
            collect_external_expr_references(&set.address, external, referenced);
            collect_external_expr_references(&set.value, external, referenced);
        }
        SemItem::Declaration(declaration) => {
            collect_external_declaration_references(declaration, external, referenced);
        }
        SemItem::Routine(routine) if !routine.is_external => {
            if let Some(address) = &routine.system_address {
                collect_external_expr_references(address, external, referenced);
            }
            for local in &routine.locals {
                collect_external_declaration_references(local, external, referenced);
            }
            collect_external_stmt_references(&routine.body, external, referenced);
        }
        SemItem::Statement(statement) => {
            collect_external_stmt_references(std::slice::from_ref(statement), external, referenced);
        }
        SemItem::Define(_)
        | SemItem::Const(_)
        | SemItem::Include(_)
        | SemItem::Routine(_)
        | SemItem::Unsupported { .. } => {}
    }
}

fn collect_external_declaration_references(
    declaration: &SemDeclaration,
    external: &HashSet<SymbolId>,
    referenced: &mut HashSet<SymbolId>,
) {
    if let Some(initializer) = &declaration.initializer {
        collect_external_expr_references(initializer, external, referenced);
    }
    collect_external_storage_references(&declaration.storage, external, referenced);
}

fn collect_external_storage_references(
    storage: &SemDeclarationStorage,
    external: &HashSet<SymbolId>,
    referenced: &mut HashSet<SymbolId>,
) {
    match storage {
        SemDeclarationStorage::Array { length, .. } => {
            if let Some(length) = length {
                collect_external_expr_references(length, external, referenced);
            }
        }
        SemDeclarationStorage::Type { fields, .. }
        | SemDeclarationStorage::Record { fields, .. } => {
            for field in fields {
                collect_external_storage_references(&field.storage, external, referenced);
            }
        }
        SemDeclarationStorage::Scalar => {}
    }
}

fn collect_external_stmt_references(
    statements: &[SemStmt],
    external: &HashSet<SymbolId>,
    referenced: &mut HashSet<SymbolId>,
) {
    for statement in statements {
        match statement {
            SemStmt::LexicalBlock {
                declarations, body, ..
            } => {
                for declaration in declarations {
                    collect_external_declaration_references(declaration, external, referenced);
                }
                collect_external_stmt_references(body, external, referenced);
            }
            SemStmt::Return { value, .. } => {
                if let Some(value) = value {
                    collect_external_expr_references(value, external, referenced);
                }
            }
            SemStmt::Assign { target, value, .. }
            | SemStmt::CompoundAssign { target, value, .. } => {
                collect_external_lvalue_references(target, external, referenced);
                collect_external_expr_references(value, external, referenced);
            }
            SemStmt::Call { call, .. } => {
                collect_external_call_references(call, external, referenced);
            }
            SemStmt::MachineBlock {
                resolved_symbols, ..
            } => {
                for symbol in resolved_symbols {
                    record_external_symbol(&symbol.symbol, external, referenced);
                }
            }
            SemStmt::InlineAsm { program, .. } => {
                for relocation in &program.relocations {
                    if let SemInlineAsmTarget::Symbol(symbol) = &relocation.target {
                        record_external_symbol(symbol, external, referenced);
                    }
                }
            }
            SemStmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    collect_external_expr_references(&branch.condition.expr, external, referenced);
                    collect_external_stmt_references(&branch.body, external, referenced);
                }
                collect_external_stmt_references(else_body, external, referenced);
            }
            SemStmt::While {
                condition, body, ..
            } => {
                collect_external_expr_references(&condition.expr, external, referenced);
                collect_external_stmt_references(body, external, referenced);
            }
            SemStmt::DoUntil {
                body, condition, ..
            } => {
                collect_external_stmt_references(body, external, referenced);
                if let Some(condition) = condition {
                    collect_external_expr_references(&condition.expr, external, referenced);
                }
            }
            SemStmt::For {
                target,
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_external_lvalue_references(target, external, referenced);
                collect_external_expr_references(start, external, referenced);
                collect_external_expr_references(end, external, referenced);
                if let Some(step) = step {
                    collect_external_expr_references(step, external, referenced);
                }
                collect_external_stmt_references(body, external, referenced);
            }
            SemStmt::Define(_) | SemStmt::Exit { .. } | SemStmt::Unsupported { .. } => {}
        }
    }
}

fn collect_external_call_references(
    call: &SemCall,
    external: &HashSet<SymbolId>,
    referenced: &mut HashSet<SymbolId>,
) {
    match &call.callee {
        SemCallable::User(symbol) | SemCallable::Builtin(symbol) => {
            record_external_symbol(symbol, external, referenced);
        }
        SemCallable::Indirect { target, .. } => {
            collect_external_expr_references(target, external, referenced);
        }
        SemCallable::Runtime { .. } => {}
    }
    for argument in &call.args {
        collect_external_expr_references(argument, external, referenced);
    }
}

fn collect_external_expr_references(
    expression: &SemExpr,
    external: &HashSet<SymbolId>,
    referenced: &mut HashSet<SymbolId>,
) {
    match &expression.kind {
        SemExprKind::InitializerList(elements) => {
            for element in elements {
                if let SemInitializerElementKind::Address { target, .. } = &element.kind {
                    record_external_symbol(target, external, referenced);
                }
            }
        }
        SemExprKind::Symbol(symbol) | SemExprKind::AddressOfSymbol(symbol) => {
            record_external_symbol(symbol, external, referenced);
        }
        SemExprKind::LValue(place) | SemExprKind::AddressOf(place) => {
            collect_external_lvalue_references(place, external, referenced);
        }
        SemExprKind::ImplicitAddressOf(address) => {
            collect_external_lvalue_references(&address.place, external, referenced);
        }
        SemExprKind::ArrayDecay(decay) => {
            collect_external_lvalue_references(&decay.array, external, referenced);
        }
        SemExprKind::Cast { expr, .. } | SemExprKind::Unary { expr, .. } => {
            collect_external_expr_references(expr, external, referenced);
        }
        SemExprKind::Binary { left, right, .. } => {
            collect_external_expr_references(left, external, referenced);
            collect_external_expr_references(right, external, referenced);
        }
        SemExprKind::Call(call) => collect_external_call_references(call, external, referenced),
        SemExprKind::Missing
        | SemExprKind::Raw(_)
        | SemExprKind::UnresolvedName(_)
        | SemExprKind::CurrentLocation
        | SemExprKind::Literal(_) => {}
    }
}

fn collect_external_lvalue_references(
    place: &SemLValue,
    external: &HashSet<SymbolId>,
    referenced: &mut HashSet<SymbolId>,
) {
    if let Some(symbol) = place
        .storage
        .as_ref()
        .and_then(|storage| storage.symbol.as_ref())
    {
        record_external_symbol(symbol, external, referenced);
    }
    match &place.kind {
        SemLValueKind::Symbol(symbol) => record_external_symbol(symbol, external, referenced),
        SemLValueKind::Deref { pointer } => {
            collect_external_expr_references(pointer, external, referenced);
        }
        SemLValueKind::Index { base, index, .. } => {
            collect_external_expr_references(base, external, referenced);
            collect_external_expr_references(index, external, referenced);
        }
        SemLValueKind::Field { base, .. } => {
            collect_external_lvalue_references(base, external, referenced);
        }
        SemLValueKind::UnresolvedName(_) => {}
    }
}

fn record_external_symbol(
    symbol: &SemSymbolRef,
    external: &HashSet<SymbolId>,
    referenced: &mut HashSet<SymbolId>,
) {
    if external.contains(&symbol.id) {
        referenced.insert(symbol.id);
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SemEffects {
    pub writes: Vec<SemWriteEffect>,
    pub reads: Vec<SemReadEffect>,
    pub may_call_os: bool,
    pub opaque: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemWriteEffect {
    Storage(SemStorageRef),
    ZeroPage { start: u8, end: u8 },
    Absolute { start: u16, end: u16 },
    Symbol(String),
    Unknown,
}

#[derive(Default)]
struct SemIrFormatter {
    lines: Vec<String>,
    indent: usize,
}

impl SemIrFormatter {
    fn finish(self) -> String {
        let mut output = self.lines.join("\n");
        output.push('\n');
        output
    }

    fn program(&mut self, program: &SemProgram) {
        self.line(match program.origin {
            Some(origin) => format!(
                "program modules={} origin=${:04X}",
                program.modules.len(),
                origin.address
            ),
            None => format!("program modules={}", program.modules.len()),
        });
        self.indented(|this| {
            for (index, module) in program.modules.iter().enumerate() {
                let identity = module
                    .path
                    .as_ref()
                    .map_or_else(|| format!("#{index}"), crate::ast::ModulePath::display_name);
                this.line(format!("module {identity} items={}", module.items.len()));
                this.indented(|this| this.module(module));
            }
        });
    }

    fn module(&mut self, module: &SemModule) {
        for item in &module.items {
            match item {
                SemItem::Define(define) => self.line(format!(
                    "define {} = {:?}",
                    symbol_summary(&define.symbol),
                    define.value
                )),
                SemItem::Const(constant) => self.line(format!(
                    "const {} = {}",
                    symbol_summary(&constant.symbol),
                    const_value_summary(constant.value)
                )),
                SemItem::Include(include) => self.line(format!("include {:?}", include.path)),
                SemItem::Set(set) => self.line(format!(
                    "set {} = {}",
                    expr_summary(&set.address),
                    expr_summary(&set.value)
                )),
                SemItem::Declaration(decl) => self.declaration(decl),
                SemItem::Routine(routine) => self.routine(routine),
                SemItem::Statement(stmt) => self.stmt(stmt),
                SemItem::Unsupported { note, .. } => self.line(format!("unsupported {note}")),
            }
        }
    }

    fn declaration(&mut self, decl: &SemDeclaration) {
        self.line(format!(
            "decl{} {} {} {}",
            if decl.symbol.is_volatile {
                " volatile"
            } else {
                ""
            },
            type_summary(&decl.ty.value),
            symbol_summary(&decl.symbol),
            declaration_storage_summary(&decl.storage)
        ));
        self.indented(|this| {
            if let Some(initializer) = &decl.initializer {
                this.line(format!("init {}", expr_summary(initializer)));
            }
            if decl.ty.value.is_record()
                && let Some(initializer) = &decl.static_initializer
            {
                this.line(format!(
                    "static-init extent={} writes={}",
                    initializer.initialized_extent,
                    initializer.writes.len()
                ));
                this.indented(|this| {
                    for write in &initializer.writes {
                        this.line(format!(
                            "write +{} width={} {} <- {}",
                            write.offset,
                            write.width,
                            write.display_path,
                            static_initializer_value_summary(&write.value)
                        ));
                    }
                });
            }
            match &decl.storage {
                SemDeclarationStorage::Type { fields, .. }
                | SemDeclarationStorage::Record { fields, .. } => {
                    for field in fields {
                        this.line(format!(
                            "field +{} {} {}",
                            field
                                .offset
                                .map(|offset| offset.to_string())
                                .unwrap_or_else(|| "?".to_string()),
                            type_summary(&field.ty.value),
                            field.name
                        ));
                    }
                }
                SemDeclarationStorage::Array { length, .. } => {
                    if let Some(length) = length {
                        this.line(format!("length {}", expr_summary(length)));
                    }
                }
                SemDeclarationStorage::Scalar => {}
            }
        });
    }

    fn routine(&mut self, routine: &SemRoutine) {
        self.line(format!(
            "routine {} {}",
            routine_kind_summary(&routine.signature.kind),
            symbol_summary(&routine.symbol)
        ));
        self.indented(|this| {
            this.line(format!(
                "callable {}",
                callable_type_summary(&routine.callable_type)
            ));
            if routine.is_external {
                this.line("external");
            }
            if let Some(address) = &routine.system_address {
                this.line(format!("system {}", expr_summary(address)));
            }
            this.line(format!(
                "flow {}",
                control_flow_summary(&routine.control_flow)
            ));
            if !routine.params.is_empty() {
                this.line("params:");
                this.indented(|this| {
                    for param in &routine.params {
                        let mut line = format!(
                            "{} {} {:?}",
                            type_summary(&param.ty.value),
                            symbol_summary(&param.symbol),
                            param.storage
                        );
                        if let Some(array_type) = &param.array_type {
                            line.push_str(&format!(" {}", array_type_summary(array_type)));
                        }
                        this.line(line);
                    }
                });
            }
            if !routine.locals.is_empty() {
                this.line("locals:");
                this.indented(|this| {
                    for local in &routine.locals {
                        this.declaration(local);
                    }
                });
            }
            if !routine.constants.is_empty() {
                this.line("constants:");
                this.indented(|this| {
                    for constant in &routine.constants {
                        this.line(format!(
                            "const {} = {}",
                            symbol_summary(&constant.symbol),
                            const_value_summary(constant.value)
                        ));
                    }
                });
            }
            this.line("body:");
            this.indented(|this| {
                this.stmt_list(&routine.body);
            });
        });
    }

    fn stmt_list(&mut self, statements: &[SemStmt]) {
        let flow = statement_list_flow_annotations(statements);
        for annotation in flow {
            let Some(stmt) = statements.get(annotation.index) else {
                continue;
            };

            if annotation.reachable {
                self.stmt(stmt);
            } else {
                self.line("unreachable:");
                self.indented(|this| this.stmt(stmt));
            }
        }
    }

    fn stmt(&mut self, stmt: &SemStmt) {
        match stmt {
            SemStmt::LexicalBlock {
                scope,
                declarations,
                constants,
                body,
                ..
            } => {
                self.line(format!(
                    "lexical-block syntax={} scope={} parent={} depth={} ordinal={}",
                    scope.syntax_id.0, scope.scope.0, scope.parent.0, scope.depth, scope.ordinal
                ));
                self.indented(|this| {
                    for declaration in declarations {
                        this.declaration(declaration);
                    }
                    for constant in constants {
                        this.line(format!(
                            "const {} = {}",
                            symbol_summary(&constant.symbol),
                            const_value_summary(constant.value)
                        ));
                    }
                    this.stmt_list(body);
                });
            }
            SemStmt::Define(define) => self.line(format!(
                "define {} = {:?}",
                symbol_summary(&define.symbol),
                define.value
            )),
            SemStmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.line(format!("return {}", expr_summary(value)));
                } else {
                    self.line("return");
                }
            }
            SemStmt::Exit { .. } => self.line("exit"),
            SemStmt::Assign { target, value, .. } => self.line(format!(
                "assign {} = {}",
                lvalue_summary(target),
                expr_summary(value)
            )),
            SemStmt::CompoundAssign {
                target, op, value, ..
            } => self.line(format!(
                "assign {} {}= {}",
                lvalue_summary(target),
                binary_op_summary(*op),
                expr_summary(value)
            )),
            SemStmt::Call { call, .. } => self.call(call),
            SemStmt::MachineBlock { items, effects, .. } => self.line(format!(
                "machine items={} effects={}",
                items.len(),
                effects_summary(effects)
            )),
            SemStmt::InlineAsm {
                program, effects, ..
            } => self.line(format!(
                "inline-asm items={} mode={:?} effects={}",
                program.bytes.len(),
                program.mode,
                effects_summary(effects)
            )),
            SemStmt::If {
                branches,
                else_body,
                ..
            } => {
                self.line("if");
                self.indented(|this| {
                    for branch in branches {
                        this.line(format!("when {}", condition_summary(&branch.condition)));
                        this.indented(|this| {
                            this.stmt_list(&branch.body);
                        });
                    }
                    if !else_body.is_empty() {
                        this.line("else");
                        this.indented(|this| {
                            this.stmt_list(else_body);
                        });
                    }
                });
            }
            SemStmt::While {
                condition, body, ..
            } => {
                self.line(format!("while {}", condition_summary(condition)));
                self.indented(|this| {
                    this.stmt_list(body);
                });
            }
            SemStmt::DoUntil {
                body, condition, ..
            } => {
                self.line(format!(
                    "do-until {}",
                    condition
                        .as_ref()
                        .map(condition_summary)
                        .unwrap_or_else(|| "<missing>".to_string())
                ));
                self.indented(|this| {
                    this.stmt_list(body);
                });
            }
            SemStmt::For {
                target,
                start,
                end,
                step,
                body,
                ..
            } => {
                self.line(format!(
                    "for {} = {} to {}{}",
                    lvalue_summary(target),
                    expr_summary(start),
                    expr_summary(end),
                    step.as_ref()
                        .map(|step| format!(" step {}", expr_summary(step)))
                        .unwrap_or_default()
                ));
                self.indented(|this| {
                    this.stmt_list(body);
                });
            }
            SemStmt::Unsupported { note, .. } => self.line(format!("unsupported {note}")),
        }
    }

    fn call(&mut self, call: &SemCall) {
        self.line(format!(
            "call {} -> {} callable={} effects={}",
            callable_summary(&call.callee),
            option_type_summary(call.return_type.as_ref()),
            callable_type_summary(&call.callable_type),
            effects_summary(&call.effects)
        ));
        self.indented(|this| {
            for (index, arg) in call.args.iter().enumerate() {
                this.line(format!("arg #{index} {}", expr_summary(arg)));
            }
        });
    }

    fn line(&mut self, line: impl Into<String>) {
        let mut text = String::new();
        for _ in 0..self.indent {
            text.push_str("  ");
        }
        text.push_str(&line.into());
        self.lines.push(text);
    }

    fn indented(&mut self, f: impl FnOnce(&mut Self)) {
        self.indent += 1;
        f(self);
        self.indent -= 1;
    }
}

fn expr_summary(expr: &SemExpr) -> String {
    let kind = match &expr.kind {
        SemExprKind::Missing => "<missing>".to_string(),
        SemExprKind::Raw(text) => format!("raw({text})"),
        SemExprKind::InitializerList(elements) => format!(
            "init[{}]",
            elements
                .iter()
                .map(|element| element.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        SemExprKind::UnresolvedName(name) => name.clone(),
        SemExprKind::CurrentLocation => "*".to_string(),
        SemExprKind::Literal(literal) => literal_summary(literal),
        SemExprKind::Symbol(symbol) => symbol_display_name(symbol).to_string(),
        SemExprKind::LValue(lvalue) => lvalue_summary(lvalue),
        SemExprKind::AddressOf(lvalue) => format!("@{}", lvalue_summary(lvalue)),
        SemExprKind::AddressOfSymbol(symbol) => format!("@{}", symbol.name),
        SemExprKind::Cast { ty, expr } => format!("{}({})", type_summary(ty), expr_summary(expr)),
        SemExprKind::ImplicitAddressOf(address) => {
            format!(
                "implicit_addr/{:?} {}",
                address.reason,
                lvalue_summary(&address.place)
            )
        }
        SemExprKind::ArrayDecay(decay) => {
            format!(
                "array_decay/{:?} {}",
                decay.origin,
                lvalue_summary(&decay.array)
            )
        }
        SemExprKind::Unary { op, expr } => {
            format!("({}{})", unary_op_summary(*op), expr_summary(expr))
        }
        SemExprKind::Binary { op, left, right } => format!(
            "({} {} {})",
            expr_summary(left),
            binary_op_summary(*op),
            expr_summary(right)
        ),
        SemExprKind::Call(call) => format!("{}(...)", callable_summary(&call.callee)),
    };

    format!(
        "{}:{}:{:?}:eval{}:{}",
        kind,
        type_summary(&expr.ty),
        expr.class,
        expr.eval_order
            .map(|eval| eval.0.to_string())
            .unwrap_or_else(|| "?".to_string()),
        type_facts_summary(&expr.type_facts())
    )
}

fn static_initializer_value_summary(value: &SemStaticInitializerValue) -> String {
    match value {
        SemStaticInitializerValue::Literal { value, negative } => {
            let magnitude = match value {
                SemInitializerLiteral::Number(number) => number.text.clone(),
                SemInitializerLiteral::Char(ch) => format!("'{ch}'"),
                SemInitializerLiteral::True => "TRUE".to_string(),
                SemInitializerLiteral::False => "FALSE".to_string(),
                SemInitializerLiteral::Nil => "NIL".to_string(),
            };
            if *negative {
                format!("-{magnitude}")
            } else {
                magnitude
            }
        }
        SemStaticInitializerValue::Address {
            selector,
            target,
            addend,
        } => {
            let prefix = match selector {
                Some(AddressByteSelector::Low) => "<",
                Some(AddressByteSelector::High) => ">",
                None => "@",
            };
            let addend = match addend.cmp(&0) {
                std::cmp::Ordering::Less => addend.to_string(),
                std::cmp::Ordering::Equal => String::new(),
                std::cmp::Ordering::Greater => format!("+{addend}"),
            };
            format!("{prefix}{}{addend}", target.qualified_name)
        }
    }
}

fn condition_summary(condition: &SemCondition) -> String {
    format!("{:?} {}", condition.kind, expr_summary(&condition.expr))
}

fn control_flow_summary(flow: &SemControlFlow) -> String {
    format!(
        "always_returns={} may_fall_through={} contains_return={} contains_exit={} contains_loop={} max_loop_depth={}",
        flow.always_returns,
        flow.may_fall_through,
        flow.contains_return,
        flow.contains_exit,
        flow.contains_loop,
        flow.max_loop_depth
    )
}

fn lvalue_summary(lvalue: &SemLValue) -> String {
    let kind = match &lvalue.kind {
        SemLValueKind::Symbol(symbol) => symbol_display_name(symbol).to_string(),
        SemLValueKind::UnresolvedName(name) => name.clone(),
        SemLValueKind::Deref { pointer } => format!("{}^", expr_summary(pointer)),
        SemLValueKind::Index { base, index, .. } => {
            format!("{}({})", expr_summary(base), expr_summary(index))
        }
        SemLValueKind::Field { base, field } => {
            format!("{}.{}", lvalue_summary(base), field.name)
        }
    };

    format!(
        "{}:{}:{}",
        kind,
        type_summary(&lvalue.ty),
        type_facts_summary(&lvalue.type_facts())
    )
}

fn literal_summary(literal: &SemLiteral) -> String {
    match literal {
        SemLiteral::Number(number) => number.text.clone(),
        SemLiteral::Real { source, .. } => source.text.clone(),
        SemLiteral::String(text) => format!("{text:?}"),
        SemLiteral::Char(ch) => format!("'{ch}'"),
        SemLiteral::Constant(value) => const_value_summary(*value),
    }
}

fn const_value_summary(value: ConstValue) -> String {
    let ty = match value.ty {
        ScalarType::Byte => "BYTE",
        ScalarType::Card => "CARD",
        ScalarType::Char => "CHAR",
        ScalarType::Int => "INT",
    };
    format!("{}:{ty}", value.number_literal().text)
}

fn callable_summary(callable: &SemCallable) -> String {
    match callable {
        SemCallable::User(symbol) => format!("user {}", symbol.name),
        SemCallable::Builtin(symbol) => format!("builtin {}", symbol.name),
        SemCallable::Indirect { target, .. } => format!("indirect {}", expr_summary(target)),
        SemCallable::Runtime { name, address, .. } => address
            .map(|address| format!("runtime {name}@${address:04X}"))
            .unwrap_or_else(|| format!("runtime {name}")),
    }
}

fn callable_type_summary(callable_type: &CallableType) -> String {
    let params = callable_type
        .params
        .iter()
        .map(type_summary)
        .collect::<Vec<_>>()
        .join(",");
    match callable_type.return_type.as_ref() {
        Some(return_type) => format!("{} FUNC({params})", type_summary(return_type)),
        None => format!("PROC({params})"),
    }
}

fn array_type_summary(array_type: &ArrayType) -> String {
    let length = array_type
        .length
        .map(|length| length.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!(
        "{} ARRAY({length})->{}",
        type_summary(&array_type.element),
        type_summary(&array_type.pointer_type())
    )
}

fn record_type_summary(record_type: &RecordType) -> String {
    format!("{}({} bytes)", record_type.name, record_type.size)
}

fn symbol_summary(symbol: &SemSymbolRef) -> String {
    format!(
        "{}#{}/scope{}:{:?}:{}",
        symbol_display_name(symbol),
        symbol.id.0,
        symbol.scope.0,
        symbol.class,
        option_type_summary(symbol.ty.as_ref())
    )
}

fn symbol_display_name(symbol: &SemSymbolRef) -> &str {
    if let Some(display_name) = &symbol.lexical_display_name {
        return display_name;
    }
    if symbol.defining_module.is_some() {
        &symbol.qualified_name
    } else {
        &symbol.name
    }
}

fn declaration_storage_summary(storage: &SemDeclarationStorage) -> String {
    match storage {
        SemDeclarationStorage::Scalar => "scalar".to_string(),
        SemDeclarationStorage::Array {
            array_type,
            action_storage,
            origin,
            ..
        } => {
            format!(
                "array {}/{action_storage:?}/{origin:?}",
                array_type_summary(array_type)
            )
        }
        SemDeclarationStorage::Type {
            record_type,
            fields,
        } => {
            format!(
                "type {} fields={}",
                record_type_summary(record_type),
                fields.len()
            )
        }
        SemDeclarationStorage::Record {
            record_type,
            fields,
        } => {
            format!(
                "record {} fields={}",
                record_type_summary(record_type),
                fields.len()
            )
        }
    }
}

fn routine_kind_summary(kind: &RoutineKind) -> String {
    match kind {
        RoutineKind::Proc => "PROC".to_string(),
        RoutineKind::Func { return_type } => format!("{return_type:?} FUNC"),
    }
}

fn option_type_summary(ty: Option<&ValueType>) -> String {
    ty.map(type_summary).unwrap_or_else(|| "?".to_string())
}

fn sem_array_origin_from_layout(origin: super::SemanticArrayOrigin) -> SemArrayOrigin {
    match origin {
        super::SemanticArrayOrigin::Global => SemArrayOrigin::Global,
        super::SemanticArrayOrigin::Local => SemArrayOrigin::Local,
        super::SemanticArrayOrigin::Parameter => SemArrayOrigin::Parameter,
        super::SemanticArrayOrigin::Unknown => SemArrayOrigin::Unknown,
    }
}

fn type_summary(ty: &ValueType) -> String {
    let base = match &ty.base {
        ValueTypeBase::Fund(fund) => format!("{fund:?}"),
        ValueTypeBase::Real => "REAL".to_string(),
        ValueTypeBase::Named(name) => name.clone(),
        ValueTypeBase::Callable(callable) => callable_type_summary(callable),
        ValueTypeBase::Error => "<error>".to_string(),
    };
    if ty.pointer {
        format!("{base} POINTER")
    } else {
        base
    }
}

fn type_facts_summary(facts: &SemTypeFacts) -> String {
    let mut parts = Vec::new();
    if let Some(width) = facts.width {
        parts.push(format!("w{width}"));
    }
    if let Some(signedness) = facts.signedness {
        parts.push(format!("{signedness:?}"));
    }
    if facts.is_pointer {
        parts.push("ptr".to_string());
        if let Some(pointee) = &facts.pointee {
            parts.push(format!("to={}", type_summary(pointee)));
        }
        if let Some(width) = facts.pointee_width {
            parts.push(format!("pointee_w{width}"));
        }
    }
    if let Some(record) = &facts.record_base {
        parts.push(format!("record={record}"));
    }
    if facts.is_error {
        parts.push("error".to_string());
    }
    if parts.is_empty() {
        "facts=?".to_string()
    } else {
        format!("facts={}", parts.join(","))
    }
}

fn effects_summary(effects: &SemEffects) -> String {
    if effects.opaque {
        return "opaque".to_string();
    }
    let mut parts = Vec::new();
    if !effects.reads.is_empty() {
        parts.push(format!("reads={}", effects.reads.len()));
    }
    if !effects.writes.is_empty() {
        parts.push(format!("writes={}", effects.writes.len()));
    }
    if effects.may_call_os {
        parts.push("may_call_os".to_string());
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(",")
    }
}

fn unary_op_summary(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Plus => "+",
        UnaryOp::Neg => "-",
        UnaryOp::AddressOf => "@",
        UnaryOp::Deref => "^",
    }
}

fn binary_op_summary(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "MOD",
        BinaryOp::Lsh => "LSH",
        BinaryOp::Rsh => "RSH",
        BinaryOp::And => "AND",
        BinaryOp::Or => "OR",
        BinaryOp::Xor => "XOR",
        BinaryOp::Eq => "=",
        BinaryOp::Ne => "#",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
    }
}

fn is_compare_op(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    )
}

fn sem_for_step(step: Option<&SemExpr>) -> SemForStep {
    let Some(step) = step else {
        return SemForStep::Up(1);
    };
    sem_for_step_expr(step)
}

fn sem_for_step_expr(expr: &SemExpr) -> SemForStep {
    match &expr.kind {
        SemExprKind::Cast { expr, .. } => sem_for_step_expr(expr),
        SemExprKind::Unary {
            op: UnaryOp::Plus,
            expr,
        } => sem_for_step_expr(expr),
        SemExprKind::Unary {
            op: UnaryOp::Neg,
            expr,
        } => const_u16_sem_expr(expr)
            .map(SemForStep::Down)
            .unwrap_or(SemForStep::Unknown),
        SemExprKind::Literal(SemLiteral::Constant(value))
            if value.ty == ScalarType::Int && value.bits & 0x8000 != 0 =>
        {
            SemForStep::Down(0u16.wrapping_sub(value.bits))
        }
        _ => const_u16_sem_expr(expr)
            .map(SemForStep::Up)
            .unwrap_or(SemForStep::Unknown),
    }
}

fn const_u16_sem_expr(expr: &SemExpr) -> Option<u16> {
    let value = match &expr.kind {
        SemExprKind::Literal(SemLiteral::Number(number)) => number.value,
        SemExprKind::Literal(SemLiteral::Constant(value)) => Some(value.bits),
        SemExprKind::Cast { expr, .. } => const_u16_sem_expr(expr),
        SemExprKind::Unary { op, expr } => {
            let value = const_u16_sem_expr(expr)?;
            match op {
                UnaryOp::Plus => Some(value),
                UnaryOp::Neg => Some(0u16.wrapping_sub(value)),
                UnaryOp::AddressOf | UnaryOp::Deref => None,
            }
        }
        SemExprKind::Binary { op, left, right } => {
            let left = const_u16_sem_expr(left)?;
            let right = const_u16_sem_expr(right)?;
            match op {
                BinaryOp::Add => Some(left.wrapping_add(right)),
                BinaryOp::Sub => Some(left.wrapping_sub(right)),
                BinaryOp::Mul => Some(left.wrapping_mul(right)),
                BinaryOp::Div => (right != 0).then_some(left / right),
                BinaryOp::Mod => (right != 0).then_some(left % right),
                BinaryOp::Lsh => Some(left.wrapping_shl(u32::from(right & 0x0F))),
                BinaryOp::Rsh => Some(left.wrapping_shr(u32::from(right & 0x0F))),
                BinaryOp::And => Some(left & right),
                BinaryOp::Or => Some(left | right),
                BinaryOp::Xor => Some(left ^ right),
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => None,
            }
        }
        _ => None,
    }?;
    Some(
        expr.ty
            .as_scalar()
            .map_or(value, |ty| value & super::scalar_mask(ty)),
    )
}

struct IrBuilder<'a> {
    model: &'a SemanticModel,
    next_eval_order: u32,
    numeric_defines: HashMap<SymbolId, NumberLiteral>,
}

struct SemStaticInitializerLeaf {
    offset: u16,
    ty: ValueType,
    width: u16,
    path: String,
}

impl<'a> IrBuilder<'a> {
    fn new(model: &'a SemanticModel) -> Self {
        Self {
            model,
            next_eval_order: 0,
            numeric_defines: HashMap::new(),
        }
    }

    fn lower_program(&mut self, program: &Program) -> SemProgram {
        self.numeric_defines =
            self.collect_numeric_defines(program, self.model.symbols.global_scope());
        let modules = program
            .modules
            .iter()
            .map(|module| self.lower_module(module))
            .collect::<Vec<_>>();
        let entry_routine = source_program_entry(&modules, None);
        let mut lowered = SemProgram {
            modules,
            layout: self.model.layout.clone(),
            origin: program
                .origin
                .as_ref()
                .and_then(|origin| self.lower_origin(self.model.symbols.global_scope(), origin)),
            root_module: None,
            entry_routine,
        };
        if matches!(program.source_kind, crate::ast::SourceUnitKind::Legacy) {
            self.attach_legacy_sys_interface(&mut lowered);
            retain_referenced_external_interface_items(&mut lowered);
            lowered.modules.retain(|module| {
                !module
                    .path
                    .as_ref()
                    .is_some_and(|path| path.canonical_name() == "sys" && module.items.is_empty())
            });
        }
        lowered
    }

    fn attach_legacy_sys_interface(&self, program: &mut SemProgram) {
        let Some(builtin_scope) = self.model.symbols.builtin_scope() else {
            return;
        };
        let mut interface = legacy_sys_interface().clone();
        interface.id = None;
        interface.items.retain_mut(|item| {
            let SemItem::Routine(routine) = item else {
                return false;
            };
            let public_name = routine
                .symbol
                .qualified_name
                .rsplit('.')
                .next()
                .unwrap_or(&routine.symbol.name);
            let Some(mut symbol) = self.symbol_ref(builtin_scope, public_name, routine.span) else {
                return false;
            };
            let resident = self
                .model
                .routine_signatures_by_symbol
                .get(&symbol.id)
                .is_some_and(|signature| {
                    signature.source == super::SemanticCallableSource::Resident
                });
            if !resident {
                return false;
            }
            symbol.name = public_name.to_string();
            symbol.canonical_qualified_key = format!("sys::{}", public_name.to_ascii_lowercase());
            symbol.qualified_name = format!("SYS.{public_name}");
            routine.symbol = symbol;
            true
        });
        // Keep legacy user routines first so adding a referenced interface does
        // not renumber their stable NIR routine IDs.
        program.modules.push(interface);
    }

    fn lower_named_compilation(&mut self, compilation: &LoadedCompilation) -> SemProgram {
        for module_id in &compilation.graph_order {
            let module = &compilation.modules[module_id.0 as usize];
            let scope = self
                .model
                .module(*module_id)
                .map(|module| module.scope)
                .expect("semantic module scope");
            self.numeric_defines
                .extend(self.collect_numeric_defines(&module.program, scope));
        }
        let modules = compilation
            .graph_order
            .iter()
            .map(|module_id| {
                let loaded = &compilation.modules[module_id.0 as usize];
                self.lower_named_module(*module_id, &loaded.program)
            })
            .collect::<Vec<_>>();
        let entry_routine = source_program_entry(&modules, Some(compilation.root));
        let mut program = SemProgram {
            modules,
            layout: self.model.layout.clone(),
            origin: compilation
                .root_module()
                .program
                .origin
                .as_ref()
                .and_then(|origin| {
                    let scope = self
                        .model
                        .module(compilation.root)
                        .map(|module| module.scope)?;
                    self.lower_origin(scope, origin)
                }),
            root_module: Some(compilation.root),
            entry_routine,
        };
        if !compilation
            .root_module()
            .declared_path
            .as_ref()
            .is_some_and(|path| path.canonical_name() == "sys")
        {
            retain_referenced_external_interface_items(&mut program);
        }
        program
    }

    fn lower_legacy_compilation(&mut self, compilation: &LoadedCompilation) -> SemProgram {
        self.numeric_defines = self.collect_numeric_defines(
            &compilation.root_module().program,
            self.model.symbols.global_scope(),
        );
        for module_id in &compilation.graph_order {
            if *module_id == compilation.root {
                continue;
            }
            let loaded = &compilation.modules[module_id.0 as usize];
            let scope = self
                .model
                .module(*module_id)
                .map(|module| module.scope)
                .expect("semantic module scope");
            self.numeric_defines
                .extend(self.collect_numeric_defines(&loaded.program, scope));
        }

        let mut modules = Vec::new();
        for module_id in &compilation.graph_order {
            let loaded = &compilation.modules[module_id.0 as usize];
            if *module_id == compilation.root {
                modules.extend(
                    loaded
                        .program
                        .modules
                        .iter()
                        .map(|module| self.lower_module(module)),
                );
            } else {
                modules.push(self.lower_named_module(*module_id, &loaded.program));
            }
        }
        let entry_routine = source_program_entry(&modules, None);
        let mut program = SemProgram {
            modules,
            layout: self.model.layout.clone(),
            origin: compilation
                .root_module()
                .program
                .origin
                .as_ref()
                .and_then(|origin| self.lower_origin(self.model.symbols.global_scope(), origin)),
            root_module: None,
            entry_routine,
        };
        retain_referenced_external_interface_items(&mut program);
        program
    }

    fn lower_named_module(&mut self, id: ModuleId, program: &Program) -> SemModule {
        let module = self.model.module(id).expect("semantic module scope");
        let mut items = Vec::new();
        for region in &program.modules {
            for item in &region.items {
                items.extend(self.lower_item(module.scope, item));
            }
        }
        SemModule {
            id: Some(id),
            path: Some(module.path.clone()),
            items,
        }
    }

    fn lower_module(&mut self, module: &Module) -> SemModule {
        let global_scope = self.model.symbols.global_scope();
        SemModule {
            id: None,
            path: None,
            items: module
                .items
                .iter()
                .flat_map(|item| self.lower_item(global_scope, item))
                .collect(),
        }
    }

    fn lower_item(&mut self, scope: ScopeId, item: &Item) -> Vec<SemItem> {
        match item {
            Item::Define(define) => self
                .lower_define(scope, define)
                .into_iter()
                .map(SemItem::Define)
                .collect(),
            Item::Include(include) => vec![SemItem::Include(self.lower_include(include))],
            Item::Set(set) => vec![SemItem::Set(self.lower_set(scope, set))],
            Item::Declaration(Decl::Const(constants)) => self
                .lower_const_decl(scope, constants)
                .into_iter()
                .map(SemItem::Const)
                .collect(),
            Item::Declaration(decl) => self
                .lower_decl(scope, decl)
                .into_iter()
                .map(SemItem::Declaration)
                .collect(),
            Item::Routine(routine) => vec![SemItem::Routine(self.lower_routine(scope, routine))],
            Item::Statement(stmt) => self
                .lower_stmt(scope, stmt)
                .into_iter()
                .map(SemItem::Statement)
                .collect(),
            Item::Unsupported { span, note } => vec![SemItem::Unsupported {
                span: *span,
                note: note.clone(),
            }],
        }
    }

    fn lower_include(&self, include: &IncludeDirective) -> SemInclude {
        SemInclude {
            path: include.path.clone(),
            span: include.span,
        }
    }

    fn lower_origin(&mut self, scope: ScopeId, origin: &OrgDirective) -> Option<SemOrigin> {
        let expression = self.lower_expr(scope, &origin.address);
        const_u16_sem_expr(&expression).map(|address| SemOrigin {
            address,
            span: origin.span,
        })
    }

    fn lower_set(&mut self, scope: ScopeId, set: &SetDirective) -> SemSet {
        SemSet {
            address: self.lower_expr(scope, &set.address),
            value: self.lower_expr(scope, &set.value),
            span: set.span,
        }
    }

    fn lower_define(&self, scope: ScopeId, define: &DefineDecl) -> Vec<SemDefine> {
        define
            .entries
            .iter()
            .filter_map(|entry| {
                self.symbol_ref(scope, &entry.name, entry.span)
                    .map(|symbol| SemDefine {
                        symbol,
                        value: entry.value.clone(),
                        span: entry.span,
                    })
            })
            .collect()
    }

    fn lower_const_decl(&self, scope: ScopeId, declaration: &ConstDecl) -> Vec<SemConst> {
        declaration
            .entries
            .iter()
            .filter_map(|entry| {
                let symbol = self.symbol_ref(scope, &entry.name, entry.span)?;
                let value = self.model.constants.get(&symbol.id).copied()?;
                Some(SemConst {
                    symbol,
                    value,
                    span: entry.span,
                })
            })
            .collect()
    }

    fn lower_decl(&mut self, scope: ScopeId, decl: &Decl) -> Vec<SemDeclaration> {
        match decl {
            Decl::Var(var) => self.lower_var_decl(scope, var),
            Decl::Const(_) => Vec::new(),
            Decl::Type(type_decl) => self.lower_type_decl(scope, type_decl),
            Decl::Record(record_decl) => self.lower_record_decl(scope, record_decl),
        }
    }

    fn lower_var_decl(&mut self, scope: ScopeId, decl: &VarDecl) -> Vec<SemDeclaration> {
        decl.entries
            .iter()
            .filter_map(|entry| {
                let symbol = self.symbol_ref(scope, &entry.name, entry.span)?;
                let is_array_storage =
                    decl.storage == VarStorage::Array || symbol.class == SymbolClass::Array;
                let storage = if is_array_storage {
                    SemDeclarationStorage::Array {
                        array_type: self.array_type_from_symbol(
                            scope,
                            &symbol,
                            entry.size.as_ref(),
                        ),
                        length: entry.size.as_ref().map(|size| self.lower_expr(scope, size)),
                        action_storage: VarStorage::Array,
                        origin: self.array_origin_for_symbol(&symbol),
                    }
                } else {
                    SemDeclarationStorage::Scalar
                };
                let ty = self.sem_type_from_symbol(&symbol);
                let initializer = entry
                    .initializer
                    .as_ref()
                    .map(|initializer| self.lower_expr(scope, initializer));
                let static_initializer = initializer.as_ref().and_then(|initializer| {
                    self.layout_static_initializer(&symbol, &ty, &storage, initializer)
                });
                Some(SemDeclaration {
                    ty,
                    symbol,
                    storage,
                    initializer,
                    static_initializer,
                    span: entry.span,
                    group_span: decl.span,
                })
            })
            .collect()
    }

    fn lower_type_decl(&mut self, scope: ScopeId, decl: &TypeDecl) -> Vec<SemDeclaration> {
        self.symbol_ref(scope, &decl.name, decl.span)
            .map(|symbol| {
                let fields = self.lower_record_fields(scope, symbol.id, &decl.name, &decl.fields);
                let record_type = self.record_type_from_fields(&symbol, &fields);
                vec![SemDeclaration {
                    ty: self.sem_type_from_symbol(&symbol),
                    symbol,
                    storage: SemDeclarationStorage::Type {
                        record_type,
                        fields,
                    },
                    initializer: None,
                    static_initializer: None,
                    span: decl.span,
                    group_span: decl.span,
                }]
            })
            .unwrap_or_default()
    }

    fn lower_record_decl(&mut self, scope: ScopeId, decl: &RecordDecl) -> Vec<SemDeclaration> {
        self.symbol_ref(scope, &decl.name, decl.span)
            .map(|symbol| {
                let fields = self.lower_record_fields(scope, symbol.id, &decl.name, &decl.fields);
                let record_type = self.record_type_from_fields(&symbol, &fields);
                vec![SemDeclaration {
                    ty: self.sem_type_from_symbol(&symbol),
                    symbol,
                    storage: SemDeclarationStorage::Record {
                        record_type,
                        fields,
                    },
                    initializer: None,
                    static_initializer: None,
                    span: decl.span,
                    group_span: decl.span,
                }]
            })
            .unwrap_or_default()
    }

    fn layout_static_initializer(
        &self,
        symbol: &SemSymbolRef,
        ty: &SemType,
        storage: &SemDeclarationStorage,
        initializer: &SemExpr,
    ) -> Option<SemStaticInitializer> {
        let SemExprKind::InitializerList(elements) = &initializer.kind else {
            return None;
        };
        let (element_type, repeats) = match storage {
            SemDeclarationStorage::Array { array_type, .. } => {
                (array_type.element.as_ref(), true)
            }
            SemDeclarationStorage::Scalar => (&ty.value, false),
            SemDeclarationStorage::Type { .. } | SemDeclarationStorage::Record { .. } => {
                return None;
            }
        };
        let element_width = self.value_storage_width(element_type)?;
        let mut leaves = Vec::new();
        self.append_static_initializer_leaves(element_type, 0, String::new(), &mut leaves)?;
        if leaves.is_empty() {
            return None;
        }

        let mut writes = Vec::with_capacity(elements.len());
        for (index, element) in elements.iter().enumerate() {
            let element_index = index / leaves.len();
            if !repeats && element_index != 0 {
                return None;
            }
            let leaf = &leaves[index % leaves.len()];
            let base = u16::try_from(element_index)
                .ok()?
                .checked_mul(element_width)?;
            let offset = base.checked_add(leaf.offset)?;
            let value = match &element.kind {
                SemInitializerElementKind::Literal { value, negative } => {
                    SemStaticInitializerValue::Literal {
                        value: value.clone(),
                        negative: *negative,
                    }
                }
                SemInitializerElementKind::Address {
                    selector,
                    target,
                    addend,
                } => SemStaticInitializerValue::Address {
                    selector: *selector,
                    target: target.clone(),
                    addend: *addend,
                },
                SemInitializerElementKind::Invalid => return None,
            };
            let suffix = if leaf.path.is_empty() {
                String::new()
            } else {
                format!(".{}", leaf.path)
            };
            let display_path = if repeats {
                format!("{}({element_index}){suffix}", symbol.name)
            } else {
                format!("{}{suffix}", symbol.name)
            };
            writes.push(SemStaticInitializerWrite {
                offset,
                destination: leaf.ty.clone(),
                width: leaf.width,
                value,
                span: element.span,
                display_path,
            });
        }

        let initialized_extent = if elements.is_empty() {
            0
        } else if repeats {
            let initialized_elements = elements.len().div_ceil(leaves.len());
            u16::try_from(initialized_elements)
                .ok()?
                .checked_mul(element_width)?
        } else {
            element_width
        };
        Some(SemStaticInitializer {
            initialized_extent,
            writes,
        })
    }

    fn append_static_initializer_leaves(
        &self,
        ty: &ValueType,
        base_offset: u16,
        path: String,
        leaves: &mut Vec<SemStaticInitializerLeaf>,
    ) -> Option<()> {
        if let Some(width) = ty.value_width_bytes() {
            leaves.push(SemStaticInitializerLeaf {
                offset: base_offset,
                ty: ty.clone(),
                width,
                path,
            });
            return Some(());
        }
        let record = self
            .model
            .layout
            .record_for_name(ty.as_record_name()?)?;
        for field in &record.fields {
            let field_path = if path.is_empty() {
                field.name.clone()
            } else {
                format!("{path}.{}", field.name)
            };
            self.append_static_initializer_leaves(
                &field.ty,
                base_offset.checked_add(field.offset)?,
                field_path,
                leaves,
            )?;
        }
        Some(())
    }

    fn lower_record_fields(
        &mut self,
        scope: ScopeId,
        owner: SymbolId,
        owner_name: &str,
        fields: &[VarDecl],
    ) -> Vec<SemRecordField> {
        let mut lowered = Vec::new();
        let mut offset = 0u16;
        for field in fields {
            for entry in &field.entries {
                let descriptor = self
                    .field_descriptor_by_name(owner_name, &entry.name)
                    .map(|field| (field.id, field.ty.clone(), field.offset));
                let ty = self.resolved_type_ref(scope, &field.ty);
                let field_value = descriptor
                    .as_ref()
                    .map(|(_, ty, _)| ty.clone())
                    .unwrap_or_else(|| ty.clone());
                let width = self.value_storage_width(&field_value);
                let storage = if field.storage == VarStorage::Array {
                    SemDeclarationStorage::Array {
                        array_type: ArrayType::new(
                            field_value.clone(),
                            entry
                                .size
                                .as_ref()
                                .and_then(|expr| self.const_u16_expr_in_scope(scope, expr)),
                        ),
                        length: entry.size.as_ref().map(|size| self.lower_expr(scope, size)),
                        action_storage: field.storage,
                        origin: SemArrayOrigin::RecordField,
                    }
                } else {
                    SemDeclarationStorage::Scalar
                };
                lowered.push(SemRecordField {
                    id: descriptor.as_ref().map(|(id, _, _)| *id),
                    owner: Some(owner),
                    symbol: None,
                    name: entry.name.clone(),
                    ty: SemType {
                        value: field_value,
                        width,
                    },
                    storage,
                    offset: descriptor
                        .as_ref()
                        .map(|(_, _, offset)| *offset)
                        .or(Some(offset)),
                    span: entry.span,
                });
                offset = offset.saturating_add(width.unwrap_or(0));
            }
        }
        lowered
    }

    fn record_type_from_fields(
        &mut self,
        symbol: &SemSymbolRef,
        fields: &[SemRecordField],
    ) -> RecordType {
        self.model
            .layout
            .record_for_owner(symbol.id)
            .map(|layout| layout.record_type.clone())
            .unwrap_or_else(|| {
                let record_fields = fields.iter().map(|field| RecordFieldType {
                    id: field.id,
                    name: field.name.clone(),
                    ty: field.ty.value.clone(),
                    offset: field.offset.unwrap_or(0),
                });
                let size = fields.iter().fold(0u16, |size, field| {
                    let offset = field.offset.unwrap_or(0);
                    let width = field
                        .ty
                        .width
                        .or_else(|| field.ty.value.value_width_bytes())
                        .unwrap_or(0);
                    size.max(offset.saturating_add(width))
                });
                RecordType::new(symbol.name.clone(), record_fields, size)
            })
    }

    fn value_storage_width(&self, value: &ValueType) -> Option<u16> {
        value.value_width_bytes().or_else(|| {
            value.as_record_name().and_then(|name| {
                self.model
                    .layout
                    .records
                    .iter()
                    .find(|record| record.name.eq_ignore_ascii_case(name))
                    .map(|record| record.size)
            })
        })
    }

    fn lower_routine(&mut self, parent_scope: ScopeId, routine: &Routine) -> SemRoutine {
        let symbol = self
            .symbol_ref(parent_scope, &routine.name, routine.span)
            .unwrap_or_else(|| {
                self.synthetic_symbol_ref(&routine.name, SymbolClass::Proc, routine.span)
            });
        let routine_scope = self
            .model
            .routine_scopes
            .iter()
            .find(|routine| routine.symbol == Some(symbol.id))
            .map(|routine| routine.scope)
            .unwrap_or(parent_scope);
        let params = self.lower_params(routine_scope, &routine.params);
        let signature = SemRoutineSignature::from_header(
            routine.kind.clone(),
            params.iter().map(param_signature_type),
        );

        SemRoutine {
            is_external: routine.is_external,
            callable_type: signature.callable_type(),
            signature,
            params,
            locals: routine
                .locals
                .iter()
                .flat_map(|decl| self.lower_decl(routine_scope, decl))
                .collect(),
            constants: routine
                .locals
                .iter()
                .filter_map(|decl| match decl {
                    Decl::Const(constants) => Some(constants),
                    _ => None,
                })
                .flat_map(|constants| self.lower_const_decl(routine_scope, constants))
                .collect(),
            body: routine
                .body
                .iter()
                .flat_map(|stmt| self.lower_stmt(routine_scope, stmt))
                .collect(),
            system_address: routine
                .system_address
                .as_ref()
                .map(|address| self.lower_expr(parent_scope, address)),
            annotations: routine.annotations.clone(),
            effects: SemEffects::default(),
            control_flow: {
                let facts = routine_control_flow_facts(routine);
                SemControlFlow {
                    always_returns: facts.always_returns,
                    may_fall_through: facts.may_fall_through,
                    contains_return: facts.contains_return,
                    contains_exit: facts.contains_exit,
                    contains_loop: facts.contains_loop,
                    max_loop_depth: facts.max_loop_depth,
                }
            },
            span: routine.span,
            symbol,
        }
    }

    fn lower_params(&mut self, scope: ScopeId, params: &[VarDecl]) -> Vec<SemParam> {
        let mut lowered = Vec::new();
        for param in params {
            let storage = if param.storage == VarStorage::Array || is_string_type_ref(&param.ty) {
                SemParamStorage::Array
            } else {
                SemParamStorage::Value
            };
            for entry in &param.entries {
                if let Some(symbol) = self.symbol_ref(scope, &entry.name, entry.span) {
                    let array_type = if storage == SemParamStorage::Array {
                        Some(self.array_type_from_symbol(scope, &symbol, entry.size.as_ref()))
                    } else {
                        None
                    };
                    lowered.push(SemParam {
                        ty: self.sem_type_from_symbol(&symbol),
                        symbol,
                        storage,
                        array_type,
                        span: entry.span,
                    });
                }
            }
        }
        lowered
    }

    fn lower_stmt(&mut self, scope: ScopeId, stmt: &Stmt) -> Vec<SemStmt> {
        match stmt {
            Stmt::LexicalBlock {
                syntax_id,
                declarations,
                body,
                span,
            } => {
                let Some(block) = self
                    .model
                    .lexical_blocks
                    .iter()
                    .find(|block| block.parent == scope && block.syntax_id == *syntax_id)
                else {
                    return vec![SemStmt::Unsupported {
                        span: *span,
                        note: "lexical block has no semantic scope".to_string(),
                    }];
                };
                let block_scope = block.scope;
                let scope_ref = SemLexicalScopeRef {
                    syntax_id: *syntax_id,
                    scope: block.scope,
                    parent: block.parent,
                    depth: block.depth,
                    ordinal: block.ordinal,
                };
                vec![SemStmt::LexicalBlock {
                    scope: scope_ref,
                    declarations: declarations
                        .iter()
                        .flat_map(|declaration| self.lower_decl(block_scope, declaration))
                        .collect(),
                    constants: declarations
                        .iter()
                        .filter_map(|declaration| match declaration {
                            Decl::Const(constants) => Some(constants),
                            _ => None,
                        })
                        .flat_map(|constants| self.lower_const_decl(block_scope, constants))
                        .collect(),
                    body: body
                        .iter()
                        .flat_map(|statement| self.lower_stmt(block_scope, statement))
                        .collect(),
                    span: *span,
                }]
            }
            Stmt::Define(define) => {
                let defines = self.lower_define(scope, define);
                if defines.is_empty() {
                    vec![SemStmt::Unsupported {
                        span: Span::new(0, 0),
                        note: "empty DEFINE".to_string(),
                    }]
                } else {
                    defines.into_iter().map(SemStmt::Define).collect()
                }
            }
            Stmt::Return(expr) => vec![SemStmt::Return {
                value: expr.as_ref().map(|expr| self.lower_expr(scope, expr)),
                span: expr.as_ref().map_or(Span::new(0, 0), |expr| expr.span),
            }],
            Stmt::Exit { span } => vec![SemStmt::Exit { span: *span }],
            Stmt::Assign {
                target,
                value,
                span,
            } => vec![SemStmt::Assign {
                target: self.lower_lvalue(scope, target),
                value: self.lower_assignment_value(scope, target, value),
                span: *span,
            }],
            Stmt::CompoundAssign {
                target,
                op,
                value,
                span,
            } => vec![SemStmt::CompoundAssign {
                target: self.lower_lvalue(scope, target),
                op: *op,
                value: self.lower_compound_assignment_value(scope, target, value),
                span: *span,
            }],
            Stmt::Call { expr, span } => vec![SemStmt::Call {
                call: self.lower_call_expr(scope, expr),
                span: *span,
            }],
            Stmt::MachineBlock { items, text, span } => vec![SemStmt::MachineBlock {
                items: items.clone(),
                resolved_symbols: self.resolve_machine_symbols(scope, items),
                text: text.clone(),
                effects: SemEffects::default(),
                span: *span,
            }],
            Stmt::InlineAsm { program, span } => {
                vec![SemStmt::InlineAsm {
                    program: self.lower_inline_asm(scope, program),
                    effects: SemEffects::default(),
                    span: *span,
                }]
            }
            Stmt::If {
                branches,
                else_body,
                span,
            } => vec![SemStmt::If {
                branches: branches
                    .iter()
                    .map(|branch| SemIfBranch {
                        condition: self.lower_condition(scope, &branch.condition),
                        body: branch
                            .body
                            .iter()
                            .flat_map(|stmt| self.lower_stmt(scope, stmt))
                            .collect(),
                    })
                    .collect(),
                else_body: else_body
                    .iter()
                    .flat_map(|stmt| self.lower_stmt(scope, stmt))
                    .collect(),
                span: *span,
            }],
            Stmt::While {
                condition,
                body,
                span,
            } => vec![SemStmt::While {
                condition: self.lower_condition(scope, condition),
                body: body
                    .iter()
                    .flat_map(|stmt| self.lower_stmt(scope, stmt))
                    .collect(),
                span: *span,
            }],
            Stmt::DoUntil {
                body,
                condition,
                span,
            } => vec![SemStmt::DoUntil {
                body: body
                    .iter()
                    .flat_map(|stmt| self.lower_stmt(scope, stmt))
                    .collect(),
                condition: condition
                    .as_ref()
                    .map(|expr| self.lower_condition(scope, expr)),
                span: *span,
            }],
            Stmt::For {
                target,
                start,
                end,
                step,
                body,
                span,
            } => {
                let target = self.lower_lvalue(scope, target);
                let target_ty = target.ty.clone();
                let step = step
                    .as_ref()
                    .map(|expr| self.lower_scalar_value_for_expected_type(scope, &target_ty, expr));
                let step_control = sem_for_step(step.as_ref());
                vec![SemStmt::For {
                    target,
                    start: self.lower_scalar_value_for_expected_type(scope, &target_ty, start),
                    end: self.lower_scalar_value_for_expected_type(scope, &target_ty, end),
                    step,
                    step_control,
                    body: body
                        .iter()
                        .flat_map(|stmt| self.lower_stmt(scope, stmt))
                        .collect(),
                    span: *span,
                }]
            }
            Stmt::Unsupported { span, note } => vec![SemStmt::Unsupported {
                span: *span,
                note: note.clone(),
            }],
        }
    }

    fn lower_assignment_value(&mut self, scope: ScopeId, target: &Expr, value: &Expr) -> SemExpr {
        if self
            .direct_symbol_ref_for_expr(scope, target)
            .is_some_and(|symbol| symbol.class == SymbolClass::Array)
        {
            return self.lower_expr(scope, value);
        }
        let Some(target_ty) = self.lvalue_expr_type(scope, target) else {
            return self.lower_expr(scope, value);
        };
        self.lower_value_for_expected_type(scope, &target_ty, value)
    }

    fn lower_scalar_value_for_expected_type(
        &mut self,
        scope: ScopeId,
        expected: &ValueType,
        expr: &Expr,
    ) -> SemExpr {
        let lowered = self.lower_value_for_expected_type(scope, expected, expr);
        let machine_type_differs = expected
            .as_scalar()
            .zip(lowered.ty.as_scalar())
            .is_some_and(|(expected, actual)| {
                expected.width_bytes() != actual.width_bytes()
                    || expected.signedness() != actual.signedness()
            });
        if scalar_expected_type_can_accept_expr(expected, &lowered.ty) && machine_type_differs {
            self.coerce_scalar_expr_for_expected_type(lowered, expected)
        } else {
            lowered
        }
    }

    fn lower_inline_asm(&self, scope: ScopeId, program: &InlineAsmProgram) -> SemInlineAsm {
        let program =
            super::materialize::materialize_inline_asm_constants(program, scope, self.model);
        let mut compatibility_link_names = HashMap::new();
        let relocations = program
            .relocations
            .iter()
            .filter_map(|relocation| {
                let target = match &relocation.target {
                    InlineAsmRelocationTarget::Symbol(name) => {
                        let symbol = self.symbol_ref(scope, name, relocation.span)?;
                        compatibility_link_names.insert(normalize_name(name), symbol.name.clone());
                        SemInlineAsmTarget::Symbol(symbol)
                    }
                    InlineAsmRelocationTarget::InlineOffset(offset) => {
                        SemInlineAsmTarget::InlineOffset(*offset)
                    }
                    InlineAsmRelocationTarget::Absolute(address) => {
                        SemInlineAsmTarget::Absolute(*address)
                    }
                };
                Some(SemInlineAsmRelocation {
                    offset: relocation.offset,
                    kind: relocation.kind,
                    target,
                    addend: relocation.addend,
                    requires_zero_page: relocation.requires_zero_page,
                    symbol_use: relocation.symbol_use,
                    span: relocation.span,
                })
            })
            .collect();
        let compatibility_items =
            relink_inline_asm_items(&program.items, &compatibility_link_names);
        SemInlineAsm {
            bytes: program.bytes,
            relocations,
            source: program.source,
            mode: program.mode,
            compatibility_items,
        }
    }

    fn resolve_machine_symbols(
        &self,
        scope: ScopeId,
        items: &[MachineItem],
    ) -> Vec<SemMachineSymbolRef> {
        items
            .iter()
            .enumerate()
            .filter_map(|(item_index, item)| {
                let name = match item {
                    MachineItem::Name(name) | MachineItem::AddressByte { name, .. } => name,
                    MachineItem::AddressExpr(MachineAddressExpr {
                        atom: MachineAddressAtom::Name(name),
                        ..
                    }) => name,
                    _ => return None,
                };
                self.qualified_symbol_ref(scope, name, Span::new(0, 0))
                    .or_else(|| {
                        compact_machine_symbol_name(items, item_index)
                            .and_then(|name| self.symbol_ref(scope, &name, Span::new(0, 0)))
                    })
                    .map(|symbol| SemMachineSymbolRef { item_index, symbol })
            })
            .collect()
    }

    fn lower_expr(&mut self, scope: ScopeId, expr: &Expr) -> SemExpr {
        let kind = match &expr.kind {
            ExprKind::Missing => SemExprKind::Missing,
            ExprKind::Raw => SemExprKind::Raw(expr.text.clone()),
            ExprKind::InitializerList(elements) => SemExprKind::InitializerList(
                elements
                    .iter()
                    .map(|element| self.lower_initializer_element(scope, element))
                    .collect(),
            ),
            ExprKind::CurrentLocation => SemExprKind::CurrentLocation,
            ExprKind::Number(number) if number.kind == crate::lexer::NumberKind::Real => {
                let value = AtariReal::from_decimal(&number.text)
                    .expect("semantic analysis validated the REAL literal");
                SemExprKind::Literal(SemLiteral::Real {
                    source: number.clone(),
                    value,
                })
            }
            ExprKind::Number(number) => SemExprKind::Literal(SemLiteral::Number(number.clone())),
            ExprKind::String(value) => SemExprKind::Literal(SemLiteral::String(value.clone())),
            ExprKind::Char(value) => SemExprKind::Literal(SemLiteral::Char(*value)),
            ExprKind::Name(name) => self
                .direct_symbol_ref_for_expr(scope, expr)
                .map(|symbol| self.expr_kind_for_symbol(scope, expr, symbol))
                .unwrap_or_else(|| SemExprKind::UnresolvedName(name.clone())),
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr: inner,
            } => {
                if let Some(symbol) = self.direct_symbol_ref_for_expr(scope, inner) {
                    if matches!(
                        symbol.class,
                        SymbolClass::Proc
                            | SymbolClass::Func
                            | SymbolClass::BuiltinProc
                            | SymbolClass::BuiltinFunc
                    ) {
                        SemExprKind::AddressOfSymbol(symbol)
                    } else {
                        SemExprKind::AddressOf(Box::new(self.lower_lvalue(scope, inner)))
                    }
                } else {
                    SemExprKind::AddressOf(Box::new(self.lower_lvalue(scope, inner)))
                }
            }
            ExprKind::Cast { ty, expr: inner } => {
                let ty = self.resolved_type_ref(scope, ty);
                SemExprKind::Cast {
                    ty: ty.clone(),
                    expr: Box::new(self.lower_expr(scope, inner)),
                }
            }
            ExprKind::Unary {
                op: UnaryOp::Deref, ..
            } => SemExprKind::LValue(Box::new(self.lower_lvalue(scope, expr))),
            ExprKind::Unary { op, expr: inner } => {
                let inner = self.lower_expr(scope, inner);
                let inner = if *op == UnaryOp::Neg
                    && inner.ty.as_scalar().is_some_and(|ty| ty.width_bytes() == 1)
                {
                    self.coerce_scalar_expr_for_expected_type(
                        inner,
                        &ValueType::scalar(ScalarType::Int),
                    )
                } else {
                    inner
                };
                SemExprKind::Unary {
                    op: *op,
                    expr: Box::new(inner),
                }
            }
            ExprKind::Binary { op, left, right } => self.lower_binary_expr(scope, *op, left, right),
            ExprKind::Call { callee, args }
                if args.len() == 1 && self.is_indexable_lvalue(scope, callee) =>
            {
                SemExprKind::LValue(Box::new(self.lower_lvalue(scope, expr)))
            }
            ExprKind::Call { .. } => SemExprKind::Call(self.lower_call_expr(scope, expr)),
            ExprKind::Field { .. } if self.direct_symbol_ref_for_expr(scope, expr).is_some() => {
                let symbol = self
                    .direct_symbol_ref_for_expr(scope, expr)
                    .expect("guarded direct symbol");
                self.expr_kind_for_symbol(scope, expr, symbol)
            }
            ExprKind::Index { .. } | ExprKind::Field { .. } => {
                SemExprKind::LValue(Box::new(self.lower_lvalue(scope, expr)))
            }
        };

        let ty = self.expr_type_from_kind(&kind);
        let class = expr_class_from_kind(&kind);

        SemExpr {
            kind,
            ty,
            class,
            eval_order: Some(self.next_eval_order()),
            span: expr.span,
        }
    }

    fn expr_kind_for_symbol(
        &mut self,
        scope: ScopeId,
        expr: &Expr,
        symbol: SemSymbolRef,
    ) -> SemExprKind {
        if symbol.class == SymbolClass::Const
            && let Some(value) = self.model.constants.get(&symbol.id).copied()
        {
            SemExprKind::Literal(SemLiteral::Constant(value))
        } else if symbol.class == SymbolClass::Const
            && let Some(value) = self.model.real_constants.get(&symbol.id)
        {
            SemExprKind::Literal(SemLiteral::Real {
                source: value.source.clone(),
                value: value.value,
            })
        } else if symbol.class == SymbolClass::Define
            && let Some(number) = self.numeric_defines.get(&symbol.id)
        {
            SemExprKind::Literal(SemLiteral::Number(number.clone()))
        } else if self.is_array_symbol(symbol.id) {
            SemExprKind::ArrayDecay(self.array_decay_for_symbol(scope, expr, symbol))
        } else {
            SemExprKind::Symbol(symbol)
        }
    }

    fn expr_type_from_kind(&self, kind: &SemExprKind) -> ValueType {
        match kind {
            SemExprKind::Missing
            | SemExprKind::Raw(_)
            | SemExprKind::InitializerList(_)
            | SemExprKind::UnresolvedName(_) => ValueType::error(),
            SemExprKind::CurrentLocation => card_type(),
            SemExprKind::Literal(SemLiteral::Number(number)) => value_type_for_number(number),
            SemExprKind::Literal(SemLiteral::Real { .. }) => ValueType::real(),
            SemExprKind::Literal(SemLiteral::String(_)) => ValueType::pointer_to(char_type()),
            SemExprKind::Literal(SemLiteral::Char(_)) => char_type(),
            SemExprKind::Literal(SemLiteral::Constant(value)) => value.value_type(),
            SemExprKind::Symbol(symbol) => symbol.ty.clone().unwrap_or_else(ValueType::error),
            SemExprKind::LValue(lvalue) => lvalue.ty.clone(),
            SemExprKind::AddressOf(lvalue) => ValueType::pointer_to(lvalue.ty.clone()),
            SemExprKind::AddressOfSymbol(symbol) => self.callable_pointer_type_for_symbol(symbol),
            SemExprKind::ImplicitAddressOf(address) => address.pointer_type.clone(),
            SemExprKind::ArrayDecay(decay) => decay.pointer_type.clone(),
            SemExprKind::Cast { ty, .. } => ty.clone(),
            SemExprKind::Unary {
                op: UnaryOp::Neg,
                expr,
            } if expr.ty.as_scalar().is_some() => ValueType::scalar(ScalarType::Int),
            SemExprKind::Unary { expr, .. } => expr.ty.clone(),
            SemExprKind::Binary { op, left, right } => {
                if is_compare_op(*op) {
                    byte_type()
                } else {
                    arithmetic_numeric_result_type(
                        *op,
                        &left.ty,
                        &right.ty,
                        constant_sem_binary_result(*op, left, right),
                    )
                }
            }
            SemExprKind::Call(call) => call.return_type.clone().unwrap_or_else(ValueType::error),
        }
    }

    fn lower_initializer_element(
        &self,
        scope: ScopeId,
        element: &InitializerElement,
    ) -> SemInitializerElement {
        let kind = match &element.kind {
            InitializerElementKind::Literal { value, negative } => {
                let value = match value {
                    InitializerLiteral::Number(number) => {
                        SemInitializerLiteral::Number(number.clone())
                    }
                    InitializerLiteral::Char(ch) => SemInitializerLiteral::Char(*ch),
                    InitializerLiteral::True => SemInitializerLiteral::True,
                    InitializerLiteral::False => SemInitializerLiteral::False,
                    InitializerLiteral::Nil => SemInitializerLiteral::Nil,
                };
                SemInitializerElementKind::Literal {
                    value,
                    negative: *negative,
                }
            }
            InitializerElementKind::Constant { target, negative } => {
                match self.model.resolve_name(scope, target) {
                    SemanticNameResolution::Symbol(id) => self
                        .model
                        .constants
                        .get(&id)
                        .map(|value| SemInitializerElementKind::Literal {
                            value: SemInitializerLiteral::Number(value.number_literal()),
                            negative: *negative,
                        })
                        .unwrap_or(SemInitializerElementKind::Invalid),
                    SemanticNameResolution::PrivateMember { .. }
                    | SemanticNameResolution::MissingMember { .. }
                    | SemanticNameResolution::Unknown => SemInitializerElementKind::Invalid,
                }
            }
            InitializerElementKind::Address {
                selector,
                target,
                addend,
            } => self
                .qualified_symbol_ref(scope, target, element.span)
                .map(|target| SemInitializerElementKind::Address {
                    selector: *selector,
                    target,
                    addend: *addend,
                })
                .unwrap_or(SemInitializerElementKind::Invalid),
            InitializerElementKind::Invalid => SemInitializerElementKind::Invalid,
        };
        SemInitializerElement {
            kind,
            text: element.text.clone(),
            span: element.span,
        }
    }

    fn lower_value_for_expected_type(
        &mut self,
        scope: ScopeId,
        expected: &ValueType,
        expr: &Expr,
    ) -> SemExpr {
        if expected.is_record_pointer()
            && let Some(actual) = self.lvalue_expr_type(scope, expr)
            && !actual.pointer
            && actual.same_record_family(expected)
            && self.expr_is_named_place(scope, expr)
        {
            let place = self.lower_lvalue(scope, expr);
            return SemExpr {
                kind: SemExprKind::ImplicitAddressOf(SemImplicitAddressOf {
                    place: Box::new(place),
                    reason: SemImplicitAddressReason::RecordToPointer,
                    pointer_type: expected.clone(),
                }),
                ty: expected.clone(),
                class: SemExprClass::Value,
                eval_order: Some(self.next_eval_order()),
                span: expr.span,
            };
        }

        if expected.pointer
            && let Some(mut decay) = self.array_decay_for_expected_pointer(scope, expected, expr)
        {
            decay.pointer_type = expected.clone();
            return SemExpr {
                kind: SemExprKind::ArrayDecay(decay),
                ty: expected.clone(),
                class: SemExprClass::Value,
                eval_order: Some(self.next_eval_order()),
                span: expr.span,
            };
        }

        if expected.pointer && matches!(expr.kind, ExprKind::Number(_)) {
            let lowered = self.lower_expr(scope, expr);
            if lowered.ty.as_scalar().is_some() {
                return self.coerce_scalar_expr_for_expected_type(lowered, expected);
            }
        }

        if expected.is_real() {
            let lowered = self.lower_expr(scope, expr);
            return self.coerce_assignment_expr_to_real(lowered);
        }

        if expected.is_word_sized_value()
            && let ExprKind::Name(name) = &expr.kind
            && let Some(symbol) = self.symbol_ref(scope, name, expr.span)
            && matches!(symbol.class, SymbolClass::Proc | SymbolClass::Func)
        {
            return SemExpr {
                kind: SemExprKind::AddressOfSymbol(symbol),
                ty: expected.clone(),
                class: SemExprClass::Value,
                eval_order: Some(self.next_eval_order()),
                span: expr.span,
            };
        }

        if let Some(expr) = self.lower_arithmetic_for_expected_word_scalar(scope, expected, expr) {
            return expr;
        }

        let lowered = self.lower_expr(scope, expr);
        let machine_type_differs = expected
            .as_scalar()
            .zip(lowered.ty.as_scalar())
            .is_some_and(|(expected, actual)| {
                expected.width_bytes() != actual.width_bytes()
                    || expected.signedness() != actual.signedness()
            });
        if machine_type_differs {
            self.coerce_scalar_expr_for_expected_type(lowered, expected)
        } else {
            lowered
        }
    }

    fn lower_compound_assignment_value(
        &mut self,
        scope: ScopeId,
        target: &Expr,
        value: &Expr,
    ) -> SemExpr {
        if self
            .lvalue_expr_type(scope, target)
            .is_some_and(|ty| ty.is_real())
        {
            let lowered = self.lower_expr(scope, value);
            self.coerce_assignment_expr_to_real(lowered)
        } else {
            self.lower_expr(scope, value)
        }
    }

    fn coerce_assignment_expr_to_real(&mut self, expr: SemExpr) -> SemExpr {
        if expr.ty.as_scalar().is_some()
            && let SemExprKind::Unary {
                op: op @ (UnaryOp::Plus | UnaryOp::Neg),
                expr: operand,
            } = expr.kind
        {
            let span = expr.span;
            return SemExpr {
                kind: SemExprKind::Unary {
                    op,
                    expr: Box::new(self.coerce_integer_expr_to_real(*operand)),
                },
                ty: ValueType::real(),
                class: SemExprClass::Value,
                eval_order: Some(self.next_eval_order()),
                span,
            };
        }
        self.coerce_integer_expr_to_real(expr)
    }

    fn lower_binary_expr(
        &mut self,
        scope: ScopeId,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> SemExprKind {
        let mut left = self.lower_expr(scope, left);
        let mut right = self.lower_expr(scope, right);
        if left.ty.is_real() || right.ty.is_real() {
            left = self.coerce_integer_expr_to_real(left);
            right = self.coerce_integer_expr_to_real(right);
        } else if is_compare_op(op)
            && left.ty.as_scalar().is_some()
            && right.ty.as_scalar().is_some()
        {
            let operand_ty = promote_numeric_types(&left.ty, &right.ty);
            left = self.widen_arithmetic_tree_for_expected_type(left, &operand_ty);
            left = self.coerce_scalar_comparison_expr(left, &operand_ty);
            right = self.widen_arithmetic_tree_for_expected_type(right, &operand_ty);
            right = self.coerce_scalar_comparison_expr(right, &operand_ty);
        } else if is_compare_op(op)
            && left.ty.is_pointer()
            && right.ty.as_scalar().is_some()
            && left.ty.value_width_bytes() != right.ty.value_width_bytes()
        {
            let operand_ty = left.ty.clone();
            right = self.coerce_scalar_expr_for_expected_type(right, &operand_ty);
        } else if is_compare_op(op)
            && right.ty.is_pointer()
            && left.ty.as_scalar().is_some()
            && right.ty.value_width_bytes() != left.ty.value_width_bytes()
        {
            let operand_ty = right.ty.clone();
            left = self.coerce_scalar_expr_for_expected_type(left, &operand_ty);
        }
        SemExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    fn coerce_integer_expr_to_real(&mut self, expr: SemExpr) -> SemExpr {
        if expr.ty.is_real() || expr.ty.as_scalar().is_none() {
            return expr;
        }
        let span = expr.span;
        SemExpr {
            kind: SemExprKind::Cast {
                ty: ValueType::real(),
                expr: Box::new(expr),
            },
            ty: ValueType::real(),
            class: SemExprClass::Value,
            eval_order: Some(self.next_eval_order()),
            span,
        }
    }

    fn lower_arithmetic_for_expected_word_scalar(
        &mut self,
        scope: ScopeId,
        expected: &ValueType,
        expr: &Expr,
    ) -> Option<SemExpr> {
        let is_arithmetic = matches!(
            &expr.kind,
            ExprKind::Binary { op, .. } if !is_compare_op(*op)
        ) || matches!(
            &expr.kind,
            ExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Neg,
                ..
            }
        );
        if !is_arithmetic {
            return None;
        }
        let expected_scalar = expected.as_scalar()?;
        if expected_scalar.width_bytes() != 2 {
            return None;
        }

        let lowered = self.lower_expr(scope, expr);
        if !scalar_expected_type_can_accept_expr(expected, &lowered.ty) {
            return None;
        }
        let widened = self.widen_arithmetic_tree_for_expected_type(lowered, expected);
        if widened.ty == *expected {
            Some(widened)
        } else {
            Some(self.coerce_scalar_expr_for_expected_type(widened, expected))
        }
    }

    fn widen_arithmetic_tree_for_expected_type(
        &mut self,
        expr: SemExpr,
        expected: &ValueType,
    ) -> SemExpr {
        if !scalar_expected_type_can_accept_expr(expected, &expr.ty) {
            return expr;
        }
        if expected
            .as_scalar()
            .zip(expr.ty.as_scalar())
            .is_some_and(|(expected, actual)| expected.width_bytes() == actual.width_bytes())
        {
            return expr;
        }

        let SemExpr {
            kind,
            ty,
            class,
            eval_order,
            span,
        } = expr;

        match kind {
            SemExprKind::Binary { op, left, right } if !is_compare_op(op) => SemExpr {
                kind: SemExprKind::Binary {
                    op,
                    left: Box::new(self.widen_arithmetic_tree_for_expected_type(*left, expected)),
                    right: Box::new(self.widen_arithmetic_tree_for_expected_type(*right, expected)),
                },
                ty: expected.clone(),
                class,
                eval_order,
                span,
            },
            SemExprKind::Unary {
                op: op @ (UnaryOp::Plus | UnaryOp::Neg),
                expr,
            } => {
                let inner = self.widen_arithmetic_tree_for_expected_type(*expr, expected);
                let inner = self.coerce_scalar_expr_for_expected_type(inner, expected);
                SemExpr {
                    kind: SemExprKind::Unary {
                        op,
                        expr: Box::new(inner),
                    },
                    ty: expected.clone(),
                    class,
                    eval_order: Some(self.next_eval_order()),
                    span,
                }
            }
            kind => SemExpr {
                kind,
                ty,
                class,
                eval_order,
                span,
            },
        }
    }

    fn coerce_scalar_expr_for_expected_type(
        &mut self,
        expr: SemExpr,
        expected: &ValueType,
    ) -> SemExpr {
        if expr.ty == *expected {
            return expr;
        }
        let span = expr.span;
        SemExpr {
            kind: SemExprKind::Cast {
                ty: expected.clone(),
                expr: Box::new(expr),
            },
            ty: expected.clone(),
            class: SemExprClass::Value,
            eval_order: Some(self.next_eval_order()),
            span,
        }
    }

    fn coerce_scalar_comparison_expr(&mut self, expr: SemExpr, expected: &ValueType) -> SemExpr {
        let same_machine_type =
            expected
                .as_scalar()
                .zip(expr.ty.as_scalar())
                .is_some_and(|(expected, actual)| {
                    expected.width_bytes() == actual.width_bytes()
                        && expected.signedness() == actual.signedness()
                });
        if same_machine_type {
            expr
        } else {
            self.coerce_scalar_expr_for_expected_type(expr, expected)
        }
    }

    fn array_decay_for_expected_pointer(
        &mut self,
        scope: ScopeId,
        expected: &ValueType,
        expr: &Expr,
    ) -> Option<SemArrayDecay> {
        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        let symbol = self.symbol_ref(scope, name, expr.span)?;
        if !self.is_array_symbol(symbol.id) {
            return None;
        }
        let decay = self.array_decay_for_symbol(scope, expr, symbol);
        array_decay_pointer_types_compatible(expected, &decay.pointer_type).then_some(decay)
    }

    fn expr_is_named_place(&self, scope: ScopeId, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Name(name) => self
                .symbol_ref(scope, name, expr.span)
                .is_some_and(|symbol| {
                    matches!(
                        symbol.class,
                        SymbolClass::Var | SymbolClass::Array | SymbolClass::Param
                    )
                }),
            _ => false,
        }
    }

    fn array_decay_for_symbol(
        &mut self,
        scope: ScopeId,
        expr: &Expr,
        symbol: SemSymbolRef,
    ) -> SemArrayDecay {
        let layout = self.model.layout.array_for_symbol(symbol.id);
        let element_type = layout
            .map(|layout| layout.element_type.clone())
            .or_else(|| symbol.ty.clone())
            .unwrap_or_else(ValueType::error);
        let pointer_type = layout
            .map(|layout| layout.pointer_type.clone())
            .unwrap_or_else(|| ValueType::pointer_to(element_type.clone()));
        let origin = layout
            .map(|layout| sem_array_origin_from_layout(layout.origin))
            .unwrap_or(SemArrayOrigin::Unknown);

        SemArrayDecay {
            array: Box::new(self.lower_lvalue(scope, expr)),
            element_type,
            pointer_type,
            origin,
        }
    }

    fn is_array_symbol(&self, symbol_id: SymbolId) -> bool {
        self.model.layout.array_for_symbol(symbol_id).is_some()
    }

    fn array_origin_for_symbol(&self, symbol: &SemSymbolRef) -> SemArrayOrigin {
        self.model
            .layout
            .array_for_symbol(symbol.id)
            .map(|layout| sem_array_origin_from_layout(layout.origin))
            .unwrap_or(SemArrayOrigin::Unknown)
    }

    fn array_type_from_symbol(
        &self,
        scope: ScopeId,
        symbol: &SemSymbolRef,
        length: Option<&Expr>,
    ) -> ArrayType {
        let element = self
            .model
            .layout
            .array_for_symbol(symbol.id)
            .map(|layout| layout.element_type.clone())
            .or_else(|| symbol.ty.clone())
            .unwrap_or_else(ValueType::error);
        ArrayType::new(
            element,
            length.and_then(|expr| self.const_u16_expr_in_scope(scope, expr)),
        )
    }

    fn const_u16_expr_in_scope(&self, scope: ScopeId, expr: &Expr) -> Option<u16> {
        match &expr.kind {
            ExprKind::Number(number) => number.value,
            ExprKind::Name(name) => {
                let symbol = self.symbol_ref(scope, name, expr.span)?;
                self.model
                    .constants
                    .get(&symbol.id)
                    .map(|value| value.bits)
                    .or_else(|| {
                        self.numeric_defines
                            .get(&symbol.id)
                            .and_then(|number| number.value)
                    })
            }
            ExprKind::Unary {
                op: UnaryOp::Plus,
                expr,
            } => self.const_u16_expr_in_scope(scope, expr),
            ExprKind::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(0u16.wrapping_sub(self.const_u16_expr_in_scope(scope, expr)?)),
            ExprKind::Binary { op, left, right } => {
                let left = self.const_u16_expr_in_scope(scope, left)?;
                let right = self.const_u16_expr_in_scope(scope, right)?;
                match op {
                    BinaryOp::Add => Some(left.wrapping_add(right)),
                    BinaryOp::Sub => Some(left.wrapping_sub(right)),
                    BinaryOp::Mul => Some(left.wrapping_mul(right)),
                    BinaryOp::Div if right != 0 => Some(left / right),
                    BinaryOp::Mod if right != 0 => Some(left % right),
                    BinaryOp::Div | BinaryOp::Mod => None,
                    BinaryOp::Lsh => Some(if right >= 16 {
                        0
                    } else {
                        left.wrapping_shl(u32::from(right))
                    }),
                    BinaryOp::Rsh => Some(if right >= 16 {
                        0
                    } else {
                        left.wrapping_shr(u32::from(right))
                    }),
                    BinaryOp::And => Some(left & right),
                    BinaryOp::Or => Some(left | right),
                    BinaryOp::Xor => Some(left ^ right),
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge => None,
                }
            }
            _ => None,
        }
    }

    fn lower_condition(&mut self, scope: ScopeId, expr: &Expr) -> SemCondition {
        let lowered = self.lower_expr(scope, expr);
        let kind = match &lowered.kind {
            _ if lowered.ty.is_error() => SemConditionKind::Error,
            SemExprKind::Literal(SemLiteral::Number(number)) if number.value == Some(0) => {
                SemConditionKind::ConstantFalse
            }
            SemExprKind::Literal(SemLiteral::Number(number)) if number.value.is_some() => {
                SemConditionKind::ConstantTrue
            }
            SemExprKind::Literal(SemLiteral::Constant(value)) if value.bits == 0 => {
                SemConditionKind::ConstantFalse
            }
            SemExprKind::Literal(SemLiteral::Constant(_)) => SemConditionKind::ConstantTrue,
            SemExprKind::Binary { op, .. } if is_compare_op(*op) => SemConditionKind::Compare,
            SemExprKind::Binary {
                op: BinaryOp::And | BinaryOp::Or,
                left,
                right,
            } if is_logical_condition_tree(left) && is_logical_condition_tree(right) => {
                SemConditionKind::Logical
            }
            _ if lowered.class == SemExprClass::Condition => SemConditionKind::Compare,
            _ => SemConditionKind::NonZeroValue,
        };

        SemCondition {
            span: expr.span,
            expr: lowered,
            kind,
        }
    }

    fn lower_lvalue(&mut self, scope: ScopeId, expr: &Expr) -> SemLValue {
        let is_volatile = self.lvalue_expr_is_volatile(scope, expr);
        let kind = match &expr.kind {
            ExprKind::Name(name) => self
                .symbol_ref(scope, name, expr.span)
                .map(SemLValueKind::Symbol)
                .unwrap_or_else(|| SemLValueKind::UnresolvedName(name.clone())),
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: pointer,
            } => SemLValueKind::Deref {
                pointer: Box::new(self.lower_expr(scope, pointer)),
            },
            ExprKind::Index { base, index } => {
                let base = self.lower_expr(scope, base);
                SemLValueKind::Index {
                    element_type: indexed_expr_type(&base),
                    base: Box::new(base),
                    index: Box::new(self.lower_expr(scope, index)),
                    syntax: SemIndexSyntax::Index,
                }
            }
            ExprKind::Call { callee, args }
                if args.len() == 1 && self.is_indexable_lvalue(scope, callee) =>
            {
                let base = self.lower_expr(scope, callee);
                SemLValueKind::Index {
                    element_type: indexed_expr_type(&base),
                    base: Box::new(base),
                    index: Box::new(self.lower_expr(scope, &args[0])),
                    syntax: SemIndexSyntax::Call,
                }
            }
            ExprKind::Field { base, field } => {
                if let Some(symbol) = self.direct_symbol_ref_for_expr(scope, expr) {
                    SemLValueKind::Symbol(symbol)
                } else {
                    let base_lvalue = self.lower_lvalue(scope, base);
                    let descriptor = if base_lvalue.ty.is_error() {
                        None
                    } else {
                        self.field_descriptor(&base_lvalue.ty, field)
                    };
                    SemLValueKind::Field {
                        base: Box::new(base_lvalue),
                        field: SemFieldRef {
                            id: descriptor.map(|field| field.id),
                            owner: descriptor.map(|field| field.owner),
                            name: field.clone(),
                            ty: descriptor
                                .map(|field| field.ty.clone())
                                .unwrap_or_else(byte_type),
                            offset: descriptor.map(|field| field.offset),
                            span: expr.span,
                        },
                    }
                }
            }
            _ => SemLValueKind::Deref {
                pointer: Box::new(self.lower_expr(scope, expr)),
            },
        };
        let ty = self.lvalue_type_from_kind(&kind);
        let access = self.lvalue_access(&kind);

        SemLValue {
            kind,
            ty,
            access,
            is_volatile,
            storage: None,
            span: expr.span,
        }
    }

    fn lvalue_expr_is_volatile(&self, scope: ScopeId, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Name(name) => self
                .model
                .symbols
                .lookup(scope, name)
                .is_some_and(|id| self.model.symbols.symbols[id.0].is_volatile),
            ExprKind::Field { .. } if self.direct_symbol_ref_for_expr(scope, expr).is_some() => {
                self.direct_symbol_ref_for_expr(scope, expr)
                    .is_some_and(|symbol| symbol.is_volatile)
            }
            ExprKind::Index { base, .. }
            | ExprKind::Field { base, .. }
            | ExprKind::Cast { expr: base, .. } => self.lvalue_expr_is_volatile(scope, base),
            ExprKind::Call { callee, args } if args.len() == 1 => {
                self.lvalue_expr_is_volatile(scope, callee)
            }
            ExprKind::Missing
            | ExprKind::Raw
            | ExprKind::InitializerList(_)
            | ExprKind::CurrentLocation
            | ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Char(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Call { .. } => false,
        }
    }

    fn lvalue_type_from_kind(&self, kind: &SemLValueKind) -> ValueType {
        match kind {
            SemLValueKind::Symbol(symbol) => symbol.ty.clone().unwrap_or_else(ValueType::error),
            SemLValueKind::UnresolvedName(_) => ValueType::error(),
            SemLValueKind::Deref { pointer } => pointer
                .ty
                .as_pointer()
                .map_or_else(ValueType::error, |ty| *ty.pointee),
            SemLValueKind::Index { element_type, .. } => element_type.clone(),
            SemLValueKind::Field { field, .. } => field.ty.clone(),
        }
    }

    fn lvalue_access(&self, kind: &SemLValueKind) -> PlaceAccess {
        match kind {
            SemLValueKind::Symbol(symbol) => match symbol.class {
                SymbolClass::Var | SymbolClass::Array | SymbolClass::Param => {
                    PlaceAccess::Assignable
                }
                SymbolClass::Proc | SymbolClass::Func => PlaceAccess::ReadOnly,
                SymbolClass::BuiltinProc
                | SymbolClass::BuiltinFunc
                | SymbolClass::Define
                | SymbolClass::Const
                | SymbolClass::Type
                | SymbolClass::Record => PlaceAccess::ReadOnly,
            },
            SemLValueKind::UnresolvedName(_) => PlaceAccess::Error,
            SemLValueKind::Deref { .. } | SemLValueKind::Index { .. } => PlaceAccess::Assignable,
            SemLValueKind::Field { base, .. } => base.access,
        }
    }

    fn is_indexable_lvalue(&self, scope: ScopeId, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Name(name) => self
                .symbol_ref(scope, name, expr.span)
                .is_some_and(|symbol| {
                    matches!(
                        symbol.class,
                        SymbolClass::Array | SymbolClass::Param | SymbolClass::Var
                    )
                }),
            ExprKind::Field { .. } if self.direct_symbol_ref_for_expr(scope, expr).is_some() => {
                self.direct_symbol_ref_for_expr(scope, expr)
                    .is_some_and(|symbol| {
                        matches!(
                            symbol.class,
                            SymbolClass::Array | SymbolClass::Param | SymbolClass::Var
                        )
                    })
            }
            ExprKind::Field { .. }
            | ExprKind::Unary {
                op: UnaryOp::Deref, ..
            } => true,
            _ => false,
        }
    }

    fn lower_call_expr(&mut self, scope: ScopeId, expr: &Expr) -> SemCall {
        let ExprKind::Call { callee, args } = &expr.kind else {
            return SemCall {
                callee: SemCallable::Indirect {
                    target: Box::new(self.lower_expr(scope, expr)),
                    signature: SemRoutineSignature::unknown_proc(),
                },
                callable_type: CallableType::unknown_proc(),
                args: Vec::new(),
                return_type: None,
                effects: SemEffects::default(),
                span: expr.span,
            };
        };

        let callee = match self.direct_symbol_ref_for_expr(scope, callee) {
            Some(symbol) => Some(symbol)
                .map(|symbol| match symbol.class {
                    SymbolClass::BuiltinProc | SymbolClass::BuiltinFunc
                        if self
                            .model
                            .routine_signatures_by_symbol
                            .get(&symbol.id)
                            .is_some_and(|signature| {
                                signature.source == super::SemanticCallableSource::Runtime
                            }) =>
                    {
                        SemCallable::User(symbol)
                    }
                    SymbolClass::BuiltinProc | SymbolClass::BuiltinFunc => {
                        SemCallable::Builtin(symbol)
                    }
                    SymbolClass::Proc | SymbolClass::Func => SemCallable::User(symbol),
                    _ if symbol
                        .ty
                        .as_ref()
                        .and_then(ValueType::as_callable_pointer)
                        .is_some() =>
                    {
                        let signature = SemRoutineSignature::from_callable_type(
                            symbol
                                .ty
                                .as_ref()
                                .and_then(ValueType::as_callable_pointer)
                                .unwrap(),
                        );
                        SemCallable::Indirect {
                            target: Box::new(SemExpr {
                                kind: SemExprKind::Symbol(symbol.clone()),
                                ty: symbol.ty.clone().unwrap_or_else(ValueType::error),
                                class: SemExprClass::Value,
                                eval_order: Some(self.next_eval_order()),
                                span: callee.span,
                            }),
                            signature,
                        }
                    }
                    _ => SemCallable::User(symbol),
                })
                .unwrap_or_else(|| SemCallable::Indirect {
                    target: Box::new(self.lower_expr(scope, callee)),
                    signature: SemRoutineSignature::unknown_proc(),
                }),
            None => SemCallable::Indirect {
                target: Box::new(self.lower_expr(scope, callee)),
                signature: SemRoutineSignature::unknown_proc(),
            },
        };

        let callable_type = self.callable_type_for_callee(&callee);
        let expected_params = callable_type.params.clone();
        let return_type = callable_type.return_type.clone();
        let effects = self.call_effects_for_callee(&callee);

        SemCall {
            callee,
            callable_type,
            args: args
                .iter()
                .enumerate()
                .map(|(index, arg)| {
                    expected_params
                        .get(index)
                        .map(|expected| self.lower_value_for_expected_type(scope, expected, arg))
                        .unwrap_or_else(|| self.lower_expr(scope, arg))
                })
                .collect(),
            return_type,
            effects,
            span: expr.span,
        }
    }

    fn call_effects_for_callee(&self, callee: &SemCallable) -> SemEffects {
        let external = match callee {
            SemCallable::User(symbol) | SemCallable::Builtin(symbol) => self
                .model
                .routine_signatures_by_symbol
                .get(&symbol.id)
                .is_some_and(|signature| {
                    matches!(
                        signature.source,
                        super::SemanticCallableSource::Resident
                            | super::SemanticCallableSource::Runtime
                    )
                }),
            SemCallable::Runtime { .. } => true,
            SemCallable::Indirect { .. } => false,
        };
        if external {
            SemEffects {
                writes: vec![SemWriteEffect::Unknown],
                reads: vec![SemReadEffect::Unknown],
                may_call_os: true,
                opaque: true,
            }
        } else {
            SemEffects::default()
        }
    }

    fn callable_type_for_callee(&self, callee: &SemCallable) -> CallableType {
        match callee {
            SemCallable::User(symbol) | SemCallable::Builtin(symbol) => self
                .model
                .routine_signatures_by_symbol
                .get(&symbol.id)
                .map(|signature| {
                    callable_type_from_signature_parts(
                        signature.kind.clone(),
                        signature.params.clone(),
                        signature.variadic.clone(),
                        signature.return_type.clone(),
                    )
                })
                .unwrap_or_else(|| {
                    let return_type = symbol.ty.clone();
                    let kind = callable_kind_from_return_type(return_type.as_ref());
                    CallableType::new(kind, Vec::new(), return_type)
                }),
            SemCallable::Indirect { signature, .. } | SemCallable::Runtime { signature, .. } => {
                signature.callable_type()
            }
        }
    }

    fn callable_pointer_type_for_symbol(&self, symbol: &SemSymbolRef) -> ValueType {
        ValueType::callable_pointer(
            self.model
                .routine_signatures_by_symbol
                .get(&symbol.id)
                .map(|signature| {
                    callable_type_from_signature_parts(
                        signature.kind.clone(),
                        signature.params.clone(),
                        signature.variadic.clone(),
                        signature.return_type.clone(),
                    )
                })
                .unwrap_or_else(|| {
                    let return_type = symbol.ty.clone();
                    let kind = callable_kind_from_return_type(return_type.as_ref());
                    CallableType::new(kind, Vec::new(), return_type)
                }),
        )
    }

    fn field_descriptor_by_name(
        &self,
        owner_name: &str,
        field_name: &str,
    ) -> Option<&super::SemanticField> {
        let id = self
            .model
            .field_lookup
            .get(&normalize_name(owner_name))?
            .get(&normalize_name(field_name))?;
        self.model.fields.get(id.0)
    }

    fn field_descriptor(
        &self,
        base: &ValueType,
        field_name: &str,
    ) -> Option<&super::SemanticField> {
        let owner_name = base.as_record_identity()?.name;
        self.field_descriptor_by_name(owner_name, field_name)
    }

    fn symbol_ref(&self, scope: ScopeId, name: &str, span: Span) -> Option<SemSymbolRef> {
        let name =
            crate::ast::QualifiedName::new(name.split('.').map(str::to_string).collect::<Vec<_>>());
        self.qualified_symbol_ref(scope, &name, span)
    }

    fn qualified_symbol_ref(
        &self,
        scope: ScopeId,
        name: &crate::ast::QualifiedName,
        span: Span,
    ) -> Option<SemSymbolRef> {
        let SemanticNameResolution::Symbol(id) = self.model.resolve_name(scope, name) else {
            return None;
        };
        Some(self.symbol_ref_by_id(id, span))
    }

    fn symbol_ref_by_id(&self, id: SymbolId, span: Span) -> SemSymbolRef {
        let symbol = &self.model.symbols.symbols[id.0];
        SemSymbolRef {
            id,
            name: symbol
                .defining_module
                .map(|_| {
                    module_link_name(
                        &symbol.qualified_name,
                        &symbol.canonical_qualified_key,
                        &symbol.class,
                    )
                })
                .unwrap_or_else(|| symbol.name.clone()),
            defining_module: symbol.defining_module,
            canonical_qualified_key: symbol.canonical_qualified_key.clone(),
            qualified_name: symbol.qualified_name.clone(),
            lexical_display_name: matches!(
                self.model.symbols.scopes[symbol.scope.0].kind,
                crate::semantic::ScopeKind::LexicalBlock
            )
            .then(|| symbol.qualified_name.replace('.', "::")),
            class: symbol.class.clone(),
            ty: symbol.ty.clone(),
            is_volatile: symbol.is_volatile,
            scope: symbol.scope,
            span,
        }
    }

    fn direct_symbol_ref_for_expr(&self, scope: ScopeId, expr: &Expr) -> Option<SemSymbolRef> {
        match &expr.kind {
            ExprKind::Name(name) => self.symbol_ref(scope, name, expr.span),
            ExprKind::Field { base, field } => {
                let ExprKind::Name(alias) = &base.kind else {
                    return None;
                };
                let name = crate::ast::QualifiedName::new(vec![alias.clone(), field.clone()]);
                self.qualified_symbol_ref(scope, &name, expr.span)
            }
            _ => None,
        }
    }

    fn synthetic_symbol_ref(&self, name: &str, class: SymbolClass, span: Span) -> SemSymbolRef {
        SemSymbolRef {
            id: SymbolId(usize::MAX),
            name: name.to_string(),
            defining_module: None,
            canonical_qualified_key: normalize_name(name),
            qualified_name: name.to_string(),
            lexical_display_name: None,
            class,
            ty: None,
            is_volatile: false,
            scope: self.model.symbols.global_scope(),
            span,
        }
    }

    fn sem_type_from_symbol(&self, symbol: &SemSymbolRef) -> SemType {
        SemType {
            value: symbol.ty.clone().unwrap_or_else(byte_type),
            width: symbol.ty.as_ref().and_then(value_width),
        }
    }

    fn collect_numeric_defines(
        &self,
        program: &Program,
        top_scope: ScopeId,
    ) -> HashMap<SymbolId, NumberLiteral> {
        let mut defines = HashMap::new();
        for module in &program.modules {
            for item in &module.items {
                match item {
                    Item::Define(define) => {
                        self.collect_numeric_define_decl(top_scope, define, &mut defines);
                    }
                    Item::Routine(routine) => {
                        let routine_symbol =
                            self.model.symbols.lookup_exact(top_scope, &routine.name);
                        let routine_scope = self
                            .model
                            .routine_scopes
                            .iter()
                            .find(|scope| scope.symbol == routine_symbol)
                            .map(|routine| routine.scope)
                            .unwrap_or(top_scope);
                        for stmt in &routine.body {
                            self.collect_numeric_define_stmt(routine_scope, stmt, &mut defines);
                        }
                    }
                    Item::Statement(Stmt::Define(define)) => {
                        self.collect_numeric_define_decl(top_scope, define, &mut defines);
                    }
                    Item::Include(_)
                    | Item::Set(_)
                    | Item::Declaration(_)
                    | Item::Statement(_)
                    | Item::Unsupported { .. } => {}
                }
            }
        }
        defines
    }

    fn collect_numeric_define_stmt(
        &self,
        scope: ScopeId,
        stmt: &Stmt,
        defines: &mut HashMap<SymbolId, NumberLiteral>,
    ) {
        match stmt {
            Stmt::LexicalBlock {
                syntax_id, body, ..
            } => {
                let block_scope = self
                    .model
                    .lexical_blocks
                    .iter()
                    .find(|block| block.parent == scope && block.syntax_id == *syntax_id)
                    .map(|block| block.scope)
                    .unwrap_or(scope);
                for stmt in body {
                    self.collect_numeric_define_stmt(block_scope, stmt, defines);
                }
            }
            Stmt::Define(define) => self.collect_numeric_define_decl(scope, define, defines),
            Stmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    for stmt in &branch.body {
                        self.collect_numeric_define_stmt(scope, stmt, defines);
                    }
                }
                for stmt in else_body {
                    self.collect_numeric_define_stmt(scope, stmt, defines);
                }
            }
            Stmt::While { body, .. } | Stmt::DoUntil { body, .. } | Stmt::For { body, .. } => {
                for stmt in body {
                    self.collect_numeric_define_stmt(scope, stmt, defines);
                }
            }
            Stmt::Return(_)
            | Stmt::Exit { .. }
            | Stmt::Assign { .. }
            | Stmt::CompoundAssign { .. }
            | Stmt::Call { .. }
            | Stmt::MachineBlock { .. }
            | Stmt::InlineAsm { .. }
            | Stmt::Unsupported { .. } => {}
        }
    }

    fn collect_numeric_define_decl(
        &self,
        scope: ScopeId,
        define: &DefineDecl,
        defines: &mut HashMap<SymbolId, NumberLiteral>,
    ) {
        for entry in &define.entries {
            let Some(number) = parse_numeric_define_value(&entry.value) else {
                continue;
            };
            if let Some(id) = self.model.symbols.lookup(scope, &entry.name) {
                defines.insert(id, number);
            }
        }
    }

    fn fallback_expr_type(&self, scope: ScopeId, expr: &Expr) -> Option<ValueType> {
        match &expr.kind {
            ExprKind::CurrentLocation => Some(card_type()),
            ExprKind::Number(number) => Some(value_type_for_number(number)),
            ExprKind::Char(_) => Some(char_type()),
            ExprKind::Name(name) => self
                .symbol_ref(scope, name, expr.span)
                .and_then(|symbol| symbol.ty),
            ExprKind::Call { callee, .. } => self
                .direct_symbol_ref_for_expr(scope, callee)
                .and_then(|symbol| symbol.ty),
            ExprKind::Index { base, .. } => self.fallback_expr_type(scope, base),
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => self
                .fallback_expr_type(scope, expr)
                .and_then(|ty| ty.as_pointer())
                .map(|ty| *ty.pointee),
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr,
            } => {
                if self.direct_symbol_ref_for_expr(scope, expr).is_some() {
                    if let Some(symbol) = self.direct_symbol_ref_for_expr(scope, expr) {
                        if matches!(
                            symbol.class,
                            SymbolClass::Proc
                                | SymbolClass::Func
                                | SymbolClass::BuiltinProc
                                | SymbolClass::BuiltinFunc
                        ) {
                            Some(self.callable_pointer_type_for_symbol(&symbol))
                        } else {
                            self.fallback_expr_type(scope, expr)
                                .map(ValueType::pointer_to)
                        }
                    } else {
                        self.fallback_expr_type(scope, expr)
                            .map(ValueType::pointer_to)
                    }
                } else {
                    self.fallback_expr_type(scope, expr)
                        .map(ValueType::pointer_to)
                }
            }
            ExprKind::Cast { ty, .. } => Some(self.resolved_type_ref(scope, ty)),
            _ => None,
        }
    }

    fn lvalue_expr_type(&self, scope: ScopeId, expr: &Expr) -> Option<ValueType> {
        match &expr.kind {
            ExprKind::Name(name) => self
                .symbol_ref(scope, name, expr.span)
                .and_then(|symbol| symbol.ty),
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => self
                .fallback_expr_type(scope, expr)
                .and_then(|ty| ty.as_pointer())
                .map(|ty| *ty.pointee),
            ExprKind::Index { base, .. } => {
                self.fallback_expr_type(scope, base).map(indexed_value_type)
            }
            ExprKind::Call { callee, args }
                if args.len() == 1 && self.is_indexable_lvalue(scope, callee) =>
            {
                self.fallback_expr_type(scope, callee)
                    .map(indexed_value_type)
            }
            ExprKind::Field { .. } if self.direct_symbol_ref_for_expr(scope, expr).is_some() => {
                self.direct_symbol_ref_for_expr(scope, expr)
                    .and_then(|symbol| symbol.ty)
            }
            ExprKind::Field { base, field } => self
                .lvalue_expr_type(scope, base)
                .and_then(|base_ty| self.field_descriptor(&base_ty, field))
                .map(|field| field.ty.clone()),
            _ => self.fallback_expr_type(scope, expr),
        }
    }

    fn next_eval_order(&mut self) -> SemEvalOrderId {
        let id = SemEvalOrderId(self.next_eval_order);
        self.next_eval_order += 1;
        id
    }

    fn resolved_type_ref(&self, scope: ScopeId, ty: &TypeRef) -> ValueType {
        let mut value = ValueType::from(ty);
        let TypeBase::Named(name) = &ty.base else {
            return value;
        };
        if name.eq_ignore_ascii_case("STRING") {
            return value;
        }
        if let Some(symbol) = self.qualified_symbol_ref(scope, name, Span::new(0, 0))
            && matches!(symbol.class, SymbolClass::Type | SymbolClass::Record)
        {
            value.base = if symbol.ty.as_ref().is_some_and(ValueType::is_real) {
                ValueTypeBase::Real
            } else {
                ValueTypeBase::Named(symbol.qualified_name)
            };
        }
        value
    }
}

impl From<&VarDecl> for ValueType {
    fn from(value: &VarDecl) -> Self {
        ValueType::from(&value.ty)
    }
}

impl From<&crate::ast::TypeRef> for ValueType {
    fn from(value: &crate::ast::TypeRef) -> Self {
        let base = match &value.base {
            crate::ast::TypeBase::Fund(fund) => ValueTypeBase::Fund(*fund),
            crate::ast::TypeBase::NativeReal => ValueTypeBase::Real,
            crate::ast::TypeBase::Named(name) if name.eq_ignore_ascii_case("STRING") => {
                ValueTypeBase::Fund(FundType::Char)
            }
            crate::ast::TypeBase::Named(name) => ValueTypeBase::Named(name.to_string()),
            crate::ast::TypeBase::Callable(kind) => ValueTypeBase::Callable(Box::new(
                CallableType::from_routine_kind(kind.clone(), Vec::new()),
            )),
        };

        Self {
            base,
            pointer: value.pointer && !matches!(value.base, crate::ast::TypeBase::Callable(_)),
        }
    }
}

impl From<ExprClass> for SemExprClass {
    fn from(value: ExprClass) -> Self {
        match value {
            ExprClass::Unknown => SemExprClass::Unknown,
            ExprClass::Value => SemExprClass::Value,
            ExprClass::LValue => SemExprClass::LValue,
            ExprClass::Callable => SemExprClass::Callable,
            ExprClass::Condition => SemExprClass::Condition,
        }
    }
}

impl SemRoutineSignature {
    fn from_header(kind: RoutineKind, params: impl IntoIterator<Item = ValueType>) -> Self {
        let return_type = match kind {
            RoutineKind::Func { return_type } => Some(return_type),
            RoutineKind::Proc => None,
        };

        Self {
            kind,
            params: params.into_iter().collect(),
            return_type,
        }
    }

    fn from_callable_type(callable: &CallableType) -> Self {
        Self {
            kind: callable.kind.clone(),
            params: callable.params.clone(),
            return_type: fund_type_from_value_ref(callable.return_type.as_ref()),
        }
    }

    fn unknown_proc() -> Self {
        Self {
            kind: RoutineKind::Proc,
            params: Vec::new(),
            return_type: None,
        }
    }

    fn callable_type(&self) -> CallableType {
        CallableType::new(
            self.kind.clone(),
            self.params.clone(),
            self.return_type.map(ValueType::fund),
        )
    }
}

fn callable_kind_from_return_type(return_type: Option<&ValueType>) -> RoutineKind {
    match return_type.and_then(fund_type_from_value) {
        Some(return_type) => RoutineKind::Func { return_type },
        None => RoutineKind::Proc,
    }
}

fn callable_type_from_signature_parts(
    kind: RoutineKind,
    params: Vec<ValueType>,
    variadic: Option<ValueType>,
    return_type: Option<ValueType>,
) -> CallableType {
    if let Some(variadic) = variadic {
        CallableType::new_variadic(kind, params, variadic, return_type)
    } else {
        CallableType::new(kind, params, return_type)
    }
}

fn fund_type_from_value(ty: &ValueType) -> Option<FundType> {
    match (&ty.base, ty.pointer) {
        (ValueTypeBase::Fund(fund), false) => Some(*fund),
        _ => None,
    }
}

fn fund_type_from_value_ref(ty: Option<&ValueType>) -> Option<FundType> {
    ty.and_then(fund_type_from_value)
}

fn param_signature_type(param: &SemParam) -> ValueType {
    match param.storage {
        SemParamStorage::Value => param.ty.value.clone(),
        SemParamStorage::Array => ValueType::pointer_to(param.ty.value.clone()),
    }
}

fn byte_type() -> ValueType {
    ValueType::fund(FundType::Byte)
}

fn char_type() -> ValueType {
    ValueType::fund(FundType::Char)
}

fn card_type() -> ValueType {
    ValueType::fund(FundType::Card)
}

fn int_type() -> ValueType {
    ValueType::fund(FundType::Int)
}

fn value_type_for_number(number: &NumberLiteral) -> ValueType {
    match number.kind {
        crate::lexer::NumberKind::Byte => byte_type(),
        crate::lexer::NumberKind::Int => int_type(),
        crate::lexer::NumberKind::Card => card_type(),
        crate::lexer::NumberKind::Real => ValueType::real(),
    }
}

fn scalar_expected_type_can_accept_expr(expected: &ValueType, actual: &ValueType) -> bool {
    expected.as_scalar().is_some()
        && actual.as_scalar().is_some()
        && expected.assignment_compatibility(actual).is_allowed()
}

fn parse_numeric_define_value(value: &str) -> Option<NumberLiteral> {
    let tokens = tokenize(value).ok()?;
    match tokens.as_slice() {
        [
            crate::lexer::Token {
                kind: TokenKind::Number(number),
                ..
            },
            crate::lexer::Token {
                kind: TokenKind::Eof,
                ..
            },
        ] => Some(number.clone()),
        _ => None,
    }
}

fn indexed_expr_type(expr: &SemExpr) -> ValueType {
    indexed_value_type(expr.ty.clone())
}

fn indexed_value_type(ty: ValueType) -> ValueType {
    if ty.pointer { ty.pointee_type() } else { ty }
}

fn expr_class_from_kind(kind: &SemExprKind) -> SemExprClass {
    match kind {
        SemExprKind::Missing | SemExprKind::Raw(_) | SemExprKind::UnresolvedName(_) => {
            SemExprClass::Unknown
        }
        SemExprKind::Symbol(symbol) => match symbol.class {
            SymbolClass::Proc
            | SymbolClass::Func
            | SymbolClass::BuiltinProc
            | SymbolClass::BuiltinFunc => SemExprClass::Callable,
            SymbolClass::Var | SymbolClass::Array | SymbolClass::Param => SemExprClass::LValue,
            SymbolClass::Define | SymbolClass::Const | SymbolClass::Type | SymbolClass::Record => {
                SemExprClass::Unknown
            }
        },
        SemExprKind::LValue(_) => SemExprClass::LValue,
        SemExprKind::Cast { .. } => SemExprClass::Value,
        SemExprKind::Binary { op, .. } if is_compare_op(*op) => SemExprClass::Condition,
        _ => SemExprClass::Value,
    }
}

fn is_logical_condition_tree(expr: &SemExpr) -> bool {
    match &expr.kind {
        SemExprKind::Binary { op, .. } if is_compare_op(*op) => true,
        SemExprKind::Binary {
            op: BinaryOp::And | BinaryOp::Or,
            left,
            right,
        } => is_logical_condition_tree(left) && is_logical_condition_tree(right),
        _ => false,
    }
}

fn compact_machine_symbol_name(items: &[MachineItem], item_index: usize) -> Option<String> {
    let MachineItem::Number(number) = items.get(item_index.checked_sub(1)?)? else {
        return None;
    };
    let digits = number.text.strip_prefix('$')?;
    if digits.len() <= 2 || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let suffix = match items.get(item_index)? {
        MachineItem::Name(name) | MachineItem::AddressByte { name, .. } => name.display_name(),
        MachineItem::AddressExpr(MachineAddressExpr {
            atom: MachineAddressAtom::Name(name),
            ..
        }) => name.display_name(),
        _ => return None,
    };
    Some(format!("{}{suffix}", &digits[2..]))
}

fn relink_inline_asm_items(
    items: &[MachineItem],
    link_names: &HashMap<String, String>,
) -> Vec<MachineItem> {
    let relink = |name: &QualifiedName| {
        link_names
            .get(&normalize_name(name.display_name()))
            .map(|link_name| QualifiedName::simple(link_name.clone()))
            .unwrap_or_else(|| name.clone())
    };

    items
        .iter()
        .cloned()
        .map(|item| match item {
            MachineItem::Name(name) => MachineItem::Name(relink(&name)),
            MachineItem::AddressByte { selector, name } => MachineItem::AddressByte {
                selector,
                name: relink(&name),
            },
            MachineItem::AddressExpr(mut expression) => {
                if let MachineAddressAtom::Name(name) = &expression.atom {
                    expression.atom = MachineAddressAtom::Name(relink(name));
                }
                MachineItem::AddressExpr(expression)
            }
            MachineItem::Number(_)
            | MachineItem::StringLiteral(_)
            | MachineItem::CharLiteral(_)
            | MachineItem::Raw(_) => item,
        })
        .collect()
}

fn promote_numeric_types(left: &ValueType, right: &ValueType) -> ValueType {
    if left.is_error() || right.is_error() {
        return ValueType::error();
    }
    if left.pointer || right.pointer {
        return card_type();
    }
    if left.is_real() || right.is_real() {
        return if left.is_numeric_value() && right.is_numeric_value() {
            ValueType::real()
        } else {
            ValueType::error()
        };
    }

    let Some(left) = left.as_scalar() else {
        return ValueType::error();
    };
    let Some(right) = right.as_scalar() else {
        return ValueType::error();
    };

    ValueType::scalar(ScalarType::promote_binary(left, right))
}

fn arithmetic_numeric_result_type(
    op: BinaryOp,
    left: &ValueType,
    right: &ValueType,
    constant_result: Option<u16>,
) -> ValueType {
    if left.is_error() || right.is_error() {
        return ValueType::error();
    }
    if left.pointer || right.pointer {
        return card_type();
    }
    if left.is_real() || right.is_real() {
        return if left.is_numeric_value() && right.is_numeric_value() {
            ValueType::real()
        } else {
            ValueType::error()
        };
    }

    let Some(left) = left.as_scalar() else {
        return ValueType::error();
    };
    let Some(right) = right.as_scalar() else {
        return ValueType::error();
    };

    ValueType::scalar(ScalarType::arithmetic_result(
        op,
        left,
        right,
        constant_result,
    ))
}

fn constant_sem_binary_result(op: BinaryOp, left: &SemExpr, right: &SemExpr) -> Option<u16> {
    if !matches!(op, BinaryOp::Add | BinaryOp::Sub) {
        return None;
    }
    let left = const_u16_sem_expr(left)?;
    let right = const_u16_sem_expr(right)?;
    Some(match op {
        BinaryOp::Add => left.wrapping_add(right),
        BinaryOp::Sub => left.wrapping_sub(right),
        _ => unreachable!("only addition and subtraction reach constant-result typing"),
    })
}

fn normalize_name(name: &str) -> String {
    name.to_ascii_uppercase()
}

/// Produce the backend-facing name for a named-module symbol.  The readable
/// prefix keeps maps and listings useful; the stable hash makes the identity
/// independent of module load order and distinguishes names which sanitize to
/// the same assembler spelling.
fn module_link_name(qualified_name: &str, canonical_key: &str, class: &SymbolClass) -> String {
    let mut stem = String::from("M_");
    for ch in qualified_name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            stem.push(ch.to_ascii_uppercase());
        } else if !stem.ends_with('_') {
            stem.push('_');
        }
    }
    while stem.ends_with('_') {
        stem.pop();
    }

    let identity = format!("{canonical_key}:{class:?}");
    let hash = identity.bytes().fold(0x811C_9DC5u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    format!("{stem}_{hash:08X}")
}

#[cfg(test)]
mod module_link_name_tests {
    use super::*;

    #[test]
    fn sanitized_collisions_have_stable_identity_suffixes() {
        let first = module_link_name("LIB.A_B.C", "lib.a_b.c", &SymbolClass::Var);
        let second = module_link_name("LIB.A.B_C", "lib.a.b_c", &SymbolClass::Var);

        assert!(first.starts_with("M_LIB_A_B_C_"));
        assert!(second.starts_with("M_LIB_A_B_C_"));
        assert_ne!(first, second);
        assert_eq!(
            first,
            module_link_name("LIB.A_B.C", "lib.a_b.c", &SymbolClass::Var)
        );
    }
}

fn value_width(value: &ValueType) -> Option<u16> {
    if value.pointer {
        return Some(2);
    }

    value.scalar_width_bytes()
}

fn array_decay_pointer_types_compatible(expected: &ValueType, actual: &ValueType) -> bool {
    expected.pointer
        && actual.pointer
        && (expected.base == actual.base
            || matches!(
                (&expected.base, &actual.base),
                (
                    ValueTypeBase::Fund(FundType::Byte),
                    ValueTypeBase::Fund(FundType::Char)
                ) | (
                    ValueTypeBase::Fund(FundType::Char),
                    ValueTypeBase::Fund(FundType::Byte)
                )
            )
            || matches!(expected.base, ValueTypeBase::Fund(_))
                && matches!(actual.base, ValueTypeBase::Fund(_)))
}

fn is_string_type_ref(ty: &TypeRef) -> bool {
    matches!(&ty.base, TypeBase::Named(name) if name.eq_ignore_ascii_case("STRING")) && !ty.pointer
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemReadEffect {
    Storage(SemStorageRef),
    ZeroPage { start: u8, end: u8 },
    Absolute { start: u16, end: u16 },
    Symbol(String),
    Unknown,
}
