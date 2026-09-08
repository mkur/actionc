//! Classic physical projection of already typed aggregate call boundaries.
use super::*;

pub(super) fn validate_entry(program: &SemProgram) -> Result<(), Vec<Diagnostic>> {
    if let Some(entry) = program.program_entry_routine()
        && entry
            .callable_type
            .params
            .iter()
            .chain(entry.callable_type.return_type.iter())
            .any(ValueType::is_record)
    {
        return Err(vec![Diagnostic::new(
            entry.span,
            "program entry cannot have aggregate parameters/results",
        )]);
    }
    Ok(())
}

pub(super) fn address(value: Expr) -> Expr {
    Expr {
        text: format!("@{}", value.text),
        span: value.span,
        kind: ExprKind::Unary {
            op: UnaryOp::AddressOf,
            expr: Box::new(value),
        },
    }
}

impl SemIrAstLowerer<'_> {
    fn aggregate_name(&mut self, stem: &str, span: Span) -> Expr {
        let mut index = 0;
        let name = loop {
            let name = format!("__ADT_{stem}_{index}");
            if self
                .occupied_capture_names
                .insert(name.to_ascii_uppercase())
            {
                break name;
            }
            index += 1;
        };
        Expr {
            kind: ExprKind::Name(name.clone()),
            text: name,
            span,
        }
    }

    fn aggregate_pointer_decl(&self, name: &Expr, ty: &ValueType) -> VarDecl {
        VarDecl {
            visibility: Visibility::Private,
            qualifiers: VarQualifiers::default(),
            ty: self.type_ref(&ValueType::pointer_to(ty.clone())),
            storage: VarStorage::Plain,
            entries: vec![DeclEntry {
                name: name.text.clone(),
                size: None,
                initializer: None,
                span: name.span,
            }],
            span: name.span,
        }
    }

    fn aggregate_copy(
        &mut self,
        target: Expr,
        value: Expr,
        ty: &ValueType,
        span: Span,
    ) -> Option<Stmt> {
        let size = ty
            .as_aggregate_identity()
            .and_then(|id| id.symbol)
            .and_then(|id| self.aggregate_sizes.get(&id))
            .copied()?;
        let Ok(size) = u16::try_from(size) else {
            self.diagnostics.push(Diagnostic::new(
                span,
                "classic backend cannot copy an aggregate larger than 65535 bytes",
            ));
            return None;
        };
        self.record_copies.insert(
            self.native_real_scope.as_deref(),
            span,
            target.clone(),
            value.clone(),
            size,
        );
        Some(Stmt::Assign {
            target,
            value,
            span,
        })
    }

    pub(super) fn aggregate_parameters(
        &mut self,
        routine: &SemRoutine,
        locals: &mut Vec<Decl>,
    ) -> (Vec<VarDecl>, Vec<Stmt>) {
        let mut params = Vec::new();
        let mut prologue = Vec::new();
        if let Some(ty) = routine
            .callable_type
            .return_type
            .as_ref()
            .filter(|ty| ty.is_record())
        {
            let name = self.aggregate_name("RESULT", routine.span);
            params.push(self.aggregate_pointer_decl(&name, ty));
            self.aggregate_result = Some((name, ty.clone()));
        }
        for param in &routine.params {
            let original = self.param(param);
            if param.storage == SemParamStorage::Value && param.ty.value.is_record() {
                let incoming = self.aggregate_name("ARGUMENT", param.span);
                params.push(self.aggregate_pointer_decl(&incoming, &param.ty.value));
                let target = Expr {
                    text: original.entries[0].name.clone(),
                    span: param.span,
                    kind: ExprKind::Name(original.entries[0].name.clone()),
                };
                let source = Expr {
                    text: format!("{}^", incoming.text),
                    span: param.span,
                    kind: ExprKind::Unary {
                        op: UnaryOp::Deref,
                        expr: Box::new(incoming),
                    },
                };
                locals.push(Decl::Var(original));
                prologue.extend(self.aggregate_copy(target, source, &param.ty.value, param.span));
            } else {
                params.push(original);
            }
        }
        (params, prologue)
    }

    pub(super) fn aggregate_return(&mut self, value: &SemExpr, span: Span) -> Option<Stmt> {
        let (result, ty) = self.aggregate_result.clone()?;
        let target = Expr {
            text: format!("{}^", result.text),
            span,
            kind: ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: Box::new(result),
            },
        };
        let value = self.expr(value)?;
        self.aggregate_copy(target, value, &ty, span)
    }
}
