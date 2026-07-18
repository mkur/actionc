use super::*;

pub(super) fn expr_contains_routine_call(
    expr: &Expr,
    routines: &HashMap<String, RoutineInfo>,
) -> bool {
    match &expr.kind {
        ExprKind::Call { callee, args } => {
            let callee_is_routine = match &callee.kind {
                ExprKind::Name(name) => routines.contains_key(&normalize_name(name)),
                _ => false,
            };
            callee_is_routine
                || expr_contains_routine_call(callee, routines)
                || args
                    .iter()
                    .any(|arg| expr_contains_routine_call(arg, routines))
        }
        ExprKind::Unary { expr, .. } => expr_contains_routine_call(expr, routines),
        ExprKind::Binary { left, right, .. } => {
            expr_contains_routine_call(left, routines)
                || expr_contains_routine_call(right, routines)
        }
        ExprKind::Index { base, index } => {
            expr_contains_routine_call(base, routines)
                || expr_contains_routine_call(index, routines)
        }
        ExprKind::Field { base, .. } => expr_contains_routine_call(base, routines),
        _ => false,
    }
}

pub(super) fn expr_is_pointer_deref_name(expr: &Expr, name: &str) -> bool {
    let ExprKind::Unary {
        op: UnaryOp::Deref,
        expr,
    } = &expr.kind
    else {
        return false;
    };
    let ExprKind::Name(deref_name) = &expr.kind else {
        return false;
    };
    normalize_name(deref_name) == normalize_name(name)
}

pub(super) fn routine_absolute_system_address(routine: &Routine) -> Option<u16> {
    let expr = routine.system_address.as_ref()?;
    if matches!(expr.kind, ExprKind::CurrentLocation) {
        None
    } else {
        constant_u16(expr)
    }
}

pub(super) fn routine_is_current_location(routine: &Routine) -> bool {
    matches!(
        routine.system_address.as_ref().map(|expr| &expr.kind),
        Some(ExprKind::CurrentLocation)
    )
}

pub(super) fn routine_parameter_names(routine: &Routine) -> HashSet<String> {
    routine
        .params
        .iter()
        .flat_map(|param| param.entries.iter())
        .map(|entry| normalize_name(&entry.name))
        .collect()
}

pub(super) fn stmt_list_contains_machine_block(body: &[Stmt]) -> bool {
    body.iter().any(stmt_contains_machine_block)
}

fn stmt_contains_machine_block(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::MachineBlock { .. } => true,
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            branches
                .iter()
                .any(|branch| stmt_list_contains_machine_block(&branch.body))
                || stmt_list_contains_machine_block(else_body)
        }
        Stmt::While { body, .. } | Stmt::DoUntil { body, .. } | Stmt::For { body, .. } => {
            stmt_list_contains_machine_block(body)
        }
        Stmt::Define(_)
        | Stmt::Assign { .. }
        | Stmt::CompoundAssign { .. }
        | Stmt::Return(_)
        | Stmt::Exit { .. }
        | Stmt::Call { .. }
        | Stmt::Unsupported { .. } => false,
    }
}

pub(super) fn stmt_list_takes_address_of_names(body: &[Stmt], names: &HashSet<String>) -> bool {
    stmt_list_exprs_any(body, &|expr| {
        matches!(
            &expr.kind,
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr,
            } if expr_references_names(expr, names)
        )
    })
}

pub(super) fn stmt_list_contains_current_location(body: &[Stmt]) -> bool {
    stmt_list_exprs_any(body, &|expr| matches!(expr.kind, ExprKind::CurrentLocation))
}

fn stmt_list_exprs_any(body: &[Stmt], predicate: &impl Fn(&Expr) -> bool) -> bool {
    body.iter().any(|stmt| stmt_exprs_any(stmt, predicate))
}

