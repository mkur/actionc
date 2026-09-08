//! Logical value boundaries must remain nominal and verifier-checked.
use actionc::{
    lexer::tokenize,
    nir::*,
    parser::parse,
    semantic::{self, SemanticOptions},
};

fn options() -> SemanticOptions {
    let mut options = SemanticOptions::modern();
    options.algebraic_types.aggregate_calls = true;
    options
}

fn lower(source: &str) -> NirProgram {
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, options()).unwrap();
    actionc::nir::lower_program(&semantic::ir::lower_program(&ast, &model))
}

const CALL: &str = "TYPE Pair=[BYTE a CARD b] Pair data,out \
    Pair FUNC Copy(Pair input) RETURN(input) \
    PROC Main() out=Copy(data) RETURN";

#[test]
fn aggregate_direct_logical_nir_retains_value_types_and_separate_result_places() {
    let program = lower(CALL);
    actionc::nir::verify_program(&program).unwrap();
    let copy = &program.routines[0];
    assert!(matches!(
        copy.signature.result.as_ref().unwrap().kind,
        NirTypeKind::Record { .. }
    ));
    assert!(copy.blocks.iter().any(|block| matches!(
        block.terminator,
        NirTerminator::Return(Some(NirValue::Aggregate { .. }))
    )));
    assert!(
        program
            .routines
            .iter()
            .flat_map(|r| &r.temps)
            .all(|temp| !matches!(temp.ty.kind, NirTypeKind::Record { .. }))
    );
    let main = program.routines.last().unwrap();
    assert!(main.blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(op,
        NirOp::Call { args, result: None, aggregate_result: Some(_), .. } if matches!(args[0], NirValue::Aggregate { .. }))));
    actionc::mir6502::lower_program(&actionc::nir::optimize_program(&program).unwrap()).unwrap();
}

#[test]
fn aggregate_direct_verifier_rejects_missing_or_unowned_result_buffers_and_weak_effects() {
    for mutation in 0..5 {
        let mut program = lower(CALL);
        let main = program.routines.last_mut().unwrap();
        let call = main
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.ops)
            .find(|op| matches!(op, NirOp::Call { .. }))
            .unwrap();
        let NirOp::Call {
            args,
            result,
            aggregate_result,
            effects,
            ..
        } = call
        else {
            unreachable!()
        };
        match mutation {
            0 => *aggregate_result = None,
            1 => {
                let place = aggregate_result.as_mut().unwrap();
                place.kind = NirPlaceKind::Global {
                    id: program.globals[1].id,
                    name: "data".into(),
                };
            }
            2 => effects.opaque = false,
            3 => args.clear(),
            _ => {
                let place = aggregate_result.as_ref().unwrap();
                *result = Some(NirCallResult {
                    dest: TempId(999),
                    ty: place.ty.clone().unwrap(),
                });
            }
        }
        assert!(
            actionc::nir::verify_program(&program).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn aggregate_direct_semantics_reject_nominal_mismatch_omitted_arguments_and_foreign_entries() {
    for source in [
        "TYPE One=[BYTE value] TYPE Two=[BYTE value] Two data PROC Take(One input) RETURN PROC Main() Take(data) RETURN",
        "TYPE One=[BYTE value] One data PROC Take(One input BYTE n) RETURN PROC Main() Take(data) RETURN",
        "TYPE One=[BYTE value] TYPE Two=[BYTE value] Two data One FUNC Bad() RETURN(data) PROC Main() RETURN",
        "TYPE One=[BYTE value] PROC Foreign=$9000(One input) RETURN PROC Main() RETURN",
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        assert!(
            semantic::analyze_with_options(&ast, options()).is_err(),
            "{source}"
        );
    }
}
