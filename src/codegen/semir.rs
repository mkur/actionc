use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::lexer::NumberLiteral;
use crate::runtime::Runtime;
use crate::semantic::ir::*;
use crate::semantic::{SymbolId, ValueType, ValueTypeBase};
use crate::source::Span;
use std::collections::{BTreeMap, HashMap};

pub(crate) fn semir_to_ast(program: &SemProgram) -> Result<Program, Vec<Diagnostic>> {
    let mut lowerer = SemIrAstLowerer {
        diagnostics: Vec::new(),
        type_link_names: module_type_link_names(program),
        external_addresses: None,
    };
    let program = lowerer.program(program);
    if lowerer.diagnostics.is_empty() {
        Ok(program)
    } else {
        Err(lowerer.diagnostics)
    }
}

pub(crate) fn semir_to_cart_ast(program: &SemProgram) -> Result<Program, Vec<Diagnostic>> {
    let addresses = cart_external_addresses(program)?;
    let mut lowerer = SemIrAstLowerer {
        diagnostics: Vec::new(),
        type_link_names: module_type_link_names(program),
        external_addresses: Some(&addresses),
    };
    let program = lowerer.program(program);
    if lowerer.diagnostics.is_empty() {
        Ok(program)
    } else {
        Err(lowerer.diagnostics)
    }
}

pub(crate) fn cart_external_addresses(
    program: &SemProgram,
) -> Result<HashMap<SymbolId, u16>, Vec<Diagnostic>> {
    let bindings = crate::runtime_bindings::parse_bindings(Runtime::ActionCart)?;
    let mut addresses = HashMap::new();
    for routine in program
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            SemItem::Routine(routine) if routine.is_external => Some(routine),
            _ => None,
        })
    {
        let key = crate::runtime_bindings::binding_key(&routine.symbol.qualified_name);
        match bindings.get(&key) {
            Some(crate::runtime_bindings::BindingTarget::Absolute(address)) => {
                addresses.insert(routine.symbol.id, *address);
            }
            Some(crate::runtime_bindings::BindingTarget::RuntimeRoutine { .. }) => {
                return Err(vec![Diagnostic::new(
                    routine.span,
                    format!(
                        "cart binding for external `{}` is not an absolute address",
                        routine.symbol.qualified_name
                    ),
                )]);
            }
            None => {}
        }
    }
    Ok(addresses)
}

struct SemIrAstLowerer<'a> {
    diagnostics: Vec<Diagnostic>,
    type_link_names: BTreeMap<String, String>,
    external_addresses: Option<&'a HashMap<SymbolId, u16>>,
}

