use super::*;

impl SemCaseBindings {
    pub(super) fn into_block(self, span: Span) -> SemStmt {
        SemStmt::LexicalBlock {
            scope: self.scope,
            declarations: self.declarations,
            constants: Vec::new(),
            body: self.initialization,
            span,
        }
    }
}

impl IrBuilder<'_> {
    pub(super) fn lower_case_value(
        &mut self,
        scope: ScopeId,
        selector: &Expr,
        arms: &[crate::ast::CaseValueArm],
        span: Span,
    ) -> SemCaseValue {
        if self
            .model
            .variants
            .matches
            .contains_key(&super::super::ExpressionSite::new(scope, span))
        {
            let (preparation, selector, arms) = self.lower_variant_dispatch(
                scope,
                selector,
                arms.iter().map(|arm| &arm.header),
                span,
                |builder, index, bindings| {
                    let value = builder.lower_expr(bindings.scope.scope, &arms[index].value);
                    SemCaseResult::Yield {
                        preparation: vec![bindings.into_block(arms[index].header.span)],
                        value,
                    }
                },
                SemCaseResult::Fault {
                    kind: crate::runtime_fault::RuntimeFault::InvalidVariantTag,
                    span,
                },
            );
            SemCaseValue {
                preparation,
                selector,
                arms,
            }
        } else {
            SemCaseValue {
                preparation: Vec::new(),
                selector: self.lower_expr(scope, selector),
                arms: self.lower_scalar_case_arms(
                    scope,
                    arms.iter().map(|arm| &arm.header),
                    span,
                    |builder, index| SemCaseResult::Yield {
                        preparation: Vec::new(),
                        value: builder.lower_expr(scope, &arms[index].value),
                    },
                ),
            }
        }
    }

    /// Resolve scalar dispatch once for either statement or value bodies.
    pub(super) fn lower_scalar_case_arms<'a, B>(
        &mut self,
        scope: ScopeId,
        arms: impl Iterator<Item = &'a crate::ast::CaseArm>,
        span: Span,
        mut body: impl FnMut(&mut Self, usize) -> B,
    ) -> Vec<SemCaseArm<B>> {
        let labels = self
            .model
            .case_labels
            .get(&super::super::ExpressionSite::new(scope, span))
            .expect("validated CASE labels")
            .clone();
        arms.zip(labels)
            .enumerate()
            .map(|(index, (arm, labels))| SemCaseArm {
                guard: arm.guard.as_ref().map(|guard| SemCaseGuard {
                    bindings: None,
                    condition: self.lower_condition(scope, guard),
                }),
                tests: Vec::new(),
                labels,
                body: body(self, index),
                span: arm.span,
            })
            .collect()
    }
}
