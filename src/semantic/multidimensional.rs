//! Checked compile-time shapes. Descriptor mutations never change these facts.
use super::*;

impl Analyzer {
    pub(super) fn resolve_multidimensional_shape(
        &mut self,
        scope: ScopeId,
        entry: &DeclEntry,
    ) -> Option<ArrayShape> {
        if !self.options.multidimensional_arrays {
            self.diagnostics.push(Diagnostic::new(
                entry.span,
                "multidimensional arrays require the modern profile (feature not yet enabled)",
            ));
            return None;
        }
        if entry.size.is_some() || entry.dimensions.len() < 2 {
            self.diagnostics.push(Diagnostic::new(
                entry.span,
                "invalid multidimensional declaration shape",
            ));
            return None;
        }
        let mut dimensions = Vec::new();
        let layout = TargetLayout::for_target(self.options.target);
        let limit = integer_value_limit(layout.size_integer_bits);
        for bound in &entry.dimensions {
            let before = self.diagnostics.len();
            let expression = self.lower_expr(scope, bound);
            if self.diagnostics.len() != before {
                return None;
            }
            if expression.ty.as_scalar().is_none() || expression.ty.is_pointer() {
                self.diagnostics.push(Diagnostic::new(
                    bound.span,
                    "array dimension must be a positive integer constant",
                ));
                return None;
            }
            match self.evaluate_const_expr(&expression) {
                Ok(value) if exact_const_value(value) > 0 && value.bits <= limit => {
                    dimensions.push(u32::try_from(value.bits).ok()?);
                }
                Ok(_) => {
                    self.diagnostics.push(Diagnostic::new(
                        bound.span,
                        "array dimension must be positive and fit the target SIZE type",
                    ));
                    return None;
                }
                Err(message) => {
                    self.diagnostics.push(Diagnostic::new(
                        bound.span,
                        format!("array dimension must be an integer constant: {message}"),
                    ));
                    return None;
                }
            }
        }
        match ArrayShape::fixed(dimensions) {
            Ok(shape) if u64::from(shape.length().unwrap()) <= limit => Some(shape),
            Ok(_) => {
                self.diagnostics.push(Diagnostic::new(
                    entry.span,
                    "array element count exceeds the target SIZE limit",
                ));
                None
            }
            Err(message) => {
                self.diagnostics.push(Diagnostic::new(entry.span, message));
                None
            }
        }
    }
}

impl Analyzer {
    /// Resolve array identity before arity. A malformed array access must never
    /// become a call, and shape is retained even when its descriptor is rebound.
    pub(super) fn index_subject(
        &mut self,
        scope: ScopeId,
        base: &Expr,
        coordinates: &[Expr],
        span: Span,
    ) -> subject::SemSubject {
        let base = self.expect_place(scope, base, base.span);
        let array = self.array_place_type(&base);
        let rank = array.as_ref().map_or(1, |a| a.shape().rank());
        if coordinates.len() != rank {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!(
                    "array rank {rank} requires {rank} indexes, received {}",
                    coordinates.len()
                ),
            ));
            return self.subject_error(span);
        }
        let coordinates: Vec<_> = coordinates
            .iter()
            .map(|index| self.expect_expr(scope, index, index.span))
            .collect();
        let access = self.indexed_access(&base);
        if rank == 1 {
            let index = coordinates.into_iter().next().unwrap();
            let ty = self.indexed_place_type_or_diagnostic(&base, &index, span);
            return subject::SemSubject::Place(subject::SemPlace {
                ty,
                access,
                span,
                kind: subject::SemPlaceKind::Index {
                    base: Box::new(base),
                    index: Box::new(index),
                },
            });
        }
        let array = array.expect("only declared arrays have multiple dimensions");
        for (axis, (coordinate, &bound)) in coordinates
            .iter()
            .zip(array.shape().dimensions())
            .enumerate()
        {
            if coordinate.ty.as_scalar().is_none() {
                if !coordinate.ty.is_error() {
                    self.diagnostics.push(Diagnostic::new(coordinate.span,
                        "multidimensional index requires an integer; convert enum indexes explicitly"));
                }
            } else if let Ok(value) = self.evaluate_const_expr(coordinate) {
                let value = exact_const_value(value);
                if value < 0 || value >= i64::from(bound) {
                    self.diagnostics.push(Diagnostic::new(
                        coordinate.span,
                        format!(
                            "array index {value} is outside dimension {} (0..{})",
                            axis + 1,
                            bound - 1
                        ),
                    ));
                }
            }
        }
        subject::SemSubject::Place(subject::SemPlace {
            ty: (*array.element).clone(),
            access,
            span,
            kind: subject::SemPlaceKind::MultiIndex {
                base: Box::new(base),
                coordinates,
                shape: array.shape().clone(),
            },
        })
    }
}
