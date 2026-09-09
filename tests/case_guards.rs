use actionc::{
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, SemanticOptions},
};

fn errors(source: &str) -> Vec<actionc::diagnostic::Diagnostic> {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    semantic::analyze_with_options(&ast, SemanticOptions::modern())
        .err()
        .expect(source)
}

#[test]
fn case_guards_preserve_ordered_interval_coverage_and_old_disjointness() {
    for (arms, valid) in [
        ("WHEN 1 IF 0 THEN\nWHEN 1 THEN", true),
        ("WHEN 1 TO 3 IF 0 THEN\nWHEN 2 TO 4 THEN", true),
        ("WHEN 1 TO 3 THEN\nWHEN 2 TO 4 IF 1 THEN", true),
        ("WHEN 1 TO 3 THEN\nWHEN 2 TO 4 THEN", false),
        ("WHEN 1 TO 3 THEN\nWHEN 2 IF 1 THEN", false),
        (
            "WHEN 0 TO 127 THEN\nWHEN 128 TO 255 THEN\nWHEN _ IF 1 THEN",
            false,
        ),
        ("WHEN 1 THEN\nWHEN 2 THEN\nWHEN 1,2 IF 1 THEN", false),
        ("WHEN 1 THEN\nWHEN 1,2 IF 1 THEN", true),
        ("WHEN 1,1 IF 1 THEN", false),
    ] {
        let source = format!("BYTE input PROC Main()\nCASE input OF\n{arms}\nESAC\nRETURN");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let result = semantic::analyze_with_options(&ast, SemanticOptions::modern());
        assert_eq!(result.is_ok(), valid, "{source}: {:?}", result.err());
    }
}

#[test]
fn case_guards_do_not_establish_exhaustiveness_and_bindings_stay_local() {
    let prefix = "TYPE Event=VARIANT [NONE KEY [BYTE code]] Event current BYTE output\nPROC Main()\nCASE current OF\n";
    for (arms, needle) in [
        (
            "WHEN Event.NONE THEN\nWHEN Event.KEY(x) IF 1 THEN",
            "non-exhaustive",
        ),
        (
            "WHEN Event.KEY(x) THEN\nWHEN Event.KEY(x) IF 1 THEN\nELSE",
            "shadowed",
        ),
        ("WHEN Event.KEY(x) IF missing THEN\nELSE", "undefined"),
        ("WHEN Event.KEY(x) IF @x<>0 THEN\nELSE", "immutable"),
        ("WHEN Event.KEY(x) IF x>0 THEN\nx=1\nELSE", "immutable"),
        (
            "WHEN Event.KEY(x) IF x>0 THEN\nWHEN Event.NONE IF x THEN\nELSE",
            "undefined",
        ),
        ("WHEN _ IF current THEN\nELSE", "truth values"),
    ] {
        let source = format!("{prefix}{arms}\nESAC\nRETURN");
        let errors = errors(&source);
        assert!(
            errors.iter().any(|e| e.message.contains(needle)),
            "{source}: {errors:?}"
        );
    }
    let source = format!("{prefix}WHEN _ IF 0 THEN\nELSE\nESAC\nRETURN");
    let ast = parse(&tokenize(&source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap();
    nir::optimize_program(&program).unwrap();
}

#[test]
fn guards_are_profile_gated_and_headers_remain_unambiguous() {
    for header in [
        "WHEN _ THEN",
        "WHEN 1 IF THEN",
        "WHEN 1 IF 1 IF 1 THEN",
        "ELSE IF 1 THEN",
    ] {
        let source = format!("BYTE value PROC Main()\nCASE value OF\n{header}\nESAC\nRETURN");
        assert!(parse(&tokenize(&source).unwrap()).is_err(), "{source}");
    }
    let ast = parse(
        &tokenize("BYTE value PROC Main()\nCASE value OF\nWHEN 1 IF 1 THEN\nESAC\nRETURN").unwrap(),
    )
    .unwrap();
    for mut options in [SemanticOptions::default(), SemanticOptions::modern()] {
        options.algebraic_types.case_guards = false;
        assert!(semantic::analyze_with_options(&ast, options).is_err());
    }
}

#[test]
fn scalar_guard_cfg_verifies_and_lowers_on_native_targets() {
    let source = "INT input BYTE output BYTE FUNC Test() RETURN(1) PROC Main()\nCASE input OF\nWHEN -1 TO 2 IF Test() THEN\noutput=1\nWHEN _ IF input<>0 THEN\noutput=2\nELSE\noutput=3\nESAC\nRETURN";
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    for target in [
        actionc::target::TargetId::Atari6502,
        actionc::target::TargetId::Motorola68000,
        actionc::target::TargetId::Wdc65816Native,
        actionc::target::TargetId::Wdc65816Small,
    ] {
        let model =
            semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
                .unwrap();
        let raw = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&raw).unwrap();
        let optimized = nir::optimize_program(&raw).unwrap();
        for program in [&raw, &optimized] {
            match target {
                actionc::target::TargetId::Motorola68000 => {
                    actionc::mir68k::lower_program(program).unwrap();
                }
                actionc::target::TargetId::Wdc65816Native
                | actionc::target::TargetId::Wdc65816Small => {
                    actionc::mir65816::lower_program(program).unwrap();
                }
                _ => {}
            }
        }
    }
}

#[test]
fn guard_only_module_calls_retain_link_dependencies_and_binding_scopes() {
    use actionc::includes::{ModuleLoadOptions, load_compilation_from_provider};
    use actionc::source::{InMemorySourceProvider, SourceOrigin};
    let root = SourceOrigin::host("guard/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE App USE Lib AS L TYPE V=VARIANT [KEY [BYTE code]] V current BYTE output PROC Main()\ncurrent=V.KEY(7)\nCASE current OF\nWHEN V.KEY(n) IF L.Allow(n) THEN\noutput=n\nELSE\noutput=0\nESAC\nRETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("guard/lib.act"), b"MODULE Lib PUBLIC BYTE FUNC Allow(BYTE input) RETURN(input>3) ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model =
        semantic::analyze_compilation_with_options(&loaded, SemanticOptions::modern()).unwrap();
    let semir = semantic::ir::lower_compilation(&loaded, &model);
    let raw = nir::lower_program(&semir);
    nir::verify_program(&raw).unwrap();
    let optimized = nir::optimize_program(&raw).unwrap();
    for runtime in [
        actionc::runtime::Runtime::ActionCart,
        actionc::runtime::Runtime::Standalone,
    ] {
        actionc::mir6502::generate_output_with_config_and_runtime(
            &optimized,
            0x3000,
            &actionc::mir6502::Mir6502Config::default(),
            runtime,
        )
        .unwrap();
    }
    actionc::codegen::generate_semir_profile_with_origin(
        &semir,
        0x3000,
        actionc::codegen::CodegenProfile::Modern,
    )
    .unwrap();
}

#[test]
fn guard_truth_does_not_replace_a_required_fallback_return() {
    for source in [
        "BYTE input BYTE FUNC Choose()\nCASE input OF\nWHEN _ IF 1 THEN\nRETURN(7)\nESAC\nPROC Main() RETURN",
        "TYPE V=VARIANT [KEY [BYTE code]] V current BYTE FUNC Choose()\nCASE current OF\nWHEN V.KEY(n) IF 1 THEN\nRETURN(n)\nESAC\nPROC Main() RETURN",
    ] {
        assert!(!errors(source).is_empty());
    }
}
