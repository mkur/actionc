use actionc::lexer::tokenize;
use actionc::nir::{self, NirBinaryOp, NirOp, NirValue};
use actionc::parser::parse;
use actionc::semantic::{self, SemanticOptions};
use actionc::target::TargetId;

fn checked(source: &str, target: TargetId) -> (semantic::SemanticModel, nir::NirProgram) {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    let program = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    (model, program)
}

#[test]
fn wide_constants_preserve_modern_signed_arithmetic_and_sign_extension() {
    let cases = [
        ("LONGINT(-7)/LONGINT(3)", -2i64),
        ("LONGINT(-7) MOD LONGINT(3)", -1),
        ("LONGINT(7)/LONGINT(-3)", -2),
        ("LONGINT(7) MOD LONGINT(-3)", 1),
        ("LONGINT($80000000)/LONGINT(-1)", -2147483648),
        ("LONGINT($80000000) MOD LONGINT(-1)", 0),
        ("SYS.LONGINT(INT(-1))", -1),
        ("SYS.LONGCARD(INT(-1))", 4294967295),
        ("LONGINT(CARD($FFFF))", 65535),
        ("LONGCARD($FEDCBA98)/LONGCARD(3)", 0xFEDCBA98i64 / 3),
        ("LONGCARD(1) LSH 31", 2147483648),
        ("LONGCARD($80000000) RSH 31", 1),
        ("LONGCARD(1) LSH 32", 0),
        ("LONGINT(-200000)", -200000),
    ];
    for (expression, expected) in cases {
        let source = format!(
            "CONST Answer={expression} LONGCARD result PROC Main() result=LONGCARD({expression}) RETURN"
        );
        let (model, program) = checked(&source, TargetId::Motorola68000);
        let symbol = model
            .symbols
            .lookup(model.symbols.global_scope(), "Answer")
            .unwrap();
        let expected = expected as u64 & 0xFFFF_FFFF;
        assert_eq!(model.constants[&symbol].bits, expected, "{expression}");
        let optimized = nir::optimize_program(&program).unwrap();
        assert!(optimized.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).any(|op|
            matches!(op, NirOp::Store { src: NirValue::IntegerConst { bits, .. }, .. } if *bits == expected)), "{expression}: {optimized:#?}");
    }
}

#[test]
fn narrow_operations_only_widen_after_the_result_unless_an_operand_is_wide() {
    let (_, program) = checked(
        "CARD a,b LONGCARD narrow,wide PROC Main() narrow=a*b wide=LONGCARD(a)*b RETURN",
        TargetId::Atari6502,
    );
    let widths: Vec<_> = program
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .filter_map(|op| match op {
            NirOp::Binary {
                op: NirBinaryOp::Mul,
                ty,
                ..
            } => Some(ty.width.unwrap().get()),
            _ => None,
        })
        .collect();
    assert_eq!(widths, [2, 4]);
}

#[test]
fn case_keeps_high_label_bits_and_selector_comparison_width() {
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
    ] {
        let (_, program) = checked(
            "LONGCARD selector BYTE result PROC Main()\nCASE selector OF\nWHEN 1 THEN\nresult=1\nWHEN 65537 THEN\nresult=2\nWHEN $FFFFFFFE TO $FFFFFFFF THEN\nresult=3\nELSE\nresult=4\nESAC\nRETURN",
            target,
        );
        let comparisons: Vec<_> = program
            .routines
            .iter()
            .flat_map(|r| &r.blocks)
            .flat_map(|b| &b.ops)
            .filter_map(|op| match op {
                NirOp::Compare {
                    operand_ty,
                    right: NirValue::IntegerConst { bits, .. },
                    ..
                } => Some((operand_ty.width.unwrap().get(), *bits)),
                _ => None,
            })
            .collect();
        assert_eq!(
            comparisons,
            [(4, 1), (4, 65537), (4, 0xFFFF_FFFE), (4, 0xFFFF_FFFF)]
        );
        nir::optimize_program(&program).unwrap();
    }
    let (_, program) = checked(
        "CARD selector BYTE result PROC Main()\nCASE selector OF\nWHEN 1 THEN\nresult=1\nELSE\nresult=0\nESAC\nRETURN",
        TargetId::Atari6502,
    );
    assert!(program.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops).any(|op|
        matches!(op, NirOp::Compare { operand_ty, .. } if operand_ty.width.unwrap().get() == 2)));
}

#[test]
fn scalar_named_const_annotations_do_not_implicitly_erase_enum_identity() {
    for scalar in ["LONGINT", "LONGCARD", "SYS.LONGINT", "SYS.LONGCARD"] {
        let source = format!("TYPE E=ENUM [A] CONST {scalar} Wrong=E.A");
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        assert!(semantic::analyze_with_options(&ast, SemanticOptions::modern()).is_err());
    }
}

#[test]
fn target_sized_constant_casts_and_case_labels_use_the_selected_layout() {
    for target in [TargetId::Wdc65816Native, TargetId::Motorola68000] {
        let (model, program) = checked(
            "CONST SIZE Limit=70000\nCONST Sum=SIZE($FFFF)+SIZE(2)\nSIZE selector BYTE result PROC Main()\nCASE selector OF\nWHEN Limit THEN\nresult=1\nESAC\nRETURN",
            target,
        );
        for (name, expected) in [("Limit", 70000), ("Sum", 65537)] {
            let symbol = model
                .symbols
                .lookup(model.symbols.global_scope(), name)
                .unwrap();
            assert_eq!(model.constants[&symbol].bits, expected);
        }
        assert!(
            program
                .routines
                .iter()
                .flat_map(|routine| &routine.blocks)
                .flat_map(|block| &block.ops)
                .any(|op| matches!(
                    op,
                    NirOp::Compare {
                        right: NirValue::IntegerConst { bits: 70000, .. },
                        ..
                    }
                ))
        );
    }
}
