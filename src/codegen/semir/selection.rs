use super::*;

impl SemIrAstLowerer<'_> {
    pub(super) fn if_value(
        &mut self,
        selection: &SemIfValue,
        ty: &ValueType,
        span: Span,
    ) -> Option<Expr> {
        // This is a classic-projection storage home, never a source SymbolId.
        let name = loop {
            let name = format!("__actionc_value_{}", self.next_case_capture);
            self.next_case_capture += 1;
            if self
                .occupied_capture_names
                .insert(name.to_ascii_uppercase())
            {
                break name;
            }
        };
        let target = Expr {
            kind: ExprKind::Name(name.clone()),
            text: name.clone(),
            span,
        };
        self.case_captures.push(Decl::Var(VarDecl {
            visibility: Visibility::Private,
            qualifiers: VarQualifiers::default(),
            ty: self.type_ref(ty),
            storage: VarStorage::Plain,
            entries: vec![DeclEntry {
                name,
                size: None,
                initializer: None,
                span,
            }],
            span,
        }));
        let mut branches = Vec::new();
        for (condition, value) in &selection.branches {
            branches.push(IfBranch {
                condition: self.condition(condition)?,
                body: vec![Stmt::Assign {
                    target: target.clone(),
                    value: self.expr(value)?,
                    span: value.span,
                }],
            });
        }
        let otherwise = &selection.otherwise;
        Some(Expr {
            text: target.text.clone(),
            span,
            kind: ExprKind::Prepared {
                statements: vec![Stmt::If {
                    branches,
                    else_body: vec![Stmt::Assign {
                        target: target.clone(),
                        value: self.expr(otherwise)?,
                        span: otherwise.span,
                    }],
                    span,
                }],
                value: Box::new(target),
            },
        })
    }
}
