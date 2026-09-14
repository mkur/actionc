use actionc::{
    nir::{self, NirOp, NirPlace, NirPlaceKind},
    target::TargetId,
};

fn lower(target: TargetId) -> nir::NirProgram {
    let source = "TYPE Pair=[LONGCARD a,b] Pair data Pair POINTER ptr LONGCARD result PROC Main() ptr=@data ptr.b=7 result=ptr.b RETURN";
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &ast,
        actionc::semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model))
}
#[test]
fn native_named_pointer_fields_have_explicit_dereferences() {
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
                .any(|op| matches!(op,
            NirOp::Store { place: NirPlace {kind: NirPlaceKind::Field {base,..},..},..}
            if matches!(base.kind, NirPlaceKind::Deref { .. })))
        );
    }
}
#[test]
fn verifier_rejects_implicit_pointer_field_dereferencing() {
    let mut p = lower(TargetId::Motorola68000);
    let ptr = p.globals.iter().find(|g| g.name == "ptr").unwrap().clone();
    let field = p
        .routines
        .iter_mut()
        .flat_map(|r| &mut r.blocks)
        .flat_map(|b| &mut b.ops)
        .find_map(|op| match op {
            NirOp::Store {
                place:
                    NirPlace {
                        kind: NirPlaceKind::Field { base, .. },
                        ..
                    },
                ..
            } => Some(base),
            _ => None,
        })
        .unwrap();
    **field = NirPlace {
        kind: NirPlaceKind::Global {
            id: ptr.id,
            name: ptr.name,
        },
        ty: ptr.ty,
    };
    assert!(
        nir::verify_program(&p)
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("explicit pointer load and dereference"))
    );
}
