use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

fn optimized_kalscope() -> nir::NirProgram {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("toolkit")
        .join("modern")
        .join("KALSCOPE.DEM");
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
fn kalscope_exposes_the_expected_codegen_baseline() {
    let nir_program = optimized_kalscope();
    let mir = mir6502::lower_program(&nir_program).expect("lower KALSCOPE MIR6502");
    let materialized = mir6502::materialize_program(mir, &mir6502::Mir6502Config::default())
        .expect("materialize KALSCOPE MIR6502");
    mir6502::verify_program(&materialized, mir6502::MirPhase::PreEmission)
        .expect("verify KALSCOPE materialized MIR6502");

    let formatted = mir6502::format_program(&materialized);
    assert_eq!(
        formatted
            .matches("call r2 args=[a.b -> a, x.b -> x]")
            .count(),
        6,
        "{formatted}"
    );
    assert_eq!(
        formatted
            .matches("call r3 args=[a.b -> a, x.b -> x]")
            .count(),
        6,
        "{formatted}"
    );
    assert_eq!(
        formatted.matches("inc.w local l6+0").count(),
        9,
        "{formatted}"
    );
    assert_eq!(
        formatted.matches("a =.b a xor *global").count(),
        16,
        "{formatted}"
    );
    assert!(!formatted.contains("spill sp78"), "{formatted}");
    assert!(!formatted.contains("spill sp86"), "{formatted}");

    let output =
        mir6502::generate_output(&nir_program, CODE_ORIGIN).expect("emit KALSCOPE MIR6502");
    assert!(
        output.bytes.len() <= 3_383,
        "expected KALSCOPE MIR6502 output no larger than 3383 bytes, got {}",
        output.bytes.len()
    );
}
