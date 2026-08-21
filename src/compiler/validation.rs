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
        SemStmt::Define(_) | SemStmt::Exit { .. } | SemStmt::Unsupported { .. } => {}
    }
}

fn collect_standalone_call_diagnostics(call: &SemCall, diagnostics: &mut Vec<Diagnostic>) {
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
