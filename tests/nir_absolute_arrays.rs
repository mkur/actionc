use actionc::{
    nir::{self, NirOp, NirPlace, NirPlaceKind, NirValue},
    target::TargetId,
};

fn lower(target: TargetId) -> nir::NirProgram {
    let source = "BYTE ARRAY bytes(1024)=$FF00 CARD ARRAY words(512)=$A000 CARD index,result PROC Main() index=300 bytes(index)=7 words(index)=1234 result=CARD(bytes(index))+words(index) RETURN";
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &ast,
        actionc::semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model))
}

#[test]
fn native_absolute_array_bases_are_typed_addresses() {
    for target in [
        TargetId::Motorola68000,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
    ] {
        let p = lower(target);
        nir::verify_program(&p).unwrap();
        nir::verify_program(&nir::optimize_program(&p).unwrap()).unwrap();
        assert!(
            p.routines
                .iter()
                .flat_map(|r| &r.blocks)
                .flat_map(|b| &b.ops)
                .any(|op| matches!(
                    op,
                    NirOp::Store {
                        place: NirPlace {
                            kind: NirPlaceKind::Index {
                                base_addr: NirValue::AddressConst { .. },
                                ..
                            },
                            ..
                        },
                        ..
                    }
                ))
        );
    }
}

#[test]
fn verifier_rejects_integer_native_index_bases() {
    let mut p = lower(TargetId::Wdc65816Native);
    let base = p
        .routines
        .iter_mut()
        .flat_map(|r| &mut r.blocks)
        .flat_map(|b| &mut b.ops)
        .find_map(|op| match op {
            NirOp::Store {
                place:
                    NirPlace {
                        kind: NirPlaceKind::Index { base_addr, .. },
                        ..
                    },
                ..
            } => Some(base_addr),
            _ => None,
        })
        .unwrap();
    *base = NirValue::ConstU16(0xFF00);
    assert!(nir::verify_program(&p).unwrap_err().iter().any(|d| {
        d.message
            .contains("native index base requires a typed address")
    }));
}
