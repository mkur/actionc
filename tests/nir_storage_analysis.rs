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
}
