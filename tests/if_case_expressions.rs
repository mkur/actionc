use actionc::{
    ast::*,
    lexer::tokenize,
    parser::parse,
    semantic::{self, SemanticOptions},
};

fn program(source: &str) -> Program {
    parse(&tokenize(source).unwrap()).unwrap_or_else(|errors| panic!("{source}: {errors:?}"))
}

#[test]
fn parses_nested_if_values_and_preserves_the_following_statement() {
    let ast = program(
        "BYTE a,b,x\nPROC Main()\nLET v=IF a THEN IF b THEN 1 ELSE 2 FI ELSEIF b THEN 3 ELSE 4 FI\nx=v\nIF x THEN x=0 FI\nRETURN",
    );
    let Item::Routine(main) = ast.modules[0].items.last().unwrap() else {
        panic!()
    };
    assert_eq!(main.body.len(), 4);
    let Stmt::Let { value, .. } = &main.body[0] else {
        panic!()
    };
    let ExprKind::Selection(selection) = &value.kind else {
        panic!("{value:?}")
    };
    let SelectionExpr::If { branches, .. } = selection.as_ref() else {
        panic!()
    };
    assert_eq!(branches.len(), 2);
    assert!(matches!(branches[0].1.kind, ExprKind::Selection(_)));
    assert!(matches!(main.body[2], Stmt::If { .. }));
}

#[test]
fn parses_case_values_in_calls_and_uses_unique_scope_ids() {
    let ast = program(
        r#"
PROC Main()
  BEGIN
    LET v=CASE Read() OF
    WHEN 1 THEN
      IF Test() THEN 2 ELSE 3 FI
    ELSE
      CASE Read() OF
      WHEN 2 THEN
        4
      ELSE
        5
      ESAC
    ESAC
    PrintBE(CASE v OF
    WHEN 2 THEN
      1
    ELSE
      0
    ESAC)
  END
RETURN
"#,
    );
    let Item::Routine(main) = &ast.modules[0].items[0] else {
        panic!()
    };
    let Stmt::LexicalBlock {
        syntax_id, body, ..
    } = &main.body[0]
    else {
        panic!()
    };
    let mut ids = vec![*syntax_id];
    fn visit(expr: &Expr, ids: &mut Vec<LexicalBlockSyntaxId>) {
        if let ExprKind::Selection(selection) = &expr.kind {
            if let SelectionExpr::Case { arms, .. } = selection.as_ref() {
                ids.extend(arms.iter().map(|arm| arm.header.syntax_id));
            }
            for child in selection.expressions() {
                visit(child, ids);
            }
        }
    }
    let Stmt::Let {
        syntax_id, value, ..
    } = &body[0]
    else {
        panic!()
    };
    ids.push(*syntax_id);
    visit(value, &mut ids);
    let Stmt::Call { expr, .. } = &body[1] else {
        panic!()
    };
    let ExprKind::Call { args, .. } = &expr.kind else {
        panic!()
    };
    visit(&args[0], &mut ids);
    assert_eq!(ids.len(), 8);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8);
    assert!(!ids.contains(&LexicalBlockSyntaxId(u32::MAX)));
}

#[test]
fn expression_consumers_collect_whole_selection_operands() {
    for source in [
        "x=IF a THEN 1 ELSE 2 FI",
        "x=1+(IF a THEN 1 ELSE 2 FI)",
        "x==+IF a THEN 1 ELSE 2 FI",
        "PrintBE(IF a THEN 1 ELSE 2 FI)",
        "x=table(IF a THEN 1 ELSE 2 FI)",
        "IF IF a THEN 1 ELSE 0 FI THEN x=1 FI",
        "WHILE IF a THEN 1 ELSE 0 FI DO x=1 OD",
        "FOR x=IF a THEN 0 ELSE 1 FI TO IF a THEN 2 ELSE 3 FI DO x=1 OD",
        "RETURN(IF a THEN 1 ELSE 2 FI)",
    ] {
        let ast = program(&format!(
            "BYTE a,x BYTE ARRAY table(4) BYTE FUNC Main()\n{source}\nRETURN(0)"
        ));
        let errors = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("value expressions are not enabled")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn malformed_and_over_nested_selections_are_parser_errors() {
    for value in [
        "IF a THEN 1 FI",
        "IF a THEN ELSE 2 FI",
        "IF a THEN 1 ELSE 2",
        "IF a 1 ELSE 2 FI",
        "IF a THEN RETURN(1) ELSE 2 FI",
        "CASE a OF\nWHEN 1 THEN\nELSE\n0\nESAC",
        "CASE a OF\nWHEN THEN\n1\nELSE\n0\nESAC",
        "CASE a OF\nWHEN 1 THEN\n1 WHEN 2 THEN\n2\nELSE\n0\nESAC",
        "CASE a OF\nWHEN 1 THEN 2\nELSE\n0\nESAC",
        "CASE a OF\nWHEN 1 THEN\n2\nELSE\n0",
        "CASE a OF\nWHEN 1 THEN\n2\nELSE\n0\nWHEN 2 THEN\n3\nESAC",
    ] {
        let source = format!("PROC Main()\nLET v={value}\nRETURN");
        assert!(parse(&tokenize(&source).unwrap()).is_err(), "{source}");
    }
    let value = format!("{}0{}", "IF a THEN ".repeat(65), " ELSE 1 FI".repeat(65));
    let errors =
        parse(&tokenize(&format!("PROC Main() LET v={value} RETURN")).unwrap()).unwrap_err();
    assert!(errors.iter().any(|e| e.message.contains("64 levels")));
}

#[test]
fn selection_words_remain_contextual_and_values_are_gated() {
    program("BYTE case,of PROC Main()\nLET a=case+of\nLET b=Case(1)+of\nRETURN");
    program(
        "BYTE case,when,of,esac PROC Case() RETURN PROC Main() case=1 when=2 of=3 esac=4 Case() RETURN",
    );
    let ast = program("BYTE v PROC Main() v=IF 1 THEN 2 ELSE 3 FI RETURN");
    for options in [SemanticOptions::default(), SemanticOptions::modern()] {
        assert!(
            semantic::analyze_with_options(&ast, options)
                .unwrap_err()
                .iter()
                .any(|e| e.message.contains("not enabled"))
        );
    }
}
