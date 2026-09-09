//! Runtime bindings reuse child lexical scopes. Never mutate an earlier scope
//! when a later LET shadows one of its names.
use super::*;

impl Analyzer {
    pub(super) fn reject_static_binding_value(&mut self, expr: &subject::SemExpr) -> bool {
        let uses_binding = expr.any_place(&|place| {
            matches!(place.kind,
            subject::SemPlaceKind::Symbol(id) if self.symbols.symbols[id.0].is_immutable)
        });
        if uses_binding {
            self.diagnostics.push(Diagnostic::new(expr.span,
                "an immutable LET binding is a runtime value, not a static initializer or array bound"));
        }
        uses_binding
    }

    pub(super) fn analyze_binding_statements(
        &mut self,
        mut scope: ScopeId,
        statements: &[Stmt],
        context: ControlContext<'_>,
    ) {
        let original_depth = self.active_lexical_path.len();
        for statement in statements {
            let Stmt::Let {
                syntax_id,
                name,
                declared_type,
                value,
                span,
            } = statement
            else {
                self.analyze_stmt(scope, statement, context);
                continue;
            };
            if !self.options.let_bindings {
                self.diagnostics
                    .push(Diagnostic::new(*span, "LET requires the modern profile"));
                continue;
            }
            let Some(routine) = self.active_routine_symbol else {
                self.diagnostics.push(Diagnostic::new(
                    *span,
                    "LET is only allowed inside routines",
                ));
                continue;
            };
            let diagnostic_count = self.diagnostics.len();
            let explicit = declared_type.as_ref().map(|ty| {
                self.validate_type_ref(scope, ty, *span);
                self.value_type_from_type_ref(scope, ty)
            });
            // The initializer belongs to the parent scope, including its casts,
            // enum members, array decay and callable resolution.
            let initializer = self.lower_expr_for_expected_type(scope, value, explicit.as_ref());
            let ty = explicit.unwrap_or_else(|| initializer.ty.clone());
            if ty.is_error() || initializer.ty.is_error() {
                // Some non-value subjects (for example a bare routine name)
                // have an error type without an expression diagnostic. Never
                // accept a LET whose binding scope could not be constructed.
                if self.diagnostics.len() == diagnostic_count {
                    self.diagnostics.push(Diagnostic::new(
                        value.span,
                        "LET requires a value expression; use @routine for a callable value",
                    ));
                }
                continue;
            }
            if self
                .array_place_types
                .contains_key(&ExpressionSite::new(scope, value.span))
                && !declared_type.as_ref().is_some_and(|_| ty.is_pointer())
            {
                self.diagnostics.push(Diagnostic::new(*span,
                    "LET array values require an explicit pointer type or cast; owned arrays are not supported"));
                continue;
            }
            if !(ty.as_scalar().is_some()
                || ty.as_enum().is_some()
                || ty.is_real()
                || ty.is_pointer()
                || (self.options.algebraic_types.aggregate_values && ty.is_record())
                || ty.as_callable_pointer().is_some())
                || matches!(
                    value.kind,
                    ExprKind::String(_) | ExprKind::InitializerList(_)
                )
            {
                self.diagnostics.push(Diagnostic::new(*span,
                    "LET requires a scalar, enum, REAL or pointer value; owned aggregates are not supported"));
                continue;
            }
            self.validate_assignment_value_type(scope, &ty, value);
            if ty.as_callable_pointer().is_some() && initializer.ty.as_callable_pointer().is_none()
            {
                self.diagnostics.push(Diagnostic::new(
                    value.span,
                    "LET callable binding requires a matching typed callable value",
                ));
            }

            let parent = scope;
            scope = self
                .symbols
                .add_scope(ScopeKind::LexicalBlock, Some(parent));
            self.active_lexical_path.push(syntax_id.0);
            self.lexical_blocks.push(SemanticLexicalBlock {
                syntax_id: *syntax_id,
                scope,
                parent,
                routine,
                module: self.active_module,
                depth: self.active_lexical_path.len(),
                ordinal: syntax_id.0,
                span: *span,
            });
            if let Some(symbol) =
                self.declare(scope, name.clone(), SymbolClass::Var, Some(ty), *span)
            {
                self.symbols.symbols[symbol.0].is_immutable = true;
            }
        }
        self.active_lexical_path.truncate(original_depth);
    }

    pub(super) fn reject_read_only_write(&mut self, place: &subject::SemPlace, span: Span) -> bool {
        if place.access != subject::PlaceAccess::ReadOnly {
            return false;
        }
        self.diagnostics.push(Diagnostic::new(
            span,
            "cannot modify an immutable LET binding",
        ));
        true
    }

    pub(super) fn reject_binding_address(&mut self, symbol: SymbolId, span: Span) -> bool {
        if !self.symbols.symbols[symbol.0].is_immutable {
            return false;
        }
        self.diagnostics.push(Diagnostic::new(
            span,
            "cannot expose the storage address of an immutable LET binding",
        ));
        true
    }

    pub(super) fn reject_read_only_address(&mut self, place: &subject::SemPlace, span: Span) -> bool {
        if place.access != subject::PlaceAccess::ReadOnly {
            return false;
        }
        self.diagnostics.push(Diagnostic::new(span,
            "cannot expose the storage address of an immutable LET binding"));
        true
    }

    pub(super) fn indexed_access(&self, base: &subject::SemPlace) -> subject::PlaceAccess {
        if self.inline_array_type(base).is_some() {
            base.access
        } else {
            // A pointer binding is immutable; its pointee is not.
            subject::PlaceAccess::Assignable
        }
    }

    pub(super) fn reject_aggregate_address_conversion(&mut self, expr: &subject::SemExpr) -> bool {
        if self.contains_union(&expr.ty) {
            self.diagnostics.push(Diagnostic::new(expr.span,
                "union-containing values cannot be used as scalars or cast to addresses; select a member or take an explicit typed address"));
            return true;
        }
        if self.contains_variant(&expr.ty) {
            self.diagnostics.push(Diagnostic::new(expr.span,
                "variant values cannot be used as scalars or cast to addresses; use CASE or an explicit typed pointer"));
            return true;
        }
        if let subject::SemExprKind::Load(place) = &expr.kind
            && place.ty.is_record()
        {
            return self.reject_read_only_address(place, expr.span);
        }
        false
    }
}
