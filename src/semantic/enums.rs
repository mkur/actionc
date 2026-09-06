use super::*;

/// Nominal identity is the defining symbol, not its byte representation. The
/// stable qualified key is retained for serialized callable signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumIdentity {
    pub symbol: SymbolId,
    pub name: String,
    pub canonical_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumValue {
    pub identity: EnumIdentity,
    pub bits: u8,
}

impl EnumValue {
    pub fn value_type(&self) -> ValueType {
        ValueType::enumeration(self.identity.clone())
    }

    pub fn representation(&self) -> ConstValue {
        ConstValue {
            ty: ScalarType::Byte,
            bits: u16::from(self.bits),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumMemberValue {
    pub name: String,
    pub bits: u8,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumType {
    pub identity: EnumIdentity,
    pub members: Vec<EnumMemberValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnumFacts {
    pub types: HashMap<SymbolId, EnumType>,
    pub constants: HashMap<SymbolId, EnumValue>,
    pub(super) member_values: HashMap<ExpressionSite, EnumValue>,
    pub(super) casts: HashMap<ExpressionSite, ValueType>,
    pub(super) initializers: HashMap<ExpressionSite, EnumValue>,
}

impl Analyzer {
    /// Handle enum leaves before the legacy numeric/address initializer encoder.
    pub(super) fn validate_enum_initializer(
        &mut self,
        scope: ScopeId,
        destination: &ValueType,
        element: &InitializerElement,
    ) -> bool {
        if let InitializerElementKind::Constant { target, negative } = &element.kind {
            let mut parts = target.components.iter();
            let Some(first) = parts.next() else {
                return false;
            };
            let mut expr = Expr {
                kind: ExprKind::Name(first.clone()),
                text: first.clone(),
                span: element.span,
            };
            for field in parts {
                expr = Expr {
                    kind: ExprKind::Field {
                        base: Box::new(expr),
                        field: field.clone(),
                    },
                    text: target.to_string(),
                    span: element.span,
                };
            }
            let value = self.lower_expr(scope, &expr);
            if destination.as_enum().is_none() && value.ty.as_enum().is_none() {
                return false;
            }
            if *negative || value.ty != *destination || destination.as_enum().is_none() {
                self.diagnostics.push(Diagnostic::new(element.span, "initializer requires the exact enum type; use a typed CONST with an explicit conversion"));
            } else if let Ok(payload) = evaluate_const_expr(&value) {
                self.enums.initializers.insert(
                    ExpressionSite::new(scope, element.span),
                    EnumValue {
                        identity: destination.as_enum().unwrap().clone(),
                        bits: payload.bits as u8,
                    },
                );
            } else {
                self.diagnostics.push(Diagnostic::new(
                    element.span,
                    "enum initializer must be constant",
                ));
            }
            return true;
        }
        if destination.as_enum().is_some() {
            self.diagnostics.push(Diagnostic::new(
                element.span,
                "enum initializer requires a same-enum member or CONST",
            ));
            return true;
        }
        false
    }

    pub(super) fn resolve_routine_result(
        &mut self,
        scope: ScopeId,
        result: &RoutineResultType,
        span: Span,
    ) -> ValueType {
        let syntax = result.type_ref();
        self.validate_type_ref(scope, &syntax, span);
        let ty = self.value_type_from_type_ref(scope, &syntax);
        if matches!(result, RoutineResultType::Named(_)) && ty.as_enum().is_none() {
            self.diagnostics.push(Diagnostic::new(
                span,
                "named FUNC result must be an enum type",
            ));
            return ValueType::error();
        }
        ty
    }

    pub(super) fn define_enum(
        &mut self,
        scope: ScopeId,
        owner: SymbolId,
        members: &[EnumMember],
        span: Span,
    ) {
        if !self.options.enum_types {
            self.diagnostics.push(Diagnostic::new(
                span,
                "ENUM requires the modern profile (feature not yet enabled)",
            ));
            return;
        }
        let symbol = &self.symbols.symbols[owner.0];
        let routine_local = self.active_module.is_none() && self.active_lexical_path.is_empty();
        let identity = EnumIdentity {
            symbol: owner,
            name: if routine_local && let Some(routine) = &self.active_routine {
                format!("{routine}.{}", symbol.name)
            } else {
                symbol.qualified_name.clone()
            },
            canonical_name: if routine_local && let Some(routine) = &self.active_routine {
                format!(
                    "{}::{}",
                    routine.to_ascii_lowercase(),
                    symbol.canonical_qualified_key
                )
            } else {
                symbol.canonical_qualified_key.clone()
            },
        };
        self.symbols.symbols[owner.0].ty = Some(ValueType::enumeration(identity.clone()));
        self.enums.types.insert(
            owner,
            EnumType {
                identity: identity.clone(),
                members: Vec::new(),
            },
        );
        let mut next = Some(0u16);
        let mut names = HashMap::new();
        let mut values = HashMap::new();
        for member in members {
            let bits = if let Some(value) = &member.value {
                let value = self.lower_expr(scope, value);
                if value.ty.as_scalar().is_none() && value.ty.as_enum() != Some(&identity) {
                    self.diagnostics.push(Diagnostic::new(member.span, "ENUM member value requires an integer constant or an earlier member of this enum"));
                    None
                } else {
                    match evaluate_const_expr(&value) {
                        Ok(value) => u16::try_from(exact_const_value(value)).ok(),
                        Err(message) => {
                            self.diagnostics.push(Diagnostic::new(
                                member.span,
                                format!("ENUM member value must be constant: {message}"),
                            ));
                            None
                        }
                    }
                }
            } else {
                next
            };
            let Some(bits) = bits.filter(|bits| *bits <= 255) else {
                self.diagnostics.push(Diagnostic::new(
                    member.span,
                    "ENUM member value is out of BYTE range (0..255)",
                ));
                next = None;
                continue;
            };
            next = Some(bits + 1);
            if let Some(previous) = names.insert(normalize_name(&member.name), member.span) {
                self.diagnostics.push(Diagnostic::new(
                    member.span,
                    format!("duplicate ENUM member `{}`", member.name),
                ));
                self.diagnostics
                    .push(Diagnostic::new(previous, "previous ENUM member is here"));
            }
            if let Some(previous) = values.insert(bits, member.span) {
                self.diagnostics.push(Diagnostic::new(
                    member.span,
                    format!("duplicate ENUM value {bits} for `{}`", member.name),
                ));
                self.diagnostics
                    .push(Diagnostic::new(previous, "previous ENUM value is here"));
            }
            self.enums
                .types
                .get_mut(&owner)
                .unwrap()
                .members
                .push(EnumMemberValue {
                    name: member.name.clone(),
                    bits: bits as u8,
                    span: member.span,
                });
        }
    }

    pub(super) fn enum_type_for_expr(&self, scope: ScopeId, expr: &Expr) -> Option<EnumIdentity> {
        let name = expression_name(expr)?;
        let SemanticNameResolution::Symbol(id) =
            resolve_semantic_name(&self.symbols, &self.modules, scope, &name)
        else {
            return None;
        };
        let symbol = &self.symbols.symbols[id.0];
        if symbol.class != SymbolClass::Type {
            return None;
        }
        symbol.ty.as_ref()?.as_enum().cloned()
    }

    pub(super) fn enum_member_subject(
        &mut self,
        scope: ScopeId,
        base: &Expr,
        name: &str,
        span: Span,
    ) -> Option<subject::SemSubject> {
        let identity = self.enum_type_for_expr(scope, base)?;
        let Some(member) = self.enums.types.get(&identity.symbol).and_then(|ty| {
            ty.members
                .iter()
                .find(|member| member.name.eq_ignore_ascii_case(name))
        }) else {
            self.diagnostics.push(Diagnostic::new(
                span,
                format!("enum `{}` has no available member `{name}`", identity.name),
            ));
            return Some(self.subject_error(span));
        };
        let value = EnumValue {
            identity,
            bits: member.bits,
        };
        self.enums
            .member_values
            .insert(ExpressionSite::new(scope, span), value.clone());
        Some(subject::SemSubject::Expr(subject::SemExpr {
            ty: value.value_type(),
            kind: subject::SemExprKind::Literal(subject::SemLiteral::Enum(value)),
            span,
        }))
    }

    pub(super) fn enum_cast_subject(
        &mut self,
        scope: ScopeId,
        identity: EnumIdentity,
        args: &[Expr],
        span: Span,
    ) -> subject::SemSubject {
        if args.len() != 1 {
            self.diagnostics.push(Diagnostic::new(
                span,
                "enum conversion requires one integer argument",
            ));
            return self.subject_error(span);
        }
        let inner = self.lower_expr(scope, &args[0]);
        if inner.ty.as_scalar().is_none() && !inner.ty.is_error() {
            self.diagnostics.push(Diagnostic::new(span, "enum conversion requires an integer; convert through BYTE, CHAR, CARD, or INT explicitly"));
            return self.subject_error(span);
        }
        let ty = ValueType::enumeration(identity);
        self.enums
            .casts
            .insert(ExpressionSite::new(scope, span), ty.clone());
        subject::SemSubject::Expr(subject::SemExpr {
            ty: ty.clone(),
            kind: subject::SemExprKind::Cast {
                ty,
                expr: Box::new(inner),
            },
            span,
        })
    }
}

fn expression_name(expr: &Expr) -> Option<QualifiedName> {
    match &expr.kind {
        ExprKind::Name(name) => Some(QualifiedName::new(vec![name.clone()])),
        ExprKind::Field { base, field } => {
            let mut name = expression_name(base)?;
            name.components.push(field.clone());
            Some(name)
        }
        _ => None,
    }
}
