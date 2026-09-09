//! Nominal alternatives share the aggregate dependency resolver and target layout.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariantConstructorId {
    pub owner: SymbolId,
    pub tag: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantConstructor {
    pub id: VariantConstructorId,
    pub name: String,
    /// Canonical fields; each alternative owns a disjoint field-ID set even
    /// when its physical payload overlaps another alternative.
    pub fields: Vec<FieldId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantType {
    pub identity: AggregateIdentity,
    pub constructors: Vec<VariantConstructor>,
    pub tag_field: FieldId,
    pub payload_offset: u32,
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariantFacts {
    pub types: HashMap<SymbolId, VariantType>,
    pub(super) expressions: HashMap<ExpressionSite, VariantConstructorId>,
    pub(super) matches: HashMap<ExpressionSite, Vec<VariantArm>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VariantArm {
    pub pattern: super::patterns::Pattern,
    pub scope: ScopeId,
    pub bindings: Vec<(SymbolId, Vec<(VariantConstructorId, FieldId)>)>,
}

impl Analyzer {
    pub(super) fn contains_variant(&self, ty: &ValueType) -> bool {
        fn visit(this: &Analyzer, ty: &ValueType, seen: &mut HashSet<SymbolId>) -> bool {
            if ty.is_pointer() {
                return false;
            }
            let Some(owner) = ty.as_aggregate_identity().and_then(|id| id.symbol) else {
                return false;
            };
            if this.variants.types.contains_key(&owner) {
                return true;
            }
            if !seen.insert(owner) {
                return false;
            }
            this.record_fields_by_owner
                .get(&owner)
                .is_some_and(|fields| {
                    fields.values().any(|id| {
                        let field = &this.fields[id.0];
                        let ty = match &field.storage {
                            RecordFieldStorage::Value => &field.ty,
                            RecordFieldStorage::InlineArray { array_type, .. } => {
                                &array_type.element
                            }
                        };
                        visit(this, ty, seen)
                    })
                })
        }
        visit(self, ty, &mut HashSet::new())
    }

    pub(super) fn validate_variant_storage(&mut self, ty: &ValueType, decl: &VarDecl) {
        if self.contains_variant(ty)
            && (decl.qualifiers.is_volatile
                || decl.entries.iter().any(|entry| entry.initializer.is_some()))
        {
            self.diagnostics.push(Diagnostic::new(decl.span,
                "variant-containing storage cannot be VOLATILE, absolute/alias-backed or statically initialized; construct it by runtime assignment"));
        }
    }

    pub(super) fn analyze_variant_case(
        &mut self,
        scope: ScopeId,
        ty: &ValueType,
        arms: &[CaseArm],
        span: Span,
        context: ControlContext<'_>,
    ) {
        use super::patterns::{Coverage, Pattern};
        let initial_errors = self.diagnostics.len();
        let mut normalized = Vec::new();
        for arm in arms {
            let child = self.symbols.add_scope(ScopeKind::LexicalBlock, Some(scope));
            self.active_lexical_path.push(arm.syntax_id.0);
            let mut bindings = Vec::new();
            if let Some(routine) = self.active_routine_symbol {
                self.lexical_blocks.push(SemanticLexicalBlock {
                    syntax_id: arm.syntax_id,
                    scope: child,
                    parent: scope,
                    routine,
                    module: self.active_module,
                    depth: self.active_lexical_path.len(),
                    ordinal: arm.syntax_id.0,
                    span: arm.span,
                });
            } else {
                self.diagnostics.push(Diagnostic::new(
                    span,
                    "variant CASE is only allowed inside routines",
                ));
            }
            let pattern = if let Some(labels) = &arm.labels {
                if labels.len() != 1 || labels[0].high.is_some() {
                    self.diagnostics.push(Diagnostic::new(
                        arm.span,
                        "variant WHEN requires one constructor pattern",
                    ));
                    Pattern::Wild
                } else {
                    self.analyze_variant_pattern(
                        scope,
                        child,
                        &labels[0].low,
                        ty,
                        &mut Vec::new(),
                        &mut bindings,
                        0,
                    )
                }
            } else {
                Pattern::Wild
            };
            self.analyze_statements(child, &arm.body, context);
            self.active_lexical_path.pop();
            normalized.push(VariantArm {
                pattern,
                scope: child,
                bindings,
            });
        }
        // Only typed patterns reach coverage; error recovery wildcards must not
        // establish exhaustiveness or produce misleading reachability reports.
        if self.diagnostics.len() == initial_errors {
            let mut coverage = Coverage::new(&self.variants.types, &self.fields);
            let mut previous = Vec::new();
            for (arm, facts) in arms.iter().zip(&normalized) {
                match coverage.useful(&previous, &facts.pattern, ty) {
                    Ok(None) => self.diagnostics.push(Diagnostic::new(
                        arm.span,
                        "duplicate or fully shadowed variant pattern",
                    )),
                    Ok(Some(_)) => {}
                    Err(message) => {
                        self.diagnostics.push(Diagnostic::new(span, message));
                        break;
                    }
                }
                previous.push(facts.pattern.clone());
            }
            if self.diagnostics.len() == initial_errors {
                match coverage.useful(&previous, &Pattern::Wild, ty) {
                    Ok(Some(witness)) => self.diagnostics.push(Diagnostic::new(span,
                        format!("non-exhaustive variant CASE; missing {}; add a covering pattern or ELSE", coverage.display(&witness)))),
                    Ok(None) => {},
                    Err(message) => self.diagnostics.push(Diagnostic::new(span, message)),
                }
            }
        }
        self.variants
            .matches
            .insert(ExpressionSite::new(scope, span), normalized);
    }

    fn analyze_variant_pattern(
        &mut self,
        scope: ScopeId,
        child: ScopeId,
        expression: &Expr,
        ty: &ValueType,
        path: &mut Vec<(VariantConstructorId, FieldId)>,
        bindings: &mut Vec<(SymbolId, Vec<(VariantConstructorId, FieldId)>)>,
        depth: usize,
    ) -> super::patterns::Pattern {
        use super::patterns::{MAX_PATTERN_DEPTH, Pattern};
        if depth > MAX_PATTERN_DEPTH {
            self.diagnostics.push(Diagnostic::new(
                expression.span,
                "pattern nesting exceeds 64 levels",
            ));
            return Pattern::Wild;
        }
        if depth != 0 {
            if let ExprKind::Name(name) = &expression.kind {
                if name != "_" {
                    if let Some(symbol) = self.declare(
                        child,
                        name.clone(),
                        SymbolClass::Var,
                        Some(ty.clone()),
                        expression.span,
                    ) {
                        self.symbols.symbols[symbol.0].is_immutable = true;
                        bindings.push((symbol, path.clone()));
                    }
                }
                return Pattern::Wild;
            }
        }
        let (head, args) = match &expression.kind {
            ExprKind::Call { callee, args } => (callee.as_ref(), Some(args.as_slice())),
            _ => (expression, None),
        };
        if let Some(variant) = self.variant_for_type(ty).cloned() {
            let constructor = self
                .variant_constructor_head(scope, head)
                .filter(|(owner, _)| Some(*owner) == variant.identity.symbol)
                .and_then(|(_, name)| {
                    variant
                        .constructors
                        .iter()
                        .find(|c| c.name.eq_ignore_ascii_case(&name))
                        .cloned()
                });
            let Some(constructor) = constructor else {
                self.diagnostics.push(Diagnostic::new(
                    expression.span,
                    "variant pattern must name a constructor of the exact selector type",
                ));
                return Pattern::Wild;
            };
            if args.unwrap_or(&[]).len() != constructor.fields.len()
                || (constructor.fields.is_empty() && args.is_some())
            {
                self.diagnostics.push(Diagnostic::new(
                    expression.span,
                    "variant pattern payload arity mismatch",
                ));
            }
            let mut fields = Vec::new();
            for (arg, field) in args.unwrap_or(&[]).iter().zip(&constructor.fields) {
                path.push((constructor.id, *field));
                let field_ty = self.fields[field.0].ty.clone();
                fields.push(self.analyze_variant_pattern(
                    scope,
                    child,
                    arg,
                    &field_ty,
                    path,
                    bindings,
                    depth + 1,
                ));
                path.pop();
            }
            return Pattern::Constructor(constructor.id, fields);
        }
        // This is a literal grammar, not a constant-expression evaluator for
        // arbitrary source computations. Bare names are always fresh binders.
        fn literal(expr: &Expr) -> bool {
            match &expr.kind {
                ExprKind::Number(_) | ExprKind::Char(_) | ExprKind::Field { .. } => true,
                ExprKind::Unary {
                    op: UnaryOp::Neg | UnaryOp::Plus,
                    expr,
                } => matches!(expr.kind, ExprKind::Number(_)),
                _ => false,
            }
        }
        if depth != 0 && !ty.is_pointer() && literal(expression) {
            if let Some(scalar) = ty.representation_scalar() {
                if let Some(value) = self.case_constant(scope, expression, ty, scalar) {
                    let mask = scalar_mask_for_layout(
                        scalar,
                        TargetLayout::for_target(self.options.target),
                    );
                    return Pattern::Literal(value as u64 & mask);
                }
                return Pattern::Wild;
            }
        }
        self.diagnostics.push(Diagnostic::new(expression.span,
            "payload pattern requires a binder, _, integer/enum literal or by-value variant constructor; pointers are not implicitly dereferenced"));
        Pattern::Wild
    }

    pub(super) fn define_variant(
        &mut self,
        scope: ScopeId,
        owner: SymbolId,
        alternatives: &[crate::ast::VariantAlternative],
        span: Span,
    ) {
        if !self.options.algebraic_types.variants {
            self.diagnostics.push(Diagnostic::new(
                span,
                "VARIANT construction and matching are not enabled yet",
            ));
            return;
        }
        if alternatives.is_empty() || alternatives.len() > 255 {
            self.diagnostics.push(Diagnostic::new(
                span,
                "VARIANT requires 1..255 alternatives; tag zero is invalid",
            ));
            return;
        }
        let identity = self.symbols.symbols[owner.0].aggregate_identity(owner);
        self.symbols.symbols[owner.0].ty = Some(ValueType::aggregate(identity.clone()));
        let mut constructors = Vec::new();
        let mut names = HashSet::new();
        let mut alignment = 1;
        let mut payload_size = 0;
        for (index, alternative) in alternatives.iter().enumerate() {
            if !names.insert(normalize_name(&alternative.name)) {
                self.diagnostics.push(Diagnostic::new(
                    alternative.span,
                    "duplicate VARIANT alternative",
                ));
            }
            let mut payload_names = HashSet::new();
            for field in &alternative.fields {
                for entry in &field.entries {
                    if !payload_names.insert(normalize_name(&entry.name)) {
                        self.diagnostics.push(Diagnostic::new(
                            entry.span,
                            "duplicate variant payload field",
                        ));
                    }
                }
                self.validate_record_field_decl(scope, field);
                if field.storage != VarStorage::Plain {
                    self.diagnostics.push(Diagnostic::new(field.span,
                        "direct ARRAY variant payloads are not supported; embed a record containing the array"));
                }
                let ty = self.value_type_from_type_ref(scope, &field.ty);
                if ty.as_callable_pointer().is_some() {
                    self.diagnostics.push(Diagnostic::new(
                        field.span,
                        "callable variant payloads are not supported yet",
                    ));
                }
            }
            let first = self.fields.len();
            self.remember_record_fields(scope, owner, &identity.name, &alternative.fields);
            let fields: Vec<_> = (first..self.fields.len()).map(FieldId).collect();
            for (position, id) in fields.iter().enumerate() {
                let field = &mut self.fields[id.0];
                alignment = alignment.max(field.alignment);
                payload_size =
                    payload_size.max(field.offset.checked_add(field.size).unwrap_or(u32::MAX));
                // These are compiler field names, never source payload access.
                field.name = format!("__variant_{}_{}", index + 1, position);
            }
            constructors.push(VariantConstructor {
                id: VariantConstructorId {
                    owner,
                    tag: (index + 1) as u8,
                },
                name: alternative.name.clone(),
                fields,
                span: alternative.span,
            });
        }
        let Some(payload_offset) = align_u32(1, alignment) else {
            return;
        };
        let max_extent =
            u32::MAX >> (32 - TargetLayout::for_target(self.options.target).size_integer_bits);
        let Some(size) = payload_offset
            .checked_add(payload_size)
            .and_then(|n| align_u32(n, alignment))
            .filter(|n| *n <= max_extent)
        else {
            self.diagnostics.push(Diagnostic::new(
                span,
                "VARIANT storage extent exceeds the target limit",
            ));
            return;
        };
        let tag_field = FieldId(self.fields.len());
        self.fields.push(SemanticField {
            id: tag_field,
            owner,
            name: "__variant_tag".into(),
            ty: ValueType::fund(FundType::Byte),
            storage: RecordFieldStorage::Value,
            size: 1,
            alignment: 1,
            offset: 0,
            span,
        });
        let mut field_ids = HashMap::new();
        field_ids.insert("__VARIANT_TAG".into(), tag_field);
        for constructor in &constructors {
            for id in &constructor.fields {
                let field = &mut self.fields[id.0];
                field.offset += payload_offset;
                field_ids.insert(normalize_name(&field.name), *id);
            }
        }
        self.record_fields_by_owner.insert(owner, field_ids);
        self.variants.types.insert(
            owner,
            VariantType {
                identity,
                constructors,
                tag_field,
                payload_offset,
                size,
                alignment,
            },
        );
    }

    pub(super) fn variant_for_type(&self, ty: &ValueType) -> Option<&VariantType> {
        if ty.is_pointer() {
            return None;
        }
        self.variants
            .types
            .get(&ty.as_aggregate_identity()?.symbol?)
    }

    pub(super) fn variant_type_for_expr(
        &mut self,
        scope: ScopeId,
        expr: &Expr,
    ) -> Option<SymbolId> {
        if let ExprKind::TypeRef(ty) = &expr.kind {
            self.validate_type_ref(scope, ty, expr.span);
            let value = self.value_type_from_type_ref(scope, ty);
            return self
                .variant_for_type(&value)
                .and_then(|v| v.identity.symbol);
        }
        let name = enums::expression_name(expr)?;
        let SemanticNameResolution::Symbol(id) =
            resolve_semantic_name(&self.symbols, &self.modules, scope, &name)
        else {
            return None;
        };
        (self.symbols.symbols[id.0].class == SymbolClass::Type
            && self.variants.types.contains_key(&id))
        .then_some(id)
    }

    pub(super) fn variant_constructor_head(
        &mut self,
        scope: ScopeId,
        expr: &Expr,
    ) -> Option<(SymbolId, String)> {
        let ExprKind::Field { base, field } = &expr.kind else {
            return None;
        };
        Some((self.variant_type_for_expr(scope, base)?, field.clone()))
    }

    pub(super) fn variant_constructor_subject(
        &mut self,
        scope: ScopeId,
        owner: SymbolId,
        name: &str,
        args: Option<&[Expr]>,
        span: Span,
    ) -> subject::SemSubject {
        let variant = self.variants.types[&owner].clone();
        if self.active_routine_symbol.is_none() {
            self.diagnostics.push(Diagnostic::new(
                span,
                "variant construction requires runtime code inside a routine",
            ));
            return self.subject_error(span);
        }
        let Some(constructor) = variant
            .constructors
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .cloned()
        else {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!("unknown constructor `{name}` for {}", variant.identity.name),
            ));
            return self.subject_error(span);
        };
        if (constructor.fields.is_empty() && args.is_some())
            || (!constructor.fields.is_empty() && args.is_none())
            || args.unwrap_or(&[]).len() != constructor.fields.len()
        {
            self.diagnostics.push(Diagnostic::new(span, format!("constructor `{name}` requires {} payload arguments; nullary constructors are values without parentheses", constructor.fields.len())));
            return self.subject_error(span);
        }
        let mut values = Vec::new();
        for (arg, field) in args.unwrap_or(&[]).iter().zip(&constructor.fields) {
            let expected = self.fields[field.0].ty.clone();
            self.validate_assignment_value_type(scope, &expected, arg);
            values.push(self.lower_expr_for_expected_type(scope, arg, Some(&expected)));
        }
        self.variants
            .expressions
            .insert(ExpressionSite::new(scope, span), constructor.id);
        subject::SemSubject::Expr(subject::SemExpr {
            ty: ValueType::aggregate(variant.identity),
            kind: subject::SemExprKind::VariantConstructor {
                constructor: constructor.id,
                args: values,
            },
            span,
        })
    }
}