impl SemIrAstLowerer<'_> {
    fn program(&mut self, program: &SemProgram) -> Program {
        Program {
            modules: program
                .modules
                .iter()
                .map(|module| self.module(module))
                .collect(),
            source_kind: SourceUnitKind::Legacy,
        }
    }

    fn module(&mut self, module: &SemModule) -> Module {
        let mut items = Vec::new();
        let mut index = 0usize;
        while index < module.items.len() {
            if let SemItem::Declaration(_) = &module.items[index] {
                let end = self.declaration_group_end(&module.items, index);
                items.extend(
                    self.declarations(
                        &module.items[index..end]
                            .iter()
                            .filter_map(|item| match item {
                                SemItem::Declaration(decl) => Some(decl),
                                _ => None,
                            })
                            .collect::<Vec<_>>(),
                    )
                    .into_iter()
                    .map(Item::Declaration),
                );
                index = end;
                continue;
            }

            if let Some(item) = self.item(&module.items[index]) {
                items.push(item);
            }
            index += 1;
        }

        Module { items }
    }

    fn declaration_group_end(&self, items: &[SemItem], start: usize) -> usize {
        let SemItem::Declaration(first) = &items[start] else {
            return start + 1;
        };
        if !is_var_declaration(first) {
            return start + 1;
        }

        let mut end = start + 1;
        while end < items.len() {
            let SemItem::Declaration(next) = &items[end] else {
                break;
            };
            if !is_var_declaration(next) || next.group_span != first.group_span {
                break;
            }
            end += 1;
        }
        end
    }

    fn item(&mut self, item: &SemItem) -> Option<Item> {
        Some(match item {
            SemItem::Define(define) => Item::Define(DefineDecl {
                entries: vec![DefineEntry {
                    name: define.symbol.name.clone(),
                    value: define.value.clone(),
                    span: define.span,
                }],
            }),
            // CONST declarations are source metadata by this boundary.  Every
            // executable reference already carries its typed literal value.
            SemItem::Const(_) => return None,
            SemItem::Include(include) => Item::Include(IncludeDirective {
                path: include.path.clone(),
                span: include.span,
            }),
            SemItem::Set(set) => Item::Set(SetDirective {
                address: self.expr(&set.address)?,
                value: self.expr(&set.value)?,
                span: set.span,
            }),
            SemItem::Declaration(decl) => Item::Declaration(self.declaration(decl)?),
            SemItem::Routine(routine)
                if routine.is_external
                    && self
                        .external_addresses
                        .is_some_and(|addresses| !addresses.contains_key(&routine.symbol.id)) =>
            {
                return None;
            }
            SemItem::Routine(routine) => Item::Routine(self.routine(routine)?),
            SemItem::Statement(stmt) => Item::Statement(self.stmt(stmt)?),
            SemItem::Unsupported { span, note } => {
                self.unsupported(*span, note);
                return None;
            }
        })
    }

    fn declaration(&mut self, decl: &SemDeclaration) -> Option<Decl> {
        if is_var_declaration(decl) {
            return self.var_declaration_group(&[decl]);
        }

        match &decl.storage {
            SemDeclarationStorage::Type { fields, .. } => {
                return Some(Decl::Type(TypeDecl {
                    visibility: Visibility::Private,
                    name: decl.symbol.name.clone(),
                    fields: self.record_fields(fields),
                    span: decl.span,
                }));
            }
            SemDeclarationStorage::Record { fields, .. } => {
                return Some(Decl::Record(RecordDecl {
                    visibility: Visibility::Private,
                    name: decl.symbol.name.clone(),
                    fields: self.record_fields(fields),
                    span: decl.span,
                }));
            }
            SemDeclarationStorage::Scalar | SemDeclarationStorage::Array { .. } => {}
        }

        None
    }

    fn declarations(&mut self, decls: &[&SemDeclaration]) -> Vec<Decl> {
        let Some(first) = decls.first() else {
            return Vec::new();
        };
        if !is_var_declaration(first) {
            return self.declaration(first).into_iter().collect();
        }
        self.var_declaration_group(decls).into_iter().collect()
    }

    fn var_declaration_group(&mut self, decls: &[&SemDeclaration]) -> Option<Decl> {
        let first = decls.first()?;
        let storage = match &first.storage {
            SemDeclarationStorage::Scalar => VarStorage::Plain,
            SemDeclarationStorage::Array { .. } => VarStorage::Array,
            SemDeclarationStorage::Type { .. } | SemDeclarationStorage::Record { .. } => {
                return None;
            }
        };

        Some(Decl::Var(VarDecl {
            visibility: Visibility::Private,
            qualifiers: VarQualifiers {
                is_volatile: decls.iter().any(|decl| decl.symbol.is_volatile),
            },
            ty: self.type_ref(&first.ty.value),
            storage,
            entries: decls
                .iter()
                .map(|decl| DeclEntry {
                    name: decl.symbol.name.clone(),
                    size: match &decl.storage {
                        SemDeclarationStorage::Array { length, .. } => {
                            length.as_ref().and_then(|expr| self.expr(expr))
                        }
                        SemDeclarationStorage::Scalar => None,
                        SemDeclarationStorage::Type { .. }
                        | SemDeclarationStorage::Record { .. } => None,
                    },
                    initializer: decl
                        .initializer
                        .as_ref()
                        .and_then(|initializer| self.expr(initializer)),
                    span: decl.span,
                })
                .collect(),
            span: first.group_span,
        }))
    }

    fn routine(&mut self, routine: &SemRoutine) -> Option<Routine> {
        Some(Routine {
            visibility: Visibility::Private,
            is_external: routine.is_external,
            kind: routine.signature.kind.clone(),
            name: routine.symbol.name.clone(),
            system_address: routine
                .system_address
                .as_ref()
                .and_then(|address| self.expr(address))
                .or_else(|| {
                    self.external_addresses
                        .and_then(|addresses| addresses.get(&routine.symbol.id))
                        .map(|address| Expr {
                            kind: ExprKind::Number(NumberLiteral {
                                text: format!("${address:04X}"),
                                kind: crate::lexer::NumberKind::Card,
                                value: Some(*address),
                            }),
                            text: format!("${address:04X}"),
                            span: routine.span,
                        })
                }),
            params: routine
                .params
                .iter()
                .map(|param| self.param(param))
                .collect(),
            locals: self.local_declarations(&routine.locals),
            body: routine
                .body
                .iter()
                .filter_map(|stmt| self.stmt(stmt))
                .collect(),
            annotations: routine.annotations.clone(),
            span: routine.span,
        })
    }

    fn local_declarations(&mut self, locals: &[SemDeclaration]) -> Vec<Decl> {
        let mut output = Vec::new();
        let mut index = 0usize;
        while index < locals.len() {
            let first = &locals[index];
            if !is_var_declaration(first) {
                if let Some(decl) = self.declaration(first) {
                    output.push(decl);
                }
                index += 1;
                continue;
            }

            let mut end = index + 1;
            while end < locals.len()
                && is_var_declaration(&locals[end])
                && locals[end].group_span == first.group_span
            {
                end += 1;
            }
            let group = locals[index..end].iter().collect::<Vec<_>>();
            output.extend(self.declarations(&group));
            index = end;
        }
        output
    }

    fn param(&mut self, param: &SemParam) -> VarDecl {
        VarDecl {
            visibility: Visibility::Private,
            qualifiers: VarQualifiers::default(),
            ty: self.type_ref(&param.ty.value),
            storage: match param.storage {
                SemParamStorage::Value => VarStorage::Plain,
                SemParamStorage::Array => VarStorage::Array,
            },
            entries: vec![DeclEntry {
                name: param.symbol.name.clone(),
                size: None,
                initializer: None,
                span: param.span,
            }],
            span: param.span,
        }
    }

    fn record_fields(&mut self, fields: &[SemRecordField]) -> Vec<VarDecl> {
        fields
            .iter()
            .map(|field| {
                let (storage, size) = match &field.storage {
                    SemDeclarationStorage::Scalar
                    | SemDeclarationStorage::Type { .. }
                    | SemDeclarationStorage::Record { .. } => (VarStorage::Plain, None),
                    SemDeclarationStorage::Array { length, .. } => (
                        VarStorage::Array,
                        length.as_ref().and_then(|expr| self.expr(expr)),
                    ),
                };
                VarDecl {
                    visibility: Visibility::Private,
                    qualifiers: VarQualifiers::default(),
                    ty: self.type_ref(&field.ty.value),
                    storage,
                    entries: vec![DeclEntry {
                        name: field.name.clone(),
                        size,
                        initializer: None,
                        span: field.span,
                    }],
                    span: field.span,
                }
            })
            .collect()
    }

    fn stmt(&mut self, stmt: &SemStmt) -> Option<Stmt> {
        match stmt {
            SemStmt::Define(define) => Some(Stmt::Define(DefineDecl {
                entries: vec![DefineEntry {
                    name: define.symbol.name.clone(),
                    value: define.value.clone(),
                    span: define.span,
                }],
            })),
            SemStmt::Return { value, .. } => Some(Stmt::Return(
                value.as_ref().and_then(|expr| self.expr(expr)),
            )),
            SemStmt::Exit { span } => Some(Stmt::Exit { span: *span }),
            SemStmt::Assign {
                target,
                value,
                span,
            } => Some(Stmt::Assign {
                target: self.lvalue(target)?,
                value: self.expr(value)?,
                span: *span,
            }),
            SemStmt::CompoundAssign {
                target,
                op,
                value,
                span,
            } => Some(Stmt::CompoundAssign {
                target: self.lvalue(target)?,
                op: *op,
                value: self.expr(value)?,
                span: *span,
            }),
            SemStmt::Call { call, span } => Some(Stmt::Call {
                expr: self.call_stmt_expr(call)?,
                span: *span,
            }),
            SemStmt::MachineBlock {
                items,
                resolved_symbols,
                text,
                span,
                ..
            } => Some(Stmt::MachineBlock {
                items: self.machine_items(items, resolved_symbols),
                text: text.clone(),
                span: *span,
            }),
            SemStmt::InlineAsm { program, span, .. } => Some(Stmt::InlineAsm {
                program: crate::asm6502::InlineAsmProgram {
                    items: program.compatibility_items.clone(),
                    bytes: program.bytes.clone(),
                    relocations: Vec::new(),
                    source: program.source.clone(),
                    mode: program.mode,
                },
                span: *span,
            }),
            SemStmt::If {
                branches,
                else_body,
                span,
            } => Some(Stmt::If {
                branches: branches
                    .iter()
                    .filter_map(|branch| self.if_branch(branch))
                    .collect(),
                else_body: else_body
                    .iter()
                    .filter_map(|stmt| self.stmt(stmt))
                    .collect(),
                span: *span,
            }),
            SemStmt::While {
                condition,
                body,
                span,
            } => Some(Stmt::While {
                condition: self.condition(condition)?,
                body: body.iter().filter_map(|stmt| self.stmt(stmt)).collect(),
                span: *span,
            }),
            SemStmt::DoUntil {
                body,
                condition,
                span,
            } => Some(Stmt::DoUntil {
                body: body.iter().filter_map(|stmt| self.stmt(stmt)).collect(),
                condition: condition
                    .as_ref()
                    .and_then(|condition| self.condition(condition)),
                span: *span,
            }),
            SemStmt::For {
                target,
                start,
                end,
                step,
                body,
                span,
            } => Some(Stmt::For {
                target: self.lvalue(target)?,
                start: self.expr(start)?,
                end: self.expr(end)?,
                step: step.as_ref().and_then(|step| self.expr(step)),
                body: body.iter().filter_map(|stmt| self.stmt(stmt)).collect(),
                span: *span,
            }),
            SemStmt::Unsupported { span, note } => {
                self.unsupported(*span, note);
                None
            }
        }
    }

    fn if_branch(&mut self, branch: &SemIfBranch) -> Option<IfBranch> {
        Some(IfBranch {
            condition: self.condition(&branch.condition)?,
            body: branch
                .body
                .iter()
                .filter_map(|stmt| self.stmt(stmt))
                .collect(),
        })
    }

    fn condition(&mut self, condition: &SemCondition) -> Option<Expr> {
        self.expr(&condition.expr)
    }

    fn expr(&mut self, expr: &SemExpr) -> Option<Expr> {
        let kind = match &expr.kind {
            SemExprKind::Missing => ExprKind::Missing,
            SemExprKind::Raw(text) => {
                return Some(Expr {
                    kind: ExprKind::Raw,
                    text: text.clone(),
                    span: expr.span,
                });
            }
            SemExprKind::InitializerList(elements) => ExprKind::InitializerList(
                elements
                    .iter()
                    .map(sem_initializer_element_to_ast)
                    .collect(),
            ),
            SemExprKind::UnresolvedName(name) => ExprKind::Name(name.clone()),
            SemExprKind::CurrentLocation => ExprKind::CurrentLocation,
            SemExprKind::Literal(literal) => return Some(self.literal(literal, expr.span)),
            SemExprKind::Symbol(symbol) => ExprKind::Name(symbol.name.clone()),
            SemExprKind::LValue(lvalue) => return self.lvalue(lvalue),
            SemExprKind::ArrayDecay(decay) => return self.lvalue(&decay.array),
            SemExprKind::ImplicitAddressOf(address) => ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr: Box::new(self.lvalue(&address.place)?),
            },
            SemExprKind::AddressOf(lvalue) => ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr: Box::new(self.lvalue(lvalue)?),
            },
            SemExprKind::AddressOfSymbol(symbol) => ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr: Box::new(Expr {
                    kind: ExprKind::Name(symbol.name.clone()),
                    text: symbol.name.clone(),
                    span: symbol.span,
                }),
            },
            SemExprKind::Cast { ty, expr: inner } => ExprKind::Cast {
                ty: self.type_ref(ty),
                expr: Box::new(self.expr(inner)?),
            },
            SemExprKind::Unary { op, expr: inner } => ExprKind::Unary {
                op: *op,
                expr: Box::new(self.expr(inner)?),
            },
            SemExprKind::Binary { op, left, right } => ExprKind::Binary {
                op: *op,
                left: Box::new(self.expr(left)?),
                right: Box::new(self.expr(right)?),
            },
            SemExprKind::Call(call) => return self.call_expr(call),
        };

        let text = expr_text(&kind);
        Some(Expr {
            kind,
            text,
            span: expr.span,
        })
    }

    fn lvalue(&mut self, lvalue: &SemLValue) -> Option<Expr> {
        let kind = match &lvalue.kind {
            SemLValueKind::Symbol(symbol) => ExprKind::Name(symbol.name.clone()),
            SemLValueKind::UnresolvedName(name) => ExprKind::Name(name.clone()),
            SemLValueKind::Deref { pointer } => ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: Box::new(self.expr(pointer)?),
            },
            SemLValueKind::Index {
                base,
                index,
                syntax,
                ..
            } => match syntax {
                SemIndexSyntax::Call => ExprKind::Call {
                    callee: Box::new(self.expr(base)?),
                    args: vec![self.expr(index)?],
                },
                SemIndexSyntax::Index => ExprKind::Index {
                    base: Box::new(self.expr(base)?),
                    index: Box::new(self.expr(index)?),
                },
            },
            SemLValueKind::Field { base, field } => ExprKind::Field {
                base: Box::new(self.lvalue(base)?),
                field: field.name.clone(),
            },
        };
        let text = expr_text(&kind);
        Some(Expr {
            kind,
            text,
            span: lvalue.span,
        })
    }

    fn call_expr(&mut self, call: &SemCall) -> Option<Expr> {
        let callee = match &call.callee {
            SemCallable::User(symbol) | SemCallable::Builtin(symbol) => Expr {
                kind: ExprKind::Name(symbol.name.clone()),
                text: symbol.name.clone(),
                span: symbol.span,
            },
            SemCallable::Indirect { target, .. } => self.expr(target)?,
            SemCallable::Runtime { name, .. } => Expr {
                kind: ExprKind::Name(name.clone()),
                text: name.clone(),
                span: call.span,
            },
        };
        let args = call
            .args
            .iter()
            .filter_map(|arg| self.expr(arg))
            .collect::<Vec<_>>();
        let kind = ExprKind::Call {
            callee: Box::new(callee),
            args,
        };
        let text = expr_text(&kind);
        Some(Expr {
            kind,
            text,
            span: call.span,
        })
    }

    fn call_stmt_expr(&mut self, call: &SemCall) -> Option<Expr> {
        if call.args.is_empty()
            && let SemCallable::Indirect { target, .. } = &call.callee
            && let Some(name) = bare_call_stmt_name(target)
        {
            let kind = ExprKind::Name(name);
            let text = expr_text(&kind);
            return Some(Expr {
                kind,
                text,
                span: call.span,
            });
        }

        self.call_expr(call)
    }

    fn literal(&self, literal: &SemLiteral, span: Span) -> Expr {
        let (kind, text) = match literal {
            SemLiteral::Number(number) => (ExprKind::Number(number.clone()), number.text.clone()),
            SemLiteral::Real { source, .. } => {
                (ExprKind::Number(source.clone()), source.text.clone())
            }
            SemLiteral::String(text) => (ExprKind::String(text.clone()), format!("{text:?}")),
            SemLiteral::Char(ch) => (ExprKind::Char(*ch), format!("'{ch}'")),
            SemLiteral::Constant(value) => {
                let number = value.number_literal();
                (ExprKind::Number(number.clone()), number.text)
            }
        };
        Expr { kind, text, span }
    }

    fn type_ref(&self, ty: &ValueType) -> TypeRef {
        TypeRef {
            base: match &ty.base {
                ValueTypeBase::Fund(fund) => TypeBase::Fund(*fund),
                // The classic bridge still projects SemIR back to an AST. REAL
                // backend semantics remain in SemIR; this spelling only keeps
                // the source-shaped carrier intact until classic parity lands.
                ValueTypeBase::Real => TypeBase::Named("REAL".into()),
                ValueTypeBase::Named(name) => TypeBase::Named(
                    self.type_link_names
                        .get(&name.to_ascii_uppercase())
                        .cloned()
                        .unwrap_or_else(|| name.clone())
                        .into(),
                ),
                ValueTypeBase::Callable(callable) => TypeBase::Callable(callable.kind.clone()),
                ValueTypeBase::Error => TypeBase::Fund(FundType::Byte),
            },
            pointer: ty.pointer && !matches!(ty.base, ValueTypeBase::Callable(_)),
        }
    }

    fn machine_items(
        &self,
        items: &[MachineItem],
        resolved_symbols: &[SemMachineSymbolRef],
    ) -> Vec<MachineItem> {
        let resolved = resolved_symbols
            .iter()
            .map(|target| (target.item_index, target.symbol.name.as_str()))
            .collect::<BTreeMap<_, _>>();
        items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let Some(name) = resolved.get(&index) else {
                    return item.clone();
                };
                match item {
                    MachineItem::Name(_) => MachineItem::Name((*name).into()),
                    MachineItem::AddressByte { selector, .. } => MachineItem::AddressByte {
                        selector: *selector,
                        name: (*name).into(),
                    },
                    MachineItem::AddressExpr(expr) => {
                        let mut expr = expr.clone();
                        expr.atom = MachineAddressAtom::Name((*name).into());
                        MachineItem::AddressExpr(expr)
                    }
                    MachineItem::Number(_)
                    | MachineItem::StringLiteral(_)
                    | MachineItem::CharLiteral(_)
                    | MachineItem::Raw(_) => item.clone(),
                }
            })
            .collect()
    }

    fn unsupported(&mut self, span: Span, feature: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(
            span,
            format!("{} is not supported by SemIR codegen yet", feature.into()),
        ));
    }
}

