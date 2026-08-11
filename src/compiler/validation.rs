use std::collections::HashSet;

use crate::ast::{Expr, ExprKind, Item, Program, Stmt};
use crate::diagnostic::Diagnostic;

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
