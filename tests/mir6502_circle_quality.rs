use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn circle_uses_direct_binary_call_arg_materialization() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("toolkit")
        .join("original")
        .join("extracted")
        .join("CIRCLE.ACT");
    if !fixture.exists() {
        eprintln!("skipping optional fixture {}", fixture.display());
        return;
    }
    let loaded = load_program_with_expanded_source(&fixture)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", fixture.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", fixture.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let nir_program = nir::optimize_program(&nir::lower_program(&semir))
        .unwrap_or_else(|err| panic!("optimize NIR for {}: {err:?}", fixture.display()));

    let mir = mir6502::lower_program(&nir_program)
        .unwrap_or_else(|err| panic!("lower MIR6502 for {}: {err:?}", fixture.display()));
    let materialized = mir6502::materialize_program(mir, &mir6502::Mir6502Config::default())
        .unwrap_or_else(|err| panic!("materialize MIR6502 for {}: {err:?}", fixture.display()));
    mir6502::verify_program(&materialized, mir6502::MirPhase::PreEmission).unwrap_or_else(|err| {
        panic!(
            "verify materialized MIR6502 for {}: {err:?}",
            fixture.display()
        )
    });

    let formatted = mir6502::format_program(&materialized);
    assert!(
        !formatted.contains("store.b spill sp34+0") && !formatted.contains("store.b spill sp35+0"),
        "{formatted}"
    );
    assert!(formatted.contains("store.b fixed_zp $A0, a"), "{formatted}");
    assert!(
        formatted.contains(
            "a =.b load local l4+0\n  store.b fixed_zp $A0, a\n  a =.b load param p1+0\n  a =.b a add *fixed_zp $A0"
        ),
        "{formatted}"
    );
    assert!(
        formatted.contains(
            "a =.b load local l3+0\n  store.b fixed_zp $A1, a\n  a =.b load param p0+0\n  a =.b a sub *fixed_zp $A1"
        ),
        "{formatted}"
    );
    assert!(
        !formatted.contains(
            "a =.b load local l3+0\n  store.b fixed_zp $A1, a\n  a =.b a sub *fixed_zp $A1"
        ),
        "{formatted}"
    );
    assert!(
        formatted.matches("call Plot@$A6C3").count() >= 8,
        "{formatted}"
    );

    let output = mir6502::generate_output(&nir_program, CODE_ORIGIN)
        .unwrap_or_else(|err| panic!("emit MIR6502 for {}: {err:?}", fixture.display()));
    assert!(
        output.bytes.len() < 1000,
        "expected CIRCLE.ACT MIR6502 output under 1000 bytes, got {}",
        output.bytes.len()
    );
}
