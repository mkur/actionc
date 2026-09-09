use std::collections::HashSet;

use crate::ast::{Expr, ExprKind, Item, Program, Stmt};
use crate::diagnostic::Diagnostic;
use crate::semantic::{
    SymbolClass,
    ir::{
        SemCall, SemCallable, SemExpr, SemExprKind, SemInlineAsmTarget, SemItem, SemLValue,
        SemLValueKind, SemProgram, SemStmt,
    },
};

/// Capability check shared by the compiler facade and the inspection CLI.
/// Use semantic type identities, including casts without wide declarations.
pub(crate) fn classic_wide_integer_diagnostic(
    model: &crate::semantic::SemanticModel,
    semir: &SemProgram,
) -> Option<Diagnostic> {
    let wide = |ty: &crate::semantic::ValueType| ty.as_scalar().is_some_and(|scalar|
        matches!(scalar, crate::semantic::ScalarType::LongInt | crate::semantic::ScalarType::LongCard));
    // Interface imports are not executable uses. Reuse SemIR's existing
    // external-reference selection instead of rejecting every imported API.
    let retained_external = semir.modules.iter().flat_map(|module| &module.items)
        .filter_map(|item| match item {
            SemItem::Routine(routine) if routine.is_external => Some(routine),
            _ => None,
        }).collect::<Vec<_>>();
    let retained_ids = retained_external.iter().map(|routine| routine.symbol.id)
        .collect::<HashSet<_>>();
    let unused_external = model.routine_signatures_by_symbol.iter()
        .filter(|(id, signature)| signature.source == crate::semantic::SemanticCallableSource::Runtime
            && !retained_ids.contains(id))
        .map(|(id, _)| *id).collect::<HashSet<_>>();
    let unused_scopes = model.routine_scopes.iter()
        .filter(|scope| scope.symbol.is_some_and(|id| unused_external.contains(&id)))
        .map(|scope| scope.scope).collect::<HashSet<_>>();
    let span = model.symbols.symbols.iter().enumerate().find_map(|(index, symbol)| {
        (symbol.class != SymbolClass::Type
            && !unused_external.contains(&crate::semantic::SymbolId(index))
            && !unused_scopes.contains(&symbol.scope)
            && symbol.ty.as_ref().is_some_and(&wide))
            .then_some(symbol.span)
    }).or_else(|| model.expression_observations.iter().find_map(|expr| {
        expr.ty.as_ref().is_some_and(&wide).then_some(expr.span)
    })).or_else(|| retained_external.iter().find_map(|routine| {
        // A narrow literal still needs a wide ABI in e.g. SYS.PrintLC(1).
        (routine.signature.params.iter().any(&wide)
            || routine.signature.return_type.as_ref().is_some_and(&wide))
            .then_some(routine.symbol.span)
    }))?;
    Some(Diagnostic::new(span, "LONGINT/LONGCARD code generation requires the MIR6502 backend; classic supports only 8/16-bit integers"))
}

#[cfg(test)]
#[test]
fn classic_wide_guard_recognizes_union_views_before_emission() {
    for definition in ["UNION [T wide CARD word]", "[T wide CARD word]"] {
    for body in ["value.wide=1", "result=value.wide", "ptr.wide==+1", "result=LONGCARD(value.word)"] {
        let source=format!("TYPE View<T>={definition} View<LONGCARD> value View<LONGCARD> POINTER ptr CARD result PROC Main() {body} RETURN");
        let ast=crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let mut options=crate::semantic::SemanticOptions::modern(); options.algebraic_types.unions=true;
        let model=crate::semantic::analyze_with_options(&ast,options).unwrap();
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        let error=classic_wide_integer_diagnostic(&model, &semir).expect(body);
        assert!(error.message.contains("requires the MIR6502 backend"));
    }
    }
    // A declaration alone is not a wide operation: byte/word views and opaque
    // copies need no 32-bit scalar lane in classic.
    let ast=crate::parser::parse(&crate::lexer::tokenize("TYPE View=UNION [LONGCARD wide CARD word] View value PROC Main() value.word=1 RETURN").unwrap()).unwrap();
    let mut options=crate::semantic::SemanticOptions::modern(); options.algebraic_types.unions=true;
    let model=crate::semantic::analyze_with_options(&ast,options).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    assert!(classic_wide_integer_diagnostic(&model, &semir).is_none());
}

