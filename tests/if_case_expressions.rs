use actionc::{
    ast::*,
    lexer::tokenize,
    parser::parse,
    semantic::{self, SemanticOptions},
};

fn program(source: &str) -> Program {
    parse(&tokenize(source).unwrap()).unwrap_or_else(|errors| panic!("{source}: {errors:?}"))
}

fn rejected(source: &str, message: &str) {
    let errors =
        semantic::analyze_with_options(&program(source), SemanticOptions::modern()).unwrap_err();
    assert!(
        errors.iter().any(|e| e.message.contains(message)),
        "{source}: {errors:?}"
    );
}

#[test]
fn if_values_require_independently_matching_integer_or_enum_arms() {
    for (declarations, value) in [
        ("BYTE b CARD c", "IF 1 THEN b ELSE c FI"),
        ("BYTE b CARD c", "CARD(IF 1 THEN b ELSE c FI)"),
        ("BYTE b CHAR c", "IF 1 THEN b ELSE c FI"),
        (
            "TYPE E=ENUM [A] TYPE F=ENUM [A]",
            "IF 1 THEN E.A ELSE F.A FI",
        ),
        ("TYPE E=ENUM [A]", "IF 1 THEN E.A ELSE 0 FI"),
    ] {
        rejected(
            &format!("{declarations}\nPROC Main()\nLET v={value}\nRETURN"),
            "selection arms must have the same",
        );
    }
    rejected(
        "BYTE b CARD c PROC Main()\nLET CARD v=IF 1 THEN b ELSE c FI\nRETURN",
        "selection arms must have the same",
    );
    for (declarations, value) in [
        ("REAL r", "r"),
        ("BYTE b", "@b"),
        ("TYPE R=[BYTE b] R r", "r"),
        ("TYPE V=VARIANT [NONE]", "V.NONE"),
        ("PROC Empty() RETURN", "Empty()"),
    ] {
        rejected(
            &format!(
                "{declarations}\nPROC Main()\nLET v=IF 1 THEN {value} ELSE {value} FI\nRETURN"
            ),
            "selection result must be an integer or enum",
        );
    }
    rejected(
        "PROC Main()\nLET v=IF 1 THEN 2 ELSE unknown FI\nRETURN",
        "unknown",
    );
    rejected(
        "BYTE b PROC Main()\nLET p=@(IF 1 THEN b ELSE b FI)\nRETURN",
        "address",
    );
    rejected(
        "BYTE b PROC Main()\n(IF 1 THEN b ELSE b FI)=2\nRETURN",
        "assign",
    );
}