fn stmt_exprs_any(stmt: &Stmt, predicate: &impl Fn(&Expr) -> bool) -> bool {
    match stmt {
        Stmt::Assign { target, value, .. } | Stmt::CompoundAssign { target, value, .. } => {
            expr_tree_any(target, predicate) || expr_tree_any(value, predicate)
        }
        Stmt::Return(Some(expr)) | Stmt::Call { expr, .. } => expr_tree_any(expr, predicate),
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            branches.iter().any(|branch| {
                expr_tree_any(&branch.condition, predicate)
                    || stmt_list_exprs_any(&branch.body, predicate)
            }) || stmt_list_exprs_any(else_body, predicate)
        }
        Stmt::While {
            condition, body, ..
        } => expr_tree_any(condition, predicate) || stmt_list_exprs_any(body, predicate),
        Stmt::DoUntil {
            condition, body, ..
        } => {
            condition
                .as_ref()
                .is_some_and(|condition| expr_tree_any(condition, predicate))
                || stmt_list_exprs_any(body, predicate)
        }
        Stmt::For {
            target,
            start,
            end,
            step,
            body,
            ..
        } => {
            expr_tree_any(target, predicate)
                || expr_tree_any(start, predicate)
                || expr_tree_any(end, predicate)
                || step
                    .as_ref()
                    .is_some_and(|step| expr_tree_any(step, predicate))
                || stmt_list_exprs_any(body, predicate)
        }
        Stmt::Define(_)
        | Stmt::Return(None)
        | Stmt::Exit { .. }
        | Stmt::MachineBlock { .. }
        | Stmt::Unsupported { .. } => false,
    }
}

fn expr_tree_any(expr: &Expr, predicate: &impl Fn(&Expr) -> bool) -> bool {
    if predicate(expr) {
        return true;
    }
    match &expr.kind {
        ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } => {
            expr_tree_any(expr, predicate)
        }
        ExprKind::Binary { left, right, .. } => {
            expr_tree_any(left, predicate) || expr_tree_any(right, predicate)
        }
        ExprKind::Call { callee, args } => {
            expr_tree_any(callee, predicate) || args.iter().any(|arg| expr_tree_any(arg, predicate))
        }
        ExprKind::Index { base, index } => {
            expr_tree_any(base, predicate) || expr_tree_any(index, predicate)
        }
        ExprKind::Field { base, .. } => expr_tree_any(base, predicate),
        ExprKind::Number(_)
        | ExprKind::Char(_)
        | ExprKind::String(_)
        | ExprKind::Name(_)
        | ExprKind::CurrentLocation
        | ExprKind::Missing
        | ExprKind::Raw => false,
    }
}

fn expr_references_names(expr: &Expr, names: &HashSet<String>) -> bool {
    match &expr.kind {
        ExprKind::Name(name) => names.contains(&normalize_name(name)),
        ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } => {
            expr_references_names(expr, names)
        }
        ExprKind::Binary { left, right, .. } => {
            expr_references_names(left, names) || expr_references_names(right, names)
        }
        ExprKind::Call { callee, args } => {
            expr_references_names(callee, names)
                || args.iter().any(|arg| expr_references_names(arg, names))
        }
        ExprKind::Index { base, index } => {
            expr_references_names(base, names) || expr_references_names(index, names)
        }
        ExprKind::Field { base, .. } => expr_references_names(base, names),
        ExprKind::Number(_)
        | ExprKind::Char(_)
        | ExprKind::String(_)
        | ExprKind::CurrentLocation
        | ExprKind::Missing
        | ExprKind::Raw => false,
    }
}

pub(super) fn single_int_scalar_param_name(routine: &Routine) -> Option<&str> {
    if !matches!(
        routine.kind,
        RoutineKind::Func {
            return_type: FundType::Int
        }
    ) {
        return None;
    }
    let [param] = routine.params.as_slice() else {
        return None;
    };
    if param.storage != VarStorage::Plain
        || param.ty.pointer
        || !matches!(param.ty.base, TypeBase::Fund(FundType::Int))
    {
        return None;
    }
    let [entry] = param.entries.as_slice() else {
        return None;
    };
    if entry.size.is_some() || entry.initializer.is_some() {
        return None;
    }
    Some(&entry.name)
}

