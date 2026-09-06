use actionc::ast::{Decl, ExprKind, Item, TypeDefinition};
use actionc::lexer::tokenize;
use actionc::parser::parse;
use actionc::semantic::{self, SemanticOptions};

#[test]
fn enum_type_syntax_preserves_complete_member_expressions() {
    let source = "TYPE Result=ENUM [First Second=Base+1 Third=(Base=1) Fourth=BYTE(255), Fifth=Other.Value+2, Sixth]\nPROC Main() RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let Item::Declaration(Decl::Type(decl)) = &ast.modules[0].items[0] else {
        panic!("TYPE")
    };
    let TypeDefinition::Enum(members) = &decl.definition else {
        panic!("ENUM")
    };
    assert_eq!(
        members
            .iter()
            .map(|member| member.name.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second", "Third", "Fourth", "Fifth", "Sixth"]
    );
    assert!(members[0].value.is_none());
    assert!(members[5].value.is_none());
    assert!(matches!(
        members[1].value.as_ref().unwrap().kind,
        ExprKind::Binary { .. }
    ));
    assert!(matches!(
        members[2].value.as_ref().unwrap().kind,
        ExprKind::Binary { .. }
    ));
    assert!(matches!(
        members[3].value.as_ref().unwrap().kind,
        ExprKind::Cast { .. }
    ));
    for member in members {
        assert_eq!(
            &source[member.span.start..member.span.start + member.name.len()],
            member.name
        );
    }
    let multiline = source
        .replace("Base+1", "Base\n+\n1")
        .replace(" Second", "\nSecond");
    let ast = parse(&tokenize(&multiline).unwrap()).unwrap();
    let Item::Declaration(Decl::Type(decl)) = &ast.modules[0].items[0] else {
        panic!("TYPE")
    };
    let TypeDefinition::Enum(members) = &decl.definition else {
        panic!("ENUM")
    };
    assert_eq!(members.len(), 6);
}

#[test]
fn record_type_representation_and_enum_contextual_name_remain_compatible() {
    let source =
        "TYPE Enum=[BYTE x CARD ARRAY data(2)]\nEnum object\nPROC Main() object.x=1 RETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let Item::Declaration(Decl::Type(decl)) = &ast.modules[0].items[0] else {
        panic!("TYPE")
    };
    assert!(matches!(&decl.definition, TypeDefinition::Record(fields) if fields.len() == 2));
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    let nir = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    actionc::nir::verify_program(&nir).unwrap();
}

#[test]
fn enum_parser_rejects_incomplete_and_unsupported_definitions() {
    for definition in [
        "ENUM []",
        "ENUM [A=]",
        "ENUM [A=1+]",
        "ENUM [A,,B]",
        "ENUM [1]",
        "ENUM [A",
        "ENUM:INT [A]",
        "ENUM:BYTE [A]",
        "ENUM [A=(1+2]",
    ] {
        let source = format!("TYPE Result={definition}\nPROC Main() RETURN");
        assert!(parse(&tokenize(&source).unwrap()).is_err(), "{source}");
    }
}

#[test]
fn enum_types_are_modern_only() {
    let ast =
        parse(&tokenize("TYPE Result=ENUM [OK=0 BUSY=10 RETRY]\nPROC Main() RETURN").unwrap())
            .unwrap();
    semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    for options in [SemanticOptions::default(), SemanticOptions { enum_types: false, ..SemanticOptions::modern() }] {
        let errors = semantic::analyze_with_options(&ast, options).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("ENUM requires the modern profile")),
            "{errors:?}"
        );
    }
}

fn enum_model(source: &str) -> (actionc::ast::Program, semantic::SemanticModel) {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        SemanticOptions {
            enum_types: true,
            ..SemanticOptions::modern()
        },
    )
    .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    (ast, model)
}

#[test]
fn enum_numbering_and_constants_preserve_nominal_identity() {
    let (ast, model) = enum_model(
        "TYPE ResultCode=ENUM [OK=0 FAILED=1 BUSY=10 RETRY]\nCONST Alias=ResultCode.RETRY\nCONST ResultCode Named=Alias\nResultCode result\nPROC Main() result=Named RETURN",
    );
    let ty = model.enums.types.values().next().unwrap();
    assert_eq!(
        ty.members
            .iter()
            .map(|member| member.bits)
            .collect::<Vec<_>>(),
        [0, 1, 10, 11]
    );
    assert_eq!(model.enums.constants.len(), 2);
    for value in model.enums.constants.values() {
        assert_eq!(value.identity, ty.identity);
        assert_eq!(value.bits, 11);
    }
    let program = semantic::ir::lower_program(&ast, &model);
    let nir = actionc::nir::lower_program(&program);
    actionc::nir::verify_program(&nir).unwrap();
    actionc::nir::optimize_program(&nir).unwrap();
    assert!(semantic::ir::format_program(&program).contains("ResultCode"));
}

