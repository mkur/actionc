use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::resident::{RESIDENT_VARIABLES, ResidentVariableKind};
use crate::source::Span;

pub mod ir;
pub mod layout;
pub mod subject;
pub mod types;

pub use layout::{
    ArrayLayoutId, RecordLayoutId, SemanticArrayLayout, SemanticArrayOrigin, SemanticLayoutFacts,
    SemanticRecordFieldLayout, SemanticRecordLayout,
};
pub use types::{
    ArrayType, CallableType, PointerType, RecordFieldType, RecordIdentity, RecordIdentityRef,
    RecordType, ScalarSignedness, ScalarType, TypeCompatibility, ValueTypeKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticModel {
    pub symbols: SymbolTable,
    /// Compatibility/debug projection of expression categories and types.
    ///
    /// The authoritative semantic representation is the typed subject/SemIR
    /// nodes. Codegen and SemIR lowering must not depend on this side table.
    pub expression_observations: Vec<ExpressionObservation>,
    pub routine_scopes: Vec<RoutineScope>,
    pub array_symbols: HashSet<SymbolId>,
    pub fields: Vec<SemanticField>,
    pub field_lookup: HashMap<String, HashMap<String, FieldId>>,
    pub layout: SemanticLayoutFacts,
    pub routine_signatures: HashMap<String, SemanticCallableSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineScope {
    pub name: String,
    pub scope: ScopeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticField {
    pub id: FieldId,
    pub owner: SymbolId,
    pub name: String,
    pub ty: ValueType,
    pub offset: u16,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldDescriptorFacts {
    id: FieldId,
    owner: SymbolId,
    ty: ValueType,
    offset: u16,
}

/// Source-visible declarations only. Compiler-generated storage, such as loop
/// caches, must be represented by codegen/layout temps instead of SymbolIds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolTable {
    pub scopes: Vec<Scope>,
    pub symbols: Vec<Symbol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    symbols: HashMap<String, SymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Builtin,
    Global,
    Routine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupStage {
    Local,
    Global,
    Builtin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedSymbol {
    pub id: SymbolId,
    pub stage: LookupStage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub class: SymbolClass,
    pub ty: Option<ValueType>,
    pub scope: ScopeId,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolClass {
    BuiltinProc,
    BuiltinFunc,
    Define,
    Type,
    Record,
    Var,
    Array,
    Param,
    Proc,
    Func,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueType {
    pub base: ValueTypeBase,
    pub pointer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueTypeBase {
    Fund(FundType),
    Named(String),
    Callable(Box<CallableType>),
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionObservation {
    pub span: Span,
    pub class: ExprClass,
    pub ty: Option<ValueType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprClass {
    Unknown,
    Value,
    LValue,
    Callable,
    Condition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticCallableSignature {
    pub kind: RoutineKind,
    pub params: Vec<ValueType>,
    pub variadic: Option<ValueType>,
    pub return_type: Option<ValueType>,
    pub source: SemanticCallableSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticCallableSource {
    User,
    Resident,
    Runtime,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutineControlFlowFacts {
    pub may_fall_through: bool,
    pub always_returns: bool,
    pub contains_return: bool,
    pub contains_exit: bool,
    pub contains_loop: bool,
    pub max_loop_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StmtFlowFacts {
    pub may_continue: bool,
    pub may_return: bool,
    pub always_returns: bool,
    pub may_exit_loop: bool,
    pub contains_loop: bool,
    pub max_loop_depth: usize,
}

pub fn analyze(program: &Program) -> Result<SemanticModel, Vec<Diagnostic>> {
    let mut analyzer = Analyzer::new();
    analyzer.seed_builtins();
    analyzer.analyze_program(program);

    if analyzer.diagnostics.is_empty() {
        let layout = SemanticLayoutFacts::build(
            &analyzer.symbols,
            &analyzer.array_symbols,
            &analyzer.fields,
        );
        Ok(SemanticModel {
            symbols: analyzer.symbols,
            expression_observations: analyzer.expression_observations,
            routine_scopes: analyzer.routine_scopes,
            array_symbols: analyzer.array_symbols,
            fields: analyzer.fields,
            field_lookup: analyzer.field_lookup,
            layout,
            routine_signatures: analyzer.routines,
        })
    } else {
        Err(analyzer.diagnostics)
    }
}

struct Analyzer {
    symbols: SymbolTable,
    builtin_scope: ScopeId,
    global_scope: ScopeId,
    routines: HashMap<String, SemanticCallableSignature>,
    retargeted_routine_names: HashSet<String>,
    routine_scopes: Vec<RoutineScope>,
    array_symbols: HashSet<SymbolId>,
    fields: Vec<SemanticField>,
    field_lookup: HashMap<String, HashMap<String, FieldId>>,
    diagnostics: Vec<Diagnostic>,
    expression_observations: Vec<ExpressionObservation>,
}

#[derive(Clone, Copy)]
struct ControlContext<'a> {
    loop_depth: usize,
    routine_kind: Option<&'a RoutineKind>,
}

impl<'a> ControlContext<'a> {
    fn routine(routine_kind: &'a RoutineKind) -> Self {
        Self {
            loop_depth: 0,
            routine_kind: Some(routine_kind),
        }
    }

    fn global() -> Self {
        Self {
            loop_depth: 0,
            routine_kind: None,
        }
    }

    fn inside_loop(self) -> Self {
        Self {
            loop_depth: self.loop_depth + 1,
            ..self
        }
    }
}

impl Analyzer {
    fn new() -> Self {
        let mut symbols = SymbolTable {
            scopes: Vec::new(),
            symbols: Vec::new(),
        };
        let builtin_scope = symbols.add_scope(ScopeKind::Builtin, None);
        let global_scope = symbols.add_scope(ScopeKind::Global, Some(builtin_scope));

        Self {
            symbols,
            builtin_scope,
            global_scope,
            routines: HashMap::new(),
            retargeted_routine_names: HashSet::new(),
            routine_scopes: Vec::new(),
            array_symbols: HashSet::new(),
            fields: Vec::new(),
            field_lookup: HashMap::new(),
            diagnostics: Vec::new(),
            expression_observations: Vec::new(),
        }
    }

    fn seed_builtins(&mut self) {
        self.declare(
            self.builtin_scope,
            "STRING".to_string(),
            SymbolClass::Type,
            None,
            Span::new(0, 0),
        );

        let builtin_procs = [
            "Print",
            "PrintE",
            "PrintD",
            "PrintDE",
            "PrintF",
            "PrintB",
            "PrintBE",
            "PrintC",
            "PrintCE",
            "PrintBD",
            "PrintBDE",
            "PrintCD",
            "PrintCDE",
            "PrintI",
            "PrintIE",
            "PrintID",
            "PrintIDE",
            "PrintH",
            "Put",
            "PutE",
            "PutD",
            "PutDE",
            "InputS",
            "InputD",
            "InputSD",
            "InputMD",
            "Open",
            "Close",
            "Error",
            "Break",
            "XIO",
            "Note",
            "Point",
            "Poke",
            "PokeC",
            "Position",
            "Graphics",
            "DrawTo",
            "Plot",
            "SetColor",
            "Fill",
            "Sound",
            "SndRst",
            "Zero",
            "SetBlock",
            "MoveBlock",
            "SCopy",
            "SCopyS",
            "SAssign",
        ];

        for name in builtin_procs {
            self.declare(
                self.builtin_scope,
                name.to_string(),
                SymbolClass::BuiltinProc,
                None,
                Span::new(0, 0),
            );
        }

        let builtin_funcs = [
            ("GetD", FundType::Char),
            ("InputB", FundType::Byte),
            ("InputBD", FundType::Byte),
            ("InputC", FundType::Card),
            ("InputCD", FundType::Card),
            ("InputI", FundType::Int),
            ("InputID", FundType::Int),
            ("ValB", FundType::Byte),
            ("ValC", FundType::Card),
            ("ValI", FundType::Int),
            ("Locate", FundType::Byte),
            ("Rand", FundType::Byte),
            ("Paddle", FundType::Byte),
            ("PTrig", FundType::Byte),
            ("Stick", FundType::Byte),
            ("STrig", FundType::Byte),
            ("Peek", FundType::Byte),
            ("PeekC", FundType::Card),
            ("SCompare", FundType::Int),
        ];

        for (name, fund) in builtin_funcs {
            self.declare(
                self.builtin_scope,
                name.to_string(),
                SymbolClass::BuiltinFunc,
                Some(fund_value(fund)),
                Span::new(0, 0),
            );
        }

        for variable in RESIDENT_VARIABLES {
            let class = match variable.kind {
                ResidentVariableKind::Byte => SymbolClass::Var,
                ResidentVariableKind::ByteArray { .. } => SymbolClass::Array,
            };
            if let Some(symbol_id) = self.declare(
                self.builtin_scope,
                variable.name.to_string(),
                class,
                Some(fund_value(FundType::Byte)),
                Span::new(0, 0),
            ) && matches!(variable.kind, ResidentVariableKind::ByteArray { .. })
            {
                self.array_symbols.insert(symbol_id);
            }
        }

        self.seed_builtin_signatures();
    }

    fn seed_builtin_signatures(&mut self) {
        let byte = fund_value(FundType::Byte);
        let card = fund_value(FundType::Card);
        let char_ty = fund_value(FundType::Char);
        let int = fund_value(FundType::Int);
        let string_address = card.clone();
        let byte_pointer = ValueType::pointer_to(byte.clone());
        let card_pointer = ValueType::pointer_to(card.clone());

        let signatures = [
            ("Print", vec![string_address.clone()]),
            ("PrintE", vec![string_address.clone()]),
            ("PrintD", vec![byte.clone(), string_address.clone()]),
            ("PrintDE", vec![byte.clone(), string_address.clone()]),
            ("PrintB", vec![byte.clone()]),
            ("PrintBE", vec![byte.clone()]),
            ("PrintBD", vec![byte.clone(), byte.clone()]),
            ("PrintBDE", vec![byte.clone(), byte.clone()]),
            ("PrintC", vec![card.clone()]),
            ("PrintCE", vec![card.clone()]),
            ("PrintCD", vec![byte.clone(), card.clone()]),
            ("PrintCDE", vec![byte.clone(), card.clone()]),
            ("PrintI", vec![int.clone()]),
            ("PrintIE", vec![int.clone()]),
            ("PrintID", vec![byte.clone(), int.clone()]),
            ("PrintIDE", vec![byte.clone(), int.clone()]),
            ("Put", vec![char_ty.clone()]),
            ("PutE", vec![]),
            ("PutD", vec![byte.clone(), char_ty.clone()]),
            ("PutDE", vec![byte.clone()]),
            ("InputB", vec![]),
            ("InputBD", vec![byte.clone()]),
            ("InputC", vec![]),
            ("InputCD", vec![byte.clone()]),
            ("InputI", vec![]),
            ("InputID", vec![byte.clone()]),
            ("InputS", vec![string_address.clone()]),
            ("InputSD", vec![byte.clone(), string_address.clone()]),
            (
                "InputMD",
                vec![byte.clone(), string_address.clone(), byte.clone()],
            ),
            ("GetD", vec![byte.clone()]),
            ("Error", vec![byte.clone(), byte.clone(), byte.clone()]),
            ("Break", vec![]),
            (
                "Open",
                vec![
                    byte.clone(),
                    string_address.clone(),
                    byte.clone(),
                    byte.clone(),
                ],
            ),
            ("Close", vec![byte.clone()]),
            (
                "XIO",
                vec![
                    byte.clone(),
                    byte.clone(),
                    byte.clone(),
                    byte.clone(),
                    byte.clone(),
                    string_address.clone(),
                ],
            ),
            (
                "Note",
                vec![byte.clone(), card_pointer.clone(), byte_pointer.clone()],
            ),
            ("Point", vec![byte.clone(), card.clone(), byte.clone()]),
            ("Graphics", vec![byte.clone()]),
            ("SetColor", vec![byte.clone(), byte.clone(), byte.clone()]),
            ("Plot", vec![card.clone(), byte.clone()]),
            ("DrawTo", vec![card.clone(), byte.clone()]),
            ("Fill", vec![card.clone(), byte.clone()]),
            ("Position", vec![card.clone(), byte.clone()]),
            ("Locate", vec![card.clone(), byte.clone()]),
            (
                "Sound",
                vec![byte.clone(), byte.clone(), byte.clone(), byte.clone()],
            ),
            ("SndRst", vec![]),
            ("Paddle", vec![byte.clone()]),
            ("PTrig", vec![byte.clone()]),
            ("Stick", vec![byte.clone()]),
            ("STrig", vec![byte.clone()]),
            (
                "SCompare",
                vec![string_address.clone(), string_address.clone()],
            ),
            (
                "SCopy",
                vec![string_address.clone(), string_address.clone()],
            ),
            (
                "SCopyS",
                vec![
                    string_address.clone(),
                    string_address.clone(),
                    byte.clone(),
                    byte.clone(),
                ],
            ),
            (
                "SAssign",
                vec![
                    string_address.clone(),
                    string_address.clone(),
                    byte.clone(),
                    byte.clone(),
                ],
            ),
            ("ValB", vec![string_address.clone()]),
            ("ValC", vec![string_address.clone()]),
            ("ValI", vec![string_address.clone()]),
            ("Rand", vec![byte.clone()]),
            ("Peek", vec![card.clone()]),
            ("PeekC", vec![card.clone()]),
            ("Poke", vec![card.clone(), byte.clone()]),
            ("PokeC", vec![card.clone(), card.clone()]),
            ("Zero", vec![byte_pointer.clone(), card.clone()]),
            (
                "SetBlock",
                vec![byte_pointer.clone(), card.clone(), byte.clone()],
            ),
            (
                "MoveBlock",
                vec![byte_pointer.clone(), byte_pointer.clone(), card.clone()],
            ),
        ];

        for (name, params) in signatures {
            if let Some(symbol_id) = self.lookup_symbol(self.builtin_scope, name) {
                let symbol = &self.symbols.symbols[symbol_id.0];
                self.routines.insert(
                    normalize_name(name),
                    SemanticCallableSignature::from_symbol(
                        symbol,
                        params,
                        SemanticCallableSource::Resident,
                    ),
                );
            }
        }

        if let Some(symbol_id) = self.lookup_symbol(self.builtin_scope, "PrintF") {
            let symbol = &self.symbols.symbols[symbol_id.0];
            self.routines.insert(
                normalize_name("PrintF"),
                SemanticCallableSignature::from_variadic_symbol(
                    symbol,
                    vec![string_address],
                    card,
                    SemanticCallableSource::Resident,
                ),
            );
        }
    }

    fn analyze_program(&mut self, program: &Program) {
        self.retargeted_routine_names = collect_retargeted_routine_names(program);
        for module in &program.modules {
            self.analyze_module(module);
        }
    }

    fn analyze_module(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::Define(define) => self.analyze_define(self.global_scope, define),
                Item::Declaration(decl) => self.analyze_decl(self.global_scope, decl, false),
                Item::Routine(routine) => {
                    self.declare_routine(routine);
                    self.analyze_routine_body(routine);
                }
                Item::Statement(stmt) => {
                    self.analyze_stmt(self.global_scope, stmt, ControlContext::global())
                }
                Item::Include(_) | Item::Set(_) | Item::Unsupported { .. } => {}
            }
        }
    }

    fn analyze_define(&mut self, scope: ScopeId, define: &DefineDecl) {
        for entry in &define.entries {
            if define_value_contains_define_directive(&entry.value) {
                self.diagnostics.push(Diagnostic::new(
                    entry.span,
                    "nested DEFINE directives are not allowed",
                ));
            }
            self.declare(
                scope,
                entry.name.clone(),
                SymbolClass::Define,
                None,
                entry.span,
            );
        }
    }

    fn declare_routine(&mut self, routine: &Routine) {
        let class = match routine.kind {
            RoutineKind::Proc => SymbolClass::Proc,
            RoutineKind::Func { .. } => SymbolClass::Func,
        };
        let ty = match routine.kind {
            RoutineKind::Proc => None,
            RoutineKind::Func { return_type } => Some(ValueType::fund(return_type)),
        };

        if self
            .declare(
                self.global_scope,
                routine.name.clone(),
                class,
                ty,
                routine.span,
            )
            .is_some()
        {
            self.routines.insert(
                normalize_name(&routine.name),
                SemanticCallableSignature::from_routine(routine),
            );
        }
    }

    fn analyze_routine_body(&mut self, routine: &Routine) {
        let routine_scope = self
            .symbols
            .add_scope(ScopeKind::Routine, Some(self.global_scope));
        self.routine_scopes.push(RoutineScope {
            name: routine.name.clone(),
            scope: routine_scope,
        });

        for param in &routine.params {
            self.analyze_var_decl(routine_scope, param, true);
        }

        for local in &routine.locals {
            self.analyze_decl(routine_scope, local, false);
        }

        let context = ControlContext::routine(&routine.kind);
        for stmt in &routine.body {
            self.analyze_stmt(routine_scope, stmt, context);
        }
        self.validate_routine_return_paths(routine);
    }

    fn analyze_stmt(&mut self, scope: ScopeId, stmt: &Stmt, context: ControlContext<'_>) {
        match stmt {
            Stmt::Define(define) => self.analyze_define(scope, define),
            Stmt::Return(expr) => self.validate_return(scope, expr.as_ref(), context.routine_kind),
            Stmt::Exit { span } if context.loop_depth == 0 => {
                self.diagnostics
                    .push(Diagnostic::new(*span, "EXIT outside loop"));
            }
            Stmt::Assign {
                target,
                value,
                span,
            } => self.validate_assignment(scope, target, value, *span),
            Stmt::CompoundAssign {
                target,
                op,
                value,
                span,
            } => self.validate_compound_assignment(scope, target, *op, value, *span),
            Stmt::Call { expr, span } => self.validate_call_statement(scope, expr, *span),
            Stmt::MachineBlock { items, .. } => {
                for item in items {
                    if let MachineItem::Name(name) = item {
                        self.lookup_symbol(scope, name);
                    }
                }
            }
            Stmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    self.validate_condition(scope, &branch.condition);
                    self.analyze_statements(scope, &branch.body, context);
                }
                self.analyze_statements(scope, else_body, context);
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.validate_condition(scope, condition);
                self.analyze_statements(scope, body, context.inside_loop());
            }
            Stmt::DoUntil {
                body, condition, ..
            } => {
                self.analyze_statements(scope, body, context.inside_loop());
                if let Some(condition) = condition {
                    self.validate_condition(scope, condition);
                }
            }
            Stmt::For {
                target,
                start,
                end,
                step,
                body,
                span,
            } => {
                self.validate_for_operands(scope, target, start, end, step.as_ref(), *span);
                self.analyze_statements(scope, body, context.inside_loop());
            }
            Stmt::Exit { .. } | Stmt::Unsupported { .. } => {}
        }
    }

    fn analyze_statements(
        &mut self,
        scope: ScopeId,
        statements: &[Stmt],
        context: ControlContext<'_>,
    ) {
        for stmt in statements {
            self.analyze_stmt(scope, stmt, context);
        }
    }

    fn validate_condition(&mut self, scope: ScopeId, expr: &Expr) {
        let diagnostic_count = self.diagnostics.len();
        self.expect_expr(scope, expr, expr.span);
        if self.diagnostics.len() == diagnostic_count {
            self.lower_expr(scope, expr);
        }
    }

    fn validate_return(
        &mut self,
        scope: ScopeId,
        expr: Option<&Expr>,
        routine_kind: Option<&RoutineKind>,
    ) {
        let typed = expr.map(|expr| self.lower_expr(scope, expr));

        match (routine_kind, expr, typed.as_ref()) {
            (Some(RoutineKind::Proc), Some(expr), _) => self.diagnostics.push(Diagnostic::new(
                expr.span,
                "procedure RETURN cannot include a value",
            )),
            (Some(RoutineKind::Func { .. }), None, _) => self.diagnostics.push(Diagnostic::new(
                Span::new(0, 0),
                "function RETURN requires a value",
            )),
            (Some(RoutineKind::Func { return_type }), Some(expr), Some(typed)) => {
                let expected = fund_value(*return_type);
                if !typed.ty.is_error() && !type_can_assign(&expected, &typed.ty) {
                    self.diagnostics.push(Diagnostic::new(
                        expr.span,
                        format!(
                            "cannot return {:?} from {:?} function",
                            typed.ty, return_type
                        ),
                    ));
                }
            }
            _ => {}
        }
    }

    fn validate_routine_return_paths(&mut self, routine: &Routine) {
        if !matches!(routine.kind, RoutineKind::Func { .. }) {
            return;
        }
        if routine.body.is_empty()
            && self
                .retargeted_routine_names
                .contains(&normalize_name(&routine.name))
        {
            return;
        }
        let flow = routine_control_flow_facts(routine);
        if flow.may_fall_through {
            self.diagnostics.push(Diagnostic::new(
                routine.span,
                format!("function `{}` may exit without RETURN value", routine.name),
            ));
        }
    }

    fn validate_assignment(
        &mut self,
        scope: ScopeId,
        target: &Expr,
        value_expr: &Expr,
        span: Span,
    ) {
        let target_place = self.expect_place(scope, target, span);
        let value = self.lower_expr(scope, value_expr);
        if matches!(
            target_place.access,
            subject::PlaceAccess::ReadOnly | subject::PlaceAccess::Error
        ) {
            return;
        }

        let (expected, actual) = (&target_place.ty, &value.ty);
        if expected.is_error() || actual.is_error() {
            return;
        }
        if !expected.pointer {
            return;
        }
        if !self.pointer_target_accepts_expr(scope, expected, value_expr, actual) {
            self.diagnostics.push(Diagnostic::new(
                value.span,
                format!("cannot assign {:?} to {:?}", actual, expected),
            ));
        }
    }

    fn validate_compound_assignment(
        &mut self,
        scope: ScopeId,
        target: &Expr,
        op: BinaryOp,
        value_expr: &Expr,
        span: Span,
    ) {
        let target_place = self.expect_place(scope, target, span);
        let value = self.lower_expr(scope, value_expr);
        if !matches!(target_place.access, subject::PlaceAccess::Assignable) {
            if !matches!(target_place.access, subject::PlaceAccess::Error) {
                self.diagnostics.push(Diagnostic::new(
                    span,
                    "compound assignment target must be assignable",
                ));
            }
            return;
        }

        let Some(target_ty) = self.compound_assignment_target_type(scope, target, &target_place)
        else {
            return;
        };
        let value_ty = &value.ty;
        if value_ty.is_error() {
            return;
        }

        if target_ty.is_pointer() {
            if !matches!(op, BinaryOp::Add | BinaryOp::Sub) {
                self.diagnostics.push(Diagnostic::new(
                    span,
                    "pointer compound assignment only supports + or -",
                ));
            }
            if !is_numeric_scalar(value_ty) {
                self.diagnostics.push(Diagnostic::new(
                    value_expr.span,
                    "pointer compound assignment value must be numeric",
                ));
            }
            return;
        }

        if !is_numeric_scalar(&target_ty) {
            self.diagnostics.push(Diagnostic::new(
                span,
                "compound assignment target must be numeric or pointer",
            ));
            return;
        }
        if !is_numeric_scalar(value_ty) {
            self.diagnostics.push(Diagnostic::new(
                value_expr.span,
                "compound assignment value must be numeric",
            ));
        }
    }

    fn compound_assignment_target_type(
        &self,
        scope: ScopeId,
        target: &Expr,
        target_place: &subject::SemPlace,
    ) -> Option<ValueType> {
        if let Some(symbol_id) = self.symbol_id_for_place_expr(scope, target)
            && self.is_array_symbol(symbol_id)
        {
            return self
                .array_element_type(symbol_id)
                .map(ValueType::pointer_to);
        }

        if target_place.ty.is_error() {
            None
        } else {
            Some(target_place.ty.clone())
        }
    }

    fn validate_for_operands(
        &mut self,
        scope: ScopeId,
        target: &Expr,
        start: &Expr,
        end: &Expr,
        step: Option<&Expr>,
        span: Span,
    ) {
        let target_place = self.expect_place(scope, target, span);
        let diagnostic_count = self.diagnostics.len();
        let start_expr = self.expect_expr(scope, start, start.span);
        if self.diagnostics.len() == diagnostic_count {
            self.lower_expr(scope, start);
        }
        let diagnostic_count = self.diagnostics.len();
        let end_expr = self.expect_expr(scope, end, end.span);
        if self.diagnostics.len() == diagnostic_count {
            self.lower_expr(scope, end);
        }
        let step_expr = step.map(|step| {
            let diagnostic_count = self.diagnostics.len();
            let expr = self.expect_expr(scope, step, step.span);
            if self.diagnostics.len() == diagnostic_count {
                self.lower_expr(scope, step);
            }
            expr
        });

        if !matches!(target_place.access, subject::PlaceAccess::Assignable) {
            if !matches!(target_place.access, subject::PlaceAccess::Error) {
                self.diagnostics
                    .push(Diagnostic::new(span, "FOR target must be assignable"));
            }
            return;
        }

        let target_ty = &target_place.ty;
        if target_ty.is_error() {
            return;
        }

        if !is_numeric_scalar(target_ty) {
            self.diagnostics
                .push(Diagnostic::new(span, "FOR target must be a numeric scalar"));
            return;
        }

        for (expr, typed, role) in [
            (start, &start_expr, "start value"),
            (end, &end_expr, "end value"),
        ] {
            if is_action_reference_expr(expr) {
                continue;
            }
            if !typed.ty.is_error() && !is_numeric_scalar(&typed.ty) {
                self.diagnostics.push(Diagnostic::new(
                    expr.span,
                    format!("FOR {role} must be numeric"),
                ));
            }
        }

        if let (Some(step), Some(typed)) = (step, step_expr.as_ref())
            && !typed.ty.is_error()
            && !is_numeric_scalar(&typed.ty)
        {
            self.diagnostics
                .push(Diagnostic::new(step.span, "FOR STEP must be numeric"));
        }
    }

    fn classify_subject(&mut self, scope: ScopeId, expr: &Expr) -> subject::SemSubject {
        match &expr.kind {
            ExprKind::Missing => self.subject_error(expr.span),
            ExprKind::Raw => subject::SemSubject::Expr(subject::SemExpr {
                ty: ValueType::error(),
                kind: subject::SemExprKind::Raw(expr.text.clone()),
                span: expr.span,
            }),
            ExprKind::CurrentLocation => subject::SemSubject::Expr(subject::SemExpr {
                ty: fund_value(FundType::Card),
                kind: subject::SemExprKind::CurrentLocation,
                span: expr.span,
            }),
            ExprKind::Number(number) => subject::SemSubject::Expr(subject::SemExpr {
                ty: ScalarType::from_number_kind(number.kind)
                    .map(ValueType::scalar)
                    .unwrap_or_else(ValueType::error),
                kind: subject::SemExprKind::Literal(subject::SemLiteral::Number(number.clone())),
                span: expr.span,
            }),
            ExprKind::String(value) => subject::SemSubject::Expr(subject::SemExpr {
                ty: string_literal_type(),
                kind: subject::SemExprKind::Literal(subject::SemLiteral::String(value.clone())),
                span: expr.span,
            }),
            ExprKind::Char(value) => subject::SemSubject::Expr(subject::SemExpr {
                ty: fund_value(FundType::Char),
                kind: subject::SemExprKind::Literal(subject::SemLiteral::Char(*value)),
                span: expr.span,
            }),
            ExprKind::Name(name) => self.classify_name_subject(scope, name, expr.span),
            ExprKind::Cast { ty, expr: inner } => {
                let inner = self.expect_expr(scope, inner, expr.span);
                let ty = ValueType::from_type_ref(ty);
                subject::SemSubject::Expr(subject::SemExpr {
                    ty: ty.clone(),
                    kind: subject::SemExprKind::Cast {
                        ty,
                        expr: Box::new(inner),
                    },
                    span: expr.span,
                })
            }
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr: inner,
            } => match self.classify_subject(scope, inner) {
                subject::SemSubject::Place(place) => {
                    let ty = ValueType::pointer_to(place.ty.clone());
                    subject::SemSubject::Expr(subject::SemExpr {
                        ty,
                        kind: subject::SemExprKind::AddressOf(Box::new(place)),
                        span: expr.span,
                    })
                }
                subject::SemSubject::Callable(callable) => match callable.kind {
                    subject::SemCallableKind::Function(symbol_id)
                    | subject::SemCallableKind::Builtin(symbol_id) => {
                        let ty = ValueType::callable_pointer(callable.ty);
                        subject::SemSubject::Expr(subject::SemExpr {
                            ty,
                            kind: subject::SemExprKind::AddressOfSymbol(symbol_id),
                            span: expr.span,
                        })
                    }
                    subject::SemCallableKind::FunctionValue(_)
                    | subject::SemCallableKind::Error => {
                        self.diagnostics.push(Diagnostic::new(
                            expr.span,
                            "cannot take address of this expression",
                        ));
                        subject::SemSubject::Expr(self.error_expr(expr.span))
                    }
                },
                subject::SemSubject::Expr(_)
                | subject::SemSubject::TypeRef(_)
                | subject::SemSubject::Define(_) => {
                    self.diagnostics.push(Diagnostic::new(
                        expr.span,
                        "cannot take address of this expression",
                    ));
                    subject::SemSubject::Expr(self.error_expr(expr.span))
                }
                subject::SemSubject::Error(_) => {
                    subject::SemSubject::Expr(self.error_expr(expr.span))
                }
            },
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: pointer,
            } => {
                let pointer_expr = pointer;
                let mut pointer = self.expect_expr(scope, pointer_expr, expr.span);
                if pointer.ty.as_pointer().is_none()
                    && let Some(decayed) = self.array_decay_pointer_type(scope, pointer_expr)
                {
                    pointer.ty = decayed;
                }
                let pointer_type = pointer.ty.as_pointer();
                let ty = pointer_type
                    .as_ref()
                    .map(|ty| (*ty.pointee).clone())
                    .unwrap_or_else(ValueType::error);
                if !pointer.ty.is_error() && pointer_type.is_none() {
                    self.diagnostics.push(Diagnostic::new(
                        expr.span,
                        "cannot dereference non-pointer value",
                    ));
                }
                subject::SemSubject::Place(subject::SemPlace {
                    ty,
                    access: if pointer_type.is_some() {
                        subject::PlaceAccess::Assignable
                    } else {
                        subject::PlaceAccess::Error
                    },
                    kind: subject::SemPlaceKind::Deref(Box::new(pointer)),
                    span: expr.span,
                })
            }
            ExprKind::Unary { op, expr: inner } => {
                let inner = self.expect_expr(scope, inner, expr.span);
                let ty = inner.ty.clone();
                subject::SemSubject::Expr(subject::SemExpr {
                    ty,
                    kind: subject::SemExprKind::Unary {
                        op: *op,
                        expr: Box::new(inner),
                    },
                    span: expr.span,
                })
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.expect_expr(scope, left, expr.span);
                let right = self.expect_expr(scope, right, expr.span);
                let ty = if is_condition_op(*op) {
                    fund_value(FundType::Byte)
                } else {
                    promote_numeric(&left.ty, &right.ty)
                };
                subject::SemSubject::Expr(subject::SemExpr {
                    ty,
                    kind: subject::SemExprKind::Binary {
                        op: *op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span: expr.span,
                })
            }
            ExprKind::Call { callee, args }
                if args.len() == 1 && self.can_subject_be_indexed(scope, callee) =>
            {
                let base = self.expect_place(scope, callee, callee.span);
                let index = self.expect_expr(scope, &args[0], args[0].span);
                let ty = self.indexed_place_type_or_diagnostic(&base, &index, expr.span);
                subject::SemSubject::Place(subject::SemPlace {
                    ty,
                    access: subject::PlaceAccess::Assignable,
                    kind: subject::SemPlaceKind::Index {
                        base: Box::new(base),
                        index: Box::new(index),
                    },
                    span: expr.span,
                })
            }
            ExprKind::Call { callee, args } => {
                let callee = self.expect_callable(scope, callee, expr.span);
                let sem_args = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        self.expect_argument_expr(
                            scope,
                            arg,
                            diagnostic_span(arg.span, expr.span),
                            callee.ty.params.get(index),
                        )
                    })
                    .collect();
                self.validate_callable_args(scope, &callee, args, expr.span);
                let ty = self.call_value_type_or_error(&callee, expr.span);
                subject::SemSubject::Expr(subject::SemExpr {
                    ty,
                    kind: subject::SemExprKind::Call {
                        callee: Box::new(callee),
                        args: sem_args,
                    },
                    span: expr.span,
                })
            }
            ExprKind::Index { base, index } => {
                let base = self.expect_place(scope, base, base.span);
                let index = self.expect_expr(scope, index, index.span);
                let ty = self.indexed_place_type_or_diagnostic(&base, &index, expr.span);
                subject::SemSubject::Place(subject::SemPlace {
                    ty,
                    access: subject::PlaceAccess::Assignable,
                    kind: subject::SemPlaceKind::Index {
                        base: Box::new(base),
                        index: Box::new(index),
                    },
                    span: expr.span,
                })
            }
            ExprKind::Field { base, field } => {
                let base = self.expect_place(scope, base, base.span);
                let descriptor =
                    self.record_field_facts_or_diagnostic(typed_ref(&base.ty), field, expr.span);
                let ty = descriptor
                    .as_ref()
                    .map(|field| field.ty.clone())
                    .unwrap_or_else(ValueType::error);
                subject::SemSubject::Place(subject::SemPlace {
                    ty: ty.clone(),
                    access: base.access,
                    kind: subject::SemPlaceKind::Field {
                        base: Box::new(base),
                        field: subject::SemFieldRef {
                            id: descriptor.as_ref().map(|field| field.id),
                            owner: descriptor.as_ref().map(|field| field.owner),
                            name: field.clone(),
                            ty,
                            offset: descriptor.as_ref().map(|field| field.offset),
                            span: expr.span,
                        },
                    },
                    span: expr.span,
                })
            }
        }
    }

    fn classify_name_subject(
        &mut self,
        scope: ScopeId,
        name: &str,
        span: Span,
    ) -> subject::SemSubject {
        let Some(symbol_id) = self.lookup_symbol(scope, name) else {
            self.diagnostics
                .push(Diagnostic::new(span, format!("undefined symbol `{name}`")));
            return self.subject_error(span);
        };
        let symbol = &self.symbols.symbols[symbol_id.0];

        match symbol.class {
            SymbolClass::Var | SymbolClass::Array | SymbolClass::Param => {
                subject::SemSubject::Place(subject::SemPlace {
                    ty: symbol.ty.clone().unwrap_or_else(ValueType::error),
                    access: subject::PlaceAccess::Assignable,
                    kind: subject::SemPlaceKind::Symbol(symbol_id),
                    span,
                })
            }
            SymbolClass::Proc
            | SymbolClass::Func
            | SymbolClass::BuiltinProc
            | SymbolClass::BuiltinFunc => {
                subject::SemSubject::Callable(self.callable_subject(symbol_id, span))
            }
            SymbolClass::Type | SymbolClass::Record => {
                subject::SemSubject::TypeRef(subject::SemTypeRef {
                    ty: symbol.ty.clone().unwrap_or_else(ValueType::error),
                    kind: subject::SemTypeRefKind::Symbol(symbol_id),
                    span,
                })
            }
            SymbolClass::Define => subject::SemSubject::Define(subject::SemDefineRef {
                symbol: symbol_id,
                span,
            }),
        }
    }

    fn callable_subject(&self, symbol_id: SymbolId, span: Span) -> subject::SemCallable {
        let symbol = &self.symbols.symbols[symbol_id.0];
        let signature = self.routines.get(&normalize_name(&symbol.name));
        let return_type = signature
            .and_then(|signature| signature.return_type.clone())
            .or_else(|| symbol.ty.clone());
        let kind = signature
            .map(|signature| signature.kind.clone())
            .unwrap_or_else(|| callable_kind_from_symbol(symbol));
        let params = signature
            .map(|signature| signature.params.clone())
            .unwrap_or_default();
        let variadic = signature.and_then(|signature| signature.variadic.clone());
        let callable_kind = match symbol.class {
            SymbolClass::BuiltinProc | SymbolClass::BuiltinFunc => {
                subject::SemCallableKind::Builtin(symbol_id)
            }
            _ => subject::SemCallableKind::Function(symbol_id),
        };

        subject::SemCallable {
            ty: callable_type_from_parts(kind, params, variadic, return_type),
            kind: callable_kind,
            span,
        }
    }

    fn expect_expr(&mut self, scope: ScopeId, expr: &Expr, span: Span) -> subject::SemExpr {
        match self.classify_subject(scope, expr) {
            subject::SemSubject::Expr(expr) => expr,
            subject::SemSubject::Place(place) => subject::SemExpr {
                ty: place.ty.clone(),
                kind: subject::SemExprKind::Load(Box::new(place)),
                span: expr.span,
            },
            subject::SemSubject::Define(_) => subject::SemExpr {
                ty: ValueType::error(),
                kind: subject::SemExprKind::Raw(expr.text.clone()),
                span: expr.span,
            },
            subject::SemSubject::Callable(_) | subject::SemSubject::TypeRef(_) => {
                self.diagnostics
                    .push(Diagnostic::new(span, "expected value expression"));
                self.error_expr(expr.span)
            }
            subject::SemSubject::Error(_) => self.error_expr(expr.span),
        }
    }

    fn expect_argument_expr(
        &mut self,
        scope: ScopeId,
        expr: &Expr,
        span: Span,
        expected: Option<&ValueType>,
    ) -> subject::SemExpr {
        let subject = self.classify_subject(scope, expr);
        if let (Some(expected), subject::SemSubject::Callable(callable)) = (expected, &subject)
            && routine_address_can_pass_as(expected)
        {
            return match &callable.kind {
                subject::SemCallableKind::Function(symbol_id)
                | subject::SemCallableKind::Builtin(symbol_id) => subject::SemExpr {
                    ty: routine_address_value_type(expected),
                    kind: subject::SemExprKind::AddressOfSymbol(*symbol_id),
                    span: expr.span,
                },
                subject::SemCallableKind::FunctionValue(value) => (**value).clone(),
                subject::SemCallableKind::Error => self.error_expr(expr.span),
            };
        }

        match subject {
            subject::SemSubject::Expr(expr) => expr,
            subject::SemSubject::Place(place) => subject::SemExpr {
                ty: place.ty.clone(),
                kind: subject::SemExprKind::Load(Box::new(place)),
                span: expr.span,
            },
            subject::SemSubject::Define(_) => subject::SemExpr {
                ty: ValueType::error(),
                kind: subject::SemExprKind::Raw(expr.text.clone()),
                span: expr.span,
            },
            subject::SemSubject::Callable(_) | subject::SemSubject::TypeRef(_) => {
                self.diagnostics
                    .push(Diagnostic::new(span, "expected value expression"));
                self.error_expr(expr.span)
            }
            subject::SemSubject::Error(_) => self.error_expr(expr.span),
        }
    }

    fn expect_place(&mut self, scope: ScopeId, expr: &Expr, span: Span) -> subject::SemPlace {
        match self.classify_subject(scope, expr) {
            subject::SemSubject::Place(place) => place,
            subject::SemSubject::Callable(callable) => subject::SemPlace {
                ty: ValueType::callable_pointer(callable.ty),
                access: subject::PlaceAccess::RoutineTargetOnly,
                kind: subject::SemPlaceKind::Error,
                span: expr.span,
            },
            subject::SemSubject::Expr(_)
            | subject::SemSubject::TypeRef(_)
            | subject::SemSubject::Define(_) => {
                self.diagnostics
                    .push(Diagnostic::new(span, "invalid assignment target"));
                self.error_place(expr.span)
            }
            subject::SemSubject::Error(_) => self.error_place(expr.span),
        }
    }

    fn expect_callable(&mut self, scope: ScopeId, expr: &Expr, span: Span) -> subject::SemCallable {
        match self.classify_subject(scope, expr) {
            subject::SemSubject::Callable(callable) => callable,
            subject::SemSubject::Place(place) => {
                if let Some(callable) = place.ty.as_callable_pointer().cloned() {
                    subject::SemCallable {
                        ty: callable,
                        kind: subject::SemCallableKind::FunctionValue(Box::new(subject::SemExpr {
                            ty: place.ty.clone(),
                            kind: subject::SemExprKind::Load(Box::new(place)),
                            span: expr.span,
                        })),
                        span: expr.span,
                    }
                } else {
                    let message = match &expr.kind {
                        ExprKind::Name(name) => format!("`{name}` is not callable"),
                        _ => "invalid call target".to_string(),
                    };
                    self.diagnostics.push(Diagnostic::new(span, message));
                    self.error_callable(expr.span)
                }
            }
            subject::SemSubject::Error(_) => self.error_callable(expr.span),
            _ => {
                let message = match &expr.kind {
                    ExprKind::Name(name) => format!("`{name}` is not callable"),
                    _ => "invalid call target".to_string(),
                };
                self.diagnostics.push(Diagnostic::new(span, message));
                self.error_callable(expr.span)
            }
        }
    }

    fn can_subject_be_indexed(&mut self, scope: ScopeId, expr: &Expr) -> bool {
        match self.classify_subject(scope, expr) {
            subject::SemSubject::Place(place) => self.place_can_be_indexed(&place),
            subject::SemSubject::Expr(expr) => expr.ty.is_pointer(),
            _ => false,
        }
    }

    fn place_can_be_indexed(&self, place: &subject::SemPlace) -> bool {
        if place.ty.is_pointer() {
            return true;
        }

        matches!(
            &place.kind,
            subject::SemPlaceKind::Symbol(symbol_id) if self.is_array_symbol(*symbol_id)
        )
    }

    fn indexed_place_type_or_diagnostic(
        &mut self,
        base: &subject::SemPlace,
        index: &subject::SemExpr,
        span: Span,
    ) -> ValueType {
        self.validate_index_type(&index.ty, index.span);
        if base.ty.is_pointer() {
            return indexed_value_type(Some(&base.ty)).unwrap_or_else(ValueType::error);
        }
        if let subject::SemPlaceKind::Symbol(symbol_id) = &base.kind
            && self.is_array_symbol(*symbol_id)
        {
            return self
                .array_element_type(*symbol_id)
                .unwrap_or_else(ValueType::error);
        }
        if !base.ty.is_error() {
            self.diagnostics.push(Diagnostic::new(
                span,
                "indexing requires an array or pointer",
            ));
        }
        ValueType::error()
    }

    fn validate_index_type(&mut self, ty: &ValueType, span: Span) {
        if !ty.is_error() && !is_numeric_scalar(ty) {
            self.diagnostics
                .push(Diagnostic::new(span, "index must be numeric"));
        }
    }

    fn subject_error(&self, span: Span) -> subject::SemSubject {
        subject::SemSubject::Error(subject::SemErrorSubject { span })
    }

    fn error_expr(&self, span: Span) -> subject::SemExpr {
        subject::SemExpr {
            ty: ValueType::error(),
            kind: subject::SemExprKind::Error,
            span,
        }
    }

    fn error_place(&self, span: Span) -> subject::SemPlace {
        subject::SemPlace {
            ty: ValueType::error(),
            access: subject::PlaceAccess::Error,
            kind: subject::SemPlaceKind::Error,
            span,
        }
    }

    fn error_callable(&self, span: Span) -> subject::SemCallable {
        subject::SemCallable {
            ty: CallableType::unknown_proc(),
            kind: subject::SemCallableKind::Error,
            span,
        }
    }

    fn validate_call_statement(&mut self, scope: ScopeId, expr: &Expr, span: Span) {
        let ExprKind::Call { callee, args } = &expr.kind else {
            self.lower_expr(scope, expr);
            return;
        };

        self.validate_call(scope, callee, args, span);
    }

    fn lower_expr(&mut self, scope: ScopeId, expr: &Expr) -> subject::SemExpr {
        let subject = self.classify_subject(scope, expr);
        self.record_subject_types(expr, &subject);
        self.expr_from_subject(expr, &subject)
    }

    fn observe_expr(
        &self,
        span: Span,
        class: ExprClass,
        ty: Option<ValueType>,
    ) -> ExpressionObservation {
        ExpressionObservation { span, class, ty }
    }

    fn expr_from_subject(&self, expr: &Expr, subject: &subject::SemSubject) -> subject::SemExpr {
        match subject {
            subject::SemSubject::Expr(expr_subject) => expr_subject.clone(),
            subject::SemSubject::Place(place) => {
                if matches!(
                    expr.kind,
                    ExprKind::Unary {
                        op: UnaryOp::Deref,
                        ..
                    }
                ) {
                    subject::SemExpr {
                        ty: place.ty.clone(),
                        kind: subject::SemExprKind::Load(Box::new(place.clone())),
                        span: place.span,
                    }
                } else {
                    subject::SemExpr {
                        ty: place.ty.clone(),
                        kind: subject::SemExprKind::Load(Box::new(place.clone())),
                        span: expr.span,
                    }
                }
            }
            subject::SemSubject::Define(_) => subject::SemExpr {
                ty: ValueType::error(),
                kind: subject::SemExprKind::Raw(expr.text.clone()),
                span: expr.span,
            },
            subject::SemSubject::Callable(_)
            | subject::SemSubject::TypeRef(_)
            | subject::SemSubject::Error(_) => self.error_expr(expr.span),
        }
    }

    fn record_subject_types(&mut self, expr: &Expr, subject: &subject::SemSubject) {
        match subject {
            subject::SemSubject::Expr(expr_subject) => self.record_sem_expr(expr_subject),
            subject::SemSubject::Place(place) => {
                let class = if matches!(
                    expr.kind,
                    ExprKind::Unary {
                        op: UnaryOp::Deref,
                        ..
                    }
                ) {
                    ExprClass::Value
                } else {
                    ExprClass::LValue
                };
                self.record_sem_place_with_class(place, class);
            }
            subject::SemSubject::Callable(callable) => self.record_sem_callable(callable),
            subject::SemSubject::TypeRef(type_ref) => self.expression_observations.push(
                self.observe_expr(type_ref.span, ExprClass::Unknown, Some(type_ref.ty.clone())),
            ),
            subject::SemSubject::Define(define_ref) => self
                .expression_observations
                .push(self.observe_expr(define_ref.span, ExprClass::Unknown, None)),
            subject::SemSubject::Error(error) => self
                .expression_observations
                .push(self.observe_expr(error.span, ExprClass::Unknown, Some(ValueType::error()))),
        }
    }

    fn record_sem_expr(&mut self, expr: &subject::SemExpr) {
        match &expr.kind {
            subject::SemExprKind::Load(place) => {
                self.record_sem_place_with_class(place, ExprClass::LValue);
                return;
            }
            subject::SemExprKind::AddressOf(place) => {
                self.record_sem_place_with_class(place, ExprClass::LValue);
            }
            subject::SemExprKind::AddressOfSymbol(_) => {}
            subject::SemExprKind::Cast { expr, .. } => self.record_sem_expr(expr),
            subject::SemExprKind::Unary { expr, .. } => self.record_sem_expr(expr),
            subject::SemExprKind::Binary { left, right, .. } => {
                self.record_sem_expr(left);
                self.record_sem_expr(right);
            }
            subject::SemExprKind::Call { callee, args } => {
                self.record_sem_callable(callee);
                for arg in args {
                    self.record_sem_expr(arg);
                }
            }
            subject::SemExprKind::Literal(_)
            | subject::SemExprKind::CurrentLocation
            | subject::SemExprKind::Raw(_)
            | subject::SemExprKind::Error => {}
        }

        self.expression_observations.push(self.typed_sem_expr(expr));
    }

    fn record_sem_place_with_class(&mut self, place: &subject::SemPlace, class: ExprClass) {
        match &place.kind {
            subject::SemPlaceKind::Field { base, .. } => {
                self.record_sem_place_with_class(base, ExprClass::LValue);
            }
            subject::SemPlaceKind::Index { base, index } => {
                self.record_sem_place_with_class(base, ExprClass::LValue);
                self.record_sem_expr(index);
            }
            subject::SemPlaceKind::Deref(pointer) => self.record_sem_expr(pointer),
            subject::SemPlaceKind::Symbol(_) | subject::SemPlaceKind::Error => {}
        }

        self.expression_observations.push(self.observe_expr(
            place.span,
            class,
            Some(place.ty.clone()),
        ));
    }

    fn record_sem_callable(&mut self, callable: &subject::SemCallable) {
        if let subject::SemCallableKind::FunctionValue(expr) = &callable.kind {
            self.record_sem_expr(expr);
        }
        self.expression_observations.push(self.observe_expr(
            callable.span,
            ExprClass::Callable,
            callable.ty.return_type.clone(),
        ));
    }

    fn typed_sem_expr(&self, expr: &subject::SemExpr) -> ExpressionObservation {
        self.observe_expr(expr.span, sem_expr_class(expr), Some(expr.ty.clone()))
    }

    fn validate_call(&mut self, scope: ScopeId, callee: &Expr, args: &[Expr], span: Span) {
        for arg in args {
            self.lower_expr(scope, arg);
        }

        if args.len() == 1 && self.can_subject_be_indexed(scope, callee) {
            return;
        }

        let callable = self.expect_callable(scope, callee, span);
        self.validate_callable_args(scope, &callable, args, span);
    }

    fn call_value_type_or_error(
        &mut self,
        callable: &subject::SemCallable,
        span: Span,
    ) -> ValueType {
        if matches!(callable.kind, subject::SemCallableKind::Error) {
            return ValueType::error();
        }
        if callable.ty.return_type.is_none() {
            self.diagnostics.push(Diagnostic::new(
                span,
                "procedure call cannot be used as a value",
            ));
            return ValueType::error();
        }

        callable
            .ty
            .return_type
            .clone()
            .unwrap_or_else(ValueType::error)
    }

    fn validate_callable_args(
        &mut self,
        scope: ScopeId,
        callable: &subject::SemCallable,
        args: &[Expr],
        span: Span,
    ) {
        let symbol_id = match &callable.kind {
            subject::SemCallableKind::Function(symbol_id)
            | subject::SemCallableKind::Builtin(symbol_id) => *symbol_id,
            subject::SemCallableKind::Error => return,
            _ => {
                self.validate_signature_args(scope, "<function pointer>", &callable.ty, args, span);
                return;
            }
        };
        let name = self.symbols.symbols[symbol_id.0].name.clone();
        self.validate_user_call_args(scope, &name, args, span);
    }

    fn validate_user_call_args(&mut self, scope: ScopeId, name: &str, args: &[Expr], span: Span) {
        let Some(signature) = self.routines.get(&normalize_name(name)).cloned() else {
            return;
        };

        if signature.variadic.is_none() && args.len() > signature.params.len() {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!(
                    "`{name}` expects at most {} argument(s), got {}",
                    signature.params.len(),
                    args.len()
                ),
            ));
            return;
        }

        for (index, arg) in args.iter().enumerate() {
            let Some(expected) = signature.params.get(index).or(signature.variadic.as_ref()) else {
                continue;
            };
            let actual = self.lower_expr(scope, arg);
            if !actual.ty.is_error()
                && !self.type_can_pass_arg_expr(scope, expected, arg, &actual.ty)
            {
                self.diagnostics.push(Diagnostic::new(
                    diagnostic_span(arg.span, span),
                    format!(
                        "`{name}` argument {} expects {:?}, got {:?}",
                        index + 1,
                        expected,
                        actual.ty
                    ),
                ));
            }
        }
    }

    fn validate_signature_args(
        &mut self,
        scope: ScopeId,
        name: &str,
        signature: &CallableType,
        args: &[Expr],
        span: Span,
    ) {
        if signature.variadic.is_none() && args.len() > signature.params.len() {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!(
                    "`{name}` expects at most {} argument(s), got {}",
                    signature.params.len(),
                    args.len()
                ),
            ));
            return;
        }

        for (index, arg) in args.iter().enumerate() {
            let Some(expected) = signature.params.get(index).or(signature.variadic.as_ref()) else {
                continue;
            };
            let actual = self.lower_expr(scope, arg);
            if !actual.ty.is_error()
                && !self.type_can_pass_arg_expr(scope, expected, arg, &actual.ty)
            {
                self.diagnostics.push(Diagnostic::new(
                    diagnostic_span(arg.span, span),
                    format!(
                        "`{name}` argument {} expects {:?}, got {:?}",
                        index + 1,
                        expected,
                        actual.ty
                    ),
                ));
            }
        }
    }

    fn type_can_pass_arg_expr(
        &self,
        scope: ScopeId,
        expected: &ValueType,
        arg: &Expr,
        actual: &ValueType,
    ) -> bool {
        if expected.pointer {
            return self.pointer_target_accepts_expr(scope, expected, arg, actual);
        }

        type_can_pass_arg(expected, actual)
    }

    fn pointer_target_accepts_expr(
        &self,
        scope: ScopeId,
        expected: &ValueType,
        expr: &Expr,
        actual: &ValueType,
    ) -> bool {
        debug_assert!(expected.pointer);
        if actual.pointer {
            return pointer_value_types_compatible(expected, actual);
        }
        if let Some(actual) = self.array_decay_pointer_type(scope, expr) {
            return array_decay_pointer_types_compatible(expected, &actual);
        }
        if self.routine_address_expr_type(scope, expr).is_some() {
            return true;
        }
        if actual.base == ValueTypeBase::Fund(FundType::Card) {
            return true;
        }
        if expected.is_error() {
            return true;
        }
        if !expected.same_record_family(actual) {
            return false;
        }

        if expected.as_record_identity().is_some() {
            self.expr_is_addressable_place(scope, expr)
        } else {
            false
        }
    }

    fn array_decay_pointer_type(&self, scope: ScopeId, expr: &Expr) -> Option<ValueType> {
        let symbol_id = self.symbol_id_for_place_expr(scope, expr)?;
        self.array_element_type(symbol_id)
            .map(ValueType::pointer_to)
    }

    fn routine_address_expr_type(&self, scope: ScopeId, expr: &Expr) -> Option<ValueType> {
        let ExprKind::Name(name) = &expr.kind else {
            return None;
        };
        let symbol_id = self.lookup_symbol(scope, name)?;
        let symbol = &self.symbols.symbols[symbol_id.0];
        matches!(
            symbol.class,
            SymbolClass::Proc
                | SymbolClass::Func
                | SymbolClass::BuiltinProc
                | SymbolClass::BuiltinFunc
        )
        .then(|| fund_value(FundType::Card))
    }

    fn array_element_type(&self, symbol_id: SymbolId) -> Option<ValueType> {
        if self.is_array_symbol(symbol_id) {
            self.symbols.symbols[symbol_id.0].ty.clone()
        } else {
            None
        }
    }

    fn is_array_symbol(&self, symbol_id: SymbolId) -> bool {
        self.array_symbols.contains(&symbol_id)
    }

    fn expr_is_addressable_place(&self, scope: ScopeId, expr: &Expr) -> bool {
        self.symbol_id_for_place_expr(scope, expr)
            .map(|symbol_id| &self.symbols.symbols[symbol_id.0])
            .is_some_and(|symbol| {
                matches!(
                    symbol.class,
                    SymbolClass::Var | SymbolClass::Array | SymbolClass::Param
                )
            })
    }

    fn symbol_id_for_place_expr(&self, scope: ScopeId, expr: &Expr) -> Option<SymbolId> {
        match &expr.kind {
            ExprKind::Name(name) => self.lookup_symbol(scope, name),
            _ => None,
        }
    }

    fn lookup_symbol(&self, scope: ScopeId, name: &str) -> Option<SymbolId> {
        self.symbols.lookup(scope, name)
    }

    fn analyze_decl(&mut self, scope: ScopeId, decl: &Decl, is_param: bool) {
        match decl {
            Decl::Var(var) => self.analyze_var_decl(scope, var, is_param),
            Decl::Type(type_decl) => {
                if let Some(owner) = self.declare(
                    scope,
                    type_decl.name.clone(),
                    SymbolClass::Type,
                    None,
                    type_decl.span,
                ) {
                    self.remember_record_fields(owner, &type_decl.name, &type_decl.fields);
                }
                for field in &type_decl.fields {
                    self.validate_record_field_decl(scope, field);
                }
            }
            Decl::Record(record_decl) => {
                if let Some(owner) = self.declare(
                    scope,
                    record_decl.name.clone(),
                    SymbolClass::Record,
                    None,
                    record_decl.span,
                ) {
                    self.remember_record_fields(owner, &record_decl.name, &record_decl.fields);
                }
                for field in &record_decl.fields {
                    self.validate_record_field_decl(scope, field);
                }
            }
        }
    }

    fn remember_record_fields(&mut self, owner: SymbolId, name: &str, fields: &[VarDecl]) {
        let mut field_ids = HashMap::new();
        let mut offset = 0u16;
        for field in fields {
            let ty = ValueType::from_type_ref(&field.ty);
            let width = self.value_storage_width(&ty).unwrap_or(0);
            for entry in &field.entries {
                let id = FieldId(self.fields.len());
                self.fields.push(SemanticField {
                    id,
                    owner,
                    name: entry.name.clone(),
                    ty: ty.clone(),
                    offset,
                    span: entry.span,
                });
                field_ids.insert(normalize_name(&entry.name), id);
                offset = offset.saturating_add(width);
            }
        }
        self.field_lookup.insert(normalize_name(name), field_ids);
    }

    fn record_field_descriptor(&self, base: &ValueType, field: &str) -> Option<&SemanticField> {
        let record_name = base.as_record_identity()?.name;

        let id = self
            .field_lookup
            .get(&normalize_name(record_name))?
            .get(&normalize_name(field))?;
        self.fields.get(id.0)
    }

    fn record_field_facts(&self, base: &ValueType, field: &str) -> Option<FieldDescriptorFacts> {
        self.record_field_descriptor(base, field)
            .map(|field| FieldDescriptorFacts {
                id: field.id,
                owner: field.owner,
                ty: field.ty.clone(),
                offset: field.offset,
            })
    }

    fn record_field_facts_or_diagnostic(
        &mut self,
        base: Option<&ValueType>,
        field: &str,
        span: Span,
    ) -> Option<FieldDescriptorFacts> {
        let base = base?;
        if let Some(field) = self.record_field_facts(base, field) {
            return Some(field);
        }

        if let Some(name) = base.as_record_base_name() {
            if self.field_lookup.contains_key(&normalize_name(name)) {
                self.diagnostics.push(Diagnostic::new(
                    span,
                    format!("unknown field `{field}` for record `{name}`"),
                ));
            } else {
                self.diagnostics.push(Diagnostic::new(
                    span,
                    format!("type `{name}` has no record fields"),
                ));
            }
        } else if !matches!(base.kind(), ValueTypeKind::Error) {
            self.diagnostics
                .push(Diagnostic::new(span, "field access requires record type"));
        }
        None
    }

    fn validate_record_field_decl(&mut self, scope: ScopeId, field: &VarDecl) {
        self.validate_type_ref(scope, &field.ty, field.span);
        let valid_value_type = matches!(field.ty.base, TypeBase::Fund(_))
            || (!field.ty.pointer && self.type_ref_is_record(scope, &field.ty));
        if field.storage != VarStorage::Plain
            || field.ty.pointer
            || !valid_value_type
            || field
                .entries
                .iter()
                .any(|entry| entry.size.is_some() || entry.initializer.is_some())
        {
            self.diagnostics.push(Diagnostic::new(
                field.span,
                "record fields must be fundamental variables",
            ));
        }
    }

    fn type_ref_is_record(&self, scope: ScopeId, ty: &TypeRef) -> bool {
        let TypeBase::Named(name) = &ty.base else {
            return false;
        };
        self.symbols
            .lookup(scope, name)
            .and_then(|symbol_id| self.symbols.symbols.get(symbol_id.0))
            .is_some_and(|symbol| matches!(symbol.class, SymbolClass::Type | SymbolClass::Record))
    }

    fn value_storage_width(&self, value: &ValueType) -> Option<u16> {
        value.value_width_bytes().or_else(|| {
            value
                .as_record_name()
                .and_then(|name| self.record_storage_width(name))
        })
    }

    fn record_storage_width(&self, name: &str) -> Option<u16> {
        let fields = self.field_lookup.get(&normalize_name(name))?;
        fields.values().try_fold(0u16, |size, id| {
            let field = self.fields.get(id.0)?;
            let width = self.value_storage_width(&field.ty).unwrap_or(0);
            Some(size.max(field.offset.saturating_add(width)))
        })
    }

    fn analyze_var_decl(&mut self, scope: ScopeId, decl: &VarDecl, is_param: bool) {
        self.validate_type_ref(scope, &decl.ty, decl.span);
        let ty = Some(ValueType::from_type_ref(&decl.ty));

        for entry in &decl.entries {
            let class = if is_param {
                SymbolClass::Param
            } else if decl.storage == VarStorage::Array || is_string_type_ref(&decl.ty) {
                SymbolClass::Array
            } else {
                SymbolClass::Var
            };

            if let Some(symbol_id) =
                self.declare(scope, entry.name.clone(), class, ty.clone(), entry.span)
                && (decl.storage == VarStorage::Array || is_string_type_ref(&decl.ty))
            {
                self.array_symbols.insert(symbol_id);
            }
        }
    }

    fn validate_type_ref(&mut self, scope: ScopeId, ty: &TypeRef, span: Span) {
        let TypeBase::Named(name) = &ty.base else {
            return;
        };

        match self.symbols.lookup(scope, name) {
            Some(symbol_id) => {
                let symbol = &self.symbols.symbols[symbol_id.0];
                if !matches!(
                    symbol.class,
                    SymbolClass::Type | SymbolClass::Record | SymbolClass::Define
                ) {
                    self.diagnostics
                        .push(Diagnostic::new(span, format!("`{name}` is not a type")));
                }
            }
            None => self
                .diagnostics
                .push(Diagnostic::new(span, format!("unknown type `{name}`"))),
        }
    }

    fn declare(
        &mut self,
        scope: ScopeId,
        name: String,
        class: SymbolClass,
        ty: Option<ValueType>,
        span: Span,
    ) -> Option<SymbolId> {
        match self.symbols.declare(scope, name.clone(), class, ty, span) {
            Ok(id) => Some(id),
            Err(existing) => {
                let existing = &self.symbols.symbols[existing.0];
                self.diagnostics.push(Diagnostic::new(
                    span,
                    format!(
                        "duplicate symbol `{}`; first declared as {:?}",
                        name, existing.class
                    ),
                ));
                None
            }
        }
    }
}

impl SymbolTable {
    fn add_scope(&mut self, kind: ScopeKind, parent: Option<ScopeId>) -> ScopeId {
        let id = ScopeId(self.scopes.len());
        self.scopes.push(Scope {
            kind,
            parent,
            symbols: HashMap::new(),
        });
        id
    }

    fn declare(
        &mut self,
        scope: ScopeId,
        name: String,
        class: SymbolClass,
        ty: Option<ValueType>,
        span: Span,
    ) -> Result<SymbolId, SymbolId> {
        let key = normalize_name(&name);
        if let Some(existing) = self.scopes[scope.0].symbols.get(&key) {
            return Err(*existing);
        }

        let id = SymbolId(self.symbols.len());
        self.symbols.push(Symbol {
            name,
            class,
            ty,
            scope,
            span,
        });
        self.scopes[scope.0].symbols.insert(key, id);
        Ok(id)
    }

    pub fn lookup(&self, scope: ScopeId, name: &str) -> Option<SymbolId> {
        let key = normalize_name(name);
        let mut current = Some(scope);

        while let Some(scope_id) = current {
            let scope = &self.scopes[scope_id.0];
            if let Some(symbol_id) = scope.symbols.get(&key) {
                return Some(*symbol_id);
            }
            current = scope.parent;
        }

        None
    }

    pub fn resolve_action_name(&self, scope: ScopeId, name: &str) -> Option<ResolvedSymbol> {
        let scope_kind = self.scopes.get(scope.0)?.kind;
        match scope_kind {
            ScopeKind::Routine => self
                .lookup_exact(scope, name)
                .map(|id| ResolvedSymbol {
                    id,
                    stage: LookupStage::Local,
                })
                .or_else(|| self.resolve_global_then_builtin(name)),
            ScopeKind::Global => self.resolve_global_then_builtin(name),
            ScopeKind::Builtin => self.lookup_exact(scope, name).map(|id| ResolvedSymbol {
                id,
                stage: LookupStage::Builtin,
            }),
        }
    }

    fn resolve_global_then_builtin(&self, name: &str) -> Option<ResolvedSymbol> {
        self.lookup_exact(self.global_scope(), name)
            .map(|id| ResolvedSymbol {
                id,
                stage: LookupStage::Global,
            })
            .or_else(|| {
                self.builtin_scope().and_then(|builtin_scope| {
                    self.lookup_exact(builtin_scope, name)
                        .map(|id| ResolvedSymbol {
                            id,
                            stage: LookupStage::Builtin,
                        })
                })
            })
    }

    fn lookup_exact(&self, scope: ScopeId, name: &str) -> Option<SymbolId> {
        let key = normalize_name(name);
        self.scopes.get(scope.0)?.symbols.get(&key).copied()
    }

    pub fn symbols_in_scope(&self, scope: ScopeId) -> impl Iterator<Item = &Symbol> {
        self.symbols
            .iter()
            .filter(move |symbol| symbol.scope == scope)
    }

    pub fn global_scope(&self) -> ScopeId {
        ScopeId(1)
    }

    pub fn builtin_scope(&self) -> Option<ScopeId> {
        self.scopes
            .iter()
            .position(|scope| scope.kind == ScopeKind::Builtin)
            .map(ScopeId)
    }
}

impl ValueType {
    pub fn error() -> Self {
        Self {
            base: ValueTypeBase::Error,
            pointer: false,
        }
    }

    fn from_type_ref(ty: &TypeRef) -> Self {
        let base = match &ty.base {
            TypeBase::Fund(fund) => ValueTypeBase::Fund(*fund),
            TypeBase::Named(name) if is_string_type_name(name) => {
                ValueTypeBase::Fund(FundType::Char)
            }
            TypeBase::Named(name) => ValueTypeBase::Named(name.clone()),
            TypeBase::Callable(kind) => ValueTypeBase::Callable(Box::new(
                CallableType::from_routine_kind(kind.clone(), Vec::new()),
            )),
        };

        Self {
            base,
            pointer: ty.pointer && !matches!(ty.base, TypeBase::Callable(_)),
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self.base, ValueTypeBase::Error)
    }
}

impl SemanticCallableSignature {
    fn from_routine(routine: &Routine) -> Self {
        let mut params = Vec::new();
        for decl in &routine.params {
            let ty = param_signature_type(decl);
            for _ in &decl.entries {
                params.push(ty.clone());
            }
        }

        let return_type = match routine.kind {
            RoutineKind::Proc => None,
            RoutineKind::Func { return_type } => Some(fund_value(return_type)),
        };

        Self {
            kind: routine.kind.clone(),
            params,
            variadic: None,
            return_type,
            source: SemanticCallableSource::User,
        }
    }

    fn from_symbol(
        symbol: &Symbol,
        params: Vec<ValueType>,
        source: SemanticCallableSource,
    ) -> Self {
        Self {
            kind: callable_kind_from_symbol(symbol),
            params,
            variadic: None,
            return_type: symbol.ty.clone(),
            source,
        }
    }

    fn from_variadic_symbol(
        symbol: &Symbol,
        params: Vec<ValueType>,
        variadic: ValueType,
        source: SemanticCallableSource,
    ) -> Self {
        Self {
            kind: callable_kind_from_symbol(symbol),
            params,
            variadic: Some(variadic),
            return_type: symbol.ty.clone(),
            source,
        }
    }
}

fn callable_kind_from_symbol(symbol: &Symbol) -> RoutineKind {
    match (&symbol.class, symbol.ty.as_ref()) {
        (SymbolClass::Func | SymbolClass::BuiltinFunc, Some(ty)) => match ty.base {
            ValueTypeBase::Fund(fund) => RoutineKind::Func { return_type: fund },
            ValueTypeBase::Named(_) => RoutineKind::Proc,
            ValueTypeBase::Callable(_) => RoutineKind::Proc,
            ValueTypeBase::Error => RoutineKind::Proc,
        },
        _ => RoutineKind::Proc,
    }
}

fn fund_value(fund: FundType) -> ValueType {
    ValueType::fund(fund)
}

fn string_literal_type() -> ValueType {
    ValueType::pointer_to(fund_value(FundType::Char))
}

fn param_signature_type(param: &VarDecl) -> ValueType {
    let ty = ValueType::from_type_ref(&param.ty);
    if param.storage == VarStorage::Array || is_string_type_ref(&param.ty) {
        ValueType::pointer_to(ty)
    } else {
        ty
    }
}

fn promote_numeric(left: &ValueType, right: &ValueType) -> ValueType {
    if left.is_error() || right.is_error() {
        return ValueType::error();
    }
    if left.pointer || right.pointer {
        return fund_value(FundType::Card);
    }

    let Some(left) = left.as_scalar() else {
        return ValueType::error();
    };
    let Some(right) = right.as_scalar() else {
        return ValueType::error();
    };

    ValueType::scalar(ScalarType::promote_binary(left, right))
}

fn is_condition_op(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    )
}

fn diagnostic_span(span: Span, fallback: Span) -> Span {
    if span == Span::new(0, 0) {
        fallback
    } else {
        span
    }
}

fn sem_expr_class(expr: &subject::SemExpr) -> ExprClass {
    match &expr.kind {
        subject::SemExprKind::Raw(_) | subject::SemExprKind::Error => ExprClass::Unknown,
        subject::SemExprKind::Binary { op, .. } if is_condition_op(*op) => ExprClass::Condition,
        _ => ExprClass::Value,
    }
}

fn type_can_assign(expected: &ValueType, actual: &ValueType) -> bool {
    expected.assignment_compatibility(actual).is_allowed()
}

fn type_can_pass_arg(expected: &ValueType, actual: &ValueType) -> bool {
    expected.argument_compatibility(actual).is_allowed()
}

fn callable_type_from_parts(
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

fn pointer_value_types_compatible(expected: &ValueType, actual: &ValueType) -> bool {
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
            ))
}

