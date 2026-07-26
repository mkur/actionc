use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

fn optimized_warpdem() -> nir::NirProgram {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpora")
        .join("toolkit")
        .join("original")
        .join("extracted")
        .join("WARP.DEM");
    assert!(
        fixture.exists(),
        "required Toolkit fixture is missing: {}",
        fixture.display()
    );
    let loaded = load_program_with_expanded_source(&fixture)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", fixture.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", fixture.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    nir::optimize_program(&nir::lower_program(&semir))
        .unwrap_or_else(|err| panic!("optimize NIR for {}: {err:?}", fixture.display()))
}

#[test]
fn warpdem_exposes_the_expected_codegen_baseline() {
    let nir_program = optimized_warpdem();
    let mir = mir6502::lower_program(&nir_program).expect("lower WARPDEM MIR6502");
    let materialized = mir6502::materialize_program(mir, &mir6502::Mir6502Config::default())
        .expect("materialize WARPDEM MIR6502");
    mir6502::verify_program(&materialized, mir6502::MirPhase::PreEmission)
        .expect("verify WARPDEM materialized MIR6502");

    let formatted = mir6502::format_program(&materialized);
    assert_eq!(
        formatted.matches("materialize_indexed").count(),
        94,
        "{formatted}"
    );
    assert_eq!(
        formatted
            .lines()
            .filter(|line| {
                line.contains("materialize_indexed") && line.contains("<- global_addr")
            })
            .count(),
        39,
        "{formatted}"
    );
    assert!(formatted.contains("routine r17 MissileFire"), "{formatted}");
    assert!(formatted.contains("routine r18 MissileMove"), "{formatted}");

    let output = mir6502::generate_output(&nir_program, CODE_ORIGIN).expect("emit WARPDEM MIR6502");
    assert!(
        output.bytes.len() <= 7_402,
        "expected WARPDEM MIR6502 output no larger than 7402 bytes, got {}",
        output.bytes.len()
    );
}