pub(crate) fn standalone_resident_diagnostics(program: &SemProgram) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for module in &program.modules {
        for item in &module.items {
            collect_standalone_item_diagnostics(item, &mut diagnostics);
        }
    }
    diagnostics
}

fn collect_standalone_item_diagnostics(item: &SemItem, diagnostics: &mut Vec<Diagnostic>) {
    match item {
        SemItem::Set(set) => {
            collect_standalone_expr_diagnostics(&set.address, diagnostics);
            collect_standalone_expr_diagnostics(&set.value, diagnostics);
        }
        SemItem::Declaration(declaration) => {
            if let Some(initializer) = &declaration.initializer {
                collect_standalone_expr_diagnostics(initializer, diagnostics);
            }
        }
        SemItem::Routine(routine) => {
            for local in &routine.locals {
                if let Some(initializer) = &local.initializer {
                    collect_standalone_expr_diagnostics(initializer, diagnostics);
                }
            }
            if let Some(address) = &routine.system_address {
                collect_standalone_expr_diagnostics(address, diagnostics);
            }
            collect_standalone_stmt_list_diagnostics(&routine.body, diagnostics);
        }
        SemItem::Statement(statement) => {
            collect_standalone_stmt_diagnostics(statement, diagnostics);
        }
        SemItem::Define(_)
        | SemItem::Const(_)
        | SemItem::Include(_)
        | SemItem::Unsupported { .. } => {}
    }
}

fn collect_standalone_stmt_list_diagnostics(
    statements: &[SemStmt],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for statement in statements {
        collect_standalone_stmt_diagnostics(statement, diagnostics);
    }
}

fn collect_standalone_stmt_diagnostics(statement: &SemStmt, diagnostics: &mut Vec<Diagnostic>) {
    match statement {
        SemStmt::Case { selector, arms, .. } => {
            collect_standalone_expr_diagnostics(selector, diagnostics);
            for arm in arms {
                for condition in arm.conditions() { collect_standalone_expr_diagnostics(&condition.expr, diagnostics); }
                collect_standalone_stmt_list_diagnostics(arm.preparation(), diagnostics);
                collect_standalone_stmt_list_diagnostics(&arm.body, diagnostics);
            }
        }
        SemStmt::LexicalBlock {
            declarations, body, ..
        } => {
            for declaration in declarations {
                if let Some(initializer) = &declaration.initializer {
                    collect_standalone_expr_diagnostics(initializer, diagnostics);
                }
            }
            collect_standalone_stmt_list_diagnostics(body, diagnostics);
        }
        SemStmt::Return { value, .. } => {
            if let Some(value) = value {
                collect_standalone_expr_diagnostics(value, diagnostics);
            }
        }
        SemStmt::Assign { target, value, .. } | SemStmt::CompoundAssign { target, value, .. } => {
            collect_standalone_lvalue_diagnostics(target, diagnostics);
            collect_standalone_expr_diagnostics(value, diagnostics);
        }
        SemStmt::RecordCopy {
            destination,
            source,
            ..
        } => {
            collect_standalone_lvalue_diagnostics(destination, diagnostics);
            collect_standalone_lvalue_diagnostics(source, diagnostics);
        }
        SemStmt::Call { call, .. } => collect_standalone_call_diagnostics(call, diagnostics),
        SemStmt::MachineBlock {
            resolved_symbols, ..
        } => {
            for resolved in resolved_symbols {
                report_standalone_resident_symbol(
                    &resolved.symbol,
                    resolved.symbol.span,
                    diagnostics,
                );
            }
        }
        SemStmt::InlineAsm { program, .. } => {
            for relocation in &program.relocations {
                if let SemInlineAsmTarget::Symbol(symbol) = &relocation.target {
                    report_standalone_resident_symbol(symbol, relocation.span, diagnostics);
                }
            }
        }
        SemStmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                collect_standalone_expr_diagnostics(&branch.condition.expr, diagnostics);
                collect_standalone_stmt_list_diagnostics(&branch.body, diagnostics);
            }
            collect_standalone_stmt_list_diagnostics(else_body, diagnostics);
        }
        SemStmt::While {
            condition, body, ..
        } => {
            collect_standalone_expr_diagnostics(&condition.expr, diagnostics);
            collect_standalone_stmt_list_diagnostics(body, diagnostics);
        }
        SemStmt::DoUntil {
            body, condition, ..
        } => {
            collect_standalone_stmt_list_diagnostics(body, diagnostics);
            if let Some(condition) = condition {
                collect_standalone_expr_diagnostics(&condition.expr, diagnostics);
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
            collect_standalone_lvalue_diagnostics(target, diagnostics);
            collect_standalone_expr_diagnostics(start, diagnostics);
            collect_standalone_expr_diagnostics(end, diagnostics);
            if let Some(step) = step {
                collect_standalone_expr_diagnostics(step, diagnostics);
            }
            collect_standalone_stmt_list_diagnostics(body, diagnostics);
        }
        SemStmt::Define(_) | SemStmt::Exit { .. } | SemStmt::Unsupported { .. } | SemStmt::Fault { .. } => {}
    }
}

