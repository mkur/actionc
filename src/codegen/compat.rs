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
        Stmt::Return(Some(expr)) => validate_compatible_expr(expr, routines, diagnostics),
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
            if let ExprKind::Call { args, .. } = &expr.kind
                && args
                    .iter()
                    .any(|arg| expr_contains_routine_call(arg, routines))
            {
                diagnostics.push(Diagnostic::new(
                    *span,
                    "compat profile rejects function calls as routine call arguments",
                ));
            }
            validate_compatible_expr(expr, routines, diagnostics);
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
    match &expr.kind {
        ExprKind::Unary { expr, .. } => validate_compatible_expr(expr, routines, diagnostics),
        ExprKind::Binary { op, left, right } => {
            if compatible_binary_rejects_call_operands(*op)
                && (expr_contains_routine_call(left, routines)
                    || expr_contains_routine_call(right, routines))
                && !compatible_binary_call_operand_is_identity(*op, left, right)
                && !compatible_binary_call_operand_is_supported_runtime_op(
                    *op, left, right, routines,
                )
                && !compatible_binary_call_operand_is_supported_single_call_op(
                    *op, left, right, routines,
                )
            {
                diagnostics.push(Diagnostic::new(
                    expr.span,
                    "compat profile rejects function calls in arithmetic expressions",
                ));
            }
            validate_compatible_expr(left, routines, diagnostics);
            validate_compatible_expr(right, routines, diagnostics);
        }
        ExprKind::Call { callee, args } => {
            validate_compatible_expr(callee, routines, diagnostics);
            for arg in args {
                validate_compatible_expr(arg, routines, diagnostics);
            }
        }
        ExprKind::Index { base, index } => {
            validate_compatible_expr(base, routines, diagnostics);
            validate_compatible_expr(index, routines, diagnostics);
        }
        ExprKind::Field { base, .. } => validate_compatible_expr(base, routines, diagnostics),
        _ => {}
    }
}

pub(super) fn compatible_binary_rejects_call_operands(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::Lsh
            | BinaryOp::Rsh
    )
}

pub(super) fn compatible_binary_call_operand_is_identity(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
) -> bool {
    match op {
        BinaryOp::Add | BinaryOp::Or | BinaryOp::Xor => {
            constant_u16(left) == Some(0) || constant_u16(right) == Some(0)
        }
        BinaryOp::Sub | BinaryOp::Lsh | BinaryOp::Rsh => constant_u16(right) == Some(0),
        _ => false,
    }
}

pub(super) fn compatible_binary_call_operand_is_supported_runtime_op(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    routines: &HashMap<String, RoutineInfo>,
) -> bool {
    if !matches!(op, BinaryOp::Mul) {
        return false;
    }
    let left_call = expr_contains_routine_call(left, routines);
    let right_call = expr_contains_routine_call(right, routines);
    left_call ^ right_call
}

pub(super) fn compatible_binary_call_operand_is_supported_single_call_op(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    routines: &HashMap<String, RoutineInfo>,
) -> bool {
    let left_call = expr_contains_routine_call(left, routines);
    let right_call = expr_contains_routine_call(right, routines);
    match op {
        BinaryOp::Add => left_call ^ right_call,
        BinaryOp::Sub => left_call && !right_call,
        BinaryOp::Lsh | BinaryOp::Rsh => left_call && !right_call && constant_u16(right).is_some(),
        _ => false,
    }
}
