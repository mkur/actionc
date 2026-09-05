//! Resolve static subobjects once, in semantics, to the existing symbol/addend
//! relocation contract. Runtime pointer values are never storage identities.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StaticSubobjectAddress {
    pub target: SymbolId,
    pub addend: i32,
}

impl Analyzer {
    // Return false only for the existing named-symbol/routine address path.
    pub(super) fn resolve_subobject_initializer(
        &mut self,
        scope: ScopeId,
        element: &InitializerElement,
    ) -> bool {
        let (target, addend) = match &element.kind {
            InitializerElementKind::SubobjectAddress { target, addend, .. } => {
                ((**target).clone(), *addend)
            }
            InitializerElementKind::Address { target, addend, .. }
                if target.simple_name().is_none()
                    && matches!(
                        resolve_semantic_name(&self.symbols, &self.modules, scope, target),
                        SemanticNameResolution::Unknown
                    ) =>
            {
                let mut components = target.components.iter();
                let mut expr = Expr {
                    kind: ExprKind::Name(components.next().unwrap().clone()),
                    text: target.to_string(),
                    span: element.span,
                };
                for field in components {
                    expr = Expr {
                        kind: ExprKind::Field {
                            base: Box::new(expr),
                            field: field.clone(),
                        },
                        text: target.to_string(),
                        span: element.span,
                    };
                }
                (expr, *addend)
            }
            _ => return false,
        };
        if !self.options.embedded_record_arrays {
            self.diagnostics.push(Diagnostic::new(element.span, "static subobject initializer addresses require the modern embedded-array capability"));
            return true;
        }
        let expression = Expr {
            kind: ExprKind::Unary {
                op: UnaryOp::AddressOf,
                expr: Box::new(target),
            },
            text: element.text.clone(),
            span: element.span,
        };
        let value = self.lower_expr(scope, &expression);
        if let Some(mut address) = self.static_subobject_address(&value)
            && let Some(sum) = address.addend.checked_add(addend)
        {
            address.addend = sum;
            self.static_subobject_addresses
                .insert(ExpressionSite::new(scope, element.span), address);
        } else if !value.ty.is_error() {
            self.diagnostics.push(Diagnostic::new(element.span, "subobject initializer requires a static storage base and constant indexes; assign runtime addresses in a routine"));
        }
        true
    }

    pub(super) fn static_subobject_address(
        &self,
        expr: &subject::SemExpr,
    ) -> Option<StaticSubobjectAddress> {
        use subject::SemExprKind;
        match &expr.kind {
            SemExprKind::AddressOf(place) => self.static_place_address(place),
            SemExprKind::Cast { ty, expr }
                if ty.value_width_bytes_for_layout(TargetLayout::for_target(
                    self.options.target,
                )) == expr
                    .ty
                    .value_width_bytes_for_layout(TargetLayout::for_target(
                        self.options.target,
                    )) =>
            {
                self.static_subobject_address(expr)
            }
            SemExprKind::Unary {
                op: UnaryOp::Plus,
                expr,
            } => self.static_subobject_address(expr),
            SemExprKind::Binary {
                op: BinaryOp::Add | BinaryOp::Sub,
                left,
                right,
            } => {
                let mut address = self.static_subobject_address(left)?;
                let mut addend =
                    i32::try_from(exact_const_value(evaluate_const_expr(right).ok()?)).ok()?;
                if matches!(
                    &expr.kind,
                    SemExprKind::Binary {
                        op: BinaryOp::Sub,
                        ..
                    }
                ) {
                    addend = addend.checked_neg()?;
                }
                address.addend = address.addend.checked_add(addend)?;
                Some(address)
            }
            _ => None,
        }
    }

