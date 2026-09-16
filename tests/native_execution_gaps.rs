use actionc::{lexer, mir68k, mir65816, nir, parser, semantic, target::TargetId};
fn lower(source: &str, target: TargetId) -> nir::NirProgram {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    nir::lower_program(&semantic::ir::lower_program(&ast, &model))
}
#[test]
fn unsized_native_byte_array_initializers_target_distinct_automatic_backing() {
    for target in [
        TargetId::Wdc65816Native,
        TargetId::Wdc65816Small,
        TargetId::Motorola68000,
    ] {
        let p = lower(
            "BYTE result PROC Local() BYTE ARRAY values=[3 7 11] result=values(1) RETURN PROC Main() Local() RETURN",
            target,
        );
        nir::verify_program(&p).unwrap();
        let r = p.routines.iter().find(|r| r.name == "Local").unwrap();
        let backing = r
            .locals
            .iter()
            .find(|l| matches!(l.purpose, nir::NirLocalPurpose::AggregateBacking { .. }))
            .unwrap();
        assert_eq!(backing.layout.size.get(), 3);
        assert!(r.blocks.iter().flat_map(|b|&b.ops).any(|op|matches!(op,nir::NirOp::CopyBytes{destination:nir::NirPlace{kind:nir::NirPlaceKind::Local{id,..},..},..} if *id==backing.id)));
        nir::optimize_program(&p).unwrap();
        if target == TargetId::Motorola68000 {
            mir68k::lower_program(&p).unwrap();
        } else {
            mir65816::lower_program(&p).unwrap();
        }
    }
}
#[test]
fn native_pointer_offsets_accept_wide_integer_facts_and_reject_pointers() {
    for target in [TargetId::Wdc65816Native, TargetId::Motorola68000] {
        let mut p = lower(
            "BYTE POINTER p,q SIZE n PROC Main() n=$10004 q=p+n RETURN",
            target,
        );
        nir::verify_program(&p).unwrap();
        nir::optimize_program(&p).unwrap();
        let op = p
            .routines
            .iter_mut()
            .flat_map(|r| &mut r.blocks)
            .flat_map(|b| &mut b.ops)
            .find(|op| matches!(op, nir::NirOp::PointerOffset { .. }))
            .unwrap();
        if let nir::NirOp::PointerOffset { base, offset, .. } = op {
            *offset = base.clone();
        }
        assert!(nir::verify_program(&p).unwrap_err().iter().any(|e| {
            e.message
                .contains("displacement must be an Action! integer")
        }));
    }
    let mut classic = lower(
        "BYTE POINTER p,q CARD n PROC Main() q=p+n RETURN",
        TargetId::Atari6502,
    );
    nir::verify_program(&classic).unwrap();
    for op in classic
        .routines
        .iter_mut()
        .flat_map(|r| &mut r.blocks)
        .flat_map(|b| &mut b.ops)
    {
        if let nir::NirOp::PointerOffset { offset, .. } = op {
            *offset = nir::NirValue::integer_const(0x10004, nir::NirIntegerType::U32);
        }
    }
    assert!(nir::verify_program(&classic).is_err());
}
