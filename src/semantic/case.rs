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
        self.analyze_scalar_case(scope, &selector.ty, arms.iter(), span, |analyzer, index| {
            analyzer.analyze_statements(scope, &arms[index].body, context);
        });
    }

    /// The same checked intervals and guards serve statement and value bodies.
    /// Analyze each body after its header, preserving source diagnostic order.
    pub(super) fn analyze_scalar_case<'a>(
        &mut self,
        scope: ScopeId,
        selector_type: &ValueType,
        arms: impl Iterator<Item = &'a CaseArm>,
        span: Span,
        mut body: impl FnMut(&mut Self, usize),
    ) -> Vec<subject::SemExpr> {
        let Some(scalar) = selector_type.representation_scalar() else {
            self.diagnostics.push(Diagnostic::new(
                span,
                "CASE selector must be an integer or enum",
            ));
            return Vec::new();
        };
        let mut guards = Vec::new();
        let mut normalized = Vec::new();
        let mut previous: Vec<(i64, i64, Span)> = Vec::new();
        let layout = TargetLayout::for_target(self.options.target);
        for (index, arm) in arms.enumerate() {
            if let Some(guard) = self.validate_case_guard(scope, arm) {
                guards.push(guard);
            }
            let wildcard = arm.labels.as_ref().is_some_and(|labels| {
                labels.len() == 1
                    && labels[0].high.is_none()
                    && matches!(&labels[0].low.kind, ExprKind::Name(name) if name == "_")
            });
            let mut current = Vec::new();
            let labels = if wildcard {
                if arm.guard.is_none() {
                    self.diagnostics.push(Diagnostic::new(
                        arm.span,
                        "bare CASE wildcard requires a guard; use ELSE",
                    ));
                }
                None
            } else {
                arm.labels.as_ref().map(|labels| labels.iter().filter_map(|label| {
                    if selector_type.as_enum().is_some() && label.high.is_some() {
                        self.diagnostics.push(Diagnostic::new(label.span, "enum CASE ranges are not supported; convert the selector to an integer explicitly"));
                        return None;
                    }
                    let low = self.case_constant(scope, &label.low, selector_type, scalar)?;
                    let high = match &label.high {
                        Some(high) => self.case_constant(scope, high, selector_type, scalar)?,
                        None => low,
                    };
                    if low > high {
                        self.diagnostics.push(Diagnostic::new(label.span, "descending CASE range"));
                        return None;
                    }
                    // Preserve disjoint unguarded labels and reject duplicates
                    // within one header, even when that header is guarded.
                    let overlap = current.iter().chain(previous.iter().filter(|_| arm.guard.is_none()))
                        .find(|(a, b, _)| low <= *b && high >= *a);
                    if let Some((_, _, earlier)) = overlap {
                        self.diagnostics.push(Diagnostic::new(label.span, "duplicate or overlapping CASE label"));
                        self.diagnostics.push(Diagnostic::new(*earlier, "previous overlapping CASE label is here"));
                    }
                    current.push((low, high, label.span));
                    Some(CaseRange {
                        low: low as u64 & scalar_mask_for_layout(scalar, layout),
                        high: high as u64 & scalar_mask_for_layout(scalar, layout), span: label.span,
                    })
                }).collect())
            };
            if arm.guard.is_some() {
                let width = scalar_bits_for_layout(scalar, layout);
                let domain = if scalar.is_signed() {
                    (-(1i64 << (width - 1)), (1i64 << (width - 1)) - 1)
                } else {
                    (0, (1i64 << width) - 1)
                };
                let covered = if wildcard {
                    interval_covered(domain.0, domain.1, &previous)
                } else {
                    !current.is_empty()
                        && current
                            .iter()
                            .all(|(low, high, _)| interval_covered(*low, *high, &previous))
                };
                if covered {
                    self.diagnostics.push(Diagnostic::new(
                        arm.span,
                        "guarded CASE arm is fully shadowed by earlier unconditional labels",
                    ));
                }
            } else {
                previous.extend(current);
            }
            normalized.push(labels);
            body(self, index);
        }
        self.case_labels.insert(
            ExpressionSite {
                scope,
                start: span.start,
                end: span.end,
            },
            normalized,
        );
        guards
    }

    pub(super) fn validate_case_guard(&mut self, scope: ScopeId, arm: &CaseArm) -> Option<subject::SemExpr> {
        if let Some(guard) = &arm.guard {
            if !self.options.algebraic_types.case_guards {
                self.diagnostics.push(Diagnostic::new(
                    guard.span,
                    "CASE guards require the modern profile and enabled guard capability",
                ));
            } else {
                return Some(self.validate_condition(scope, guard));
            }
        }
        None
    }

    pub(super) fn case_constant(
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
        } else {
            (0, (1i64 << width) - 1)
        };
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

fn interval_covered(low: i64, high: i64, previous: &[(i64, i64, Span)]) -> bool {
    let mut intervals = previous
        .iter()
        .map(|(a, b, _)| (*a, *b))
        .collect::<Vec<_>>();
    intervals.sort_unstable();
    let mut next = low;
    for (a, b) in intervals {
        if b < next {
            continue;
        }
        if a > next {
            return false;
        }
        if b >= high {
            return true;
        }
        next = b + 1;
    }
    false
}
