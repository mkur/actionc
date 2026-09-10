//! Runtime value selection is typed before any branch can be simplified.
use super::*;

impl Analyzer {
    pub(super) fn reject_static_selection(&mut self, value: &subject::SemExpr) -> bool {
        if !value.contains_runtime_selection() {
            return false;
        }
        self.diagnostics.push(Diagnostic::new(value.span,
            "selection expressions require a runtime value context; static initializers and array bounds cannot evaluate them"));
        true
    }

    pub(super) fn classify_selection(
        &mut self,
        scope: ScopeId,
        selection: &SelectionExpr,
        span: Span,
    ) -> subject::SemSubject {
        if let SelectionExpr::Case { selector, arms } = selection {
            return self.classify_case_value(scope, selector, arms, span);
        }
        let SelectionExpr::If {
            branches,
            otherwise,
        } = selection
        else {
            unreachable!();
        };
        if !self.options.if_expressions {
            self.diagnostics.push(Diagnostic::new(
                span,
                "IF expressions require the modern profile",
            ));
            return self.subject_error(span);
        }
        let mut expressions = Vec::new();
        let mut result_type = None;
        for (condition, value) in branches {
            expressions.push(self.validate_condition(scope, condition));
            let value = self.lower_expr(scope, value);
            self.check_selection_result(&mut result_type, &value);
            expressions.push(value);
        }
        let value = self.lower_expr(scope, otherwise);
        self.check_selection_result(&mut result_type, &value);
        expressions.push(value);
        subject::SemSubject::Expr(subject::SemExpr {
            ty: result_type.unwrap_or_else(ValueType::error),
            kind: subject::SemExprKind::Selection { expressions },
            span,
        })
    }

    fn classify_case_value(
        &mut self,
        scope: ScopeId,
        selector: &Expr,
        arms: &[crate::ast::CaseValueArm],
        span: Span,
    ) -> subject::SemSubject {
        if !self.options.case_expressions {
            self.diagnostics.push(Diagnostic::new(
                span,
                "CASE expressions require the modern profile",
            ));
            return self.subject_error(span);
        }
        let selector = self.lower_expr(scope, selector);
        let variant = self.variant_for_type(&selector.ty).is_some();
        if !variant
            && !arms
                .iter()
                .any(|arm| arm.header.labels.is_none() && arm.header.guard.is_none())
        {
            self.diagnostics.push(Diagnostic::new(
                span,
                "integer and enum CASE expressions require ELSE",
            ));
        }
        let mut expressions = Vec::new();
        let mut result_type = None;
        let guards = if variant {
            self.analyze_variant_case_arms(
                scope,
                &selector.ty,
                arms.iter().map(|arm| &arm.header),
                span,
                |analyzer, index, child| {
                    let value = analyzer.lower_expr(child, &arms[index].value);
                    analyzer.check_selection_result(&mut result_type, &value);
                    expressions.push(value);
                },
            )
        } else {
            self.analyze_scalar_case(
                scope,
                &selector.ty,
                arms.iter().map(|arm| &arm.header),
                span,
                |analyzer, index| {
                    let value = analyzer.lower_expr(scope, &arms[index].value);
                    analyzer.check_selection_result(&mut result_type, &value);
                    expressions.push(value);
                },
            )
        };
        expressions.push(selector);
        expressions.extend(guards);
        subject::SemSubject::Expr(subject::SemExpr {
            ty: result_type.unwrap_or_else(ValueType::error),
            kind: subject::SemExprKind::Selection { expressions },
            span,
        })
    }

    fn check_selection_result(&mut self, result: &mut Option<ValueType>, value: &subject::SemExpr) {
        if value.ty.as_scalar().is_none() && value.ty.as_enum().is_none() {
            self.diagnostics.push(Diagnostic::new(value.span,
                "selection result must be an integer or enum value; REAL, pointer and aggregate results are not supported"));
        }
        if let Some(expected) = result {
            if !value.ty.is_error() && expected != &value.ty {
                self.diagnostics.push(Diagnostic::new(value.span,
                    "selection arms must have the same integer type or enum identity; convert inside each arm explicitly"));
            }
        } else {
            *result = Some(value.ty.clone());
        }
    }
}
