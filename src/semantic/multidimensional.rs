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
