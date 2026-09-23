use super::*;

pub(super) fn validate_compatible_source_surface(
    program: &Program,
    routines: &HashMap<String, RoutineInfo>,
) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for module in &program.modules {
        for item in &module.items {
            validate_compatible_item(item, routines, &mut diagnostics);
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

pub(super) fn validate_modern_source_surface(
    program: &Program,
    routines: &HashMap<String, RoutineInfo>,
) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for module in &program.modules {
        for item in &module.items {
            validate_modern_item(item, routines, &mut diagnostics);
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn validate_modern_item(
    item: &Item,
    routines: &HashMap<String, RoutineInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match item {
        Item::Routine(routine) => {
            for stmt in &routine.body {
                validate_modern_stmt(stmt, routines, diagnostics);
            }
        }
        Item::Statement(stmt) => validate_modern_stmt(stmt, routines, diagnostics),
        _ => {}
    }
}

fn validate_modern_stmt(
    stmt: &Stmt,
    routines: &HashMap<String, RoutineInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match stmt {
        Stmt::Assign {
            target,
            value,
            span,
        } if assignment_retargets_routine(target, value, routines) => {
            diagnostics.push(Diagnostic::new(
                *span,
                "cannot assign to routine name; assign @routine to a function pointer instead",
            ));
        }
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                for stmt in &branch.body {
                    validate_modern_stmt(stmt, routines, diagnostics);
                }
            }
            for stmt in else_body {
                validate_modern_stmt(stmt, routines, diagnostics);
            }
        }
        Stmt::While { body, .. } | Stmt::DoUntil { body, .. } => {
            for stmt in body {
                validate_modern_stmt(stmt, routines, diagnostics);
            }
        }
        Stmt::For { body, .. } => {
            for stmt in body {
                validate_modern_stmt(stmt, routines, diagnostics);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_routine_assignment_targets(
    program: &Program,
    routines: &HashMap<String, RoutineInfo>,
) -> HashSet<String> {
    let mut targets = HashSet::new();
    for module in &program.modules {
        for item in &module.items {
            match item {
                Item::Routine(routine) => {
                    for stmt in &routine.body {
                        collect_routine_assignment_targets_from_stmt(stmt, routines, &mut targets);
                    }
                }
                Item::Statement(stmt) => {
                    collect_routine_assignment_targets_from_stmt(stmt, routines, &mut targets);
                }
                _ => {}
            }
        }
    }
    targets
}

pub(super) fn collect_routine_assignment_targets_from_stmt(
    stmt: &Stmt,
    routines: &HashMap<String, RoutineInfo>,
    targets: &mut HashSet<String>,
) {
    match stmt {
        Stmt::Assign { target, value, .. } => {
            if let ExprKind::Name(target_name) = &target.kind
                && assignment_retargets_routine(target, value, routines)
            {
                targets.insert(normalize_name(target_name));
            }
        }
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                for stmt in &branch.body {
                    collect_routine_assignment_targets_from_stmt(stmt, routines, targets);
                }
            }
            for stmt in else_body {
                collect_routine_assignment_targets_from_stmt(stmt, routines, targets);
            }
        }
        Stmt::While { body, .. } | Stmt::DoUntil { body, .. } => {
            for stmt in body {
                collect_routine_assignment_targets_from_stmt(stmt, routines, targets);
            }
        }
        Stmt::For { body, .. } => {
            for stmt in body {
                collect_routine_assignment_targets_from_stmt(stmt, routines, targets);
            }
        }
        _ => {}
    }
}

fn assignment_retargets_routine(
    target: &Expr,
    value: &Expr,
    routines: &HashMap<String, RoutineInfo>,
) -> bool {
    let (ExprKind::Name(target_name), ExprKind::Name(value_name)) = (&target.kind, &value.kind)
    else {
        return false;
    };
    let normalized_target = normalize_name(target_name);
    let normalized_value = normalize_name(value_name);
    routines
        .get(&normalized_target)
        .is_some_and(|info| info.system_address.is_none())
        && routines.contains_key(&normalized_value)
}

pub(super) fn validate_compatible_item(
    item: &Item,
    routines: &HashMap<String, RoutineInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match item {
        Item::Routine(routine) => {
            for stmt in &routine.body {
                validate_compatible_stmt(stmt, routines, diagnostics);
            }
        }
        Item::Statement(stmt) => validate_compatible_stmt(stmt, routines, diagnostics),
        _ => {}
    }
}

pub(super) fn validate_compatible_stmt(
    stmt: &Stmt,
    routines: &HashMap<String, RoutineInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match stmt {
        Stmt::Return { value: Some(expr), .. } => validate_compatible_expr(expr, routines, diagnostics),
        Stmt::Assign {
            target,
            value,
            span,
        } => {
            if expr_contains_routine_call(target, routines)
                && expr_contains_routine_call(value, routines)
            {
                diagnostics.push(Diagnostic::new(
                    *span,
                    "compat profile rejects indexed assignments with function calls on both sides",
                ));
            }
            validate_compatible_expr(target, routines, diagnostics);
            validate_compatible_expr(value, routines, diagnostics);
        }
        Stmt::CompoundAssign {
            target,
            value,
            span,
            ..
        } => {
            if expr_contains_routine_call(target, routines) {
                diagnostics.push(Diagnostic::new(
                    *span,
                    "compat profile rejects compound assignments with function calls in the target",
                ));
            }
            validate_compatible_expr(target, routines, diagnostics);
            validate_compatible_expr(value, routines, diagnostics);
        }
        Stmt::Call { expr, span } => {
            if let ExprKind::Call { callee, args } = &expr.kind {
                if args
                    .iter()
                    .any(|arg| expr_contains_routine_call(arg, routines))
                {
                    diagnostics.push(Diagnostic::new(
                        *span,
                        "compat profile rejects function calls as routine call arguments",
                    ));
                }
                validate_compatible_expr(callee, routines, diagnostics);
                for arg in args {
                    validate_compatible_expr(arg, routines, diagnostics);
                }
            } else {
                validate_compatible_expr(expr, routines, diagnostics);
            }
        }
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            for branch in branches {
                validate_compatible_expr(&branch.condition, routines, diagnostics);
                for stmt in &branch.body {
                    validate_compatible_stmt(stmt, routines, diagnostics);
                }
            }
            for stmt in else_body {
                validate_compatible_stmt(stmt, routines, diagnostics);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            validate_compatible_expr(condition, routines, diagnostics);
            for stmt in body {
                validate_compatible_stmt(stmt, routines, diagnostics);
            }
        }
        Stmt::DoUntil {
            body, condition, ..
        } => {
            for stmt in body {
                validate_compatible_stmt(stmt, routines, diagnostics);
            }
            if let Some(condition) = condition {
                validate_compatible_expr(condition, routines, diagnostics);
            }
        }
        Stmt::For {
            target,
            start,
            end,
            step,
            body,
            ..
        } => {
            validate_compatible_expr(target, routines, diagnostics);
            validate_compatible_expr(start, routines, diagnostics);
            validate_compatible_expr(end, routines, diagnostics);
            if let Some(step) = step {
                validate_compatible_expr(step, routines, diagnostics);
            }
            for stmt in body {
                validate_compatible_stmt(stmt, routines, diagnostics);
            }
        }
        _ => {}
    }
}

pub(super) fn validate_compatible_expr(
    expr: &Expr,
    routines: &HashMap<String, RoutineInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    compatible_expr_calls(expr, routines, diagnostics);
}

#[derive(Default)]
struct CompatibleExprCalls {
    contains_call: bool,
    /// Width of an unconsumed result in the original $A0/$A1 return area.
    pending_result: Option<u16>,
}

fn compatible_expr_calls(
    expr: &Expr,
    routines: &HashMap<String, RoutineInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) -> CompatibleExprCalls {
    match &expr.kind {
        ExprKind::Cast { expr, .. }
        | ExprKind::Unary {
            op: UnaryOp::Plus,
            expr,
        } => compatible_expr_calls(expr, routines, diagnostics),
        ExprKind::Unary { expr, .. } => {
            let operand = compatible_expr_calls(expr, routines, diagnostics);
            CompatibleExprCalls {
                contains_call: operand.contains_call,
                pending_result: None,
            }
        }
        ExprKind::Binary { op, left, right } => {
            let left_calls = compatible_expr_calls(left, routines, diagnostics);
            let right_calls = compatible_expr_calls(right, routines, diagnostics);
            // Source AST order already expresses precedence and grouping. The
            // original compiler saves ordinary arithmetic temporaries across
            // calls, but rejects a call while a raw return value is still live.
            // Keep the existing separately supported comparison surface.
            if !matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            ) && left_calls.pending_result.is_some()
                && right_calls.contains_call
            {
                diagnostics.push(Diagnostic::new(
                    expr.span,
                    "compat profile rejects a function call while an earlier function result is still pending; compute the earlier result into a variable or an arithmetic intermediate first",
                ));
            }
            CompatibleExprCalls {
                contains_call: left_calls.contains_call || right_calls.contains_call,
                // Action! elides a BYTE shift by zero without consuming its
                // operand. Word shifts use a helper and produce a new temp.
                pending_result: if matches!(op, BinaryOp::Lsh | BinaryOp::Rsh)
                    && constant_u16(right) == Some(0)
                    && left_calls.pending_result == Some(1)
                {
                    left_calls.pending_result
                } else {
                    None
                },
            }
        }
        ExprKind::Call { callee, args } => {
            let mut contains_call =
                compatible_expr_calls(callee, routines, diagnostics).contains_call;
            for arg in args {
                contains_call |= compatible_expr_calls(arg, routines, diagnostics).contains_call;
            }
            let routine = if let ExprKind::Name(name) = &callee.kind {
                routines.get(&normalize_name(name))
            } else {
                None
            };
            if let Some(routine) = routine {
                if contains_call {
                    diagnostics.push(Diagnostic::new(
                        expr.span,
                        "compat profile rejects function calls as routine call arguments",
                    ));
                }
                CompatibleExprCalls {
                    contains_call: true,
                    pending_result: routine.return_slot.map(|slot| slot.size),
                }
            } else {
                // Array-call syntax is an indexed load, not a function return.
                CompatibleExprCalls {
                    contains_call,
                    pending_result: None,
                }
            }
        }
        ExprKind::Index { base, index } => {
            let base = compatible_expr_calls(base, routines, diagnostics);
            let index = compatible_expr_calls(index, routines, diagnostics);
            CompatibleExprCalls {
                contains_call: base.contains_call || index.contains_call,
                pending_result: None,
            }
        }
        ExprKind::Field { base, .. } => {
            let base = compatible_expr_calls(base, routines, diagnostics);
            CompatibleExprCalls {
                contains_call: base.contains_call,
                pending_result: None,
            }
        }
        ExprKind::Prepared { statements, value } => {
            for stmt in statements {
                validate_compatible_stmt(stmt, routines, diagnostics);
            }
            let mut calls = compatible_expr_calls(value, routines, diagnostics);
            calls.contains_call = true;
            calls
        }
        _ => CompatibleExprCalls::default(),
    }
}