fn module_type_link_names(program: &SemProgram) -> BTreeMap<String, String> {
    program
        .modules
        .iter()
        .flat_map(|module| module.items.iter())
        .filter_map(|item| match item {
            SemItem::Declaration(declaration)
                if matches!(
                    declaration.symbol.class,
                    crate::semantic::SymbolClass::Type | crate::semantic::SymbolClass::Record
                ) =>
            {
                Some((
                    declaration.symbol.qualified_name.to_ascii_uppercase(),
                    declaration.symbol.name.clone(),
                ))
            }
            _ => None,
        })
        .collect()
}

fn bare_call_stmt_name(expr: &SemExpr) -> Option<String> {
    match &expr.kind {
        SemExprKind::UnresolvedName(name) => Some(name.clone()),
        SemExprKind::Symbol(symbol) => Some(symbol.name.clone()),
        SemExprKind::ArrayDecay(decay) => lvalue_name(&decay.array),
        SemExprKind::Cast { expr, .. } => bare_call_stmt_name(expr),
        _ => None,
    }
}

fn lvalue_name(lvalue: &SemLValue) -> Option<String> {
    match &lvalue.kind {
        SemLValueKind::Symbol(symbol) => Some(symbol.name.clone()),
        SemLValueKind::UnresolvedName(name) => Some(name.clone()),
        _ => None,
    }
}

