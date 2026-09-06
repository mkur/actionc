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
fn incomplete_enums_are_rejected_before_public_lowering() {
    let ast =
        parse(&tokenize("TYPE Result=ENUM [OK=0 BUSY=10 RETRY]\nPROC Main() RETURN").unwrap())
            .unwrap();
    for options in [SemanticOptions::default(), SemanticOptions::modern()] {
        let errors = semantic::analyze_with_options(&ast, options).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("ENUM requires the modern profile")),
            "{errors:?}"
        );
    }
}