fn collect_standalone_call_diagnostics(call: &SemCall, diagnostics: &mut Vec<Diagnostic>) {
    collect_standalone_stmt_list_diagnostics(&call.preparation, diagnostics);
    match &call.callee {
        SemCallable::Builtin(symbol) => {
            report_standalone_resident_symbol(symbol, call.span, diagnostics);
        }
        SemCallable::Indirect { target, .. } => {
            collect_standalone_expr_diagnostics(target, diagnostics);
        }
        SemCallable::User(_) | SemCallable::Runtime { .. } => {}
    }
    for argument in &call.args {
        collect_standalone_expr_diagnostics(argument, diagnostics);
    }
}

fn collect_standalone_expr_diagnostics(expr: &SemExpr, diagnostics: &mut Vec<Diagnostic>) {
    match &expr.kind {
        SemExprKind::InitializerList(elements) => {
            for element in elements {
                if let crate::semantic::ir::SemInitializerElementKind::Address { target, .. } =
                    &element.kind
                {
                    report_standalone_resident_symbol(target, element.span, diagnostics);
                }
            }
        }
        SemExprKind::Symbol(symbol) | SemExprKind::AddressOfSymbol(symbol) => {
            report_standalone_resident_symbol(symbol, expr.span, diagnostics);
        }
        SemExprKind::LValue(place) | SemExprKind::AddressOf(place) => {
            collect_standalone_lvalue_diagnostics(place, diagnostics);
        }
        SemExprKind::ImplicitAddressOf(address) => {
            collect_standalone_lvalue_diagnostics(&address.place, diagnostics);
        }
        SemExprKind::ArrayDecay(decay) => {
            collect_standalone_lvalue_diagnostics(&decay.array, diagnostics);
        }
        SemExprKind::Cast { expr, .. } | SemExprKind::Unary { expr, .. } => {
            collect_standalone_expr_diagnostics(expr, diagnostics);
        }
        SemExprKind::Binary { left, right, .. } => {
            collect_standalone_expr_diagnostics(left, diagnostics);
            collect_standalone_expr_diagnostics(right, diagnostics);
        }
        SemExprKind::Call(call) => collect_standalone_call_diagnostics(call, diagnostics),
        SemExprKind::Missing
        | SemExprKind::Raw(_)
        | SemExprKind::UnresolvedName(_)
        | SemExprKind::CurrentLocation
        | SemExprKind::Literal(_) => {}
    }
}

