use std::path::Path;

use actionc::includes::load_program_with_expanded_source;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn tn_exposes_high_value_scalar_promotion_candidates() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("tn")
        .join("modern")
        .join("TN.ACT");
    let loaded = load_program_with_expanded_source(&source)
        .unwrap_or_else(|diagnostics| panic!("load {}: {diagnostics:?}", source.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|diagnostics| panic!("analyze {}: {diagnostics:?}", source.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let lowered = nir::lower_program(&semir);
    nir::verify_program(&lowered)
        .unwrap_or_else(|diagnostics| panic!("verify {}: {diagnostics:?}", source.display()));

    let analysis = nir::analyze_program_storage(&lowered);
    let sort = analysis.routine("Sort").expect("Sort storage facts");
    assert!(
        sort.storage_by_name("gap")
            .is_some_and(nir::NirStorageFacts::is_promotable),
        "Sort::gap should be a scalar-promotion candidate: {sort:#?}"
    );

    let copy = analysis.routine("Copy").expect("Copy storage facts");
    for name in [
        "mem", "len", "files", "j", "k", "flag", "diskswap", "isopen",
    ] {
        let facts = copy
            .storage_by_name(name)
            .unwrap_or_else(|| panic!("Copy::{name} storage facts"));
        assert!(
            facts.is_promotable(),
            "Copy::{name} should be a scalar-promotion candidate: {facts:#?}"
        );
    }

    let optimized = nir::optimize_program(&lowered)
        .unwrap_or_else(|diagnostics| panic!("optimize {}: {diagnostics:?}", source.display()));
    for routine_name in ["SetWin", "Copy", "Sort"] {
        let lowered_loads = routine_loads(&lowered, routine_name);
        let optimized_loads = routine_loads(&optimized, routine_name);
        assert!(
            optimized_loads < lowered_loads,
            "{routine_name} should contain fewer loads after exact storage-value propagation: lowered={lowered_loads} optimized={optimized_loads}"
        );
    }
}

fn routine_loads(program: &nir::NirProgram, name: &str) -> usize {
    program
        .routines
        .iter()
        .find(|routine| routine.name == name)
        .unwrap_or_else(|| panic!("{name} routine"))
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter(|op| matches!(op, nir::NirOp::Load { .. }))
        .count()
}