fn array_decay_pointer_types_compatible(expected: &ValueType, actual: &ValueType) -> bool {
    pointer_value_types_compatible(expected, actual)
        || expected.pointer
            && actual.pointer
            && matches!(expected.base, ValueTypeBase::Fund(_))
            && matches!(actual.base, ValueTypeBase::Fund(_))
}

fn routine_address_can_pass_as(expected: &ValueType) -> bool {
    expected.pointer
        || expected.as_scalar() == Some(ScalarType::Card)
        || expected.as_callable_pointer().is_some()
}

fn routine_address_value_type(expected: &ValueType) -> ValueType {
    if expected.pointer || expected.as_callable_pointer().is_some() {
        expected.clone()
    } else {
        fund_value(FundType::Card)
    }
}

fn indexed_value_type(base: Option<&ValueType>) -> Option<ValueType> {
    base.map(|ty| {
        if ty.pointer {
            ty.pointee_type()
        } else {
            ty.clone()
        }
    })
}

fn typed_ref(ty: &ValueType) -> Option<&ValueType> {
    if ty.is_error() { None } else { Some(ty) }
}

fn define_value_contains_define_directive(value: &str) -> bool {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|word| word.eq_ignore_ascii_case("DEFINE"))
}

fn is_string_type_ref(ty: &TypeRef) -> bool {
    matches!(&ty.base, TypeBase::Named(name) if is_string_type_name(name)) && !ty.pointer
}

