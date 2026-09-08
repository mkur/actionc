use super::*;

/// Constant interval in the selector's representation. Signed ordering is
/// established by semantic analysis, not by ordering these storage bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseRange {
    pub low: u64,
    pub high: u64,
    pub span: Span,
}

pub(super) type CaseLabels = Vec<Option<Vec<CaseRange>>>;

pub(super) fn case_flow_facts(
    arms: impl IntoIterator<Item = StmtFlowFacts>,
    has_else: bool,
    loop_depth: usize,
) -> StmtFlowFacts {
    let mut facts = StmtFlowFacts::empty_continuing();
    facts.may_continue = !has_else;
    facts.max_loop_depth = loop_depth;
    for arm in arms {
        facts.may_continue |= arm.may_continue;
        facts.may_return |= arm.may_return;
        facts.may_exit_loop |= arm.may_exit_loop;
        facts.contains_loop |= arm.contains_loop;
        facts.max_loop_depth = facts.max_loop_depth.max(arm.max_loop_depth);
    }
    facts.always_returns = !facts.may_continue && !facts.may_exit_loop && facts.may_return;
    facts
}

impl Analyzer {
    pub(super) fn analyze_case(
        &mut self,
        scope: ScopeId,
        selector: &Expr,
        arms: &[CaseArm],
        span: Span,
        context: ControlContext<'_>,
    ) {
        if !self.options.case_statements {
            self.diagnostics
                .push(Diagnostic::new(span, "CASE requires the modern profile"));
            return;
        }
        let selector = self.lower_expr(scope, selector);
        if self.variant_for_type(&selector.ty).is_some() {
            self.analyze_variant_case(scope, &selector.ty, arms, span, context);
            return;
        }
        let Some(scalar) = selector.ty.representation_scalar() else {
            self.diagnostics.push(Diagnostic::new(
                span,
                "CASE selector must be an integer or enum",
            ));
            return;
        };
        let mut normalized = Vec::new();
        let mut previous: Vec<(i64, i64, Span)> = Vec::new();
        for arm in arms {
            let labels = arm.labels.as_ref().map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| {
                        if selector.ty.as_enum().is_some() && label.high.is_some() {
                            self.diagnostics.push(Diagnostic::new(label.span, "enum CASE ranges are not supported; convert the selector to an integer explicitly"));
                            return None;
                        }
                        let low = self.case_constant(scope, &label.low, &selector.ty, scalar)?;
                        let high = match &label.high {
                            Some(high) => self.case_constant(scope, high, &selector.ty, scalar)?,
                            None => low,
                        };
                        if low > high {
                            self.diagnostics
                                .push(Diagnostic::new(label.span, "descending CASE range"));
                            return None;
                        }
                        if let Some((_, _, earlier)) =
                            previous.iter().find(|(a, b, _)| low <= *b && high >= *a)
                        {
                            self.diagnostics.push(Diagnostic::new(
                                label.span,
                                "duplicate or overlapping CASE label",
                            ));
                            self.diagnostics.push(Diagnostic::new(
                                *earlier,
                                "previous overlapping CASE label is here",
                            ));
                        }
                        previous.push((low, high, label.span));
                        Some(CaseRange {
                            low: low as u64 & scalar_mask_for_layout(scalar, TargetLayout::for_target(self.options.target)),
                            high: high as u64 & scalar_mask_for_layout(scalar, TargetLayout::for_target(self.options.target)),
                            span: label.span,
                        })
                    })
                    .collect()
            });
            normalized.push(labels);
            self.analyze_statements(scope, &arm.body, context);
        }
        self.case_labels.insert(
            ExpressionSite {
                scope,
                start: span.start,
                end: span.end,
            },
            normalized,
        );
    }

    fn case_constant(
        &mut self,
        scope: ScopeId,
        expr: &Expr,
        selector_type: &ValueType,
        scalar: ScalarType,
    ) -> Option<i64> {
        let value = self.lower_expr(scope, expr);
        if selector_type.as_enum().is_some() || value.ty.as_enum().is_some() {
            if &value.ty != selector_type {
                self.diagnostics.push(Diagnostic::new(
                    expr.span,
                    "enum CASE label must have the exact selector enum type",
                ));
                return None;
            }
        }
        let constant = match self.evaluate_const_expr(&value) {
            Ok(value) => value,
            Err(message) => {
                self.diagnostics.push(Diagnostic::new(
                    expr.span,
                    format!("CASE label must be an integer constant: {message}"),
                ));
                return None;
            }
        };
        let numeric = exact_const_value(constant);
        let layout = TargetLayout::for_target(self.options.target);
        let width = scalar_bits_for_layout(scalar, layout);
        let (minimum, maximum) = if scalar.is_signed() {
            (-(1i64 << (width - 1)), (1i64 << (width - 1)) - 1)
        } else { (0, (1i64 << width) - 1) };
        if !(minimum..=maximum).contains(&numeric) {
            self.diagnostics.push(Diagnostic::new(
                expr.span,
                format!("CASE label {numeric} is out of range for {scalar:?}"),
            ));
            return None;
        }
        Some(numeric)
    }
}
