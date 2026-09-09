//! Fresh runtime initialization is not replacement assignment. These bounded
//! semantic proofs avoid creating staging homes; they do not forward arbitrary
//! existing storage or infer purity from a user call's default SemEffects.
use super::*;

impl IrBuilder<'_> {
    pub(super) fn lower_fresh_aggregate_initialization(
        &mut self,
        scope: ScopeId,
        destination: SemLValue,
        value: &Expr,
        span: Span,
    ) -> Vec<SemStmt> {
        // Only called for a newly introduced binding, whose identity/address
        // cannot occur in its own RHS. A field is NOT a whole ABI result home.
        assert!(matches!(&destination.kind, SemLValueKind::Symbol(s) if s.is_immutable));
        let constructor = self
            .model
            .variants
            .expressions
            .contains_key(&super::super::ExpressionSite::new(scope, value.span));
        let call = self.is_aggregate_call_source(scope, value);
        if (constructor && self.inert_fresh_initializer(scope, value))
            || (call
                && self.model.target_layout.routine_activation
                    == crate::target::RoutineActivationModel::NativeReentrant)
        {
            // An automatic binding is inaccessible during a callee's execution
            // (including reentry); the Atari routine-static home is not. Keep
            // argument captures/result validation and the complete-result ABI.
            return self.initialize_aggregate_value(scope, destination, value);
        }
        if !constructor && !call && self.ordinary_initializer_place(scope, value) {
            let source = self.lower_lvalue(scope, value);
            // Checked snapshots validate before publication, including when a
            // custom Error handler runs. A direct ordinary source has no
            // address effects; successful validation cannot change its bytes.
            let mut result = self.validate_aggregate_value(scope, &source);
            result.push(self.copy_value(destination, source, span));
            return result;
        }
        // Unknown backing, pointer/index accesses, volatile reads, possible
        // faults and routine-static calls retain the existing staged/checking
        // paths. In particular, freshness does not establish global no-alias.
        if self.aggregate_requires_validation(&destination.ty) || call {
            self.lower_checked_aggregate_assignment(scope, destination, value, span)
        } else {
            vec![self.lower_aggregate_copy(scope, destination, value, span)]
        }
    }

    /// No user/runtime calls, possible faults, REAL operations or uncertain
    /// memory reads during construction. Runtime integer inputs are allowed;
    /// this is deliberately not a constant-constructor special case.
    pub(super) fn inert_fresh_initializer(&self, scope: ScopeId, expr: &Expr) -> bool {
        let site = super::super::ExpressionSite::new(scope, expr.span);
        if let Some(id) = self.model.variants.expressions.get(&site) {
            let constructor =
                &self.model.variants.types[&id.owner].constructors[usize::from(id.tag) - 1];
            let args = match &expr.kind {
                ExprKind::Call { args, .. } => args.as_slice(),
                _ => &[],
            };
            return args.len() == constructor.fields.len()
                && args.iter().zip(&constructor.fields).all(|(arg, field)| {
                    !self.model.fields[field.0].ty.is_real()
                        && self.inert_fresh_initializer(scope, arg)
                });
        }
        if self.model.enums.member_values.contains_key(&site)
            || self.model.layout_query_value(scope, expr.span).is_some()
        {
            return true;
        }
        if let Some(ty) = self.model.resolved_casts.get(&site) {
            return !ty.is_real()
                && matches!(&expr.kind, ExprKind::Call { args, .. }
                    if args.len() == 1 && self.inert_fresh_initializer(scope, &args[0]));
        }
        match &expr.kind {
            ExprKind::Number(number) => number.kind != crate::lexer::NumberKind::Real,
            ExprKind::Char(_) => true,
            ExprKind::Name(_) | ExprKind::Field { .. } => {
                if let Some(symbol) = self.direct_symbol_ref_for_expr(scope, expr)
                    && (self.model.constants.contains_key(&symbol.id)
                        || self.model.enums.constants.contains_key(&symbol.id)
                        || self
                            .numeric_defines
                            .get(&symbol.id)
                            .is_some_and(|n| n.kind != crate::lexer::NumberKind::Real))
                {
                    return true;
                }
                self.ordinary_initializer_place(scope, expr)
                    && self
                        .lvalue_expr_type(scope, expr)
                        .is_some_and(|ty| !ty.is_real() && !self.aggregate_requires_validation(&ty))
            }
            ExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Neg,
                expr,
            } => self.inert_fresh_initializer(scope, expr),
            ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr,
            } => self.ordinary_initializer_place(scope, expr),
            ExprKind::Cast { ty, expr } => {
                !self.resolved_type_ref(scope, ty).is_real()
                    && self.inert_fresh_initializer(scope, expr)
            }
            ExprKind::Binary { op, left, right } => {
                !matches!(op, BinaryOp::Div | BinaryOp::Mod)
                    && self.inert_fresh_initializer(scope, left)
                    && self.inert_fresh_initializer(scope, right)
            }
            _ => false,
        }
    }

    fn ordinary_initializer_place(&self, scope: ScopeId, expr: &Expr) -> bool {
        if let Some(symbol) = self.direct_symbol_ref_for_expr(scope, expr) {
            return !symbol.is_volatile
                && (self.ordinary_value_symbols.contains(&symbol.id)
                    || (symbol.class == SymbolClass::Param && !self.is_array_symbol(symbol.id)));
        }
        if let ExprKind::Field { base, field } = &expr.kind {
            return self.ordinary_initializer_place(scope, base)
                && self.lvalue_expr_type(scope, base).is_some_and(|ty| {
                    // A field of a pointer is an unknown memory access, not an
                    // inline subobject of the ordinary pointer variable.
                    ty.is_record() && self.field_descriptor(&ty, field).is_some()
                });
        }
        false
    }
}
