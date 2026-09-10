//! Shared storage walks include preparation and lexical scopes inside expressions.
use super::*;

trait Visitor<'a> {
    fn declaration(&mut self, _ordinal: u32, _declaration: &'a SemDeclaration) {}
    fn statement(&mut self, _statement: &'a SemStmt) {}
}

/// Visit every lexical declaration, including selection binders in any consumer.
pub fn visit_lexical_declarations<'a>(
    body: &'a [SemStmt],
    visitor: &mut impl FnMut(u32, &'a SemDeclaration),
) {
    struct Declarations<F>(F);
    impl<'a, F: FnMut(u32, &'a SemDeclaration)> Visitor<'a> for Declarations<F> {
        fn declaration(&mut self, ordinal: u32, declaration: &'a SemDeclaration) {
            self.0(ordinal, declaration);
        }
    }
    statements(body, &mut Declarations(visitor));
}

/// Visit executable statements recursively, including expression preparation.
pub fn visit_nested_statements<'a>(body: &'a [SemStmt], visitor: &mut impl FnMut(&'a SemStmt)) {
    struct Statements<F>(F);
    impl<'a, F: FnMut(&'a SemStmt)> Visitor<'a> for Statements<F> {
        fn statement(&mut self, statement: &'a SemStmt) {
            self.0(statement);
        }
    }
    statements(body, &mut Statements(visitor));
}

fn statements<'a>(body: &'a [SemStmt], visitor: &mut impl Visitor<'a>) {
    for statement in body {
        visitor.statement(statement);
        match statement {
            SemStmt::Case { selector, arms, .. } => {
                expression(selector, visitor);
                for arm in arms {
                    bindings(arm.bindings(), visitor);
                    for condition in arm.conditions() {
                        expression(&condition.expr, visitor);
                    }
                    statements(arm.preparation(), visitor);
                    statements(&arm.body, visitor);
                }
            }
            SemStmt::LexicalBlock {
                scope,
                declarations,
                body,
                ..
            } => {
                for declaration in declarations {
                    visitor.declaration(scope.ordinal, declaration);
                    if let Some(value) = &declaration.initializer {
                        expression(value, visitor);
                    }
                }
                statements(body, visitor);
            }
            SemStmt::Assign { target, value, .. }
            | SemStmt::CompoundAssign { target, value, .. } => {
                place(target, visitor);
                expression(value, visitor);
            }
            SemStmt::RecordCopy {
                destination,
                source,
                ..
            } => {
                place(destination, visitor);
                place(source, visitor);
            }
            SemStmt::Return { value, .. } => {
                if let Some(value) = value {
                    expression(value, visitor);
                }
            }
            SemStmt::Call { call: value, .. } => call(value, visitor),
            SemStmt::If {
                branches,
                else_body,
                ..
            } => {
                for branch in branches {
                    expression(&branch.condition.expr, visitor);
                    statements(&branch.body, visitor);
                }
                statements(else_body, visitor);
            }
            SemStmt::While {
                condition, body, ..
            } => {
                expression(&condition.expr, visitor);
                statements(body, visitor);
            }
            SemStmt::DoUntil {
                condition, body, ..
            } => {
                statements(body, visitor);
                if let Some(condition) = condition {
                    expression(&condition.expr, visitor);
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
                place(target, visitor);
                expression(start, visitor);
                expression(end, visitor);
                if let Some(step) = step {
                    expression(step, visitor);
                }
                statements(body, visitor);
            }
            SemStmt::Define(_)
            | SemStmt::Exit { .. }
            | SemStmt::Fault { .. }
            | SemStmt::MachineBlock { .. }
            | SemStmt::InlineAsm { .. }
            | SemStmt::Unsupported { .. } => {}
        }
    }
}

fn bindings<'a>(bindings: Option<&'a SemCaseBindings>, visitor: &mut impl Visitor<'a>) {
    if let Some(bindings) = bindings {
        for declaration in &bindings.declarations {
            visitor.declaration(bindings.scope.ordinal, declaration);
            if let Some(value) = &declaration.initializer {
                expression(value, visitor);
            }
        }
    }
}

fn expression<'a>(value: &'a SemExpr, visitor: &mut impl Visitor<'a>) {
    match &value.kind {
        SemExprKind::IfValue(selection) => {
            for value in selection.expressions() {
                expression(value, visitor);
            }
        }
        SemExprKind::CaseValue(selection) => {
            for arm in &selection.arms {
                bindings(arm.bindings(), visitor);
            }
            for body in selection.statement_lists() {
                statements(body, visitor);
            }
            for value in selection.expressions() {
                expression(value, visitor);
            }
        }
        SemExprKind::LValue(value) | SemExprKind::AddressOf(value) => place(value, visitor),
        SemExprKind::ImplicitAddressOf(value) => place(&value.place, visitor),
        SemExprKind::ArrayDecay(value) => place(&value.array, visitor),
        SemExprKind::Cast { expr, .. } | SemExprKind::Unary { expr, .. } => {
            expression(expr, visitor)
        }
        SemExprKind::Binary { left, right, .. } => {
            expression(left, visitor);
            expression(right, visitor);
        }
        SemExprKind::Call(value) => call(value, visitor),
        SemExprKind::Missing
        | SemExprKind::Raw(_)
        | SemExprKind::InitializerList(_)
        | SemExprKind::UnresolvedName(_)
        | SemExprKind::CurrentLocation
        | SemExprKind::Literal(_)
        | SemExprKind::Symbol(_)
        | SemExprKind::AddressOfSymbol(_) => {}
    }
}

fn place<'a>(value: &'a SemLValue, visitor: &mut impl Visitor<'a>) {
    match &value.kind {
        SemLValueKind::Deref { pointer } => expression(pointer, visitor),
        SemLValueKind::Index { base, index, .. } => {
            expression(base, visitor);
            expression(index, visitor);
        }
        SemLValueKind::Field { base, .. } => place(base, visitor),
        SemLValueKind::Symbol(_) | SemLValueKind::UnresolvedName(_) => {}
    }
}

fn call<'a>(value: &'a SemCall, visitor: &mut impl Visitor<'a>) {
    statements(&value.preparation, visitor);
    if let Some(result) = &value.aggregate_result {
        place(result, visitor);
    }
    if let SemCallable::Indirect { target, .. } = &value.callee {
        expression(target, visitor);
    }
    for argument in &value.args {
        expression(argument, visitor);
    }
}
