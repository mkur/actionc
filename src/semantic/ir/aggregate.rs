//! Shared aggregate values are places plus ordered preparation, not scalar
//! addresses disguised as values. Checked projection erases source patterns
//! only after nominal constructor, owner and extent facts have been validated.
use super::*;
use crate::ast::CaseArm;

struct AggregateValue {
    place: SemLValue,
    preparation: Vec<SemStmt>,
}

impl IrBuilder<'_> {
    pub(super) fn lower_variant_case(
        &mut self,
        scope: ScopeId,
        selector: &Expr,
        arms: &[CaseArm],
        span: Span,
    ) -> Vec<SemStmt> {
        let facts =
            self.model.variants.matches[&super::super::ExpressionSite::new(scope, span)].clone();
        let variant = self.model.variants.types[&facts.owner].clone();
        let ty = ValueType::aggregate(variant.identity.clone());
        let captured = self.capture_aggregate_value(scope, &ty, selector);
        let mut lowered_arms = Vec::new();
        for (arm, facts) in arms.iter().zip(facts.arms) {
            let block = self
                .model
                .lexical_blocks
                .iter()
                .find(|block| block.scope == facts.scope)
                .expect("resolved pattern scope");
            let scope_ref = SemLexicalScopeRef {
                syntax_id: block.syntax_id,
                scope: block.scope,
                parent: block.parent,
                depth: block.depth,
                ordinal: block.ordinal,
            };
            let mut declarations = Vec::new();
            let mut body = Vec::new();
            let mut tests = Vec::new();
            self.pattern_tests(&captured.place, &facts.pattern, true, &mut tests);
            for (id, path) in facts.bindings {
                let mut source = captured.place.clone();
                for (constructor, field) in path {
                    let variant = &self.model.variants.types[&constructor.owner];
                    assert!(variant.constructors[usize::from(constructor.tag) - 1].fields.contains(&field), "projection must belong to the guarded alternative");
                    source = self.canonical_field_place(&source, field);
                }
                let symbol = self.symbol_ref_by_id(id, self.model.symbols.symbols[id.0].span);
                let ty = self.sem_type_from_symbol(&symbol);
                let destination = SemLValue {
                    kind: SemLValueKind::Symbol(symbol.clone()),
                    ty: ty.value.clone(),
                    access: PlaceAccess::Assignable,
                    is_volatile: false,
                    storage: None,
                    span: symbol.span,
                };
                assert_eq!(
                    source.ty, destination.ty,
                    "payload binding retains nominal type"
                );
                if destination.ty.is_record() {
                    body.push(self.copy_value(destination, source, symbol.span));
                } else {
                    body.push(SemStmt::Assign {
                        target: destination,
                        value: Self::value_expr(&source),
                        span: symbol.span,
                    });
                }
                declarations.push(SemDeclaration {
                    symbol,
                    ty,
                    storage: SemDeclarationStorage::Scalar,
                    initializer: None,
                    static_initializer: None,
                    span: arm.span,
                    group_span: arm.span,
                });
            }
            let guard = arm.guard.as_ref().map(|guard| SemCaseGuard {
                bindings: Some(SemCaseBindings {
                    scope: scope_ref.clone(), declarations: std::mem::take(&mut declarations),
                    initialization: std::mem::take(&mut body),
                }),
                condition: self.lower_condition(facts.scope, guard),
            });
            body.extend(
                arm.body
                    .iter()
                    .flat_map(|stmt| self.lower_stmt(facts.scope, stmt)),
            );
            lowered_arms.push(SemCaseArm {
                guard,
                tests,
                labels: match facts.pattern { super::super::patterns::Pattern::Constructor(id, _) => Some(id), _ => None }.map(|id| {
                    vec![super::super::CaseRange {
                        low: u64::from(id.tag),
                        high: u64::from(id.tag),
                        span: arm.span,
                    }]
                }),
                body: vec![SemStmt::LexicalBlock {
                    scope: scope_ref,
                    declarations,
                    constants: Vec::new(),
                    body,
                    span: arm.span,
                }],
                span: arm.span,
            });
        }
        if !lowered_arms.iter().any(|arm| arm.labels.is_none() && arm.guard.is_none()) {
            lowered_arms.push(SemCaseArm {
                guard: None,
                tests: Vec::new(),
                labels: None,
                body: vec![SemStmt::Fault {
                    kind: crate::runtime_fault::RuntimeFault::InvalidVariant,
                    span,
                }],
                span,
            });
        }
        let mut output = captured.preparation;
        output.push(SemStmt::Case {
            selector: Self::value_expr(
                &self.canonical_field_place(&captured.place, variant.tag_field),
            ),
            arms: lowered_arms,
            span,
        });
        output
    }

    fn pattern_tests(&self, place: &SemLValue, pattern: &super::super::patterns::Pattern, outer: bool, tests: &mut Vec<SemCondition>) {
        use super::super::patterns::Pattern;
        match pattern {
            Pattern::Wild => {},
            Pattern::Literal(bits) => tests.push(Self::pattern_equal(place, *bits)),
            Pattern::Constructor(id, fields) => {
                assert_eq!(place.ty.as_aggregate_identity().and_then(|id| id.symbol), Some(id.owner));
                let variant = &self.model.variants.types[&id.owner];
                if !outer {
                    tests.push(Self::pattern_equal(&self.canonical_field_place(place, variant.tag_field), u64::from(id.tag)));
                }
                let constructor = &variant.constructors[usize::from(id.tag) - 1];
                assert_eq!(fields.len(), constructor.fields.len());
                for (pattern, field) in fields.iter().zip(&constructor.fields) {
                    let source = self.canonical_field_place(place, *field);
                    self.pattern_tests(&source, pattern, false, tests);
                }
            }
        }
    }

    fn pattern_equal(place: &SemLValue, bits: u64) -> SemCondition {
        let mut literal = Self::integer_expr(place.ty.representation_scalar().expect("typed scalar pattern"), bits, place.span);
        literal.ty = place.ty.clone();
        SemCondition {
            expr: SemExpr {
                kind: SemExprKind::Binary {
                    op: BinaryOp::Eq, left: Box::new(Self::value_expr(place)), right: Box::new(literal),
                },
                ty: ValueType::fund(FundType::Byte), class: SemExprClass::Condition,
                eval_order: None, span: place.span,
            },
            kind: SemConditionKind::Compare, span: place.span,
        }
    }

    pub(super) fn aggregate_requires_validation(&self, ty: &ValueType) -> bool {
        fn visit(model: &SemanticModel, ty: &ValueType, seen: &mut HashSet<SymbolId>) -> bool {
            if ty.is_pointer() {
                return false;
            }
            let Some(owner) = ty.as_aggregate_identity().and_then(|id| id.symbol) else {
                return false;
            };
            if model.variants.types.contains_key(&owner) {
                return true;
            }
            if !seen.insert(owner) {
                return false;
            }
            model.layout.record_for_owner(owner).is_some_and(|layout| {
                layout
                    .fields
                    .iter()
                    .any(|field| visit(model, &field.ty, seen))
            })
        }
        visit(self.model, ty, &mut HashSet::new())
    }

    pub(super) fn private_value_place(&mut self, scope: ScopeId, ty: ValueType, span: Span) -> SemLValue {
        let (id, name) = loop {
            let id = SymbolId(self.next_private_symbol);
            self.next_private_symbol += 1;
            let name = format!(
                "__actionc_value_{}",
                id.0 - self.model.symbols.symbols.len()
            );
            if !self
                .model
                .symbols
                .symbols
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(&name))
            {
                break (id, name);
            }
        };
        let symbol = SemSymbolRef {
            id,
            name: name.clone(),
            defining_module: None,
            canonical_qualified_key: format!("compiler::value::{}", id.0),
            qualified_name: name,
            lexical_display_name: None,
            class: SymbolClass::Var,
            ty: Some(ty.clone()),
            is_volatile: false,
            is_immutable: ty.is_record(),
            scope,
            span,
        };
        self.aggregate_locals.push(SemDeclaration {
            ty: self.sem_type_from_symbol(&symbol),
            symbol: symbol.clone(),
            storage: SemDeclarationStorage::Scalar,
            initializer: None,
            static_initializer: None,
            span,
            group_span: span,
        });
        SemLValue {
            kind: SemLValueKind::Symbol(symbol),
            ty,
            access: PlaceAccess::Assignable,
            is_volatile: false,
            storage: None,
            span,
        }
    }

    fn value_expr(place: &SemLValue) -> SemExpr {
        SemExpr {
            kind: SemExprKind::LValue(Box::new(place.clone())),
            ty: place.ty.clone(),
            class: SemExprClass::Value,
            eval_order: None,
            span: place.span,
        }
    }

    fn address_expr(place: &SemLValue) -> SemExpr {
        SemExpr {
            kind: SemExprKind::AddressOf(Box::new(place.clone())),
            ty: ValueType::pointer_to(place.ty.clone()),
            class: SemExprClass::Value,
            eval_order: None,
            span: place.span,
        }
    }

    fn integer_expr(ty: ScalarType, bits: u64, span: Span) -> SemExpr {
        SemExpr {
            kind: SemExprKind::Literal(SemLiteral::Constant(ConstValue { ty, bits })),
            ty: ValueType::scalar(ty),
            class: SemExprClass::Value,
            eval_order: None,
            span,
        }
    }

    fn copy_value(&self, destination: SemLValue, source: SemLValue, span: Span) -> SemStmt {
        assert_eq!(
            destination.ty, source.ty,
            "aggregate transfer retains nominal identity"
        );
        let size = self
            .value_storage_width(&destination.ty)
            .expect("complete aggregate transfer");
        SemStmt::RecordCopy {
            destination,
            source,
            size,
            span,
        }
    }

    fn canonical_field_place(&self, base: &SemLValue, id: FieldId) -> SemLValue {
        let field = &self.model.fields[id.0];
        let owner = base
            .ty
            .as_aggregate_identity()
            .and_then(|id| id.symbol)
            .expect("aggregate owner");
        assert_eq!(
            field.owner, owner,
            "projection belongs to its nominal aggregate"
        );
        let layout = self
            .model
            .layout
            .record_for_owner(owner)
            .expect("complete projection layout");
        assert!(
            field
                .offset
                .checked_add(field.size)
                .is_some_and(|end| end <= layout.size)
        );
        SemLValue {
            kind: SemLValueKind::Field {
                base: Box::new(base.clone()),
                field: SemFieldRef {
                    id: Some(id),
                    owner: Some(owner),
                    name: field.name.clone(),
                    ty: field.ty.clone(),
                    storage: field.storage.clone(),
                    size: field.size,
                    offset: Some(field.offset),
                    span: field.span,
                },
            },
            ty: field.ty.clone(),
            access: base.access,
            is_volatile: base.is_volatile,
            storage: None,
            span: base.span,
        }
    }

    fn indexed_place(&self, pointer: &SemLValue, element: &ValueType, index: SemExpr) -> SemLValue {
        assert_eq!(pointer.ty, ValueType::pointer_to(element.clone()));
        SemLValue {
            kind: SemLValueKind::Index {
                base: Box::new(Self::value_expr(pointer)),
                index: Box::new(index),
                element_type: element.clone(),
                syntax: SemIndexSyntax::Index,
            },
            ty: element.clone(),
            access: PlaceAccess::Assignable,
            is_volatile: false,
            storage: None,
            span: pointer.span,
        }
    }

    fn counted_block(
        &mut self,
        scope: ScopeId,
        range: std::ops::Range<u32>,
        span: Span,
        body: impl FnOnce(&mut Self, SemExpr) -> Vec<SemStmt>,
    ) -> SemStmt {
        assert!(range.start < range.end);
        let counter = self.private_value_place(scope, ValueType::scalar(ScalarType::Size), span);
        let body = body(self, Self::value_expr(&counter));
        SemStmt::For {
            target: counter,
            start: Self::integer_expr(ScalarType::Size, u64::from(range.start), span),
            end: Self::integer_expr(ScalarType::Size, u64::from(range.end - 1), span),
            step: None,
            step_control: SemForStep::Up(1),
            body,
            span,
        }
    }

    fn element_range(
        &mut self,
        scope: ScopeId,
        place: &SemLValue,
        element: &ValueType,
        range: std::ops::Range<u32>,
        body: impl FnOnce(&mut Self, SemLValue) -> Vec<SemStmt>,
    ) -> Vec<SemStmt> {
        assert!(range.start < range.end);
        let pointer =
            self.private_value_place(scope, ValueType::pointer_to(element.clone()), place.span);
        let value = SemExpr {
            kind: SemExprKind::Cast {
                ty: pointer.ty.clone(),
                expr: Box::new(Self::address_expr(place)),
            },
            ty: pointer.ty.clone(),
            class: SemExprClass::Value,
            eval_order: None,
            span: place.span,
        };
        let initialize = SemStmt::Assign {
            target: pointer.clone(),
            value,
            span: place.span,
        };
        if range.end - range.start == 1 {
            let index = Self::integer_expr(ScalarType::Size, u64::from(range.start), place.span);
            let projected = self.indexed_place(&pointer, element, index);
            let mut result = vec![initialize];
            result.extend(body(self, projected));
            return result;
        }
        let iteration = self.counted_block(scope, range, place.span, |this, index| {
            let projected = this.indexed_place(&pointer, element, index);
            body(this, projected)
        });
        vec![initialize, iteration]
    }

    fn constructor_zero_ranges(
        &self,
        place: &SemLValue,
        tag: FieldId,
        fields: &[FieldId],
    ) -> Vec<std::ops::Range<u32>> {
        let size = self
            .value_storage_width(&place.ty)
            .expect("complete constructor layout");
        let owner = place
            .ty
            .as_aggregate_identity()
            .and_then(|id| id.symbol)
            .expect("constructor owner");
        // Active aggregate fields are copied in full, including their own
        // padding. Only gaps outside those copies belong to this constructor.
        let mut written: Vec<_> = std::iter::once(&tag)
            .chain(fields)
            .map(|id| {
                let field = &self.model.fields[id.0];
                assert_eq!(field.owner, owner);
                let end = field
                    .offset
                    .checked_add(field.size)
                    .expect("checked field extent");
                assert!(end <= size);
                field.offset..end
            })
            .collect();
        written.sort_by_key(|range| range.start);
        let mut cursor = 0;
        let mut gaps = Vec::new();
        for range in written {
            assert!(
                cursor <= range.start,
                "active constructor fields must not overlap"
            );
            if cursor < range.start {
                gaps.push(cursor..range.start);
            }
            cursor = range.end;
        }
        if cursor < size {
            gaps.push(cursor..size);
        }
        gaps
    }

    fn zero_byte_range(
        &mut self,
        scope: ScopeId,
        place: &SemLValue,
        range: std::ops::Range<u32>,
    ) -> Vec<SemStmt> {
        // Keep large unused regions compact using the existing typed loop.
        // A singleton gap needs only one store, without a loop/counter.
        self.element_range(scope, place, &byte_type(), range, |_this, target| {
            vec![SemStmt::Assign {
                target,
                value: Self::integer_expr(ScalarType::Byte, 0, place.span),
                span: place.span,
            }]
        })
    }

    fn capture_aggregate_value(
        &mut self,
        scope: ScopeId,
        expected: &ValueType,
        expr: &Expr,
    ) -> AggregateValue {
        let capture = self.private_value_place(scope, expected.clone(), expr.span);
        let mut preparation = Vec::new();
        if let Some(id) = self
            .model
            .variants
            .expressions
            .get(&super::super::ExpressionSite::new(scope, expr.span))
            .copied()
        {
            let variant = &self.model.variants.types[&id.owner];
            assert_eq!(
                Some(id.owner),
                expected.as_aggregate_identity().and_then(|id| id.symbol)
            );
            let constructor = variant
                .constructors
                .iter()
                .find(|c| c.id == id)
                .expect("checked constructor")
                .clone();
            let tag_field = variant.tag_field;
            let args = match &expr.kind {
                ExprKind::Call { args, .. } => args.as_slice(),
                _ => &[],
            };
            assert_eq!(args.len(), constructor.fields.len());
            let zero_ranges = self.constructor_zero_ranges(&capture, tag_field, &constructor.fields);
            for (arg, field) in args.iter().zip(&constructor.fields) {
                let destination = self.canonical_field_place(&capture, *field);
                if destination.ty.is_record() {
                    let value = self.capture_aggregate_value(scope, &destination.ty, arg);
                    preparation.extend(value.preparation);
                    preparation.push(self.copy_value(destination, value.place, arg.span));
                } else {
                    let value =
                        self.lower_scalar_value_for_expected_type(scope, &destination.ty, arg);
                    preparation.push(SemStmt::Assign {
                        target: destination,
                        value,
                        span: arg.span,
                    });
                }
            }
            // The capture is private until complete. Do not pre-clear bytes
            // that payload evaluation/copies overwrite; preserve argument order
            // and zero only this alternative's alignment gaps and unused tail.
            for range in zero_ranges {
                preparation.extend(self.zero_byte_range(scope, &capture, range));
            }
            // Commit the tag after every payload and padding byte is defined.
            preparation.push(SemStmt::Assign {
                target: self.canonical_field_place(&capture, tag_field),
                value: Self::integer_expr(ScalarType::Byte, u64::from(id.tag), expr.span),
                span: expr.span,
            });
        } else if self.is_aggregate_call_source(scope, expr) {
            let mut call = self.lower_call_expr(scope, expr);
            assert_eq!(call.return_type.as_ref(), Some(expected));
            call.aggregate_result = Some(capture.clone());
            preparation.push(SemStmt::Call { call, span: expr.span });
            preparation.extend(self.validate_aggregate_value(scope, &capture));
        } else {
            let source = self.lower_lvalue(scope, expr);
            preparation.push(self.copy_value(capture.clone(), source, expr.span));
            preparation.extend(self.validate_aggregate_value(scope, &capture));
        }
        AggregateValue {
            place: capture,
            preparation,
        }
    }

    fn validate_aggregate_value(&mut self, scope: ScopeId, place: &SemLValue) -> Vec<SemStmt> {
        if !self.aggregate_requires_validation(&place.ty) {
            return Vec::new();
        }
        let owner = place
            .ty
            .as_aggregate_identity()
            .and_then(|id| id.symbol)
            .unwrap();
        if let Some(variant) = self.model.variants.types.get(&owner).cloned() {
            let mut arms = Vec::new();
            let nested = variant.constructors.iter().any(|constructor| {
                constructor
                    .fields
                    .iter()
                    .any(|field| self.aggregate_requires_validation(&self.model.fields[field.0].ty))
            });
            if !nested {
                // Canonical tags form a closed interval. Scalar/pointer payloads
                // need one bounds check, not a deep 255-way dispatch tree.
                arms.push(SemCaseArm {
                    guard: None,
                    tests: Vec::new(),
                    labels: Some(vec![super::super::CaseRange {
                        low: 1,
                        high: variant.constructors.len() as u64,
                        span: place.span,
                    }]),
                    body: Vec::new(),
                    span: place.span,
                });
            }
            for constructor in variant.constructors.into_iter().filter(|_| nested) {
                assert_ne!(constructor.id.tag, 0);
                let mut body = Vec::new();
                for field in constructor.fields {
                    let field = self.canonical_field_place(place, field);
                    body.extend(self.validate_aggregate_value(scope, &field));
                }
                arms.push(SemCaseArm {
                    guard: None,
                    tests: Vec::new(),
                    labels: Some(vec![super::super::CaseRange {
                        low: u64::from(constructor.id.tag),
                        high: u64::from(constructor.id.tag),
                        span: place.span,
                    }]),
                    body,
                    span: place.span,
                });
            }
            arms.push(SemCaseArm {
                guard: None,
                tests: Vec::new(),
                labels: None,
                body: vec![SemStmt::Fault {
                    kind: crate::runtime_fault::RuntimeFault::InvalidVariant,
                    span: place.span,
                }],
                span: place.span,
            });
            return vec![SemStmt::Case {
                selector: Self::value_expr(&self.canonical_field_place(place, variant.tag_field)),
                arms,
                span: place.span,
            }];
        }
        let mut result = Vec::new();
        let fields = self
            .model
            .layout
            .record_for_owner(owner)
            .unwrap()
            .fields
            .clone();
        for field in fields {
            if !self.aggregate_requires_validation(&field.ty) {
                continue;
            }
            let projected = self.canonical_field_place(place, field.id);
            if let super::super::RecordFieldStorage::InlineArray { array_type, .. } = field.storage
            {
                result.extend(self.element_range(
                    scope,
                    &projected,
                    &array_type.element,
                    0..array_type.length.unwrap(),
                    |this, element| this.validate_aggregate_value(scope, &element),
                ));
            } else {
                result.extend(self.validate_aggregate_value(scope, &projected));
            }
        }
        result
    }

    pub(super) fn lower_checked_aggregate_assignment(
        &mut self,
        scope: ScopeId,
        destination: SemLValue,
        value: &Expr,
        span: Span,
    ) -> Vec<SemStmt> {
        // Capture the destination before any RHS call or index evaluation.
        let pointer =
            self.private_value_place(scope, ValueType::pointer_to(destination.ty.clone()), span);
        let mut result = vec![SemStmt::Assign {
            target: pointer.clone(),
            value: Self::address_expr(&destination),
            span,
        }];
        let captured = self.capture_aggregate_value(scope, &destination.ty, value);
        result.extend(captured.preparation);
        let destination = SemLValue {
            kind: SemLValueKind::Deref {
                pointer: Box::new(Self::value_expr(&pointer)),
            },
            ..destination
        };
        result.push(self.copy_value(destination, captured.place, span));
        result
    }

    pub(super) fn is_aggregate_call_source(&self, scope: ScopeId, expr: &Expr) -> bool {
        matches!(&expr.kind, ExprKind::Call { callee, .. }
            if !self.is_indexable_lvalue(scope, callee))
            && !self.model.variants.expressions.contains_key(
                &super::super::ExpressionSite::new(scope, expr.span))
    }

    pub(super) fn lower_aggregate_return(
        &mut self, scope: ScopeId, ty: &ValueType, value: &Expr,
    ) -> Vec<SemStmt> {
        let captured = self.capture_aggregate_value(scope, ty, value);
        let mut body = captured.preparation;
        body.push(SemStmt::Return {
            value: Some(Self::value_expr(&captured.place)), span: value.span,
        });
        body
    }

    pub(super) fn capture_ordered_call_arguments(
        &mut self, scope: ScopeId, call: &mut SemCall, args: &[Expr],
    ) {
        if let SemCallable::Indirect { target, .. } = &mut call.callee {
            let capture = self.private_value_place(scope, target.ty.clone(), target.span);
            call.preparation.push(SemStmt::Assign {
                target: capture.clone(), value: *target.clone(), span: target.span,
            });
            **target = Self::value_expr(&capture);
        }
        for (arg, expected) in args.iter().zip(call.callable_type.params.clone()) {
            if expected.is_record() {
                let capture = self.capture_aggregate_value(scope, &expected, arg);
                call.preparation.extend(capture.preparation);
                call.args.push(Self::value_expr(&capture.place));
            } else {
                let capture = self.private_value_place(scope, expected.clone(), arg.span);
                let value = self.lower_value_for_expected_type(scope, &expected, arg);
                call.preparation.push(SemStmt::Assign {
                    target: capture.clone(), value, span: arg.span,
                });
                call.args.push(Self::value_expr(&capture));
            }
        }
    }
}