fn collect_standalone_lvalue_diagnostics(place: &SemLValue, diagnostics: &mut Vec<Diagnostic>) {
    match &place.kind {
        SemLValueKind::Symbol(symbol) => {
            report_standalone_resident_symbol(symbol, place.span, diagnostics);
        }
        SemLValueKind::Deref { pointer } => {
            collect_standalone_expr_diagnostics(pointer, diagnostics);
        }
        SemLValueKind::Index { base, index, .. } => {
            collect_standalone_expr_diagnostics(base, diagnostics);
            collect_standalone_expr_diagnostics(index, diagnostics);
        }
        SemLValueKind::Field { base, .. } => {
            collect_standalone_lvalue_diagnostics(base, diagnostics);
        }
        SemLValueKind::UnresolvedName(_) => {}
    }
}

fn report_standalone_resident_symbol(
    symbol: &crate::semantic::ir::SemSymbolRef,
    span: crate::source::Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if symbol.defining_module.is_some()
        || !matches!(
            symbol.class,
            SymbolClass::BuiltinProc | SymbolClass::BuiltinFunc
        )
    {
        return;
    }
    diagnostics.push(Diagnostic::new(
        span,
        format!(
            "E-RUNTIME-STANDALONE-BINDING: resident routine `{}` requires the Action! cartridge and has no standalone binding; select `--runtime cart` or use an implemented `SYS` interface",
            symbol.name
        ),
    ));
}

pub(crate) fn legacy_routine_retargeting_diagnostics(program: &Program) -> Vec<Diagnostic> {
    let routine_names = routine_names(program);
    let mut diagnostics = Vec::new();
    for module in &program.modules {
        for item in &module.items {
            match item {
                Item::Routine(routine) => {
                    for stmt in &routine.body {
                        collect_legacy_routine_retargeting_diagnostics(
                            stmt,
                            &routine_names,
                            &mut diagnostics,
                        );
                    }
                }
                Item::Statement(stmt) => {
                    collect_legacy_routine_retargeting_diagnostics(
                        stmt,
                        &routine_names,
                        &mut diagnostics,
                    );
                }
                _ => {}
            }
        }
    }
    diagnostics
}

fn routine_names(program: &Program) -> HashSet<String> {
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

fn collect_legacy_routine_retargeting_diagnostics(
    stmt: &Stmt,
    routine_names: &HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match stmt {
        Stmt::Case { arms, .. } => {
            for arm in arms { for stmt in &arm.body {
                collect_legacy_routine_retargeting_diagnostics(stmt, routine_names, diagnostics);
            } }
        }
        Stmt::Assign {
            target,
            value,
            span,
        } if assignment_retargets_routine(target, value, routine_names) => {
            diagnostics.push(Diagnostic::new(
                *span,
                "MIR/NIR backend does not support legacy routine-name retargeting; use a function pointer instead",
            ));
        }
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                for stmt in &branch.body {
                    collect_legacy_routine_retargeting_diagnostics(
                        stmt,
                        routine_names,
                        diagnostics,
                    );
                }
            }
            for stmt in else_body {
                collect_legacy_routine_retargeting_diagnostics(stmt, routine_names, diagnostics);
            }
        }
        Stmt::While { body, .. } | Stmt::DoUntil { body, .. } | Stmt::For { body, .. } => {
            for stmt in body {
                collect_legacy_routine_retargeting_diagnostics(stmt, routine_names, diagnostics);
            }
        }
        _ => {}
    }
}

fn assignment_retargets_routine(
    target: &Expr,
    value: &Expr,
    routine_names: &HashSet<String>,
) -> bool {
    let (ExprKind::Name(target_name), ExprKind::Name(value_name)) = (&target.kind, &value.kind)
    else {
        return false;
    };
    routine_names.contains(&normalize_name(target_name))
        && routine_names.contains(&normalize_name(value_name))
}

fn normalize_name(name: &str) -> String {
    name.to_ascii_uppercase()
}
