//! Untagged overlapping layouts, using the shared aggregate value machinery.
use super::*;

impl Analyzer {
    pub(super) fn define_union(
        &mut self,
        scope: ScopeId,
        owner: SymbolId,
        fields: &[VarDecl],
        span: Span,
    ) {
        if !self.options.algebraic_types.unions {
            self.diagnostics
                .push(Diagnostic::new(span, "UNION support is not enabled in this profile"));
            return;
        }
        if fields.iter().all(|field| field.entries.is_empty()) {
            self.diagnostics
                .push(Diagnostic::new(span, "UNION requires at least one member"));
            return;
        }
        let identity = self.symbols.symbols[owner.0].aggregate_identity(owner);
        self.symbols.symbols[owner.0].ty = Some(ValueType::aggregate(identity));
        self.aggregate_kinds.insert(owner, AggregateKind::Union);
        let mut names = HashSet::new();
        for field in fields {
            if field.entries.is_empty() {
                self.diagnostics.push(Diagnostic::new(
                    field.span,
                    "UNION member declaration requires a name",
                ));
            }
            self.validate_record_field_decl(scope, field);
            for entry in &field.entries {
                if !names.insert(normalize_name(&entry.name)) {
                    self.diagnostics
                        .push(Diagnostic::new(entry.span, "duplicate UNION member"));
                }
            }
        }
        let first = self.fields.len();
        self.remember_aggregate_fields(scope, owner, fields, layout::FieldPlacement::Overlapping);
        // Dependency resolution may also append fields of referenced records.
        // Check only members owned by this union; traverse their types normally.
        for field in self.fields[first..]
            .iter()
            .filter(|field| field.owner == owner)
        {
            let unsupported = self.any_inline_type(&field.ty, |ty| {
                ty.is_real()
                    || ty.as_callable_pointer().is_some()
                    || ty
                        .as_aggregate_identity()
                        .and_then(|id| id.symbol)
                        .is_some_and(|id| self.variants.types.contains_key(&id))
            });
            if unsupported {
                self.diagnostics.push(Diagnostic::new(
                    field.span,
                    "UNION members cannot contain inline VARIANT, REAL or callable pointers",
                ));
            }
            if field.size == 0 {
                self.diagnostics.push(Diagnostic::new(
                    field.span,
                    "UNION members require a complete non-zero storage extent",
                ));
            }
        }
    }

    pub(super) fn contains_union(&self, ty: &ValueType) -> bool {
        self.any_inline_type(ty, |ty| {
            ty.as_aggregate_identity()
                .and_then(|id| id.symbol)
                .is_some_and(|id| self.aggregate_kinds.get(&id) == Some(&AggregateKind::Union))
        })
    }
}