fn is_var_declaration(decl: &SemDeclaration) -> bool {
    matches!(
        decl.storage,
        SemDeclarationStorage::Scalar | SemDeclarationStorage::Array { .. }
    )
}

fn expr_text(kind: &ExprKind) -> String {
    match kind {
        ExprKind::Missing => String::new(),
        ExprKind::Raw => String::new(),
        ExprKind::InitializerList(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(|element| element.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        ExprKind::CurrentLocation => "*".to_string(),
        ExprKind::Number(NumberLiteral { text, .. }) => text.clone(),
        ExprKind::String(text) => format!("{text:?}"),
        ExprKind::Char(ch) => format!("'{ch}'"),
        ExprKind::Name(name) => name.clone(),
        ExprKind::Unary { op, expr } => format!("{}{}", unary_text(*op), expr.text),
        ExprKind::Cast { ty, expr } => format!("{}({})", type_ref_text(ty), expr.text),
        ExprKind::Binary { op, left, right } => {
            format!("{} {} {}", left.text, binary_text(*op), right.text)
        }
        ExprKind::Call { callee, args } => {
            let args = args
                .iter()
                .map(|arg| arg.text.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({args})", callee.text)
        }
        ExprKind::Index { base, index } => format!("{}({})", base.text, index.text),
        ExprKind::Field { base, field } => format!("{}.{}", base.text, field),
    }
}

fn sem_initializer_element_to_ast(element: &SemInitializerElement) -> InitializerElement {
    let kind = match &element.kind {
        SemInitializerElementKind::Literal { value, negative } => {
            let value = match value {
                SemInitializerLiteral::Number(number) => InitializerLiteral::Number(number.clone()),
                SemInitializerLiteral::Char(ch) => InitializerLiteral::Char(*ch),
                SemInitializerLiteral::True => InitializerLiteral::True,
                SemInitializerLiteral::False => InitializerLiteral::False,
                SemInitializerLiteral::Nil => InitializerLiteral::Nil,
            };
            InitializerElementKind::Literal {
                value,
                negative: *negative,
            }
        }
        SemInitializerElementKind::Address {
            selector,
            target,
            addend,
        } => InitializerElementKind::Address {
            selector: *selector,
            target: target.name.clone().into(),
            addend: *addend,
        },
        SemInitializerElementKind::Invalid => InitializerElementKind::Invalid,
    };
    InitializerElement {
        kind,
        text: element.text.clone(),
        span: element.span,
    }
}

fn type_ref_text(ty: &TypeRef) -> String {
    let base = match &ty.base {
        TypeBase::Fund(fund) => match fund {
            FundType::Byte => "BYTE".to_string(),
            FundType::Card => "CARD".to_string(),
            FundType::Char => "CHAR".to_string(),
            FundType::Int => "INT".to_string(),
        },
        TypeBase::Named(name) => name.to_string(),
        TypeBase::Callable(kind) => routine_kind_text(kind),
    };
    if ty.pointer {
        format!("{base} POINTER")
    } else {
        base
    }
}

fn routine_kind_text(kind: &RoutineKind) -> String {
    match kind {
        RoutineKind::Proc => "PROC POINTER".to_string(),
        RoutineKind::Func { return_type } => {
            format!("{} FUNC POINTER", fund_type_text(*return_type))
        }
    }
}

fn fund_type_text(fund: FundType) -> &'static str {
    match fund {
        FundType::Byte => "BYTE",
        FundType::Card => "CARD",
        FundType::Char => "CHAR",
        FundType::Int => "INT",
    }
}

fn unary_text(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Plus => "+",
        UnaryOp::Neg => "-",
        UnaryOp::AddressOf => "@",
        UnaryOp::Deref => "",
    }
}

fn binary_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "MOD",
        BinaryOp::Lsh => "LSH",
        BinaryOp::Rsh => "RSH",
        BinaryOp::And => "&",
        BinaryOp::Or => "%",
        BinaryOp::Xor => "!",
        BinaryOp::Eq => "=",
        BinaryOp::Ne => "#",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{CODE_ORIGIN, CodegenProfile, generate_profile_with_origin};
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::semantic::{analyze, ir};

    #[test]
    fn semir_codegen_matches_ast_for_scalar_assignment_slice() {
        let source = "SET $491=$3000 SET $E=$3000 BYTE x PROC Main() x=1 RETURN";
        let (program, semir) = lower_source(source);

        let ast_output =
            generate_profile_with_origin(&program, CODE_ORIGIN, CodegenProfile::Compat).unwrap();
        let semir_output = crate::codegen::generate_semir_profile_with_origin(
            &semir,
            CODE_ORIGIN,
            CodegenProfile::Compat,
        )
        .unwrap();

        assert_eq!(semir_output.bytes, ast_output.bytes);
        assert_eq!(semir_output.origin, ast_output.origin);
        assert_eq!(semir_output.run_address, ast_output.run_address);
    }

    #[test]
    fn semir_codegen_accepts_simple_if_slice() {
        let (_, semir) = lower_source("PROC Main() BYTE x IF x THEN x=1 FI RETURN");
        crate::codegen::generate_semir_profile_with_origin(
            &semir,
            CODE_ORIGIN,
            CodegenProfile::Compat,
        )
        .unwrap();
    }

    #[test]
    fn semir_codegen_matches_ast_for_control_flow_slice() {
        let source = "SET $491=$3000 SET $E=$3000 BYTE x PROC Main() WHILE x DO IF x=1 THEN x=2 ELSE x=3 FI OD FOR x=0 TO 2 DO x==+1 OD RETURN";
        let (program, semir) = lower_source(source);

        let ast_output =
            generate_profile_with_origin(&program, CODE_ORIGIN, CodegenProfile::Compat).unwrap();
        let semir_output = crate::codegen::generate_semir_profile_with_origin(
            &semir,
            CODE_ORIGIN,
            CodegenProfile::Compat,
        )
        .unwrap();

        assert_eq!(semir_output.bytes, ast_output.bytes);
    }

    #[test]
    fn semir_codegen_matches_ast_for_machine_block_slice() {
        let (program, semir) = lower_source("PROC Raw=*() [$A9 $01 $60] PROC Main() Raw() RETURN");

        let ast_output =
            generate_profile_with_origin(&program, CODE_ORIGIN, CodegenProfile::Compat).unwrap();
        let semir_output = crate::codegen::generate_semir_profile_with_origin(
            &semir,
            CODE_ORIGIN,
            CodegenProfile::Compat,
        )
        .unwrap();

        assert_eq!(semir_output.bytes, ast_output.bytes);
    }

    #[test]
    fn semir_codegen_matches_ast_for_array_and_string_slice() {
        let source = "SET $491=$3000 SET $E=$3000 DEFINE STRING=\"CHAR ARRAY\" BYTE ARRAY ba(4) STRING s(0)=\"HI\" BYTE x PROC Main() ba(0)=s(1) x=ba(0) RETURN";
        let (program, semir) = lower_source(source);

        let ast_output =
            generate_profile_with_origin(&program, CODE_ORIGIN, CodegenProfile::Compat).unwrap();
        let semir_output = crate::codegen::generate_semir_profile_with_origin(
            &semir,
            CODE_ORIGIN,
            CodegenProfile::Compat,
        )
        .unwrap();

        assert_eq!(semir_output.bytes, ast_output.bytes);
    }

    #[test]
    fn semir_codegen_matches_ast_for_record_slice() {
        let source = "SET $491=$3000 SET $E=$3000 TYPE Pair=[BYTE tag CARD word] BYTE gb CARD gw BYTE ARRAY data(4) Pair rec PROC Touch(Pair POINTER rp) rp.tag=$11 rp.word=$2233 gb=rp.tag gw=rp.word RETURN PROC Main() data(0)=$44 Touch(rec) RETURN";
        let (program, semir) = lower_source(source);

        let ast_output =
            generate_profile_with_origin(&program, CODE_ORIGIN, CodegenProfile::Compat).unwrap();
        let semir_output = crate::codegen::generate_semir_profile_with_origin(
            &semir,
            CODE_ORIGIN,
            CodegenProfile::Compat,
        )
        .unwrap();

        assert_eq!(semir_output.bytes, ast_output.bytes);
    }

    #[test]
    fn semir_codegen_matches_ast_for_grouped_declarations_slice() {
        let source = "SET $491=$3000 SET $E=$3000 BYTE alias=$D000, init=[1], scratch CARD word=[0], vector BYTE ARRAY table(4)=[1 2 3 4], text(0)=\"OK\" PROC Main() scratch=table(1) vector=word RETURN";
        let (program, semir) = lower_source(source);

        let ast_output =
            generate_profile_with_origin(&program, CODE_ORIGIN, CodegenProfile::Compat).unwrap();
        let semir_output = crate::codegen::generate_semir_profile_with_origin(
            &semir,
            CODE_ORIGIN,
            CodegenProfile::Compat,
        )
        .unwrap();

        assert_eq!(semir_output.bytes, ast_output.bytes);
    }

    #[test]
    fn semir_codegen_matches_ast_for_unresolved_builtins_and_machine_defines() {
        let source = "SET $491=$3000\nSET $E=$3000\nDEFINE Nop=\"[$EA]\"\nMODULE\nBYTE d\nPROC Main()\n  Nop\n  color=3\n  d=device\n  PutD(0,'A)\nRETURN";
        let (program, semir) = lower_source(source);

        let ast_output =
            generate_profile_with_origin(&program, CODE_ORIGIN, CodegenProfile::Compat).unwrap();
        let semir_output = crate::codegen::generate_semir_profile_with_origin(
            &semir,
            CODE_ORIGIN,
            CodegenProfile::Compat,
        )
        .unwrap();

        assert_eq!(semir_output.bytes, ast_output.bytes);
    }

    fn lower_source(source: &str) -> (Program, SemProgram) {
        let tokens = tokenize(source).unwrap();
        let program = parse(&tokens).unwrap();
        let model = analyze(&program).unwrap();
        let semir = ir::lower_program(&program, &model);
        (program, semir)
    }
}