#[test]
fn enum_member_numbering_checks_previous_value_duplicates_and_overflow() {
    for (members, diagnostic) in [
        (
            "OK=0 FAILED=11 BUSY=10 RETRY",
            "duplicate ENUM value 11 for `RETRY`",
        ),
        ("A=255 B", "out of BYTE range"),
        ("A=-1", "out of BYTE range"),
        ("A=256", "out of BYTE range"),
        ("A a", "duplicate ENUM member"),
        ("A=E.A", "no available member"),
        ("A=E.B B", "no available member"),
        ("A=1 B=E.A+1", "enum operators"),
    ] {
        let source = format!("TYPE E=ENUM [{members}]\nPROC Main() RETURN");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let errors = semantic::analyze_with_options(
            &ast,
            SemanticOptions {
                enum_types: true,
                ..SemanticOptions::modern()
            },
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains(diagnostic)),
            "{source}\n{errors:?}"
        );
    }
    let members = (0..256)
        .map(|n| format!("M{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    let (_, model) = enum_model(&format!("TYPE E=ENUM [{members}]\nPROC Main() RETURN"));
    assert_eq!(
        model
            .enums
            .types
            .values()
            .next()
            .unwrap()
            .members
            .last()
            .unwrap()
            .bits,
        255
    );
    let ast = parse(
        &tokenize(&format!(
            "TYPE E=ENUM [{members} Overflow]\nPROC Main() RETURN"
        ))
        .unwrap(),
    )
    .unwrap();
    let errors = semantic::analyze_with_options(
        &ast,
        SemanticOptions {
            enum_types: true,
            ..SemanticOptions::modern()
        },
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("out of BYTE range"))
    );
    let (_, model) =
        enum_model("TYPE E=ENUM [A=BYTE(256) B=BYTE(E.A)+1 C=255]\nPROC Main() RETURN");
    assert_eq!(
        model
            .enums
            .types
            .values()
            .next()
            .unwrap()
            .members
            .iter()
            .map(|m| m.bits)
            .collect::<Vec<_>>(),
        [0, 1, 255]
    );
}

#[test]
fn enum_case_named_member_coverage_does_not_prove_exhaustiveness() {
    let source = "TYPE E=ENUM [A=0 B=1]\nE value\nBYTE FUNC Select()\nCASE value OF\nWHEN E.A THEN\nRETURN(1)\nWHEN E.B THEN\nRETURN(2)\nESAC";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let errors = semantic::analyze_with_options(
        &ast,
        SemanticOptions {
            enum_types: true,
            ..SemanticOptions::modern()
        },
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("may exit without RETURN")),
        "{errors:?}"
    );
}

#[test]
fn enum_scalar_representation_lowers_independently_for_all_targets() {
    use actionc::target::TargetId;
    let source = "TYPE E=ENUM [A=0 B=255]\nE value=$0600 BYTE result=$0601\nPROC Main()\nCASE value OF\nWHEN E.A THEN\nresult=1\nWHEN E.B THEN\nresult=2\nELSE\nresult=3\nESAC\nRETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let model = semantic::analyze_with_options(
            &ast,
            SemanticOptions {
                enum_types: true,
                ..SemanticOptions::modern().with_target(target)
            },
        )
        .unwrap();
        for value in model
            .symbols
            .symbols
            .iter()
            .filter_map(|symbol| symbol.ty.as_ref())
            .filter(|ty| ty.as_enum().is_some())
        {
            assert_eq!(
                value.value_width_bytes_for_layout(model.target_layout),
                Some(1)
            );
            assert!(value.as_scalar().is_none());
        }
        let program = actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        actionc::nir::verify_program(&program).unwrap();
        let optimized = actionc::nir::optimize_program(&program).unwrap();
        match target {
            TargetId::Motorola68000 => {
                actionc::mir68k::lower_program(&optimized).unwrap();
            }
            TargetId::Wdc65816Small | TargetId::Wdc65816Native => {
                actionc::mir65816::lower_program(&optimized).unwrap();
            }
            _ => {}
        }
    }
}

#[test]
fn enum_misuse_is_rejected_before_type_erasure() {
    let prefix = "TYPE First=ENUM [A=0 B=255]\nTYPE Second=ENUM [A=0 B=255]\nFirst one,two Second other BYTE raw BYTE ARRAY data(4)\n";
    for body in [
        "one=raw",
        "raw=one",
        "one=other",
        "one=one+1",
        "raw=one=other",
        "raw=one=0",
        "IF one THEN raw=1 FI",
        "one==+1",
        "FOR one=First.A TO First.B DO OD",
        "raw=data(one)",
        "one=First(Second.A)",
        "raw=BYTE POINTER(one)",
        "First.A=one",
        "raw=@First.A",
        "PrintB(one)",
        "CASE one OF\nWHEN 0 THEN\nESAC",
        "CASE one OF\nWHEN Second.A THEN\nESAC",
        "CASE one OF\nWHEN First.A TO First.B THEN\nESAC",
        "CASE raw OF\nWHEN First.A THEN\nESAC",
    ] {
        let source = format!("{prefix}PROC Main()\n{body}\nRETURN");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        assert!(
            semantic::analyze_with_options(
                &ast,
                SemanticOptions {
                    enum_types: true,
                    ..SemanticOptions::modern()
                }
            )
            .is_err(),
            "accepted: {source}"
        );
    }
    for declaration in [
        "CONST First Bad=0",
        "CONST First Bad=Second.A",
        "CONST BYTE Bad=First.A",
    ] {
        let source = format!("{prefix}{declaration}\nPROC Main() RETURN");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        assert!(
            semantic::analyze_with_options(
                &ast,
                SemanticOptions {
                    enum_types: true,
                    ..SemanticOptions::modern()
                }
            )
            .is_err(),
            "accepted: {source}"
        );
    }
}
