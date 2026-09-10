use super::*;
use crate::semantic::{ConstValue, ir::SemCaseArm};

impl SemIrAstLowerer<'_> {
    pub(super) fn case_statement(
        &mut self,
        selector: &SemExpr,
        arms: &[SemCaseArm],
        span: Span,
    ) -> Vec<Stmt> {
        self.case_dispatch(selector, arms, span, |lowerer, body| {
            lowerer.stmt_list(body)
        })
    }

    pub(super) fn case_dispatch<B>(
        &mut self,
        selector: &SemExpr,
        arms: &[SemCaseArm<B>],
        span: Span,
        mut body: impl FnMut(&mut Self, &B) -> Vec<Stmt>,
    ) -> Vec<Stmt> {
        let Some(value) = self.expr(selector) else {
            return Vec::new();
        };
        let name = loop {
            let name = format!("__actionc_case_{}", self.next_case_capture);
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
            ty: self.type_ref(&selector.ty),
            storage: VarStorage::Plain,
            entries: vec![DeclEntry {
                name,
                size: None,
                initializer: None,
                span,
            }],
            span,
        }));
        let scalar = selector
            .ty
            .representation_scalar()
            .expect("validated integer/enum CASE");
        let mut branches = Vec::new();
        let mut else_body = Vec::new();
        for arm in arms {
            if arm.labels.is_none() && arm.guard.is_none() && arm.tests.is_empty() {
                else_body = body(self, &arm.body);
                break;
            }
            let labels = arm.labels.as_deref().unwrap_or(&[]);
            let mut condition = None;
            for label in labels {
                let literal = |bits| {
                    let literal = ConstValue { ty: scalar, bits }.number_literal();
                    Expr {
                        text: literal.text.clone(),
                        kind: ExprKind::Number(literal),
                        span: label.span,
                    }
                };
                let compare = if label.low == label.high {
                    case_binary(BinaryOp::Eq, target.clone(), literal(label.low), span)
                } else {
                    case_binary(
                        BinaryOp::And,
                        case_binary(BinaryOp::Ge, target.clone(), literal(label.low), span),
                        case_binary(BinaryOp::Le, target.clone(), literal(label.high), span),
                        span,
                    )
                };
                condition = Some(match condition {
                    None => compare,
                    Some(previous) => case_binary(BinaryOp::Or, previous, compare, span),
                });
            }
            for test in &arm.tests {
                let test = self.condition(test).expect("typed pattern test");
                condition = Some(match condition {
                    Some(previous) => case_binary(BinaryOp::And, previous, test, span),
                    None => test,
                });
            }
            if let Some(guard) = &arm.guard {
                let mut value = self
                    .condition(&guard.condition)
                    .expect("typed guard condition");
                if let Some(bindings) = &guard.bindings {
                    value = Expr {
                        text: value.text.clone(),
                        span: value.span,
                        kind: ExprKind::Prepared {
                            statements: self.stmt_list(&bindings.initialization),
                            value: Box::new(value),
                        },
                    };
                }
                condition = Some(match condition {
                    Some(previous) => case_binary(BinaryOp::And, previous, value, span),
                    None => value,
                });
            }
            branches.push(IfBranch {
                condition: condition.expect("validated nonempty CASE labels"),
                body: body(self, &arm.body),
            });
        }
        vec![
            Stmt::Assign {
                target,
                value,
                span,
            },
            Stmt::If {
                branches,
                else_body,
                span,
            },
        ]
    }
}

pub(super) fn capture_reserved_names(
    program: &SemProgram,
    projections: &BTreeMap<SymbolId, String>,
) -> BTreeSet<String> {
    let mut names = projections
        .values()
        .map(|name| name.to_ascii_uppercase())
        .collect::<BTreeSet<_>>();
    for item in program.modules.iter().flat_map(|module| &module.items) {
        if let Some(symbol) = sem_item_symbol(item) {
            names.insert(symbol.name.to_ascii_uppercase());
        }
        if let SemItem::Routine(routine) = item {
            names.extend(
                routine
                    .params
                    .iter()
                    .map(|param| param.symbol.name.to_ascii_uppercase()),
            );
            names.extend(
                routine
                    .locals
                    .iter()
                    .map(|decl| decl.symbol.name.to_ascii_uppercase()),
            );
            visit_lexical_declarations(&routine.body, &mut |_, decl| {
                names.insert(decl.symbol.name.to_ascii_uppercase());
            });
        }
    }
    names
}

fn case_binary(op: BinaryOp, left: Expr, right: Expr, span: Span) -> Expr {
    let kind = ExprKind::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    };
    Expr {
        text: expr_text(&kind),
        kind,
        span,
    }
}
