use super::*;
use crate::semantic::{ConstValue, ir::SemCaseArm};

impl SemIrAstLowerer<'_> {
    pub(super) fn case_statement(
        &mut self,
        selector: &SemExpr,
        arms: &[SemCaseArm],
        span: Span,
    ) -> Vec<Stmt> {
        let Some(value) = self.expr(selector) else {
            return Vec::new();
        };
        let name = loop {
            let name = format!("__actionc_case_{}", self.next_case_capture);
            self.next_case_capture += 1;
            if !self
                .projection_names
                .values()
                .any(|other| other.eq_ignore_ascii_case(&name))
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
        let scalar = selector.ty.as_scalar().expect("validated integer CASE");
        let mut branches = Vec::new();
        let mut else_body = Vec::new();
        for arm in arms {
            let Some(labels) = &arm.labels else {
                else_body = self.stmt_list(&arm.body);
                break;
            };
            let mut condition = None;
            for label in labels {
                assert_eq!(label.low, label.high, "interval support not enabled yet");
                let literal = ConstValue {
                    ty: scalar,
                    bits: label.low,
                }
                .number_literal();
                let right = Expr {
                    text: literal.text.clone(),
                    kind: ExprKind::Number(literal),
                    span: label.span,
                };
                let compare = case_binary(BinaryOp::Eq, target.clone(), right, span);
                condition = Some(match condition {
                    None => compare,
                    Some(previous) => case_binary(BinaryOp::Or, previous, compare, span),
                });
            }
            branches.push(IfBranch {
                condition: condition.expect("validated nonempty CASE labels"),
                body: self.stmt_list(&arm.body),
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
