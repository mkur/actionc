use actionc::{
    lexer::tokenize,
    nir,
    parser::parse,
    semantic::{self, SemanticOptions},
};

fn analyze(source: &str) -> Result<semantic::SemanticModel, Vec<actionc::diagnostic::Diagnostic>> {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    semantic::analyze_with_options(&ast, SemanticOptions::modern())
}

const PREFIX: &str = "TYPE Flag=VARIANT [OFF ON]\nTYPE Pair=VARIANT [BOTH [Flag first,second]]\nPair current BYTE result\n";

#[test]
fn nested_pattern_refinements_verify_on_every_target() {
    let ast =
        parse(&tokenize(include_str!("../fixtures/nir/nested_patterns.act")).unwrap()).unwrap();
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
        nir::optimize_program(&raw).unwrap();
    }
}

#[test]
fn nested_patterns_cover_products_without_flat_tag_shortcuts() {
    for patterns in [
        "WHEN Pair.BOTH(Flag.OFF,_) THEN\nWHEN Pair.BOTH(_,Flag.OFF) THEN\nWHEN Pair.BOTH(Flag.ON,Flag.ON) THEN",
        "WHEN Pair.BOTH(Flag.OFF,Flag.OFF) THEN\nWHEN Pair.BOTH(Flag.OFF,Flag.ON) THEN\nWHEN Pair.BOTH(Flag.ON,_) THEN",
    ] {
        let source = format!(
            "{PREFIX}BYTE FUNC Test()\nCASE current OF\n{}\nRETURN(1)\nESAC\nPROC Main() RETURN",
            patterns.replace("THEN\n", "THEN\nRETURN(1)\n")
        );
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let model = semantic::analyze_with_options(&ast, SemanticOptions::modern()).unwrap();
        let semir = semantic::ir::lower_program(&ast, &model);
        let nir = nir::lower_program(&semir);
        nir::verify_program(&nir).unwrap();
        nir::optimize_program(&nir).unwrap();
        actionc::codegen::generate_semir_profile_with_origin(
            &semir,
            0x3000,
            actionc::codegen::CodegenProfile::Modern,
        )
        .unwrap();
    }
}

#[test]
fn nested_patterns_report_missing_witness_and_collective_shadowing() {
    for (patterns, expected) in [
        (
            "WHEN Pair.BOTH(Flag.OFF,_) THEN\nWHEN Pair.BOTH(_,Flag.OFF) THEN",
            "Pair.BOTH(Flag.ON,Flag.ON)",
        ),
        (
            "WHEN Pair.BOTH(Flag.OFF,_) THEN\nWHEN Pair.BOTH(Flag.ON,_) THEN\nWHEN Pair.BOTH(_,_) THEN",
            "shadowed",
        ),
        (
            "WHEN Pair.BOTH(_,_) THEN\nWHEN Pair.BOTH(Flag.ON,Flag.OFF) THEN",
            "shadowed",
        ),
        ("WHEN Pair.BOTH(x,x) THEN", "duplicate"),
    ] {
        let source = format!("{PREFIX}PROC Main()\nCASE current OF\n{patterns}\nESAC\nRETURN");
        let errors = analyze(&source).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn nested_literal_patterns_are_typed_and_scalar_domains_remain_open() {
    let prefix = "TYPE Color=ENUM [RED BLUE]\nTYPE Value=VARIANT [NUMBER [INT value] SHADE [Color color] LINK [Value POINTER next]]\nValue current BYTE output\nPROC Main()\nCASE current OF\n";
    for (pattern, needle) in [
        ("Value.NUMBER(65536)", "out of range"),
        ("Value.SHADE(0)", "exact selector enum"),
        ("Value.NUMBER(1+2)", "literal"),
        ("Value.LINK(Value.NUMBER(_))", "not implicitly dereferenced"),
        ("Value.NUMBER(n)", "immutable"),
    ] {
        let body = if needle == "immutable" {
            "n=2"
        } else {
            "output=1"
        };
        let source = format!("{prefix}WHEN {pattern} THEN\n{body}\nELSE\nESAC\nRETURN");
        let errors = analyze(&source).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(needle)),
            "{errors:?}"
        );
    }
    let source = format!(
        "{prefix}WHEN Value.NUMBER(-1) THEN\nWHEN Value.NUMBER(n) THEN\nWHEN Value.SHADE(Color.RED) THEN\nWHEN Value.SHADE(Color.BLUE) THEN\nWHEN Value.LINK(_) THEN\nESAC\nRETURN"
    );
    assert!(
        analyze(&source)
            .unwrap_err()
            .iter()
            .any(|e| e.message.contains("non-exhaustive"))
    );
    analyze(&source.replace(
        "WHEN Value.LINK",
        "WHEN Value.SHADE(_) THEN\nWHEN Value.LINK",
    ))
    .unwrap();
}

#[test]
fn nested_patterns_have_bounded_parse_and_coverage_work() {
    let deep = format!(
        "TYPE V=VARIANT [C [BYTE x]] V current PROC Main()\nCASE current OF\nWHEN {}0{} THEN\nESAC\nRETURN",
        "V.C(".repeat(65),
        ")".repeat(65)
    );
    assert!(
        parse(&tokenize(&deep).unwrap())
            .unwrap_err()
            .iter()
            .any(|e| e.message.contains("64"))
    );
    // Width, not recursive source syntax, can also stress product analysis.
    let fields = (0..140)
        .map(|i| format!("Flag f{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let args = vec!["Flag.ON"; 140].join(",");
    let source = format!(
        "TYPE Flag=VARIANT [OFF ON] TYPE Wide=VARIANT [C [{fields}]] Wide current PROC Main()\nCASE current OF\nWHEN Wide.C({args}) THEN\nELSE\nESAC\nRETURN"
    );
    let errors = analyze(&source).unwrap_err();
    assert!(
        errors.iter().any(|e| e.message.contains("bounded")),
        "{errors:?}"
    );
}
