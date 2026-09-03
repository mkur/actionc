use std::path::Path;

use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502::{self, MirCallTarget, MirOp, MirRoutineAbi, MirStorageBase, RoutineId};
use actionc::nir::{
    self, NirCallConvention, NirCallee, NirLocalBacking, NirOp, NirRoutinePlacement,
    NirStorageInit, NirTypeKind,
};
use actionc::semantic::{SemanticOptions, analyze_with_options, ir};

fn classic_baseline() -> nir::NirProgram {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("runtime")
        .join("native_routine_abi_baseline.act");
    let loaded = load_program_with_expanded_source(&source)
        .unwrap_or_else(|diagnostics| panic!("load {}: {diagnostics:?}", source.display()));
    let model = analyze_with_options(&loaded.program, SemanticOptions::modern())
        .unwrap_or_else(|diagnostics| panic!("analyze {}: {diagnostics:?}", source.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let program = nir::lower_program(&semir);
    nir::verify_program(&program)
        .unwrap_or_else(|diagnostics| panic!("verify {}: {diagnostics:?}", source.display()));
    program
}

#[test]
fn classic_nir_baseline_keeps_fixed_local_initializers_and_aliases() {
    let program = classic_baseline();
    let bump = program
        .routines
        .iter()
        .find(|routine| routine.name == "Bump")
        .expect("Bump routine");

    let initial = bump
        .locals
        .iter()
        .find(|local| local.name == "initial")
        .expect("initialized local");
    assert!(matches!(initial.backing, NirLocalBacking::Ordinary));
    assert!(matches!(
        initial.init,
        Some(NirStorageInit::Bytes { ref image, .. }) if image.bytes == [7]
    ));

    let alias = bump
        .locals
        .iter()
        .find(|local| local.name == "alias")
        .expect("local alias");
    assert!(matches!(alias.backing, NirLocalBacking::Alias { .. }));

    assert!(bump.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(
            op,
            NirOp::AddrOf { place, .. }
                if matches!(&place.kind, nir::NirPlaceKind::Local { name, .. } if name == "local")
        )
    }));
}

#[test]
fn classic_nir_has_structured_routine_and_call_conventions() {
    let program = classic_baseline();

    for (index, routine) in program.routines.iter().enumerate() {
        assert_eq!(routine.id.0 as usize, index);
        assert_eq!(routine.convention, NirCallConvention::TargetPublic);
        assert_eq!(routine.signature.convention, routine.convention);
        assert!(matches!(
            routine.entry.placement,
            NirRoutinePlacement::Relocatable
        ));
    }
    assert_eq!(
        program
            .routines
            .iter()
            .filter(|routine| routine.entry.program)
            .map(|routine| routine.name.as_str())
            .collect::<Vec<_>>(),
        ["Main"]
    );

    let main = program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .expect("Main routine");
    let indirect = main
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op {
            NirOp::Call {
                callee: NirCallee::Indirect { ty, .. },
                signature: Some(signature),
                ..
            } => Some((ty, signature)),
            _ => None,
        })
        .expect("indirect callback call");
    let NirTypeKind::Callable {
        signature,
        convention,
        ..
    } = indirect.0.kind
    else {
        panic!("indirect target must retain callable facts");
    };
    assert_eq!(signature, indirect.1.id);
    assert_eq!(convention, indirect.1.convention);
    assert_eq!(convention, NirCallConvention::TargetPublic);
}

#[test]
fn convention_is_part_of_callable_signature_identity() {
    let public = nir::NirCallableSignature::empty_proc(NirCallConvention::TargetPublic);
    let runtime = nir::NirCallableSignature::empty_proc(NirCallConvention::Runtime);
    assert_ne!(public.id, runtime.id);
}

#[test]
fn verifier_rejects_call_convention_mismatches_and_missing_signatures() {
    let mut mismatched = classic_baseline();
    let call = mismatched
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Recur")
        .expect("Recur routine")
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.ops)
        .find_map(|op| match op {
            NirOp::Call {
                signature: Some(signature),
                ..
            } => Some(signature),
            _ => None,
        })
        .expect("recursive call signature");
    call.convention = NirCallConvention::Runtime;
    let diagnostics = nir::verify_program(&mismatched).expect_err("mismatched convention");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("call convention does not match")
    }));

    let mut missing = classic_baseline();
    let signature = missing
        .routines
        .iter_mut()
        .find(|routine| routine.name == "Recur")
        .expect("Recur routine")
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.ops)
        .find_map(|op| match op {
            NirOp::Call { signature, .. } => Some(signature),
            _ => None,
        })
        .expect("recursive call");
    *signature = None;
    let diagnostics = nir::verify_program(&missing).expect_err("missing convention");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("no callable signature or convention")
    }));
}

#[test]
fn classic_mir6502_baseline_reuses_one_frame_for_recursive_calls() {
    let program = classic_baseline();
    let recur_index = program
        .routines
        .iter()
        .position(|routine| routine.name == "Recur")
        .expect("Recur routine");
    let recur_nir = &program.routines[recur_index];
    let recur_id = recur_nir.id;
    assert!(recur_nir.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(op, NirOp::Call { callee: NirCallee::User { id, .. }, .. } if *id == recur_id)
    }));

    let mir = mir6502::lower_program(&program).expect("lower classic routine-storage baseline");
    let recur = mir
        .routines
        .iter()
        .find(|routine| routine.name == "Recur")
        .expect("Recur MIR routine");
    assert_eq!(recur.abi, MirRoutineAbi::Action);
    assert!(matches!(
        recur.frame.params[0].base,
        MirStorageBase::Param(_)
    ));
    assert!(matches!(
        recur.frame.locals[0].base,
        MirStorageBase::Local(_)
    ));
    assert!(recur.blocks.iter().flat_map(|block| &block.ops).any(|op| {
        matches!(
            op,
            MirOp::Call {
                target: MirCallTarget::Routine(RoutineId(id)),
                ..
            } if *id == recur_id.0
        )
    }));
}
