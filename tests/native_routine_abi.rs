use std::path::Path;

use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502::{self, MirCallTarget, MirOp, MirRoutineAbi, MirStorageBase, RoutineId};
use actionc::nir::{self, NirCallee, NirLocalBacking, NirOp, NirStorageInit};
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
fn classic_mir6502_baseline_reuses_one_frame_for_recursive_calls() {
    let program = classic_baseline();
    let recur_id = program
        .routines
        .iter()
        .position(|routine| routine.name == "Recur")
        .expect("Recur routine") as u32;
    let recur_nir = &program.routines[recur_id as usize];
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
            } if *id == recur_id
        )
    }));
}
