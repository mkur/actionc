//! Array-place queries share field/symbol identity and canonical array types.
//! Inline fields are addressable storage, never implicit scalar loads or stores.

use super::*;

impl SemanticModel {
    pub(crate) fn array_place_type(&self, scope: ScopeId, span: Span) -> Option<&ArrayType> {
        self.array_place_types
            .get(&ExpressionSite::new(scope, span))
    }
}

impl Analyzer {
    // Static subobject initialization is a later rollout slice. Inspect typed
    // expressions so unevaluated layout-query operands do not trigger the gate.
    pub(super) fn expression_uses_inline_array(&self, expr: &subject::SemExpr) -> bool {
        use subject::SemExprKind;
        match &expr.kind {
            SemExprKind::Load(place) | SemExprKind::AddressOf(place) => {
                self.place_uses_inline_array(place)
            }
            SemExprKind::Cast { expr, .. } | SemExprKind::Unary { expr, .. } => {
                self.expression_uses_inline_array(expr)
            }
            SemExprKind::Binary { left, right, .. } => {
                self.expression_uses_inline_array(left) || self.expression_uses_inline_array(right)
            }
            SemExprKind::Call { callee, args } => {
                matches!(&callee.kind, subject::SemCallableKind::FunctionValue(expr)
                    if self.expression_uses_inline_array(expr))
                    || args
                        .iter()
                        .any(|arg| self.expression_uses_inline_array(arg))
            }
            SemExprKind::Literal(_)
            | SemExprKind::CurrentLocation
            | SemExprKind::AddressOfSymbol(_)
            | SemExprKind::Raw(_)
            | SemExprKind::Error => false,
        }
    }

    fn place_uses_inline_array(&self, place: &subject::SemPlace) -> bool {
        if self.inline_array_type(place).is_some() {
            return true;
        }
        match &place.kind {
            subject::SemPlaceKind::Field { base, .. } => self.place_uses_inline_array(base),
            subject::SemPlaceKind::Index { base, index } => {
                self.place_uses_inline_array(base) || self.expression_uses_inline_array(index)
            }
            subject::SemPlaceKind::Deref(expr) => self.expression_uses_inline_array(expr),
            subject::SemPlaceKind::Symbol(_) | subject::SemPlaceKind::Error => false,
        }
    }

    pub(super) fn inline_array_type(&self, place: &subject::SemPlace) -> Option<ArrayType> {
        let subject::SemPlaceKind::Field { field, .. } = &place.kind else {
            return None;
        };
        let field = self.fields.get(field.id?.0)?;
        match &field.storage {
            RecordFieldStorage::InlineArray { array_type, .. } => Some(array_type.clone()),
            RecordFieldStorage::Value => None,
        }
    }

    pub(super) fn array_place_type(&self, place: &subject::SemPlace) -> Option<ArrayType> {
        if let subject::SemPlaceKind::Symbol(id) = &place.kind {
            self.array_element_type(*id)
                .map(|element| ArrayType::new(element, self.array_lengths.get(id).copied()))
        } else {
            self.inline_array_type(place)
        }
    }

    pub(super) fn reject_inline_array_target(
        &mut self,
        place: &subject::SemPlace,
        span: Span,
    ) -> bool {
        if self.inline_array_type(place).is_none() {
            return false;
        }
        self.diagnostics.push(Diagnostic::new(
            span,
            "embedded array storage cannot be assigned or rebound; index an element instead",
        ));
        true
    }

    pub(super) fn place_value(
        &mut self,
        place: subject::SemPlace,
        span: Span,
        expected: Option<&ValueType>,
    ) -> subject::SemExpr {
        if let Some(array_type) = self.inline_array_type(&place) {
            let pointer_type = array_type.pointer_type();
            if expected == Some(&pointer_type) {
                return subject::SemExpr {
                    ty: pointer_type,
                    kind: subject::SemExprKind::AddressOf(Box::new(place)),
                    span,
                };
            }
            self.diagnostics.push(Diagnostic::new(
                span,
                "embedded array field requires indexing or a matching element-pointer context",
            ));
            return self.error_expr(span);
        }
        subject::SemExpr {
            ty: place.ty.clone(),
            kind: subject::SemExprKind::Load(Box::new(place)),
            span,
        }
    }
}
