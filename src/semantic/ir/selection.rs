use super::*;

impl IrBuilder<'_> {
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
