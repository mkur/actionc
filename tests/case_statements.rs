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
        ("BYTE input", "2 TO 1", "descending"),
        ("BYTE input", "1 TO 2, 2 TO 3", "overlapping"),
        ("BYTE input", "0 TO 255, 1", "overlapping"),
        ("BYTE input", "1+1, BYTE(258)", "overlapping"),
        ("BYTE input", "0 TO 256", "out of range"),
        ("INT input", "$FFFF", "out of range"),
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
    let main = semir.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            ir::SemItem::Routine(routine) if routine.symbol.name == "Main" => Some(routine),
            _ => None,
        })
        .unwrap();
    let ir::SemStmt::Case { arms, .. } = &main.body[0] else {
        panic!("expected CASE")
    };
    assert!(arms[1].labels.is_none());
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
    for body in [
        "WHEN 1 THEN\nELSE\nELSE",
        "WHEN 1 THEN\nELSE\nWHEN 2 THEN",
        "ELSE",
        "WHEN THEN",
        "WHEN 1, THEN",
        "WHEN 1 TO THEN",
    ] {
        let source = format!("BYTE value\nPROC Main()\nCASE value OF\n{body}\nESAC\nRETURN");
        assert!(parse(&tokenize(&source).unwrap()).is_err(), "{source}");
    }
    let source = "BYTE value\nPROC Main()\nCASE value OF\nWHEN 1 THEN\nESAC RETURN\nRETURN";
    assert!(parse(&tokenize(source).unwrap()).is_err());
}

#[test]
fn case_overlaps_are_checked_across_arms_in_signed_numeric_order() {
    for labels in [
        "WHEN -5 TO 5 THEN\nWHEN -1 THEN",
        "WHEN -32768 TO -1 THEN\nWHEN -3 TO 1 THEN",
        "WHEN 1 THEN\nWHEN BYTE(257) THEN",
    ] {
        let source = format!("INT value\nPROC Main()\nCASE value OF\n{labels}\nESAC\nRETURN");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let errors = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("previous overlapping")),
            "{errors:?}"
        );
    }
}

#[test]
fn case_is_modern_only_and_enum_capability_remains_independent() {
    for options in [SemanticOptions::default(), SemanticOptions::modern()] {
        for target in [
            TargetId::Atari6502,
            TargetId::Wdc65816Small,
            TargetId::Wdc65816Native,
            TargetId::Motorola68000,
        ] {
            let options = options.with_target(target);
            assert_eq!(options.case_statements, options.lexical_blocks);
            assert_eq!(options.enum_types, options.lexical_blocks);
            let cases = SemanticOptions {
                case_statements: true,
                ..options
            };
            assert_eq!(cases.enum_types, options.enum_types);
            let enums = SemanticOptions {
                enum_types: true,
                ..options
            };
            assert_eq!(enums.case_statements, options.case_statements);
        }
    }
}

#[test]
fn case_ranges_keep_signedness_across_target_lowerers() {
    let source = include_str!("../fixtures/nir/case_ranges.act");
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let model =
            semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
                .unwrap();
        let program = nir::lower_program(&ir::lower_program(&ast, &model));
        nir::verify_program(&program).unwrap();
        let optimized = nir::optimize_program(&program).unwrap();
        for program in [&program, &optimized] {
            match target {
                TargetId::Motorola68000 => {
                    actionc::mir68k::lower_program(program).unwrap();
                }
                TargetId::Wdc65816Small | TargetId::Wdc65816Native => {
                    actionc::mir65816::lower_program(program).unwrap();
                }
                _ => {}
            }
        }
    }
}

#[test]
fn case_volatile_selector_is_read_once_before_and_after_optimization() {
    let semir = lower(
        "VOLATILE BYTE input=$0600 BYTE result=$0601\nPROC Main()\nCASE input OF\nWHEN 0 TO 3 THEN\nresult=1\nWHEN 127 TO 255 THEN\nresult=2\nELSE\nresult=3\nESAC\nRETURN",
    );
    let program = nir::lower_program(&semir);
    let optimized = nir::optimize_program(&program).unwrap();
    for program in [&program, &optimized] {
        assert_eq!(
            program
                .routines
                .iter()
                .flat_map(|routine| &routine.blocks)
                .flat_map(|block| &block.ops)
                .filter(|op| matches!(op, nir::NirOp::VolatileLoad { .. }))
                .count(),
            1
        );
    }
}

#[test]
fn case_public_compilation_supports_modern_modes_and_rejects_compatibility() {
    use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/case_statements.act");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        }
        let errors = compile_file(
            &path,
            &CompileOptions::for_mode(CompileMode::Compatibility).with_runtime(runtime),
        )
        .unwrap_err();
        assert!(
            errors
                .to_string()
                .contains("CASE requires the modern profile"),
            "{errors}"
        );
    }
}