fn is_string_type_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("STRING")
}

fn is_numeric_scalar(ty: &ValueType) -> bool {
    ty.is_error() || ty.is_numeric_scalar()
}

fn is_action_reference_expr(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::Call { .. } | ExprKind::Index { .. } | ExprKind::Field { .. }
    )
}

pub(super) fn routine_control_flow_facts(routine: &Routine) -> RoutineControlFlowFacts {
    if routine_uses_machine_return(routine) {
        let body = statement_list_flow_facts(&routine.body, 0);
        return RoutineControlFlowFacts {
            may_fall_through: false,
            always_returns: true,
            contains_return: body.may_return,
            contains_exit: body.may_exit_loop,
            contains_loop: body.contains_loop,
            max_loop_depth: body.max_loop_depth,
        };
    }

    let body = statement_list_flow_facts(&routine.body, 0);
    RoutineControlFlowFacts {
        may_fall_through: body.may_continue,
        always_returns: body.always_returns,
        contains_return: body.may_return,
        contains_exit: body.may_exit_loop,
        contains_loop: body.contains_loop,
        max_loop_depth: body.max_loop_depth,
    }
}

pub(super) fn statement_list_flow_facts(statements: &[Stmt], loop_depth: usize) -> StmtFlowFacts {
    let mut facts = StmtFlowFacts::empty_continuing();
    let mut reachable = true;

    for stmt in statements {
        let stmt = stmt_flow_facts(stmt, loop_depth);
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

pub(super) fn routine_uses_machine_return(routine: &Routine) -> bool {
    routine.system_address.is_some()
        || routine
            .body
            .iter()
            .any(|stmt| matches!(stmt, Stmt::MachineBlock { .. }))
}

fn stmt_flow_facts(stmt: &Stmt, loop_depth: usize) -> StmtFlowFacts {
    match stmt {
        Stmt::Return(_) => StmtFlowFacts {
            may_continue: false,
            may_return: true,
            always_returns: true,
            may_exit_loop: false,
            contains_loop: false,
            max_loop_depth: loop_depth,
        },
        Stmt::Exit { .. } => StmtFlowFacts {
            may_continue: false,
            may_return: false,
            always_returns: false,
            may_exit_loop: true,
            contains_loop: false,
            max_loop_depth: loop_depth,
        },
        Stmt::If {
            branches,
            else_body,
            ..
        } => if_flow_facts(branches, else_body, loop_depth),
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            let body = statement_list_flow_facts(body, loop_depth + 1);
            StmtFlowFacts {
                may_continue: true,
                may_return: body.may_return,
                always_returns: false,
                may_exit_loop: body.may_exit_loop,
                contains_loop: true,
                max_loop_depth: body.max_loop_depth.max(loop_depth + 1),
            }
        }
        Stmt::DoUntil {
            body, condition, ..
        } => {
            let body = statement_list_flow_facts(body, loop_depth + 1);
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
        Stmt::Define(_)
        | Stmt::Assign { .. }
        | Stmt::CompoundAssign { .. }
        | Stmt::Call { .. }
        | Stmt::MachineBlock { .. }
        | Stmt::Unsupported { .. } => StmtFlowFacts::empty_continuing(),
    }
}

fn if_flow_facts(branches: &[IfBranch], else_body: &[Stmt], loop_depth: usize) -> StmtFlowFacts {
    let mut facts = StmtFlowFacts {
        may_continue: else_body.is_empty(),
        may_return: false,
        always_returns: false,
        may_exit_loop: false,
        contains_loop: false,
        max_loop_depth: loop_depth,
    };

    for branch in branches {
        let branch = statement_list_flow_facts(&branch.body, loop_depth);
        facts.may_continue |= branch.may_continue;
        facts.may_return |= branch.may_return;
        facts.may_exit_loop |= branch.may_exit_loop;
        facts.contains_loop |= branch.contains_loop;
        facts.max_loop_depth = facts.max_loop_depth.max(branch.max_loop_depth);
    }

    let has_else = !else_body.is_empty();
    if has_else {
        let else_facts = statement_list_flow_facts(else_body, loop_depth);
        facts.may_continue |= else_facts.may_continue;
        facts.may_return |= else_facts.may_return;
        facts.may_exit_loop |= else_facts.may_exit_loop;
        facts.contains_loop |= else_facts.contains_loop;
        facts.max_loop_depth = facts.max_loop_depth.max(else_facts.max_loop_depth);
    }

    facts.always_returns =
        !branches.is_empty() && has_else && !facts.may_continue && facts.may_return;
    facts
}

impl StmtFlowFacts {
    fn empty_continuing() -> Self {
        Self {
            may_continue: true,
            may_return: false,
            always_returns: false,
            may_exit_loop: false,
            contains_loop: false,
            max_loop_depth: 0,
        }
    }
}

fn normalize_name(name: &str) -> String {
    name.to_ascii_uppercase()
}

fn collect_retargeted_routine_names(program: &Program) -> HashSet<String> {
    let routine_names = collect_routine_names(program);
    let mut targets = HashSet::new();
    for module in &program.modules {
        for item in &module.items {
            match item {
                Item::Routine(routine) => {
                    for stmt in &routine.body {
                        collect_retargeted_routine_names_from_stmt(
                            stmt,
                            &routine_names,
                            &mut targets,
                        );
                    }
                }
                Item::Statement(stmt) => {
                    collect_retargeted_routine_names_from_stmt(stmt, &routine_names, &mut targets);
                }
                _ => {}
            }
        }
    }
    targets
}

fn collect_routine_names(program: &Program) -> HashSet<String> {
    let mut names = HashSet::new();
    for module in &program.modules {
        for item in &module.items {
            if let Item::Routine(routine) = item {
                names.insert(normalize_name(&routine.name));
            }
        }
    }
    names
}

fn collect_retargeted_routine_names_from_stmt(
    stmt: &Stmt,
    routine_names: &HashSet<String>,
    targets: &mut HashSet<String>,
) {
    match stmt {
        Stmt::Assign { target, value, .. } => {
            if let (ExprKind::Name(target_name), ExprKind::Name(value_name)) =
                (&target.kind, &value.kind)
            {
                let normalized_target = normalize_name(target_name);
                let normalized_value = normalize_name(value_name);
                if routine_names.contains(&normalized_target)
                    && routine_names.contains(&normalized_value)
                {
                    targets.insert(normalized_target);
                }
            }
        }
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                for stmt in &branch.body {
                    collect_retargeted_routine_names_from_stmt(stmt, routine_names, targets);
                }
            }
            for stmt in else_body {
                collect_retargeted_routine_names_from_stmt(stmt, routine_names, targets);
            }
        }
        Stmt::While { body, .. } | Stmt::DoUntil { body, .. } => {
            for stmt in body {
                collect_retargeted_routine_names_from_stmt(stmt, routine_names, targets);
            }
        }
        Stmt::For { body, .. } => {
            for stmt in body {
                collect_retargeted_routine_names_from_stmt(stmt, routine_names, targets);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    #[test]
    fn collects_globals_defines_and_routines() {
        let model = analyze_source("DEFINE S=\"10\" BYTE X CARD ARRAY Buf(4) PROC Main() RETURN");
        let globals: Vec<_> = model
            .symbols
            .symbols_in_scope(model.symbols.global_scope())
            .map(|symbol| (&symbol.name, &symbol.class))
            .collect();

        assert!(globals.contains(&(&"S".to_string(), &SymbolClass::Define)));
        assert!(globals.contains(&(&"X".to_string(), &SymbolClass::Var)));
        assert!(globals.contains(&(&"Buf".to_string(), &SymbolClass::Array)));
        assert!(globals.contains(&(&"Main".to_string(), &SymbolClass::Proc)));
    }

    #[test]
    fn creates_routine_scope_for_params_and_locals() {
        let model = analyze_source("PROC Copy(BYTE POINTER dest, src, CARD size) BYTE tmp RETURN");
        let routine_scope = model
            .symbols
            .scopes
            .iter()
            .position(|scope| scope.kind == ScopeKind::Routine)
            .map(ScopeId)
            .expect("routine scope");
        let locals: Vec<_> = model
            .symbols
            .symbols_in_scope(routine_scope)
            .map(|symbol| (&symbol.name, &symbol.class))
            .collect();

        assert!(locals.contains(&(&"dest".to_string(), &SymbolClass::Param)));
        assert!(locals.contains(&(&"src".to_string(), &SymbolClass::Param)));
        assert!(locals.contains(&(&"size".to_string(), &SymbolClass::Param)));
        assert!(locals.contains(&(&"tmp".to_string(), &SymbolClass::Var)));
        assert_eq!(model.routine_scopes[0].name, "Copy");
        assert_eq!(model.routine_scopes[0].scope, routine_scope);
    }

    #[test]
    fn routine_define_before_local_keeps_following_body_in_routine_scope() {
        let model = analyze_source(
            "BYTE ARRAY c_t_sp(25)=[0 0] \
             PROC Throw(BYTE index) \
             DEFINE TXS=\"$9A\" \
             BYTE sp=$A2 \
             IF index>=25 OR sp+2>c_t_sp(index) THEN [] FI \
             RETURN",
        );
        let routine_scope = model.routine_scopes[0].scope;
        let locals: Vec<_> = model
            .symbols
            .symbols_in_scope(routine_scope)
            .map(|symbol| (&symbol.name, &symbol.class))
            .collect();

        assert!(locals.contains(&(&"index".to_string(), &SymbolClass::Param)));
        assert!(locals.contains(&(&"sp".to_string(), &SymbolClass::Var)));
    }

    #[test]
    fn rejects_duplicate_symbols_in_same_scope_case_insensitively() {
        let err = analyze_source_err("BYTE X BYTE x");
        assert!(err[0].message.contains("duplicate symbol"));
    }

    #[test]
    fn validates_named_types() {
        let model = analyze_source("TYPE Pair=[BYTE left, right] Pair value");
        let global = model.symbols.global_scope();
        let pair = model.symbols.lookup(global, "Pair").expect("Pair type");
        let value = model.symbols.lookup(global, "value").expect("value symbol");

        assert_eq!(model.symbols.symbols[pair.0].class, SymbolClass::Type);
        assert_eq!(
            model.symbols.symbols[value.0].ty,
            Some(ValueType {
                base: ValueTypeBase::Named("Pair".to_string()),
                pointer: false,
            })
        );
    }

    #[test]
    fn action_lookup_reports_global_stage() {
        let model = analyze_source("BYTE value");
        let resolved = model
            .symbols
            .resolve_action_name(model.symbols.global_scope(), "value")
            .expect("value symbol");

        assert_eq!(resolved.stage, LookupStage::Global);
        assert_eq!(model.symbols.symbols[resolved.id.0].name, "value");
    }

    #[test]
    fn action_lookup_finds_resident_builtin_after_global_scope() {
        let model = analyze_source("PROC Main() RETURN");
        let resolved = model
            .symbols
            .resolve_action_name(model.symbols.global_scope(), "Print")
            .expect("Print builtin");

        assert_eq!(resolved.stage, LookupStage::Builtin);
        assert_eq!(
            model.symbols.symbols[resolved.id.0].class,
            SymbolClass::BuiltinProc
        );
    }

    #[test]
    fn action_lookup_global_shadows_resident_builtin() {
        let model = analyze_source("PROC Print() RETURN PROC Main() RETURN");
        let resolved = model
            .symbols
            .resolve_action_name(model.symbols.global_scope(), "Print")
            .expect("Print global");

        assert_eq!(resolved.stage, LookupStage::Global);
        assert_eq!(
            model.symbols.symbols[resolved.id.0].class,
            SymbolClass::Proc
        );
    }

    #[test]
    fn action_lookup_local_scope_shadows_global_and_builtin() {
        let model = analyze_source("BYTE Print PROC Main() BYTE Print RETURN");
        let routine_scope = model.routine_scopes[0].scope;
        let resolved = model
            .symbols
            .resolve_action_name(routine_scope, "Print")
            .expect("local Print");

        assert_eq!(resolved.stage, LookupStage::Local);
        assert_eq!(model.symbols.symbols[resolved.id.0].scope, routine_scope);
        assert_eq!(model.symbols.symbols[resolved.id.0].class, SymbolClass::Var);
    }

    #[test]
    fn local_variable_shadows_global_variable() {
        let model = analyze_source("BYTE x PROC Main() BYTE x x=1 RETURN");
        let routine_scope = model.routine_scopes[0].scope;
        let resolved = model
            .symbols
            .resolve_action_name(routine_scope, "x")
            .expect("local x");

        assert_eq!(resolved.stage, LookupStage::Local);
        assert_eq!(model.symbols.symbols[resolved.id.0].scope, routine_scope);
        assert_eq!(model.symbols.symbols[resolved.id.0].class, SymbolClass::Var);
    }

    #[test]
    fn parameter_shadows_global_variable() {
        let model = analyze_source("BYTE x PROC Main(BYTE x) x=1 RETURN");
        let routine_scope = model.routine_scopes[0].scope;
        let resolved = model
            .symbols
            .resolve_action_name(routine_scope, "x")
            .expect("parameter x");

        assert_eq!(resolved.stage, LookupStage::Local);
        assert_eq!(model.symbols.symbols[resolved.id.0].scope, routine_scope);
        assert_eq!(
            model.symbols.symbols[resolved.id.0].class,
            SymbolClass::Param
        );
    }

    #[test]
    fn global_variable_shadows_predefined_variable() {
        let model = analyze_source("BYTE color PROC Main() color=1 RETURN");
        let resolved = model
            .symbols
            .resolve_action_name(model.symbols.global_scope(), "color")
            .expect("global color");

        assert_eq!(resolved.stage, LookupStage::Global);
        assert_eq!(
            model.symbols.symbols[resolved.id.0].scope,
            model.symbols.global_scope()
        );
        assert_eq!(model.symbols.symbols[resolved.id.0].class, SymbolClass::Var);
    }

    #[test]
    fn local_variable_shadows_predefined_variable() {
        let model = analyze_source("PROC Main() BYTE color color=1 RETURN");
        let routine_scope = model.routine_scopes[0].scope;
        let resolved = model
            .symbols
            .resolve_action_name(routine_scope, "color")
            .expect("local color");

        assert_eq!(resolved.stage, LookupStage::Local);
        assert_eq!(model.symbols.symbols[resolved.id.0].scope, routine_scope);
        assert_eq!(model.symbols.symbols[resolved.id.0].class, SymbolClass::Var);
    }

    #[test]
    fn global_variable_and_proc_share_one_namespace() {
        let err = analyze_source_err("BYTE x PROC x() RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate symbol `x`"))
        );
    }

    #[test]
    fn global_proc_and_variable_share_one_namespace() {
        let err = analyze_source_err("PROC x() RETURN BYTE x");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate symbol `x`"))
        );
    }

    #[test]
    fn routine_parameter_and_local_share_one_namespace() {
        let err = analyze_source_err("PROC Main(BYTE x) BYTE x RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate symbol `x`"))
        );
    }

    #[test]
    fn global_type_and_variable_share_one_namespace() {
        let err = analyze_source_err("TYPE x=[BYTE a] BYTE x");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate symbol `x`"))
        );
    }

    #[test]
    fn global_define_and_variable_share_one_namespace() {
        let err = analyze_source_err("DEFINE x=\"1\" BYTE x");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate symbol `x`"))
        );
    }

    #[test]
    fn local_variable_can_shadow_global_proc() {
        let model = analyze_source("PROC x() RETURN PROC Main() BYTE x x=1 RETURN");
        let routine_scope = model.routine_scopes[1].scope;
        let resolved = model
            .symbols
            .resolve_action_name(routine_scope, "x")
            .expect("local x");

        assert_eq!(resolved.stage, LookupStage::Local);
        assert_eq!(model.symbols.symbols[resolved.id.0].scope, routine_scope);
        assert_eq!(model.symbols.symbols[resolved.id.0].class, SymbolClass::Var);
    }

    #[test]
    fn shadowing_variable_call_does_not_fall_through_to_global_proc() {
        let err = analyze_source_err("PROC x() RETURN PROC Main() BYTE x x() RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("`x` is not callable"))
        );
    }

    #[test]
    fn action_lookup_routine_scope_falls_back_to_global_then_builtin() {
        let model = analyze_source("BYTE global PROC Main() RETURN");
        let routine_scope = model.routine_scopes[0].scope;
        let global = model
            .symbols
            .resolve_action_name(routine_scope, "global")
            .expect("global from routine");
        let builtin = model
            .symbols
            .resolve_action_name(routine_scope, "Print")
            .expect("builtin from routine");

        assert_eq!(global.stage, LookupStage::Global);
        assert_eq!(builtin.stage, LookupStage::Builtin);
    }

    #[test]
    fn action_lookup_does_not_see_other_routine_locals() {
        let model = analyze_source("PROC One() BYTE hidden RETURN PROC Two() RETURN");
        let one_scope = model.routine_scopes[0].scope;
        let two_scope = model.routine_scopes[1].scope;

        assert_eq!(
            model
                .symbols
                .resolve_action_name(one_scope, "hidden")
                .map(|r| r.stage),
            Some(LookupStage::Local)
        );
        assert!(
            model
                .symbols
                .resolve_action_name(two_scope, "hidden")
                .is_none()
        );
    }

    #[test]
    fn rejects_forward_routine_call() {
        let err = analyze_source_err("PROC First() Second() RETURN PROC Second() RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("undefined symbol `Second`"))
        );
    }

    #[test]
    fn accepts_backward_routine_call() {
        analyze_source("PROC Second() RETURN PROC First() Second() RETURN");
    }

    #[test]
    fn rejects_forward_global_variable_use() {
        let err = analyze_source_err("PROC Main() x=1 RETURN BYTE x");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("undefined symbol `x`"))
        );
    }

    #[test]
    fn rejects_forward_type_use() {
        let err = analyze_source_err("PROC Main() Pair p RETURN TYPE Pair=[BYTE x]");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("unknown type `Pair`"))
        );
    }

    #[test]
    fn accepts_builtin_use_before_user_declarations() {
        analyze_source("PROC Main() PrintE(\"READY\") RETURN BYTE PrintEValue");
    }

    #[test]
    fn accepts_resident_break_builtin() {
        analyze_source("PROC Main() Break() RETURN");
    }

    #[test]
    fn accepts_resident_error_builtin_with_runtime_disk_arity() {
        analyze_source("PROC Main() Error(71) Error(71,0,71) RETURN");
    }

    #[test]
    fn builtin_signatures_are_available_for_argument_typing() {
        let model =
            analyze_source("PROC Main() PrintE(\"READY\") Position(10,2) Zero($4000,16) RETURN");

        let print_e = model
            .routine_signatures
            .get(&normalize_name("PrintE"))
            .expect("PrintE signature");
        assert_eq!(print_e.kind, RoutineKind::Proc);
        assert_eq!(print_e.params, vec![fund_value(FundType::Card)]);
        assert_eq!(print_e.return_type, None);
        assert_eq!(print_e.source, SemanticCallableSource::Resident);

        let position = model
            .routine_signatures
            .get(&normalize_name("Position"))
            .expect("Position signature");
        assert_eq!(position.kind, RoutineKind::Proc);
        assert_eq!(
            position.params,
            vec![fund_value(FundType::Card), fund_value(FundType::Byte)]
        );
        assert_eq!(position.return_type, None);
        assert_eq!(position.source, SemanticCallableSource::Resident);

        let print_f = model
            .routine_signatures
            .get(&normalize_name("PrintF"))
            .expect("PrintF signature");
        assert_eq!(print_f.kind, RoutineKind::Proc);
        assert_eq!(print_f.params, vec![fund_value(FundType::Card)]);
        assert_eq!(print_f.variadic, Some(fund_value(FundType::Card)));
        assert_eq!(print_f.return_type, None);
        assert_eq!(print_f.source, SemanticCallableSource::Resident);
    }

    #[test]
    fn user_function_signatures_keep_kind_params_and_return_type() {
        let model = analyze_source("BYTE FUNC F(BYTE x CARD y) RETURN(x)");

        let signature = model
            .routine_signatures
            .get(&normalize_name("F"))
            .expect("F signature");
        assert_eq!(
            signature.kind,
            RoutineKind::Func {
                return_type: FundType::Byte
            }
        );
        assert_eq!(
            signature.params,
            vec![fund_value(FundType::Byte), fund_value(FundType::Card)]
        );
        assert_eq!(signature.return_type, Some(fund_value(FundType::Byte)));
        assert_eq!(signature.source, SemanticCallableSource::User);
    }

    #[test]
    fn rejects_builtin_call_with_incompatible_argument_type() {
        let err = analyze_source_err("PROC Main() Graphics(\"X\") RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("`Graphics` argument 1 expects")),
            "expected builtin argument diagnostic, got {err:?}"
        );
    }

    #[test]
    fn rejects_builtin_call_with_too_many_arguments() {
        let err = analyze_source_err("PROC Main() PutE(1) RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("`PutE` expects at most 0")),
            "expected builtin arity diagnostic, got {err:?}"
        );
    }

    #[test]
    fn predefined_variables_are_available_before_user_declarations() {
        let model =
            analyze_source("PROC Main() BYTE d color=2 d=device LIST=1 TRACE=0 d=EOF(1) RETURN");
        let routine_scope = model.routine_scopes[0].scope;
        let color = model
            .symbols
            .resolve_action_name(routine_scope, "color")
            .expect("predefined color");
        let device = model
            .symbols
            .resolve_action_name(routine_scope, "device")
            .expect("predefined device");
        let eof = model
            .symbols
            .resolve_action_name(routine_scope, "eof")
            .expect("predefined eof");
        let list = model
            .symbols
            .resolve_action_name(routine_scope, "list")
            .expect("predefined list");
        let trace = model
            .symbols
            .resolve_action_name(routine_scope, "trace")
            .expect("predefined trace");

        assert_eq!(color.stage, LookupStage::Builtin);
        assert_eq!(device.stage, LookupStage::Builtin);
        assert_eq!(eof.stage, LookupStage::Builtin);
        assert_eq!(list.stage, LookupStage::Builtin);
        assert_eq!(trace.stage, LookupStage::Builtin);
        assert_eq!(model.symbols.symbols[color.id.0].class, SymbolClass::Var);
        assert_eq!(model.symbols.symbols[device.id.0].class, SymbolClass::Var);
        assert_eq!(model.symbols.symbols[eof.id.0].class, SymbolClass::Array);
        assert_eq!(model.symbols.symbols[list.id.0].class, SymbolClass::Var);
        assert_eq!(model.symbols.symbols[trace.id.0].class, SymbolClass::Var);
        assert!(model.array_symbols.contains(&eof.id));
    }

    #[test]
    fn rejects_non_fundamental_record_fields() {
        let err = analyze_source_err("TYPE Pair=[BYTE tag CHAR POINTER ptr]");
        assert!(
            err[0]
                .message
                .contains("record fields must be fundamental variables")
        );

        let err = analyze_source_err("TYPE Pair=[BYTE ARRAY bytes(4)]");
        assert!(
            err[0]
                .message
                .contains("record fields must be fundamental variables")
        );
    }

    #[test]
    fn records_field_descriptors_with_offsets() {
        let model = analyze_source("TYPE Pair=[BYTE tag CARD word BYTE tail] Pair rec");
        let pair = model
            .symbols
            .lookup(model.symbols.global_scope(), "Pair")
            .expect("Pair type");

        let fields: Vec<_> = model
            .fields
            .iter()
            .filter(|field| field.owner == pair)
            .map(|field| (&field.name, field.ty.clone(), field.offset))
            .collect();

        assert_eq!(
            fields,
            vec![
                (&"tag".to_string(), fund_value(FundType::Byte), 0),
                (&"word".to_string(), fund_value(FundType::Card), 1),
                (&"tail".to_string(), fund_value(FundType::Byte), 3),
            ]
        );
    }

    #[test]
    fn semantic_layout_records_group_field_ids_and_size() {
        let model = analyze_source("TYPE Pair=[BYTE tag CARD word BYTE tail] Pair rec");
        let pair = model
            .symbols
            .lookup(model.symbols.global_scope(), "Pair")
            .expect("Pair type");
        let tag = model.field_lookup["PAIR"]["TAG"];
        let word = model.field_lookup["PAIR"]["WORD"];
        let tail = model.field_lookup["PAIR"]["TAIL"];
        let layout = model.layout.record_for_owner(pair).expect("Pair layout");

        assert_eq!(layout.name, "Pair");
        assert_eq!(
            layout
                .fields
                .iter()
                .map(|field| field.id)
                .collect::<Vec<_>>(),
            vec![tag, word, tail]
        );
        assert_eq!(
            layout
                .fields
                .iter()
                .map(|field| (&field.name, field.ty.clone(), field.offset))
                .collect::<Vec<_>>(),
            vec![
                (&"tag".to_string(), fund_value(FundType::Byte), 0),
                (&"word".to_string(), fund_value(FundType::Card), 1),
                (&"tail".to_string(), fund_value(FundType::Byte), 3),
            ]
        );
        assert_eq!(layout.size, 4);
    }

    #[test]
    fn semantic_layout_arrays_record_element_pointer_and_origin() {
        let model = analyze_source(
            "BYTE ARRAY global(4) PROC Main(BYTE ARRAY param) CARD ARRAY local(2) RETURN",
        );
        let global = model
            .symbols
            .lookup(model.symbols.global_scope(), "global")
            .expect("global array");
        let routine_scope = model.routine_scopes[0].scope;
        let param = model
            .symbols
            .lookup(routine_scope, "param")
            .expect("array parameter");
        let local = model
            .symbols
            .lookup(routine_scope, "local")
            .expect("local array");

        let global_layout = model
            .layout
            .array_for_symbol(global)
            .expect("global layout");
        let param_layout = model.layout.array_for_symbol(param).expect("param layout");
        let local_layout = model.layout.array_for_symbol(local).expect("local layout");

        assert_eq!(global_layout.element_type, fund_value(FundType::Byte));
        assert_eq!(
            global_layout.pointer_type,
            ValueType::pointer_to(fund_value(FundType::Byte))
        );
        assert_eq!(global_layout.origin, SemanticArrayOrigin::Global);
        assert_eq!(param_layout.origin, SemanticArrayOrigin::Parameter);
        assert_eq!(local_layout.element_type, fund_value(FundType::Card));
        assert_eq!(local_layout.origin, SemanticArrayOrigin::Local);
    }

    #[test]
    fn same_named_fields_have_distinct_record_relative_ids() {
        let model = analyze_source("TYPE A=[BYTE tag] TYPE B=[BYTE tag] A av B bv");
        let a_tag = model.field_lookup["A"]["TAG"];
        let b_tag = model.field_lookup["B"]["TAG"];

        assert_ne!(a_tag, b_tag);
        assert_ne!(
            model.fields[a_tag.0].owner, model.fields[b_tag.0].owner,
            "field ownership should stay record-relative"
        );
    }

    #[test]
    fn rejects_unknown_record_field_with_specific_diagnostic() {
        let err =
            analyze_source_err("TYPE Pair=[BYTE tag] Pair rec PROC Main() rec.missing=1 RETURN");
        assert!(
            err.iter().any(|diagnostic| diagnostic
                .message
                .contains("unknown field `missing` for record `Pair`")),
            "expected unknown-field diagnostic, got {err:?}"
        );
    }

    #[test]
    fn rejects_field_access_on_non_record_type() {
        let err = analyze_source_err("BYTE x PROC Main() x.tag=1 RETURN");
        assert!(
            err.iter().any(|diagnostic| diagnostic
                .message
                .contains("field access requires record type")),
            "expected non-record field diagnostic, got {err:?}"
        );
    }

    #[test]
    fn validates_routine_local_named_types() {
        analyze_source(
            "PROC Burst() TYPE IOCB=[BYTE cmd CARD blen] IOCB POINTER iptr iptr.cmd=7 RETURN",
        );
    }

    #[test]
    fn accepts_define_backed_type_aliases() {
        analyze_source("DEFINE STRING=\"CHAR ARRAY\" STRING name");
    }

    #[test]
    fn rejects_nested_define_directives() {
        let err = analyze_source_err("DEFINE BAD=\"DEFINE INNER\"");
        assert!(err[0].message.contains("nested DEFINE"));
    }

    #[test]
    fn allows_define_values_that_merely_contain_define_as_part_of_word() {
        analyze_source("DEFINE REDEFINED=\"BYTE\" REDEFINED value");
    }

    #[test]
    fn accepts_runtime_string_type_without_local_define() {
        analyze_source("PROC Print(STRING s) RETURN");
    }

    #[test]
    fn treats_string_alias_declarations_as_char_arrays() {
        let model = analyze_source("DEFINE STRING=\"CHAR ARRAY\" STRING name(0)=\"A\"");
        let symbol_id = model
            .symbols
            .lookup(model.symbols.global_scope(), "name")
            .expect("name symbol");
        let symbol = &model.symbols.symbols[symbol_id.0];

        assert_eq!(symbol.class, SymbolClass::Array);
        assert_eq!(
            symbol.ty,
            Some(ValueType {
                base: ValueTypeBase::Fund(FundType::Char),
                pointer: false,
            })
        );
    }

    #[test]
    fn string_literal_expressions_have_char_pointer_type() {
        let model = analyze_source("CHAR POINTER p PROC Main() p=\"HI\" RETURN");
        let string_expr = model
            .expression_observations
            .iter()
            .find(|expr| expr.span.start < expr.span.end && expr.ty == Some(string_literal_type()))
            .expect("typed string literal expression");

        assert_eq!(string_expr.class, ExprClass::Value);
    }

    #[test]
    fn string_literal_passes_to_string_parameter() {
        analyze_source(
            "DEFINE STRING=\"CHAR ARRAY\" PROC Take(STRING s) RETURN PROC Main() Take(\"HI\") RETURN",
        );
    }

    #[test]
    fn rejects_unknown_named_type() {
        let err = analyze_source_err("MissingType value");
        assert!(err[0].message.contains("unknown type"));
    }

    #[test]
    fn rejects_exit_outside_loop() {
        let err = analyze_source_err("PROC Main() EXIT");
        assert!(err[0].message.contains("EXIT outside loop"));
    }

    #[test]
    fn allows_exit_inside_loop() {
        analyze_source("PROC Main() WHILE 1 DO EXIT OD RETURN");
    }

    #[test]
    fn rejects_callable_if_condition() {
        let err = analyze_source_err("PROC P() RETURN PROC Main() IF P THEN RETURN FI RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("expected value expression"))
        );
    }

    #[test]
    fn rejects_type_if_condition() {
        let err =
            analyze_source_err("TYPE Pair=[BYTE x] PROC Main() IF Pair THEN RETURN FI RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("expected value expression"))
        );
    }

    #[test]
    fn accepts_define_in_condition_value_context() {
        analyze_source("DEFINE NULL=\"0\" CARD p PROC Main() WHILE p<>NULL DO p=0 OD RETURN");
    }

    #[test]
    fn control_flow_statements_do_not_create_source_scopes() {
        let model = analyze_source(
            "BYTE g PROC Main() BYTE i,x FOR i=0 TO 3 DO x=i OD IF x THEN x=1 ELSE x=2 FI WHILE x DO x=0 OD DO x=1 UNTIL x=0 OD RETURN",
        );

        assert_eq!(
            model
                .symbols
                .scopes
                .iter()
                .filter(|scope| scope.kind == ScopeKind::Builtin)
                .count(),
            1
        );
        assert_eq!(
            model
                .symbols
                .scopes
                .iter()
                .filter(|scope| scope.kind == ScopeKind::Global)
                .count(),
            1
        );
        assert_eq!(
            model
                .symbols
                .scopes
                .iter()
                .filter(|scope| scope.kind == ScopeKind::Routine)
                .count(),
            1
        );
        assert_eq!(model.symbols.scopes.len(), 3);
    }

    #[test]
    fn for_loop_generated_storage_is_not_a_source_symbol() {
        let model = analyze_source("PROC Main() BYTE i FOR i=0 TO 3 DO OD RETURN");
        let routine_scope = model.routine_scopes[0].scope;
        let locals: Vec<_> = model
            .symbols
            .symbols_in_scope(routine_scope)
            .map(|symbol| (&symbol.name, &symbol.class))
            .collect();

        assert_eq!(locals, vec![(&"i".to_string(), &SymbolClass::Var)]);
    }

    #[test]
    fn accepts_negative_for_step_on_numeric_scalar() {
        analyze_source("BYTE i PROC Main() FOR i=3 TO 1 STEP -1 DO OD RETURN");
    }

    #[test]
    fn rejects_pointer_for_target() {
        let err = analyze_source_err("BYTE POINTER p PROC Main() FOR p=0 TO 1 DO OD RETURN");
        assert!(err[0].message.contains("FOR target"));
    }

    #[test]
    fn rejects_pointer_for_bound() {
        let err = analyze_source_err("BYTE POINTER p BYTE i PROC Main() FOR i=0 TO p DO OD RETURN");
        assert!(err[0].message.contains("FOR end value"));
    }

    #[test]
    fn rejects_callable_for_bound() {
        let err =
            analyze_source_err("PROC P() RETURN BYTE i PROC Main() FOR i=0 TO P DO OD RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("expected value expression"))
        );
    }

    #[test]
    fn rejects_routine_for_target() {
        let err = analyze_source_err("PROC P() RETURN PROC Main() FOR P=0 TO 1 DO OD RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("FOR target must be assignable"))
        );
    }

    #[test]
    fn rejects_literal_assignment_target() {
        let err = analyze_source_err("PROC Main() 1=2");
        assert!(err[0].message.contains("invalid assignment target"));
    }

    #[test]
    fn rejects_function_call_as_assignment_target() {
        let err = analyze_source_err("BYTE FUNC F() RETURN(1) PROC Main() F()=2 RETURN");
        assert!(err[0].message.contains("invalid assignment target"));
    }

    #[test]
    fn rejects_calling_known_variable() {
        let err = analyze_source_err("BYTE X PROC Main() X()");
        assert!(err[0].message.contains("not callable"));
    }

    #[test]
    fn allows_parenthesized_array_references_in_expressions() {
        analyze_source("PROC Main() BYTE ARRAY values(4) BYTE x x=values(0) RETURN");
    }

    #[test]
    fn accepts_user_proc_call_with_omitted_trailing_arguments() {
        analyze_source("PROC P(BYTE x, y) RETURN PROC Main() P(1) RETURN");
    }

    #[test]
    fn accepts_user_func_call_with_omitted_trailing_arguments() {
        analyze_source("BYTE FUNC F(BYTE x) RETURN(x) PROC Main() BYTE y y=F() RETURN");
    }

    #[test]
    fn rejects_user_call_with_too_many_arguments() {
        let err = analyze_source_err("PROC P(BYTE x) RETURN PROC Main() P(1,2) RETURN");
        assert!(err[0].message.contains("expects at most 1 argument"));
    }

    #[test]
    fn unresolved_call_does_not_emit_function_pointer_arity_error() {
        let err = analyze_source_err("PROC Main() Missing(1,2,3) RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("undefined symbol `Missing`"))
        );
        assert!(
            !err.iter()
                .any(|diagnostic| diagnostic.message.contains("<function pointer>")),
            "{err:#?}"
        );
    }

    #[test]
    fn rejects_user_call_argument_type_mismatch() {
        let err =
            analyze_source_err("TYPE Pair=[BYTE x] PROC P(Pair x) RETURN PROC Main() P(1) RETURN");
        assert!(err[0].message.contains("argument 1 expects"));
    }

    #[test]
    fn accepts_user_call_argument_widening() {
        analyze_source("PROC P(CARD x) RETURN PROC Main() P(255) RETURN");
    }

    #[test]
    fn accepts_value_argument_for_matching_pointer_parameter() {
        analyze_source(
            "TYPE Real=[CARD r1] PROC P(Real POINTER r) RETURN Real value PROC Main() P(value) RETURN",
        );
    }

    #[test]
    fn accepts_record_value_argument_as_implicit_address_of() {
        analyze_source(
            "TYPE Pair=[BYTE tag CARD word] PROC Touch(Pair POINTER p) RETURN Pair rec PROC Main() Touch(rec) RETURN",
        );
    }

    #[test]
    fn accepts_explicit_record_address_argument_for_pointer_parameter() {
        analyze_source(
            "TYPE Pair=[BYTE tag CARD word] PROC Touch(Pair POINTER p) RETURN Pair rec PROC Main() Touch(@rec) RETURN",
        );
    }

    #[test]
    fn accepts_card_value_argument_for_pointer_parameter() {
        analyze_source("PROC Draw(BYTE POINTER p) RETURN PROC Main() CARD menu Draw(menu) RETURN");
    }

    #[test]
    fn rejects_scalar_value_argument_for_pointer_parameter() {
        let err =
            analyze_source_err("PROC P(BYTE POINTER p) RETURN BYTE x PROC Main() P(x) RETURN");
        assert!(err[0].message.contains("argument 1 expects"));
    }

    #[test]
    fn accepts_byte_char_pointer_argument_interchange() {
        analyze_source(
            "PROC P(CHAR POINTER p) RETURN BYTE ARRAY data(4) PROC Main() P(data) RETURN",
        );
    }

    #[test]
    fn accepts_array_argument_decay_to_matching_pointer_parameter() {
        analyze_source(
            "PROC P(BYTE POINTER p) RETURN BYTE ARRAY data(4) PROC Main() P(data) RETURN",
        );
    }

    #[test]
    fn accepts_array_parameter_decay_to_matching_pointer_parameter() {
        analyze_source(
            "PROC P(BYTE POINTER p) RETURN PROC Forward(BYTE ARRAY data) P(data) RETURN",
        );
    }

    #[test]
    fn accepts_fundamental_array_argument_decay_to_card_pointer_parameter() {
        analyze_source(
            "PROC P(CARD POINTER p) RETURN BYTE ARRAY data(4) PROC Main() P(data) RETURN",
        );
    }

    #[test]
    fn accepts_card_array_argument_decay_to_byte_pointer_parameter() {
        analyze_source(
            "PROC P(BYTE POINTER p) RETURN CARD ARRAY data(4) PROC Main() P(data) RETURN",
        );
    }

    #[test]
    fn rejects_array_element_argument_as_pointer_parameter() {
        let err = analyze_source_err(
            "PROC P(BYTE POINTER p) RETURN BYTE ARRAY data(4) PROC Main() P(data(0)) RETURN",
        );
        assert!(err[0].message.contains("argument 1 expects"));
    }

    #[test]
    fn rejects_mismatched_value_argument_for_pointer_parameter() {
        let err = analyze_source_err(
            "TYPE Real=[CARD r1] TYPE Other=[CARD r1] PROC P(Real POINTER r) RETURN Other value PROC Main() P(value) RETURN",
        );
        assert!(err[0].message.contains("argument 1 expects"));
    }

    #[test]
    fn accepts_routine_assignment_targets() {
        analyze_source(
            "PROC Rom=$D800() PROC Trampoline() RETURN PROC Main() Trampoline=Rom RETURN",
        );
    }

    #[test]
    fn accepts_assignable_local_and_array_targets() {
        analyze_source("PROC Main() BYTE x BYTE ARRAY a(2) x=1 a(0)=x RETURN");
    }

    #[test]
    fn accepts_card_literal_assignment_to_pointer() {
        analyze_source("BYTE POINTER p PROC Main() p=$4000 RETURN");
    }

    #[test]
    fn accepts_card_value_assignment_to_pointer() {
        analyze_source("BYTE POINTER p CARD addr PROC Main() addr=$4000 p=addr RETURN");
    }

    #[test]
    fn accepts_byte_char_pointer_assignment_interchange() {
        analyze_source("CHAR POINTER p BYTE ARRAY data(4) PROC Main() p=data RETURN");
    }

    #[test]
    fn accepts_array_assignment_decay_to_matching_pointer() {
        analyze_source("BYTE POINTER p BYTE ARRAY data(4) PROC Main() p=data RETURN");
    }

    #[test]
    fn accepts_array_parameter_assignment_decay_to_matching_pointer() {
        analyze_source("PROC Main(BYTE ARRAY data) BYTE POINTER p p=data RETURN");
    }

    #[test]
    fn accepts_fundamental_array_assignment_decay_to_card_pointer() {
        analyze_source("CARD POINTER p BYTE ARRAY data(4) PROC Main() p=data RETURN");
    }

    #[test]
    fn accepts_card_array_assignment_decay_to_byte_pointer() {
        analyze_source("BYTE POINTER p CARD ARRAY data(4) PROC Main() p=data RETURN");
    }

    #[test]
    fn rejects_fundamental_array_decay_to_record_pointer() {
        let err = analyze_source_err(
            "TYPE Pair=[BYTE x] Pair POINTER p BYTE ARRAY data(4) PROC Main() p=data RETURN",
        );
        assert!(err[0].message.contains("cannot assign"));
    }

    #[test]
    fn rejects_array_element_assignment_to_pointer() {
        let err =
            analyze_source_err("BYTE POINTER p BYTE ARRAY data(4) PROC Main() p=data(0) RETURN");
        assert!(err[0].message.contains("cannot assign"));
    }

    #[test]
    fn accepts_matching_value_assignment_to_pointer() {
        analyze_source("TYPE Real=[CARD r1] Real value Real POINTER p PROC Main() p=value RETURN");
    }

    #[test]
    fn accepts_record_value_assignment_as_implicit_address_of() {
        analyze_source(
            "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p PROC Main() p=rec RETURN",
        );
    }

    #[test]
    fn accepts_explicit_record_address_assignment_to_pointer() {
        analyze_source(
            "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p PROC Main() p=@rec RETURN",
        );
    }

    #[test]
    fn accepts_record_field_address_assignment_to_matching_pointer() {
        analyze_source(
            "TYPE Pair=[BYTE tag CARD word] Pair rec BYTE POINTER bp CARD POINTER cp PROC Main() bp=@rec.tag cp=@rec.word RETURN",
        );
    }

    #[test]
    fn rejects_record_field_address_assignment_to_mismatched_pointer() {
        let err = analyze_source_err(
            "TYPE Pair=[BYTE tag CARD word] Pair rec BYTE POINTER bp CARD POINTER cp PROC Main() cp=@rec.tag bp=@rec.word RETURN",
        );
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("cannot assign"))
        );
    }

    #[test]
    fn rejects_scalar_value_assignment_to_pointer() {
        let err = analyze_source_err("BYTE POINTER p BYTE x PROC Main() p=x RETURN");
        assert!(err[0].message.contains("cannot assign"));
    }

    #[test]
    fn rejects_mismatched_value_assignment_to_pointer() {
        let err = analyze_source_err(
            "TYPE Real=[CARD r1] TYPE Other=[CARD r1] Real POINTER p Other value PROC Main() p=value RETURN",
        );
        assert!(err[0].message.contains("cannot assign"));
    }

    #[test]
    fn accepts_record_field_assignment_targets() {
        analyze_source("TYPE Real=[CARD r1] PROC Assign(Real POINTER a,b) b.r1=a.r1 RETURN");
    }

    #[test]
    fn accepts_record_fields_as_typed_call_arguments() {
        analyze_source(
            "TYPE Pair=[BYTE tag CARD word] Pair rec PROC Take(BYTE b,CARD w) RETURN PROC Main() Take(rec.tag,rec.word) RETURN",
        );
    }

    #[test]
    fn rejects_value_return_from_proc() {
        let err = analyze_source_err("PROC Main() RETURN(1)");
        assert!(err[0].message.contains("procedure RETURN"));
    }

    #[test]
    fn rejects_bare_return_from_func() {
        let err = analyze_source_err("BYTE FUNC F() RETURN");
        assert!(err[0].message.contains("function RETURN"));
    }

    #[test]
    fn rejects_func_without_return_path() {
        let err = analyze_source_err("BYTE FUNC F() BYTE x x=1");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("may exit without RETURN value"))
        );
    }

    #[test]
    fn rejects_func_if_return_without_else_path() {
        let err = analyze_source_err("BYTE FUNC F(BYTE x) IF x THEN RETURN(1) FI");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("may exit without RETURN value"))
        );
    }

    #[test]
    fn accepts_bodyless_function_retargeted_by_routine_assignment() {
        analyze_source("BYTE FUNC A() RETURN(1) BYTE FUNC T() PROC Main() T=A T() RETURN");
    }

    #[test]
    fn rejects_bodyless_function_that_is_not_retargeted() {
        let err = analyze_source_err("BYTE FUNC F() PROC Main() RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("may exit without RETURN value"))
        );
    }

    #[test]
    fn accepts_func_if_else_return_paths() {
        analyze_source("BYTE FUNC F(BYTE x) IF x THEN RETURN(1) ELSE RETURN(0) FI");
    }

    #[test]
    fn accepts_func_with_unconditional_loop_conditional_returns() {
        analyze_source(
            "BYTE FUNC F() BYTE k DO k=GetD(1) IF k='y THEN RETURN(1) ELSEIF k='n THEN RETURN(0) FI OD",
        );
    }

    #[test]
    fn routine_control_flow_facts_track_return_and_fallthrough() {
        let (program, _) = analyze_program_source(
            "PROC P() BYTE x x=1 RETURN BYTE FUNC F(BYTE x) IF x THEN RETURN(1) ELSE RETURN(0) FI",
        );
        let p = routine_by_name(&program, "P");
        let f = routine_by_name(&program, "F");

        let p_flow = routine_control_flow_facts(p);
        assert!(p_flow.always_returns);
        assert!(!p_flow.may_fall_through);
        assert!(p_flow.contains_return);
        assert!(!p_flow.contains_loop);

        let f_flow = routine_control_flow_facts(f);
        assert!(f_flow.always_returns);
        assert!(!f_flow.may_fall_through);
        assert!(f_flow.contains_return);
    }

    #[test]
    fn routine_control_flow_facts_track_loops_and_exits() {
        let (program, _) =
            analyze_program_source("PROC Main() BYTE x WHILE x DO IF x=1 THEN EXIT FI OD RETURN");
        let main = routine_by_name(&program, "Main");

        let flow = routine_control_flow_facts(main);
        assert!(flow.always_returns);
        assert!(!flow.may_fall_through);
        assert!(flow.contains_exit);
        assert!(flow.contains_loop);
        assert_eq!(flow.max_loop_depth, 1);
    }

    #[test]
    fn routine_control_flow_facts_treat_bare_do_as_non_fallthrough() {
        let (program, _) = analyze_program_source("PROC Main() DO OD");
        let main = routine_by_name(&program, "Main");

        let flow = routine_control_flow_facts(main);
        assert!(!flow.may_fall_through);
        assert!(!flow.always_returns);
        assert!(flow.contains_loop);
        assert_eq!(flow.max_loop_depth, 1);
    }

    #[test]
    fn accepts_machine_code_func_without_source_return() {
        analyze_source("BYTE FUNC Raw=*() [$A9 $01 $85 $A0 $60]");
    }

    #[test]
    fn address_of_expression_has_pointer_type() {
        let model = analyze_source("BYTE x PROC Main() CARD c c=@x RETURN");
        let byte_pointer = ValueType::pointer_to(fund_value(FundType::Byte));

        assert!(model
            .expression_observations
            .iter()
            .any(|expr| expr.class == ExprClass::Value && expr.ty == Some(byte_pointer.clone())));
    }

    #[test]
    fn deref_expression_has_pointee_type() {
        let model = analyze_source("BYTE POINTER p BYTE x PROC Main() x=p^ RETURN");

        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Value && expr.ty == Some(fund_value(FundType::Byte))
        }));
    }

    #[test]
    fn array_deref_expression_has_element_type() {
        let model = analyze_source("CARD ARRAY data CARD x PROC Main() data=$4000 x=data^ RETURN");

        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Value && expr.ty == Some(fund_value(FundType::Card))
        }));
    }

    #[test]
    fn pointer_index_expression_has_pointee_type() {
        let model = analyze_source("BYTE POINTER p BYTE x PROC Main() x=p(1) RETURN");

        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::LValue && expr.ty == Some(fund_value(FundType::Byte))
        }));
    }

    #[test]
    fn byte_array_index_expression_has_element_type() {
        let model = analyze_source("BYTE ARRAY data(4) BYTE x PROC Main() x=data(1) RETURN");

        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::LValue && expr.ty == Some(fund_value(FundType::Byte))
        }));
    }

    #[test]
    fn card_array_index_expression_has_element_type() {
        let model = analyze_source("CARD ARRAY data(4) CARD x PROC Main() x=data(1) RETURN");

        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::LValue && expr.ty == Some(fund_value(FundType::Card))
        }));
    }

    #[test]
    fn rejects_indexing_non_array_non_pointer_value() {
        let err = analyze_source_err("BYTE x,y PROC Main() y=x(0) RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("`x` is not callable")),
            "expected scalar call/index diagnostic, got {err:?}"
        );
    }

    #[test]
    fn rejects_non_numeric_index_expression() {
        let err = analyze_source_err("BYTE POINTER p,q BYTE x PROC Main() x=p(q) RETURN");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("index must be numeric")),
            "expected non-numeric index diagnostic, got {err:?}"
        );
    }

    #[test]
    fn accepts_typed_compound_assignments() {
        analyze_source("BYTE b BYTE POINTER p BYTE ARRAY a PROC Main() b==&$0F p==+1 a==+2 RETURN");
    }

    #[test]
    fn rejects_pointer_compound_assignment_with_bitwise_operator() {
        let err = analyze_source_err("BYTE POINTER p PROC Main() p==&1 RETURN");
        assert!(
            err.iter().any(|diagnostic| diagnostic
                .message
                .contains("pointer compound assignment only supports + or -")),
            "expected pointer compound operator diagnostic, got {err:?}"
        );
    }

    #[test]
    fn rejects_compound_assignment_with_non_numeric_value() {
        let err = analyze_source_err("BYTE b BYTE POINTER p PROC Main() b==+p RETURN");
        assert!(
            err.iter().any(|diagnostic| diagnostic
                .message
                .contains("compound assignment value must be numeric")),
            "expected compound value diagnostic, got {err:?}"
        );
    }

    #[test]
    fn rejects_compound_assignment_to_routine_target() {
        let err = analyze_source_err("BYTE FUNC F() RETURN(1) PROC Main() F==+1 RETURN");
        assert!(
            err.iter().any(|diagnostic| diagnostic
                .message
                .contains("compound assignment target must be assignable")),
            "expected compound target diagnostic, got {err:?}"
        );
    }

    #[test]
    fn rejects_deref_of_non_pointer_value() {
        let err = analyze_source_err("BYTE x PROC Main() x^=1 RETURN");
        assert!(err.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("cannot dereference non-pointer")
        }));
    }

    #[test]
    fn lowers_literal_and_binary_expression_types() {
        let model = analyze_source("CARD FUNC F() BYTE x RETURN(x + 256)");
        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::LValue && expr.ty == Some(fund_value(FundType::Byte))
        }));
        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Value && expr.ty == Some(fund_value(FundType::Int))
        }));
    }

    #[test]
    fn lowers_relational_expression_as_condition() {
        let model = analyze_source("PROC Main() BYTE x IF x < 10 THEN x=1 FI RETURN");
        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Condition && expr.ty == Some(fund_value(FundType::Byte))
        }));
    }

    #[test]
    fn lowers_builtin_function_call_type() {
        let model = analyze_source("BYTE FUNC F() RETURN(Peek(764))");
        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Value && expr.ty == Some(fund_value(FundType::Byte))
        }));
    }

    #[test]
    fn lowers_action_runtime_function_types() {
        let model = analyze_source(
            "PROC Main() BYTE b CARD c INT i CHAR ch b=Rand(10) c=PeekC(88) i=ValI(\"1\") ch=GetD(7) RETURN",
        );
        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Value && expr.ty == Some(fund_value(FundType::Byte))
        }));
        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Value && expr.ty == Some(fund_value(FundType::Card))
        }));
        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Value && expr.ty == Some(fund_value(FundType::Int))
        }));
        assert!(model.expression_observations.iter().any(|expr| {
            expr.class == ExprClass::Value && expr.ty == Some(fund_value(FundType::Char))
        }));
    }

    #[test]
    fn lowers_semantic_ir_expression_shapes() {
        let (program, model) = analyze_program_source(
            "BYTE a,b BYTE FUNC Same(BYTE x,y) IF x = y THEN RETURN(1) FI RETURN(0)",
        );
        let ir = ir::lower_program(&program, &model);
        let routine = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("routine");

        assert_eq!(routine.params.len(), 2);
        let ir::SemStmt::If { branches, .. } = &routine.body[0] else {
            panic!("expected IF");
        };
        let condition = &branches[0].condition;

        assert_eq!(condition.kind, ir::SemConditionKind::Compare);
        assert_eq!(condition.expr.class, ir::SemExprClass::Condition);
        assert_eq!(condition.expr.ty, fund_value(FundType::Byte));
        assert!(matches!(
            condition.expr.kind,
            ir::SemExprKind::Binary {
                op: BinaryOp::Eq,
                ..
            }
        ));

        let dump = ir::format_program(&ir);
        assert!(dump.contains("routine Byte FUNC Same"));
        assert!(dump.contains("when Compare (x:Byte:LValue"));
        assert!(dump.contains("return 1:Byte:Value"));
        assert!(dump.contains("facts=w1,Unsigned"));
    }

    #[test]
    fn lowers_semantic_ir_call_arguments_left_to_right() {
        let (program, model) =
            analyze_program_source("PROC P(BYTE x,y) RETURN PROC Main() BYTE a P(a, a+1) RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .filter_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .find(|routine| routine.symbol.name == "Main")
            .expect("Main");
        let ir::SemStmt::Call { call, .. } = &main.body[0] else {
            panic!("expected call");
        };

        assert_eq!(call.args.len(), 2);
        assert!(
            call.args[0].eval_order < call.args[1].eval_order,
            "call arguments should retain left-to-right evaluation order"
        );
        assert!(matches!(call.callee, ir::SemCallable::User(_)));
    }

    #[test]
    fn semantic_ir_widens_expected_word_unary_negation_before_negating() {
        let (program, model) = analyze_program_source(
            "PROC P(INT v) RETURN PROC Main() BYTE x INT s s=-1 s=-x P(-x) RETURN",
        );
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .filter_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .next()
            .expect("Main");

        let neg_values: Vec<_> = main
            .body
            .iter()
            .filter_map(|stmt| match stmt {
                ir::SemStmt::Assign { value, .. } => Some(value),
                ir::SemStmt::Call { call, .. } => call.args.first(),
                _ => None,
            })
            .collect();
        assert_eq!(neg_values.len(), 3);

        for value in neg_values {
            assert_eq!(value.ty, fund_value(FundType::Int));
            let ir::SemExprKind::Unary {
                op: UnaryOp::Neg,
                expr,
            } = &value.kind
            else {
                panic!("expected unary negation, got {value:?}");
            };
            let ir::SemExprKind::Cast { ty, .. } = &expr.kind else {
                panic!("expected widened operand cast before negation, got {expr:?}");
            };
            assert_eq!(ty, &fund_value(FundType::Int));
        }
    }

    #[test]
    fn semantic_ir_widens_expected_word_binary_arithmetic_before_evaluating() {
        let (program, model) = analyze_program_source(
            "CARD n PROC Take(CARD v) RETURN PROC Main() n=40*90 Take(40*90) RETURN",
        );
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .expect("Main");

        let values: Vec<_> = main
            .body
            .iter()
            .filter_map(|stmt| match stmt {
                ir::SemStmt::Assign { value, .. } => Some(value),
                ir::SemStmt::Call { call, .. } => call.args.first(),
                _ => None,
            })
            .collect();
        assert_eq!(values.len(), 2);

        for value in values {
            assert_eq!(value.ty, fund_value(FundType::Card));
            let ir::SemExprKind::Binary {
                op: BinaryOp::Mul,
                left,
                right,
            } = &value.kind
            else {
                panic!("expected word multiplication, got {value:?}");
            };
            for operand in [left, right] {
                assert_eq!(operand.ty, fund_value(FundType::Byte));
            }
        }
    }

    #[test]
    fn semantic_ir_array_decay_marks_global_local_and_parameter_origins() {
        let (program, model) = analyze_program_source(
            "BYTE ARRAY global(4) PROC Take(BYTE POINTER p) RETURN \
             PROC Main(BYTE ARRAY param) BYTE ARRAY local(4) BYTE POINTER p \
             p=global p=local p=param Take(global) RETURN",
        );
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .filter_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .next()
            .expect("Main");

        let origins: Vec<_> = main
            .body
            .iter()
            .filter_map(|stmt| match stmt {
                ir::SemStmt::Assign { value, .. } => match &value.kind {
                    ir::SemExprKind::ArrayDecay(decay) => Some(decay.origin),
                    _ => None,
                },
                ir::SemStmt::Call { call, .. } => match &call.args[0].kind {
                    ir::SemExprKind::ArrayDecay(decay) => Some(decay.origin),
                    _ => None,
                },
                _ => None,
            })
            .collect();

        assert_eq!(
            origins,
            vec![
                ir::SemArrayOrigin::Global,
                ir::SemArrayOrigin::Local,
                ir::SemArrayOrigin::Parameter,
                ir::SemArrayOrigin::Global,
            ]
        );
    }

    #[test]
    fn semantic_ir_array_decay_has_pointer_type() {
        let (program, model) =
            analyze_program_source("BYTE ARRAY data(4) BYTE POINTER p PROC Main() p=data RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .expect("Main");
        let ir::SemStmt::Assign { value, .. } = &main.body[0] else {
            panic!("expected assignment");
        };
        let ir::SemExprKind::ArrayDecay(decay) = &value.kind else {
            panic!("expected array decay");
        };

        assert_eq!(value.ty, ValueType::pointer_to(fund_value(FundType::Byte)));
        assert_eq!(decay.element_type, fund_value(FundType::Byte));
        assert_eq!(
            decay.pointer_type,
            ValueType::pointer_to(fund_value(FundType::Byte))
        );
    }

    #[test]
    fn semantic_ir_array_decay_uses_expected_fundamental_pointer_type() {
        let (program, model) =
            analyze_program_source("BYTE ARRAY data(4) CARD POINTER p PROC Main() p=data RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .expect("Main");
        let ir::SemStmt::Assign { value, .. } = &main.body[0] else {
            panic!("expected assignment");
        };
        let ir::SemExprKind::ArrayDecay(decay) = &value.kind else {
            panic!("expected array decay");
        };

        assert_eq!(value.ty, ValueType::pointer_to(fund_value(FundType::Card)));
        assert_eq!(decay.element_type, fund_value(FundType::Byte));
        assert_eq!(
            decay.pointer_type,
            ValueType::pointer_to(fund_value(FundType::Card))
        );
    }

    #[test]
    fn semantic_ir_record_assignment_uses_implicit_address() {
        let (program, model) = analyze_program_source(
            "TYPE Pair=[BYTE tag] Pair rec Pair POINTER p PROC Main() p=rec RETURN",
        );
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .expect("Main");
        let ir::SemStmt::Assign { value, .. } = &main.body[0] else {
            panic!("expected assignment");
        };
        let ir::SemExprKind::ImplicitAddressOf(address) = &value.kind else {
            panic!("expected implicit address-of");
        };

        assert_eq!(
            address.reason,
            ir::SemImplicitAddressReason::RecordToPointer
        );
        assert_eq!(
            address.pointer_type,
            ValueType::pointer_to(ValueType {
                base: ValueTypeBase::Named("Pair".to_string()),
                pointer: false,
            })
        );
    }

    #[test]
    fn semantic_ir_exposes_type_facts_for_exprs_and_places() {
        let (program, model) = analyze_program_source(
            "TYPE Pair=[BYTE tag CARD word] Pair rec Pair POINTER p PROC Main() p=rec p.word=1 RETURN",
        );
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .expect("Main");

        let ir::SemStmt::Assign { value, .. } = &main.body[0] else {
            panic!("expected pointer assignment");
        };
        let value_facts = value.type_facts();
        assert_eq!(value_facts.width, Some(2));
        assert!(value_facts.is_pointer);
        assert_eq!(value_facts.record_base.as_deref(), Some("Pair"));
        assert_eq!(
            value_facts.pointee,
            Some(ValueType {
                base: ValueTypeBase::Named("Pair".to_string()),
                pointer: false,
            })
        );
        assert_eq!(value_facts.pointee_width, None);

        let ir::SemStmt::Assign { target, .. } = &main.body[1] else {
            panic!("expected field assignment");
        };
        let target_facts = target.type_facts();
        assert_eq!(target_facts.width, Some(2));
        assert_eq!(
            target_facts.signedness,
            Some(crate::semantic::ScalarSignedness::Unsigned)
        );
        assert!(!target_facts.is_pointer);

        let dump = ir::format_program(&ir);
        assert!(dump.contains("facts=w2,ptr,to=Pair,record=Pair"));
        assert!(dump.contains("facts=w2,Unsigned"));
    }

    #[test]
    fn semantic_ir_record_call_argument_uses_implicit_address() {
        let (program, model) = analyze_program_source(
            "TYPE Pair=[BYTE tag] PROC Touch(Pair POINTER p) RETURN Pair rec PROC Main() Touch(rec) RETURN",
        );
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .expect("Main");
        let ir::SemStmt::Call { call, .. } = &main.body[0] else {
            panic!("expected call");
        };
        let ir::SemExprKind::ImplicitAddressOf(address) = &call.args[0].kind else {
            panic!("expected implicit address-of");
        };

        assert_eq!(
            address.reason,
            ir::SemImplicitAddressReason::RecordToPointer
        );
    }

    #[test]
    fn semantic_ir_field_refs_carry_field_identity() {
        let (program, model) = analyze_program_source(
            "TYPE Pair=[BYTE tag CARD word] Pair rec BYTE b CARD c PROC Main() b=rec.tag c=rec.word RETURN",
        );
        let pair = model
            .symbols
            .lookup(model.symbols.global_scope(), "Pair")
            .expect("Pair type");
        let tag = model.field_lookup["PAIR"]["TAG"];
        let word = model.field_lookup["PAIR"]["WORD"];
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .expect("Main");

        let ir::SemStmt::Assign {
            value: tag_value, ..
        } = &main.body[0]
        else {
            panic!("expected tag assignment");
        };
        let ir::SemExprKind::LValue(tag_place) = &tag_value.kind else {
            panic!("expected tag lvalue");
        };
        let ir::SemLValueKind::Field { field: tag_ref, .. } = &tag_place.kind else {
            panic!("expected tag field");
        };

        assert_eq!(tag_ref.id, Some(tag));
        assert_eq!(tag_ref.owner, Some(pair));
        assert_eq!(tag_ref.offset, Some(0));
        assert_eq!(tag_ref.ty, fund_value(FundType::Byte));

        let ir::SemStmt::Assign {
            value: word_value, ..
        } = &main.body[1]
        else {
            panic!("expected word assignment");
        };
        let ir::SemExprKind::LValue(word_place) = &word_value.kind else {
            panic!("expected word lvalue");
        };
        let ir::SemLValueKind::Field {
            field: word_ref, ..
        } = &word_place.kind
        else {
            panic!("expected word field");
        };

        assert_eq!(word_ref.id, Some(word));
        assert_eq!(word_ref.owner, Some(pair));
        assert_eq!(word_ref.offset, Some(1));
        assert_eq!(word_ref.ty, fund_value(FundType::Card));
    }

    #[test]
    fn semantic_ir_non_error_values_and_places_have_types() {
        let (program, model) = analyze_program_source(
            "TYPE Pair=[BYTE tag CARD word] Pair rec BYTE ARRAY bytes(4) CARD ARRAY words(2) \
             DEFINE STRING=\"CHAR ARRAY\" \
             BYTE b CARD c BYTE POINTER bp CARD POINTER cp \
             PROC Touch(BYTE x CARD y BYTE POINTER p) RETURN PROC Text(STRING s) RETURN \
             BYTE FUNC Pick(BYTE i) RETURN(bytes(i)) \
             PROC Main(BYTE ARRAY param) \
             b=1 c=2 rec.tag=b rec.word=c bp=bytes cp=words bp=param \
             Touch(rec.tag, rec.word, bytes) Text(\"OK\") b=Pick(0) \
             IF rec.tag = b THEN b=bytes(0) FI \
             WHILE b DO b==-1 OD \
             RETURN",
        );
        let ir = ir::lower_program(&program, &model);

        assert_semir_types_complete(&ir);
    }

    #[test]
    fn semantic_analyzer_non_unknown_values_have_types() {
        let model = analyze_source(
            "TYPE Pair=[BYTE tag CARD word] Pair rec BYTE ARRAY bytes(4) CARD ARRAY words(2) \
             DEFINE STRING=\"CHAR ARRAY\" \
             BYTE b CARD c BYTE POINTER bp CARD POINTER cp STRING text(0)=\"OK\" \
             PROC Touch(BYTE x CARD y BYTE POINTER p STRING s) RETURN \
             BYTE FUNC Pick(BYTE i) RETURN(bytes(i)) \
             PROC Main(BYTE ARRAY param) \
             b=1 c=2 rec.tag=b rec.word=c bp=bytes cp=words bp=param \
             Touch(rec.tag, rec.word, bytes, \"HI\") b=Pick(0) b=text(1) \
             IF rec.tag = b THEN b=bytes(0) FI \
             WHILE b DO b==-1 OD \
             RETURN",
        );

        assert_analyzer_value_types_complete(&model);
    }

    #[test]
    fn typed_casts_allow_explicit_pointer_reinterpretation() {
        analyze_source(
            "BYTE FUNC First(CHAR POINTER text) RETURN(text^) \
             PROC Main(BYTE POINTER menu) BYTE c c=First(CHAR POINTER(menu)) RETURN",
        );
    }

    #[test]
    fn typed_casts_allow_routine_data_label_addresses() {
        analyze_source(
            "BYTE FUNC PopUp(BYTE POINTER menu BYTE x BYTE y) RETURN(menu^) \
             PROC delcancel=*() [\"Delete\" 'D \"Cancel\" 'C 0] \
             PROC Main() BYTE c c=PopUp(BYTE POINTER(@delcancel), 1, 4) RETURN",
        );
    }

    #[test]
    fn function_pointers_are_callable_values() {
        analyze_source(
            "PROC Target() RETURN \
             BYTE FUNC Key() RETURN(1) \
             PROC Main() PROC POINTER p BYTE FUNC POINTER key BYTE b \
             p=@Target key=@Key p() b=key() RETURN",
        );
    }

    #[test]
    fn semantic_ir_routine_headers_carry_signatures() {
        let (program, model) = analyze_program_source(
            "DEFINE STRING=\"CHAR ARRAY\" PROC P(BYTE x STRING s) RETURN \
             CARD FUNC F(BYTE x CARD y) RETURN(y)",
        );
        let ir = ir::lower_program(&program, &model);
        let routines: Vec<_> = ir.modules[0]
            .items
            .iter()
            .filter_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .collect();

        assert_eq!(routines[0].signature.kind, RoutineKind::Proc);
        assert_eq!(
            routines[0].signature.params,
            vec![
                fund_value(FundType::Byte),
                ValueType::pointer_to(fund_value(FundType::Char))
            ]
        );
        assert_eq!(routines[0].signature.return_type, None);

        assert_eq!(
            routines[1].signature.kind,
            RoutineKind::Func {
                return_type: FundType::Card
            }
        );
        assert_eq!(
            routines[1].signature.params,
            vec![fund_value(FundType::Byte), fund_value(FundType::Card)]
        );
        assert_eq!(routines[1].signature.return_type, Some(FundType::Card));
        assert_semir_types_complete(&ir);
    }

    #[test]
    fn lowers_semantic_ir_statement_conditions_by_kind() {
        let (program, model) =
            analyze_program_source("PROC Main() BYTE x WHILE x DO IF x=1 THEN EXIT FI OD RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("Main");
        let ir::SemStmt::While {
            condition, body, ..
        } = &main.body[0]
        else {
            panic!("expected WHILE");
        };

        assert_eq!(condition.kind, ir::SemConditionKind::NonZeroValue);
        let ir::SemStmt::If { branches, .. } = &body[0] else {
            panic!("expected IF");
        };
        assert_eq!(branches[0].condition.kind, ir::SemConditionKind::Compare);
    }

    #[test]
    fn lowers_semantic_ir_constant_condition_shapes() {
        let (program, model) =
            analyze_program_source("PROC Main() IF 0 THEN RETURN FI IF 1 THEN RETURN FI RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("Main");

        let ir::SemStmt::If { branches, .. } = &main.body[0] else {
            panic!("expected first IF");
        };
        assert_eq!(
            branches[0].condition.kind,
            ir::SemConditionKind::ConstantFalse
        );

        let ir::SemStmt::If { branches, .. } = &main.body[1] else {
            panic!("expected second IF");
        };
        assert_eq!(
            branches[0].condition.kind,
            ir::SemConditionKind::ConstantTrue
        );
    }

    #[test]
    fn lowers_semantic_ir_statement_define_entries_individually() {
        let (program, model) = analyze_program_source("PROC Main() DEFINE A=\"1\", B=\"2\" RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("Main");

        assert!(matches!(&main.body[0], ir::SemStmt::Define(define) if define.symbol.name == "A"));
        assert!(matches!(&main.body[1], ir::SemStmt::Define(define) if define.symbol.name == "B"));
        assert!(matches!(&main.body[2], ir::SemStmt::Return { .. }));
    }

    #[test]
    fn accepts_assignment_to_routine_names() {
        analyze_source("PROC A() RETURN PROC B() RETURN PROC Main() A=B RETURN");
    }

    #[test]
    fn semantic_ir_lvalues_carry_place_access() {
        let (program, model) =
            analyze_program_source("PROC POINTER p PROC B() RETURN PROC Main() p=@B RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
                _ => None,
            })
            .expect("Main");
        let ir::SemStmt::Assign { target, .. } = &main.body[0] else {
            panic!("expected assignment");
        };

        assert_eq!(target.access, subject::PlaceAccess::Assignable);
    }

    #[test]
    fn semantic_ir_routines_carry_control_flow_facts() {
        let (program, model) = analyze_program_source(
            "PROC P() BYTE x x=1 BYTE FUNC F(BYTE x) IF x THEN RETURN(1) ELSE RETURN(0) FI PROC L() WHILE 1 DO EXIT OD RETURN",
        );
        let ir = ir::lower_program(&program, &model);
        let routines: Vec<_> = ir.modules[0]
            .items
            .iter()
            .filter_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .collect();

        assert!(!routines[0].control_flow.always_returns);
        assert!(routines[0].control_flow.may_fall_through);
        assert!(routines[1].control_flow.always_returns);
        assert!(!routines[1].control_flow.may_fall_through);
        assert!(routines[1].control_flow.contains_return);
        assert!(!routines[1].control_flow.contains_loop);
        assert!(routines[2].control_flow.contains_return);
        assert!(routines[2].control_flow.contains_exit);
        assert!(routines[2].control_flow.contains_loop);
        assert_eq!(routines[2].control_flow.max_loop_depth, 1);

        let dump = ir::format_program(&ir);
        assert!(dump.contains(
            "flow always_returns=true may_fall_through=false contains_return=true contains_exit=true contains_loop=true max_loop_depth=1"
        ));
    }

    #[test]
    fn semantic_ir_statement_flow_facts_are_available_without_ast() {
        let (program, model) =
            analyze_program_source("PROC Main() WHILE 1 DO IF 1 THEN EXIT FI OD RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("Main");

        let facts = ir::statement_list_flow_facts(&main.body);
        assert!(facts.always_returns);
        assert!(!facts.may_continue);
        assert!(facts.may_return);
        assert!(facts.may_exit_loop);
        assert!(facts.contains_loop);
        assert_eq!(facts.max_loop_depth, 1);

        let while_facts = ir::statement_flow_facts(&main.body[0]);
        assert!(while_facts.may_continue);
        assert!(while_facts.may_exit_loop);
        assert!(while_facts.contains_loop);
    }

    #[test]
    fn semantic_ir_statement_flow_facts_prune_constant_if_branches() {
        let (program, model) = analyze_program_source(
            "PROC Main() BYTE x IF 0 THEN RETURN ELSE x=1 FI IF 1 THEN RETURN ELSE x=2 FI",
        );
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("Main");

        let first_if = ir::statement_flow_facts(&main.body[0]);
        assert!(first_if.may_continue);
        assert!(!first_if.may_return);
        assert!(!first_if.always_returns);

        let second_if = ir::statement_flow_facts(&main.body[1]);
        assert!(!second_if.may_continue);
        assert!(second_if.may_return);
        assert!(second_if.always_returns);
    }

    #[test]
    fn semantic_ir_statement_flow_annotations_mark_unreachable_statements() {
        let (program, model) = analyze_program_source("PROC Main() IF 1 THEN RETURN FI RETURN");
        let ir = ir::lower_program(&program, &model);
        let main = ir.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                ir::SemItem::Routine(routine) => Some(routine),
                _ => None,
            })
            .expect("Main");

        let annotations = ir::statement_list_flow_annotations(&main.body);
        assert_eq!(annotations.len(), 2);
        assert_eq!(annotations[0].index, 0);
        assert!(annotations[0].reachable);
        assert!(annotations[0].facts.always_returns);
        assert_eq!(annotations[1].index, 1);
        assert!(!annotations[1].reachable);

        let dump = ir::format_program(&ir);
        assert!(dump.contains("unreachable:\n          return"));
    }

    #[test]
    fn subject_expect_expr_loads_places() {
        let (program, mut analyzer) = analyzed_state("BYTE x PROC Main() BYTE y y=x RETURN");
        let routine_scope = analyzer.routine_scopes[0].scope;
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };
        let Stmt::Assign { value, .. } = &routine.body[0] else {
            panic!("expected assignment");
        };

        let expr = analyzer.expect_expr(routine_scope, value, value.span);

        let subject::SemExprKind::Load(place) = expr.kind else {
            panic!("expected place load");
        };
        assert!(matches!(place.kind, subject::SemPlaceKind::Symbol(_)));
    }

    #[test]
    fn subject_call_index_syntax_can_be_a_place() {
        let (program, mut analyzer) = analyzed_state("BYTE ARRAY a(4) PROC Main() a(0)=1 RETURN");
        let routine_scope = analyzer.routine_scopes[0].scope;
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };
        let Stmt::Assign { target, .. } = &routine.body[0] else {
            panic!("expected assignment");
        };

        let place = analyzer.expect_place(routine_scope, target, target.span);

        assert!(matches!(place.kind, subject::SemPlaceKind::Index { .. }));
    }

    #[test]
    fn subject_field_places_carry_field_identity() {
        let (program, mut analyzer) =
            analyzed_state("TYPE Pair=[BYTE tag CARD word] Pair rec PROC Main() rec.word=1 RETURN");
        let pair = analyzer
            .symbols
            .lookup(analyzer.symbols.global_scope(), "Pair")
            .expect("Pair type");
        let word = analyzer.field_lookup["PAIR"]["WORD"];
        let routine_scope = analyzer.routine_scopes[0].scope;
        let Item::Routine(routine) = &program.modules[0].items[2] else {
            panic!("expected routine");
        };
        let Stmt::Assign { target, .. } = &routine.body[0] else {
            panic!("expected assignment");
        };

        let place = analyzer.expect_place(routine_scope, target, target.span);
        let subject::SemPlaceKind::Field { field, .. } = &place.kind else {
            panic!("expected field place");
        };

        assert_eq!(field.id, Some(word));
        assert_eq!(field.owner, Some(pair));
        assert_eq!(field.offset, Some(1));
        assert_eq!(field.ty, fund_value(FundType::Card));
    }

    #[test]
    fn subject_function_call_stays_callable_in_expr_context() {
        let (program, mut analyzer) =
            analyzed_state("BYTE FUNC F() RETURN(1) PROC Main() BYTE y y=F() RETURN");
        let routine_scope = analyzer.routine_scopes[1].scope;
        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };
        let Stmt::Assign { value, .. } = &routine.body[0] else {
            panic!("expected assignment");
        };

        let expr = analyzer.expect_expr(routine_scope, value, value.span);

        assert!(matches!(expr.kind, subject::SemExprKind::Call { .. }));
        assert_eq!(expr.ty, fund_value(FundType::Byte));
    }

    #[test]
    fn rejects_user_proc_call_as_value() {
        let err = analyze_source_err("PROC P() RETURN PROC Main() BYTE b b=P() RETURN");
        assert!(
            err.iter().any(|diagnostic| diagnostic
                .message
                .contains("procedure call cannot be used as a value")),
            "expected proc-call-as-value diagnostic, got {err:?}"
        );
    }

    #[test]
    fn rejects_user_proc_call_as_nested_value() {
        let err = analyze_source_err("PROC P() RETURN PROC Main() BYTE b b=P()+1 RETURN");
        assert!(
            err.iter().any(|diagnostic| diagnostic
                .message
                .contains("procedure call cannot be used as a value")),
            "expected proc-call-as-value diagnostic, got {err:?}"
        );
    }

    #[test]
    fn nested_call_value_context_still_validates_arguments() {
        let err = analyze_source_err(
            "BYTE FUNC F(BYTE x) RETURN(x) PROC Main() BYTE b b=F(\"BAD\")+1 RETURN",
        );
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.message.contains("`F` argument 1 expects")),
            "expected nested call argument diagnostic, got {err:?}"
        );
    }

    #[test]
    fn proc_call_as_value_keeps_explicit_error_type() {
        let tokens = tokenize("PROC P() RETURN PROC Main() BYTE b b=P() RETURN").unwrap();
        let program = parse(&tokens).unwrap();
        let mut analyzer = Analyzer::new();
        analyzer.seed_builtins();
        analyzer.analyze_program(&program);

        let Item::Routine(routine) = &program.modules[0].items[1] else {
            panic!("expected routine");
        };
        let Stmt::Assign { value, .. } = &routine.body[0] else {
            panic!("expected assignment");
        };
        let typed = analyzer
            .expression_observations
            .iter()
            .rev()
            .find(|typed| typed.span == value.span)
            .expect("typed call expression");

        assert_eq!(typed.class, ExprClass::Value);
        assert_eq!(typed.ty, Some(ValueType::error()));
        assert!(analyzer.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("procedure call cannot be used as a value")
        }));
    }

    #[test]
    fn rejects_builtin_proc_call_as_value() {
        let err = analyze_source_err("BYTE b PROC Main() b=PrintE(\"X\") RETURN");
        assert!(
            err.iter().any(|diagnostic| diagnostic
                .message
                .contains("procedure call cannot be used as a value")),
            "expected proc-call-as-value diagnostic, got {err:?}"
        );
    }

    #[test]
    fn rejects_known_incompatible_return_type() {
        let err = analyze_source_err("BYTE FUNC F() RETURN(256)");
        assert!(err[0].message.contains("cannot return"));
    }

    #[test]
    fn allows_known_widening_return_type() {
        analyze_source("CARD FUNC F() RETURN(255)");
    }

    fn analyze_source(source: &str) -> SemanticModel {
        analyze_program_source(source).1
    }

    fn analyze_program_source(source: &str) -> (Program, SemanticModel) {
        let tokens = tokenize(source).unwrap();
        let program = parse(&tokens).unwrap();
        let model = analyze(&program).unwrap();
        (program, model)
    }

    fn routine_by_name<'a>(program: &'a Program, name: &str) -> &'a Routine {
        program.modules[0]
            .items
            .iter()
            .find_map(|item| match item {
                Item::Routine(routine) if routine.name.eq_ignore_ascii_case(name) => Some(routine),
                _ => None,
            })
            .expect("routine")
    }

    fn analyzed_state(source: &str) -> (Program, Analyzer) {
        let tokens = tokenize(source).unwrap();
        let program = parse(&tokens).unwrap();
        let mut analyzer = Analyzer::new();
        analyzer.seed_builtins();
        analyzer.analyze_program(&program);
        assert!(
            analyzer.diagnostics.is_empty(),
            "{:?}",
            analyzer.diagnostics
        );
        (program, analyzer)
    }

    fn analyze_source_err(source: &str) -> Vec<Diagnostic> {
        let tokens = tokenize(source).unwrap();
        let program = parse(&tokens).unwrap();
        analyze(&program).unwrap_err()
    }

    fn assert_analyzer_value_types_complete(model: &SemanticModel) {
        for expr in &model.expression_observations {
            match expr.class {
                ExprClass::Value | ExprClass::LValue | ExprClass::Condition => {
                    assert!(
                        expr.ty.is_some(),
                        "semantic analyzer {:?} expression lacks type at {:?}",
                        expr.class,
                        expr.span
                    );
                }
                ExprClass::Unknown | ExprClass::Callable => {}
            }
        }
    }

    fn assert_semir_types_complete(program: &ir::SemProgram) {
        for module in &program.modules {
            for item in &module.items {
                match item {
                    ir::SemItem::Declaration(decl) => assert_semir_declaration_types(decl),
                    ir::SemItem::Routine(routine) => {
                        assert_semir_routine_header_types(routine);
                        for local in &routine.locals {
                            assert_semir_declaration_types(local);
                        }
                        assert_semir_stmt_list_types(&routine.body);
                    }
                    ir::SemItem::Statement(stmt) => assert_semir_stmt_types(stmt),
                    ir::SemItem::Define(_)
                    | ir::SemItem::Include(_)
                    | ir::SemItem::Set(_)
                    | ir::SemItem::Unsupported { .. } => {}
                }
            }
        }
    }

    fn assert_semir_routine_header_types(routine: &ir::SemRoutine) {
        let param_types: Vec<_> = routine
            .params
            .iter()
            .map(|param| match param.storage {
                ir::SemParamStorage::Value => param.ty.value.clone(),
                ir::SemParamStorage::Array => ValueType::pointer_to(param.ty.value.clone()),
            })
            .collect();
        for param in &routine.params {
            match param.storage {
                ir::SemParamStorage::Value => {
                    assert_eq!(
                        param.array_type, None,
                        "scalar SemIR param should not carry an array type for {}",
                        param.symbol.name
                    );
                }
                ir::SemParamStorage::Array => {
                    let array_type = param.array_type.as_ref().unwrap_or_else(|| {
                        panic!(
                            "array SemIR param lacks array type for {}",
                            param.symbol.name
                        )
                    });
                    assert_eq!(*array_type.element, param.ty.value);
                    assert_eq!(
                        array_type.pointer_type(),
                        ValueType::pointer_to(param.ty.value.clone())
                    );
                }
            }
        }
        assert_eq!(
            routine.signature.params, param_types,
            "SemIR routine signature params disagree with lowered params for {}",
            routine.symbol.name
        );
        assert_eq!(
            routine.callable_type.params, param_types,
            "SemIR routine callable params disagree with lowered params for {}",
            routine.symbol.name
        );

        match (&routine.signature.kind, routine.signature.return_type) {
            (RoutineKind::Proc, None) => {
                assert_eq!(routine.symbol.class, SymbolClass::Proc);
                assert_eq!(routine.symbol.ty, None);
                assert_eq!(routine.callable_type.kind, RoutineKind::Proc);
                assert_eq!(routine.callable_type.return_type, None);
            }
            (RoutineKind::Func { return_type }, Some(signature_return)) => {
                assert_eq!(*return_type, signature_return);
                assert_eq!(routine.symbol.class, SymbolClass::Func);
                assert_eq!(routine.symbol.ty, Some(fund_value(*return_type)));
                assert_eq!(routine.callable_type.kind, routine.signature.kind);
                assert_eq!(
                    routine.callable_type.return_type,
                    Some(fund_value(*return_type))
                );
            }
            _ => panic!(
                "SemIR routine signature return type is inconsistent for {}",
                routine.symbol.name
            ),
        }
    }

    fn assert_semir_declaration_types(decl: &ir::SemDeclaration) {
        if let ir::SemDeclarationStorage::Array { array_type, .. } = &decl.storage {
            assert_eq!(
                *array_type.element, decl.ty.value,
                "SemIR array declaration element type disagrees with declaration type for {}",
                decl.symbol.name
            );
            assert_eq!(
                array_type.pointer_type(),
                ValueType::pointer_to(decl.ty.value.clone()),
                "SemIR array declaration pointer type disagrees with declaration type for {}",
                decl.symbol.name
            );
        }
        assert_semir_storage_types(&decl.storage);
        if let Some(initializer) = &decl.initializer {
            assert_semir_value_expr_typed(initializer);
        }
    }

    fn assert_semir_storage_types(storage: &ir::SemDeclarationStorage) {
        match storage {
            ir::SemDeclarationStorage::Scalar => {}
            ir::SemDeclarationStorage::Array { length, .. } => {
                if let Some(length) = length {
                    assert_semir_value_expr_typed(length);
                }
            }
            ir::SemDeclarationStorage::Type {
                record_type,
                fields,
            }
            | ir::SemDeclarationStorage::Record {
                record_type,
                fields,
            } => {
                assert_eq!(
                    record_type.fields.len(),
                    fields.len(),
                    "SemIR record type field count disagrees with lowered fields for {}",
                    record_type.name
                );
                for field in fields {
                    let typed_field = record_type
                        .field(&field.name)
                        .unwrap_or_else(|| panic!("missing field {} in RecordType", field.name));
                    assert_eq!(typed_field.id, field.id);
                    assert_eq!(typed_field.ty, field.ty.value);
                    assert_eq!(Some(typed_field.offset), field.offset);
                    assert_semir_storage_types(&field.storage);
                }
            }
        }
    }

    fn assert_semir_stmt_list_types(stmts: &[ir::SemStmt]) {
        for stmt in stmts {
            assert_semir_stmt_types(stmt);
        }
    }

    fn assert_semir_stmt_types(stmt: &ir::SemStmt) {
        match stmt {
            ir::SemStmt::Define(_)
            | ir::SemStmt::Exit { .. }
            | ir::SemStmt::MachineBlock { .. } => {}
            ir::SemStmt::Return { value, .. } => {
                if let Some(value) = value {
                    assert_semir_value_expr_typed(value);
                }
            }
            ir::SemStmt::Assign { target, value, .. } => {
                assert_semir_lvalue_typed(target);
                assert_semir_value_expr_typed(value);
            }
            ir::SemStmt::CompoundAssign { target, value, .. } => {
                assert_semir_lvalue_typed(target);
                assert_semir_value_expr_typed(value);
            }
            ir::SemStmt::Call { call, .. } => assert_semir_call_types(call),
            ir::SemStmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    assert_semir_condition_types(&branch.condition);
                    assert_semir_stmt_list_types(&branch.body);
                }
                assert_semir_stmt_list_types(else_body);
            }
            ir::SemStmt::While {
                condition, body, ..
            } => {
                assert_semir_condition_types(condition);
                assert_semir_stmt_list_types(body);
            }
            ir::SemStmt::DoUntil {
                body, condition, ..
            } => {
                assert_semir_stmt_list_types(body);
                if let Some(condition) = condition {
                    assert_semir_condition_types(condition);
                }
            }
            ir::SemStmt::For {
                target,
                start,
                end,
                step,
                body,
                ..
            } => {
                assert_semir_lvalue_typed(target);
                assert_semir_value_expr_typed(start);
                assert_semir_value_expr_typed(end);
                if let Some(step) = step {
                    assert_semir_value_expr_typed(step);
                }
                assert_semir_stmt_list_types(body);
            }
            ir::SemStmt::Unsupported { .. } => {}
        }
    }

    fn assert_semir_condition_types(condition: &ir::SemCondition) {
        assert_ne!(condition.kind, ir::SemConditionKind::Unknown);
        assert_semir_value_expr_typed(&condition.expr);
    }

    fn assert_semir_call_types(call: &ir::SemCall) {
        assert_eq!(
            call.return_type, call.callable_type.return_type,
            "SemIR call return type disagrees with callable type for {:?}",
            call.callee
        );
        if !call.callable_type.params.is_empty() {
            assert_eq!(
                call.args.len(),
                call.callable_type.params.len(),
                "SemIR call argument count disagrees with callable type for {:?}",
                call.callee
            );
        }
        for arg in &call.args {
            assert_semir_value_expr_typed(arg);
        }
    }

    fn assert_semir_value_expr_typed(expr: &ir::SemExpr) {
        match &expr.kind {
            ir::SemExprKind::Missing
            | ir::SemExprKind::Raw(_)
            | ir::SemExprKind::UnresolvedName(_) => return,
            ir::SemExprKind::Call(call) if call.return_type.is_none() => {
                assert_semir_call_types(call);
                return;
            }
            _ => {}
        }

        assert!(
            !expr.ty.is_error(),
            "SemIR expression has error type: {:?}",
            expr.kind
        );

        match &expr.kind {
            ir::SemExprKind::Missing
            | ir::SemExprKind::Raw(_)
            | ir::SemExprKind::UnresolvedName(_)
            | ir::SemExprKind::CurrentLocation
            | ir::SemExprKind::Literal(_) => {}
            ir::SemExprKind::Symbol(symbol) => {
                if let Some(symbol_ty) = &symbol.ty {
                    assert_eq!(&expr.ty, symbol_ty);
                }
            }
            ir::SemExprKind::LValue(lvalue) => {
                assert_eq!(expr.ty, lvalue.ty);
                assert_semir_lvalue_typed(lvalue);
            }
            ir::SemExprKind::AddressOf(lvalue) => {
                assert_eq!(expr.ty, ValueType::pointer_to(lvalue.ty.clone()));
                assert_semir_lvalue_typed(lvalue);
            }
            ir::SemExprKind::AddressOfSymbol(_) => {
                assert!(expr.ty.as_callable_pointer().is_some());
            }
            ir::SemExprKind::ImplicitAddressOf(address) => {
                assert!(!address.pointer_type.is_error());
                assert_eq!(expr.ty, address.pointer_type);
                assert_eq!(address.pointer_type.pointee_type(), address.place.ty);
                match address.reason {
                    ir::SemImplicitAddressReason::RecordToPointer => {
                        assert!(address.place.ty.is_record());
                        assert!(address.pointer_type.is_record_pointer());
                        assert!(address.place.ty.same_record_family(&address.pointer_type));
                    }
                }
                assert_semir_lvalue_typed(&address.place);
            }
            ir::SemExprKind::ArrayDecay(decay) => {
                assert!(!decay.element_type.is_error());
                assert!(!decay.pointer_type.is_error());
                assert_eq!(expr.ty, decay.pointer_type);
                assert!(
                    decay.pointer_type == ValueType::pointer_to(decay.element_type.clone())
                        || decay.pointer_type.pointer
                            && matches!(decay.pointer_type.base, ValueTypeBase::Fund(_))
                            && matches!(decay.element_type.base, ValueTypeBase::Fund(_))
                );
                assert_eq!(decay.array.ty, decay.element_type);
                assert_semir_lvalue_typed(&decay.array);
            }
            ir::SemExprKind::Cast { ty, expr: inner } => {
                assert_eq!(&expr.ty, ty);
                assert_semir_value_expr_typed(inner);
            }
            ir::SemExprKind::Unary { expr, .. } => assert_semir_value_expr_typed(expr),
            ir::SemExprKind::Binary { left, right, .. } => {
                assert_semir_value_expr_typed(left);
                assert_semir_value_expr_typed(right);
            }
            ir::SemExprKind::Call(call) => {
                if let Some(return_type) = &call.return_type {
                    assert_eq!(&expr.ty, return_type);
                }
                assert_semir_call_types(call);
            }
        }
    }

    fn assert_semir_lvalue_typed(lvalue: &ir::SemLValue) {
        if matches!(lvalue.kind, ir::SemLValueKind::UnresolvedName(_)) {
            return;
        }
        assert!(
            !lvalue.ty.is_error(),
            "SemIR lvalue has error type: {:?}",
            lvalue.kind
        );

        match &lvalue.kind {
            ir::SemLValueKind::Symbol(symbol) => {
                if let Some(symbol_ty) = &symbol.ty {
                    assert_eq!(&lvalue.ty, symbol_ty);
                }
            }
            ir::SemLValueKind::UnresolvedName(_) => {}
            ir::SemLValueKind::Deref { pointer } => {
                assert!(pointer.ty.is_pointer());
                assert_eq!(lvalue.ty, pointer.ty.pointee_type());
                assert_semir_value_expr_typed(pointer);
            }
            ir::SemLValueKind::Index {
                base,
                index,
                element_type,
                ..
            } => {
                assert!(!element_type.is_error());
                assert_eq!(&lvalue.ty, element_type);
                assert_semir_value_expr_typed(base);
                assert_semir_value_expr_typed(index);
            }
            ir::SemLValueKind::Field { base, field } => {
                assert_eq!(lvalue.ty, field.ty);
                assert!(base.ty.has_record_base());
                assert_semir_lvalue_typed(base);
            }
        }
    }
}