pub(super) fn routine_body_is_abs_return(body: &[Stmt], param_name: &str) -> bool {
    let [
        Stmt::If {
            branches,
            else_body,
            ..
        },
        Stmt::Return(Some(fallback)),
    ] = body
    else {
        return false;
    };
    if !else_body.is_empty() {
        return false;
    }
    let [branch] = branches.as_slice() else {
        return false;
    };
    let [Stmt::Return(Some(negative))] = branch.body.as_slice() else {
        return false;
    };
    expr_is_signed_name_zero_compare(&branch.condition, BinaryOp::Lt, param_name)
        && expr_is_negated_name(negative, param_name)
        && expr_is_name(fallback, param_name)
}

pub(super) fn expr_is_signed_name_zero_compare(expr: &Expr, op: BinaryOp, name: &str) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Binary { op: actual, left, right }
            if *actual == op && expr_is_name(left, name) && constant_u16(right) == Some(0)
    )
}

pub(super) fn expr_is_negated_name(expr: &Expr, name: &str) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Unary { op: UnaryOp::Neg, expr } if expr_is_name(expr, name)
    )
}

pub(super) fn expr_is_name(expr: &Expr, name: &str) -> bool {
    matches!(&expr.kind, ExprKind::Name(actual) if normalize_name(actual) == normalize_name(name))
}

pub(super) fn routine_body_ends_explicitly(routine: &Routine) -> bool {
    matches!(
        routine.body.last(),
        Some(Stmt::Return(_)) | Some(Stmt::MachineBlock { .. })
    ) || matches!(
        routine.body.last(),
        Some(Stmt::DoUntil {
            condition: None,
            body,
            ..
        }) if !loop_body_can_exit_current_loop(body)
    )
}

pub(super) fn is_bare_return(stmt: Option<&Stmt>) -> bool {
    matches!(stmt, Some(Stmt::Return(None)))
}

pub(super) fn stmt_list_ends_with_terminal_flow(body: &[Stmt]) -> bool {
    body.last().is_some_and(stmt_ends_with_terminal_flow)
}

pub(super) fn stmt_list_ends_with_value_return(body: &[Stmt]) -> bool {
    body.last().is_some_and(stmt_ends_with_value_return)
}

pub(super) fn stmt_ends_with_value_return(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return(Some(_)) => true,
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            !else_body.is_empty()
                && branches
                    .iter()
                    .all(|branch| stmt_list_ends_with_value_return(&branch.body))
                && stmt_list_ends_with_value_return(else_body)
        }
        _ => false,
    }
}

pub(super) fn stmt_ends_with_terminal_flow(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return(_) | Stmt::MachineBlock { .. } => true,
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            !else_body.is_empty()
                && branches
                    .iter()
                    .all(|branch| stmt_list_ends_with_terminal_flow(&branch.body))
                && stmt_list_ends_with_terminal_flow(else_body)
        }
        Stmt::DoUntil {
            condition: None,
            body,
            ..
        } => !loop_body_can_exit_current_loop(body),
        _ => false,
    }
}

pub(super) fn loop_body_can_exit_current_loop(body: &[Stmt]) -> bool {
    body.iter().any(stmt_can_exit_current_loop)
}

pub(super) fn stmt_can_exit_current_loop(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Exit { .. } => true,
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            branches
                .iter()
                .any(|branch| loop_body_can_exit_current_loop(&branch.body))
                || loop_body_can_exit_current_loop(else_body)
        }
        // EXIT targets the innermost loop, so exits inside nested loops do not
        // make the enclosing final DO fall through.
        Stmt::While { .. } | Stmt::DoUntil { .. } | Stmt::For { .. } => false,
        _ => false,
    }
}
