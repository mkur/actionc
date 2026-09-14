use actionc::{
    nir::{self, NirPromotionPolicy, NirTypeKind},
    semantic::{self, SemanticOptions},
    target::TargetId,
};

const SOURCE: &str = "\
LONGINT ARRAY data(16)\n\
LONGINT result\n\
LONGINT POINTER escaped\n\
BYTE flag\n\
LONGINT FUNC Delta(LONGINT value) RETURN(value*3)\n\
LONGINT FUNC Sum(CARD count)\n\
  LONGINT POINTER p\n\
  CARD i\n\
  LONGINT total\n\
  IF flag THEN p=data ELSE p=data p==+4 FI\n\
  total=0 i=0\n\
  WHILE i<count DO\n\
    IF flag THEN total==+Delta(p^) ELSE total==-Delta(p^) FI\n\
    p==+4 i==+1\n\
  OD\n\
RETURN(total)\n\
PROC Main() result=Sum(8) RETURN\n";

fn lower(source: &str, target: TargetId) -> nir::NirProgram {
    let tokens = actionc::lexer::tokenize(source).unwrap();
    let ast = actionc::parser::parse(&tokens).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap();
    nir::lower_program(&semantic::ir::lower_program(&ast, &model))
}

#[test]
fn native_policy_reuses_verified_promotion_for_pointer_counter_and_accumulator() {
    let input = lower(SOURCE, TargetId::Motorola68000);
    let facts = nir::analyze_program_storage(&input);
    assert!(
        facts
            .routine("Sum")
            .unwrap()
            .storage_by_name("p")
            .unwrap()
            .is_promotable()
    );
    let conservative = nir::optimize_program(&input).unwrap();
    assert_eq!(
        conservative,
        nir::optimize_program_with_promotion(&input, NirPromotionPolicy::Conservative).unwrap()
    );
    let promoted =
        nir::optimize_program_with_promotion(&input, NirPromotionPolicy::NativeLoops).unwrap();
    nir::verify_program(&promoted).unwrap();
    let sum = promoted.routines.iter().find(|r| r.name == "Sum").unwrap();
    for name in ["p", "i", "total"] {
        assert!(
            !sum.locals.iter().any(|l| l.name == name),
            "promoted home remains: {name}\n{}",
            nir::format_program(&promoted)
        );
    }
    assert!(
        sum.blocks
            .iter()
            .flat_map(|b| &b.params)
            .any(|p| matches!(p.ty.kind, NirTypeKind::Pointer { .. }))
    );
    let atari = lower(SOURCE, TargetId::Atari6502);
    assert_eq!(
        nir::optimize_program(&atari).unwrap(),
        nir::optimize_program_with_promotion(&atari, NirPromotionPolicy::NativeLoops).unwrap()
    );
}

#[test]
fn native_policy_keeps_volatile_escaped_and_potentially_uninitialized_homes() {
    for source in [
        SOURCE.replace("LONGINT total", "VOLATILE LONGINT total"),
        SOURCE.replace("total=0 i=0", "total=0 escaped=@total i=0"),
        SOURCE.replace("total=0 i=0", "IF flag THEN total=0 FI i=0"),
    ] {
        let input = lower(&source, TargetId::Motorola68000);
        let promoted =
            nir::optimize_program_with_promotion(&input, NirPromotionPolicy::NativeLoops).unwrap();
        let sum = promoted.routines.iter().find(|r| r.name == "Sum").unwrap();
        assert!(sum.locals.iter().any(|l| l.name == "total"));
    }
}
