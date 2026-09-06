use actionc::semantic::SemanticOptions;
use actionc::target::TargetId;
use actionc::{
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, ir},
};

fn lower(source: &str) -> ir::SemProgram {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        SemanticOptions {
            case_statements: true,
            ..SemanticOptions::modern()
        },
    )
    .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    ir::lower_program(&ast, &model)
}

#[test]
fn case_semantic_diagnostics_check_constants_types_and_flow() {
    for (declaration, labels, expected) in [
        ("BYTE input", "256", "out of range"),
        ("BYTE input", "-1", "out of range"),
        ("CARD input", "-1", "out of range"),
        ("BYTE input", "1,1", "overlapping"),
        ("BYTE input", "input", "constant"),
        ("BYTE input", "1 TO 2", "ranges are not enabled"),
        ("BYTE POINTER input", "1", "selector"),
    ] {
        let source = format!(
            "{declaration}\nPROC Main()\nCASE input OF\nWHEN {labels} THEN\nRETURN\nESAC\nRETURN\n"
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let errors = semantic::analyze_with_options(
            &ast,
            SemanticOptions {
                case_statements: true,
                ..SemanticOptions::modern()
            },
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{source}\n{errors:#?}"
        );
    }
    for body in ["EXIT", "RETURN(1)"] {
        let source =
            format!("BYTE input\nBYTE FUNC Select()\nCASE input OF\nWHEN 1 THEN\n{body}\nESAC\n");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        assert!(
            semantic::analyze_with_options(
                &ast,
                SemanticOptions {
                    case_statements: true,
                    ..SemanticOptions::modern()
                }
            )
            .is_err()
        );
    }
}

#[test]
fn case_nir_keeps_one_selector_call_and_explicit_else() {
    let semir = lower(
        "BYTE input\nBYTE FUNC Select() RETURN(input)\nPROC Main()\nCASE Select() OF\nWHEN 0,1 THEN\nELSE\nESAC\nRETURN",
    );
    let main = &semir.modules[0].items[2];
    assert!(format!("{main:?}").contains("labels: None"));
    let nir = nir::lower_program(&semir);
    nir::verify_program(&nir).unwrap();
    let main = nir
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .unwrap();
    assert_eq!(
        main.blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter(|op| matches!(op, nir::NirOp::Call { .. }))
            .count(),
        1
    );
    nir::optimize_program(&nir).unwrap();
}

#[test]
fn case_contextual_names_and_rejected_header_forms() {
    lower(
        "BYTE value\nPROC Case() RETURN\nPROC When() RETURN\nPROC Esac() RETURN\nPROC Main()\nCase() When() Esac()\nCASE value OF\nWHEN 1 THEN\nCase() When() Esac()\nESAC\nRETURN",
    );
    for arm in [
        "WHEN _ THEN",
        "WHEN 1 IF 1 THEN",
        "WHEN 1 THEN value=2",
        "WHEN 1",
    ] {
        let source = format!("BYTE value\nPROC Main()\nCASE value OF\n{arm}\nESAC\nRETURN");
        assert!(parse(&tokenize(&source).unwrap()).is_err(), "{source}");
    }
    let source = "BYTE value\nPROC Main()\nCASE value OF\nWHEN 1 THEN\nENDCASE\nRETURN";
    assert!(parse(&tokenize(source).unwrap()).is_err());
}

#[test]
fn incomplete_language_capabilities_are_independent_and_not_public() {
    for options in [SemanticOptions::default(), SemanticOptions::modern()] {
        for target in [
            TargetId::Atari6502,
            TargetId::Wdc65816Small,
            TargetId::Wdc65816Native,
            TargetId::Motorola68000,
        ] {
            let options = options.with_target(target);
            assert!(!options.case_statements);
            assert!(!options.enum_types);
            let cases = SemanticOptions {
                case_statements: true,
                ..options
            };
            assert!(!cases.enum_types);
            let enums = SemanticOptions {
                enum_types: true,
                ..options
            };
            assert!(!enums.case_statements);
        }
    }
}
