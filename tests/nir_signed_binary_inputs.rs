use actionc::{
    nir::{self, NirIntegerType, NirOp, NirValue},
    target::TargetId,
};
fn lower(target: TargetId) -> nir::NirProgram {
    let ast=actionc::parser::parse(&actionc::lexer::tokenize("LONGINT total,product,bits INT value PROC Main() total==+value product=total*value bits=total OR value RETURN").unwrap()).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &ast,
        actionc::semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model))
}
#[test]
fn signed_binary_widening_is_explicit_for_all_targets() {
    for target in [
        TargetId::Atari6502,
        TargetId::Motorola68000,
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
    ] {
        let raw = lower(target);
        nir::verify_program(&raw).unwrap();
        for p in [raw.clone(), nir::optimize_program(&raw).unwrap()] {
            nir::verify_program(&p).unwrap();
            assert!(p.routines.iter().flat_map(|r| &r.blocks).flat_map(|b|&b.ops).any(|op| matches!(op,NirOp::Cast{from,to,..} if from.kind.integer()==Some(NirIntegerType::I16) && to.kind.integer()==Some(NirIntegerType::I32))));
        }
    }
}
#[test]
fn verifier_rejects_hidden_signed_binary_widening() {
    let mut p = lower(TargetId::Motorola68000);
    let op = p
        .routines
        .iter_mut()
        .flat_map(|r| &mut r.blocks)
        .flat_map(|b| &mut b.ops)
        .find(|op| matches!(op, NirOp::Binary { .. }))
        .unwrap();
    if let NirOp::Binary { right, .. } = op {
        *right = NirValue::integer_const(0xffff, NirIntegerType::I16);
    }
    assert!(
        nir::verify_program(&p)
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("signed binary operand widening"))
    );
}
