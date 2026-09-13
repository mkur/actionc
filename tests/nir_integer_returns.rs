use actionc::{
    nir::{self, NirIntegerType, NirTerminator, NirValue},
    target::TargetId,
};
fn lower(source: &str, target: TargetId) -> nir::NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &ast,
        actionc::semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model))
}
#[test]
fn integer_returns_explicitly_convert_to_signature_width_for_every_target() {
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let raw = lower(
            "LONGINT FUNC WideConstant() RETURN(-1) LONGINT FUNC WideInput(INT input) RETURN(input) INT FUNC Zero() RETURN(0) BYTE FUNC Narrow(LONGINT input) RETURN(BYTE(input)) PROC Main() RETURN",
            target,
        );
        for (optimized, program) in [raw.clone(), nir::optimize_program(&raw).unwrap()]
            .into_iter()
            .enumerate()
        {
            nir::verify_program(&program).unwrap();
            for routine in program
                .routines
                .iter()
                .filter(|r| r.signature.result.is_some())
            {
                for block in &routine.blocks {
                    if let NirTerminator::Return(Some(value)) = &block.terminator {
                        let width = match value {
                            NirValue::IntegerConst { ty, .. } => Some(ty.storage_width()),
                            NirValue::Temp { ty, .. } => ty.width,
                            _ => panic!("{value:?}"),
                        };
                        assert_eq!(
                            width,
                            routine.signature.result.as_ref().unwrap().width,
                            "{target}/{routine:?}"
                        );
                        if routine.name == "WideConstant" && optimized == 1 {
                            assert!(matches!(
                                value,
                                NirValue::IntegerConst {
                                    bits: 0xffff_ffff,
                                    ..
                                }
                            ));
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn verifier_rejects_implicit_integer_return_widths() {
    let mut program = lower(
        "LONGINT FUNC Wide() RETURN(-1) PROC Main() RETURN",
        TargetId::Motorola68000,
    );
    program.routines[0].blocks[0].terminator =
        NirTerminator::Return(Some(NirValue::integer_const(0xffff, NirIntegerType::I16)));
    let diagnostics = nir::verify_program(&program).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("explicit conversion"))
    );
}