#[test]
fn if_values_are_runtime_expressions_even_with_literal_conditions() {
    for source in [
        "CONST V=IF 1 THEN 2 ELSE 3 FI PROC Main() RETURN",
        "BYTE v=IF 1 THEN 2 ELSE 3 FI PROC Main() RETURN",
        "BYTE ARRAY v(IF 1 THEN 2 ELSE 3 FI) PROC Main() RETURN",
        "PROC Main() CONST V=IF 1 THEN 2 ELSE 3 FI RETURN",
        "PROC Main() BYTE v=IF 1 THEN 2 ELSE 3 FI RETURN",
        "PROC Main() BYTE ARRAY v(IF 1 THEN 2 ELSE 3 FI) RETURN",
        "PROC Main() BYTE v=1+(IF 1 THEN 2 ELSE 3 FI) RETURN",
        "PROC Main() BYTE ARRAY v(4)=(IF 1 THEN $600 ELSE $700 FI) RETURN",
        "PROC Main()\nCASE 2 OF\nWHEN (IF 1 THEN 2 ELSE 3 FI) THEN\nRETURN\nESAC\nRETURN",
    ] {
        assert!(
            semantic::analyze_with_options(&program(source), SemanticOptions::modern()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn nested_if_conditions_are_checked_once_per_expression() {
    let value = format!("{}1{}", "IF ".repeat(20), " THEN 1 ELSE 0 FI".repeat(20));
    let ast = program(&format!("PROC Main()\nLET v={value}\nRETURN"));
    semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
}

#[test]
fn layout_queries_do_not_evaluate_if_values_inside_place_indexes() {
    let ast = program(
        "BYTE ARRAY table(8)\nBYTE FUNC Read() RETURN(7)\nPROC Main()\nCONST Width=SIZEOF(table(IF 1 THEN Read() ELSE 2 FI))\nBYTE value=Width\nRETURN",
    );
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    actionc::nir::verify_program(&nir).unwrap();
    let text = actionc::nir::format_program(&nir);
    assert!(!text.contains("call "), "{text}");
}

#[test]
fn if_value_joins_preserve_types_on_every_target_layout() {
    use actionc::target::TargetId;
    let ast = program(
        "TYPE E=ENUM [A B] BYTE flag,b CHAR ch CARD c INT i LONGINT si LONGCARD ui ADDRESS a SIZE s E state\nPROC Main()\nb=IF flag THEN b ELSE 0 FI\nch=IF flag THEN ch ELSE CHAR(1) FI\nc=IF flag THEN CARD(b) ELSE c FI\ni=IF flag THEN i ELSE -1 FI\nsi=IF flag THEN si ELSE LONGINT(-1) FI\nui=IF flag THEN ui ELSE LONGCARD(1) FI\na=IF flag THEN a ELSE ADDRESS(1) FI\ns=IF flag THEN s ELSE SIZE(1) FI\nstate=IF flag THEN E.A ELSE E.B FI\nRETURN",
    );
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
        TargetId::Motorola68000,
    ] {
        let model =
            semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
                .unwrap();
        let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&nir).unwrap();
        actionc::nir::optimize_program(&nir).unwrap();
    }
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
        let model = semantic::analyze_with_options(&ast, SemanticOptions::modern())
            .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        let semir = semantic::ir::lower_program(&ast, &model);
        let nir = actionc::nir::lower_program(&semir);
        actionc::nir::verify_program(&nir).unwrap();
        actionc::nir::optimize_program(&nir).unwrap();
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
    let mut gated = SemanticOptions::modern();
    gated.if_expressions = false;
    for options in [SemanticOptions::default(), gated] {
        assert!(
            semantic::analyze_with_options(&ast, options)
                .unwrap_err()
                .iter()
                .any(|e| e.message.contains("modern profile"))
        );
    }
    let ast = program("PROC Main()\nLET v=CASE 1 OF\nWHEN 1 THEN\n2\nELSE\n3\nESAC\nRETURN");
    let mut gated = SemanticOptions::modern();
    gated.case_expressions = false;
    for options in [SemanticOptions::default(), gated] {
        assert!(
            semantic::analyze_with_options(&ast, options)
                .unwrap_err()
                .iter()
                .any(|e| e.message.contains("modern profile"))
        );
    }
    let ast = program(
        "TYPE V=VARIANT [NONE]\nV value\nPROC Main()\nLET v=CASE value OF\nWHEN V.NONE THEN\n2\nESAC\nRETURN",
    );
    assert!(
        semantic::analyze_with_options(&ast, SemanticOptions::modern())
            .unwrap_err()
            .iter()
            .any(|e| e
                .message
                .contains("variant CASE expressions are not enabled"))
    );
}

fn checked_case(source: &str, target: actionc::target::TargetId) -> semantic::ir::SemProgram {
    let ast = program(source);
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let semir = semantic::ir::lower_program(&ast, &model);
    let nir = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&nir).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    actionc::nir::optimize_program(&nir).unwrap();
    semir
}

#[test]
fn case_values_require_else_and_preserve_statement_coverage_rules() {
    for (declarations, selector, labels) in [
        ("BYTE input", "input", "0 TO 255"),
        ("TYPE E=ENUM [A B] E input", "input", "E.A, E.B"),
    ] {
        let source = format!(
            "{declarations}\nPROC Main()\nLET v=CASE {selector} OF\nWHEN {labels} THEN\n1\nESAC\nRETURN"
        );
        rejected(&source, "CASE expressions require ELSE");
        let statement = source
            .replace("LET v=CASE", "CASE")
            .replace("THEN\n1", "THEN\nRETURN");
        checked_case(&statement, actionc::target::TargetId::Atari6502);
    }
    for (declarations, selector, arms, message) in [
        (
            "BYTE input",
            "input",
            "WHEN 1 TO 3 THEN\n1\nWHEN 3 THEN\n2",
            "overlapping CASE label",
        ),
        (
            "BYTE input",
            "input",
            "WHEN 3 TO 1 THEN\n1",
            "descending CASE range",
        ),
        ("BYTE input", "input", "WHEN 256 THEN\n1", "out of range"),
        (
            "BYTE input",
            "input",
            "WHEN input THEN\n1",
            "integer constant",
        ),
        (
            "BYTE input",
            "input",
            "WHEN 1 THEN\n1\nWHEN 1 IF 1 THEN\n2",
            "fully shadowed",
        ),
        (
            "BYTE input",
            "input",
            "WHEN 1,1 IF 1 THEN\n1",
            "overlapping CASE label",
        ),
        (
            "TYPE E=ENUM [A B] E input",
            "input",
            "WHEN E.A TO E.B THEN\n1",
            "enum CASE ranges",
        ),
        (
            "TYPE E=ENUM [A] TYPE F=ENUM [A] E input",
            "input",
            "WHEN F.A THEN\n1",
            "exact selector enum",
        ),
        (
            "TYPE E=ENUM [A] E input",
            "input",
            "WHEN 0 THEN\n1",
            "exact selector enum",
        ),
        (
            "REAL input",
            "input",
            "WHEN 1 THEN\n1",
            "selector must be an integer or enum",
        ),
        (
            "BYTE input",
            "@input",
            "WHEN 1 THEN\n1",
            "selector must be an integer or enum",
        ),
    ] {
        rejected(
            &format!(
                "{declarations}\nPROC Main()\nLET v=CASE {selector} OF\n{arms}\nELSE\n0\nESAC\nRETURN"
            ),
            message,
        );
    }
}

#[test]
fn case_values_keep_exact_result_types_and_runtime_rvalue_legality() {
    let selection = |a: &str, b: &str| format!("CASE 1 OF\nWHEN 1 THEN\n{a}\nELSE\n{b}\nESAC");
    for (declarations, a, b, message) in [
        ("BYTE b CARD c", "b", "c", "same integer type"),
        (
            "TYPE E=ENUM [A] TYPE F=ENUM [A]",
            "E.A",
            "F.A",
            "same integer type or enum identity",
        ),
        ("REAL r", "r", "r", "integer or enum value"),
        ("BYTE b", "@b", "@b", "integer or enum value"),
        ("TYPE R=[BYTE b] R r", "r", "r", "integer or enum value"),
        (
            "PROC Empty() RETURN",
            "Empty()",
            "Empty()",
            "integer or enum value",
        ),
        ("", "1", "unknown", "unknown"),
    ] {
        rejected(
            &format!(
                "{declarations}\nPROC Main()\nLET v={}\nRETURN",
                selection(a, b)
            ),
            message,
        );
    }
    rejected(
        &format!(
            "BYTE b CARD c PROC Main()\nLET CARD v={}\nRETURN",
            selection("b", "c")
        ),
        "same integer type",
    );
    rejected(
        &format!(
            "BYTE b PROC Main()\nLET p=@({})\nRETURN",
            selection("b", "b")
        ),
        "address",
    );
    rejected(
        &format!("BYTE b PROC Main()\n({})=2\nRETURN", selection("b", "b")),
        "assign",
    );
    for declaration in ["CONST Value=", "BYTE value=", "BYTE ARRAY value("] {
        let source = format!(
            "PROC Main()\n{declaration}{}{}\nRETURN",
            selection("1", "2"),
            if declaration.ends_with('(') { ")" } else { "" }
        );
        assert!(
            semantic::analyze_with_options(&program(&source), SemanticOptions::modern()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn case_values_compose_in_runtime_consumers_and_preserve_target_types() {
    use actionc::target::TargetId;
    let value = "CASE a OF\nWHEN 1, 3 TO 5 IF IF a THEN 1 ELSE 0 FI THEN\nIF a THEN 1 ELSE 2 FI\nWHEN _ IF 0 THEN\n3\nELSE\nCASE a OF\nWHEN 2 THEN\n2\nELSE\n0\nESAC\nESAC";
    for expression in [
        format!("LET v={value}"),
        format!("x={value}"),
        format!("x=1+({value})"),
        format!("x==+{value}"),
        format!("PrintBE({value})"),
        format!("x=table({value})"),
        format!("table({value})={value}"),
        format!("IF {value} THEN x=1 FI"),
        format!("WHILE {value} DO x=1 OD"),
        format!("RETURN({value})"),
        format!("FOR x={value} TO {value} DO x=1 OD"),
    ] {
        checked_case(
            &format!("BYTE a,x BYTE ARRAY table(8) BYTE FUNC Main()\n{expression}\nRETURN(0)"),
            TargetId::Atari6502,
        );
    }
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
        TargetId::Motorola68000,
    ] {
        for ty in [
            "BYTE", "CHAR", "CARD", "INT", "LONGINT", "LONGCARD", "ADDRESS", "SIZE", "E",
        ] {
            let (label, result) = if ty == "E" {
                ("E.A".to_owned(), "E.B".to_owned())
            } else {
                ("1".to_owned(), format!("{ty}(2)"))
            };
            checked_case(
                &format!(
                    "TYPE E=ENUM [A B]\n{ty} input,value\nPROC Main()\nvalue=CASE input OF\nWHEN {label} THEN\n{result}\nELSE\ninput\nESAC\nRETURN"
                ),
                target,
            );
        }
    }
}
