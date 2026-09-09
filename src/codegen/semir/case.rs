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
            let Some(labels) = &arm.labels else {
                else_body = self.stmt_list(&arm.body);
                break;
            };
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
                condition = Some(case_binary(BinaryOp::And, condition.expect("tag condition"), test, span));
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