    fn static_place_address(&self, place: &subject::SemPlace) -> Option<StaticSubobjectAddress> {
        use subject::SemPlaceKind;
        match &place.kind {
            SemPlaceKind::Symbol(id) => {
                let symbol = self.symbols.symbols.get(id.0)?;
                if !matches!(symbol.class, SymbolClass::Var | SymbolClass::Array) {
                    return None;
                }
                Some(StaticSubobjectAddress {
                    target: *id,
                    addend: 0,
                })
            }
            SemPlaceKind::Field { base, field } if !base.ty.is_pointer() => {
                let mut address = self.static_place_address(base)?;
                address.addend = address.addend.checked_add(i32::from(field.offset?))?;
                Some(address)
            }
            SemPlaceKind::Index { base, index } => {
                let array = self.array_place_type(base)?;
                // Inferred initialized arrays have backing too, but unsized
                // pointer-backed arrays and parameters do not have a static base.
                if let SemPlaceKind::Symbol(id) = &base.kind {
                    if !self.static_array_backings.contains(id) {
                        return None;
                    }
                } else {
                    array.length?;
                }
                let stride = self.value_storage_width(&array.element)?;
                let index =
                    i32::try_from(exact_const_value(evaluate_const_expr(index).ok()?)).ok()?;
                let mut address = self.static_place_address(base)?;
                address.addend = address
                    .addend
                    .checked_add(index.checked_mul(i32::from(stride))?)?;
                Some(address)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::source::{InMemorySourceProvider, SourceOrigin};

    fn options() -> SemanticOptions {
        SemanticOptions {
            embedded_record_arrays: true,
            ..SemanticOptions::modern()
        }
    }

    #[test]
    fn static_subobject_initializers_preserve_module_storage_identity() {
        let root = SourceOrigin::host("project/main.act");
        let provider = InMemorySourceProvider::default()
            .with_source(root.clone(), b"MODULE App USE Data INT POINTER p=@Data.value.y(1) CARD ARRAY refs=[@Data.value.x @Data.value.y(2)] PROC Main() RETURN ENDMODULE".to_vec())
            .with_source(SourceOrigin::host("project/data.act"), b"MODULE Data PUBLIC CONST Count=100 PUBLIC TYPE Buffers=[INT ARRAY x(Count),y(Count)] PUBLIC Buffers value ENDMODULE".to_vec());
        let compilation =
            load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
        let model = analyze_compilation_with_options(&compilation, options()).unwrap();
        let target = SymbolId(
            model
                .symbols
                .symbols
                .iter()
                .position(|symbol| symbol.name == "value")
                .unwrap(),
        );
        let semir = ir::lower_compilation(&compilation, &model);
        let writes = semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .filter_map(|item| match item {
                ir::SemItem::Declaration(decl) => decl.static_initializer.as_ref(),
                _ => None,
            })
            .flat_map(|plan| &plan.writes)
            .map(|write| match &write.value {
                ir::SemStaticInitializerValue::Address { target, addend, .. } => {
                    (target.id, *addend)
                }
                _ => panic!("expected relocation"),
            })
            .collect::<Vec<_>>();
        assert_eq!(writes, [(target, 202), (target, 0), (target, 204)]);
        crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
    }

    #[test]
    fn static_subobject_initializers_reject_runtime_bases_indexes_and_wrong_types() {
        for (declarations, message) in [
            (
                "INT POINTER p=@ptr.values",
                "requires a static storage base",
            ),
            (
                "INT POINTER p=@data.values(index)",
                "requires a static storage base",
            ),
            (
                "CARD ARRAY refs=[@ptr.values(1)]",
                "requires a static storage base",
            ),
            (
                "CARD ARRAY refs=[@data.values(index)]",
                "requires a static storage base",
            ),
            (
                "BYTE ARRAY refs=[@data.values(1)]",
                "requires a 2-byte array element",
            ),
            ("BYTE POINTER p=@data.values", "pointer type does not match"),
            (
                "INT POINTER p=BYTE POINTER(@data.values)",
                "pointer type does not match",
            ),
        ] {
            let source = format!(
                "TYPE Buffer=[INT ARRAY values(2)] Buffer data Buffer POINTER ptr CARD index {declarations} PROC Main() RETURN"
            );
            let ast = parse(&tokenize(&source).unwrap()).unwrap();
            let diagnostics = analyze_with_options(&ast, options()).unwrap_err();
            assert!(
                diagnostics.iter().any(|d| d.message.contains(message)),
                "{source}: {diagnostics:?}"
            );
        }
    }
}
