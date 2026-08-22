use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn turtle_exposes_the_expected_codegen_baseline() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpora")
        .join("toolkit")
        .join("original")
        .join("extracted")
        .join("TURTLE.DM1");
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
    assert!(formatted.contains("routine r0 Turn"), "{formatted}");
    assert!(formatted.contains("routine r5 Forward"), "{formatted}");
    assert!(formatted.contains("routine r6 SetTurtle"), "{formatted}");
    assert_eq!(
        formatted.matches("helper mul args=").count(),
        4,
        "{formatted}"
    );
    assert_eq!(
        formatted.matches("helper lsh args=").count(),
        2,
        "{formatted}"
    );
    let forward = formatted
        .split_once("routine r5 Forward")
        .and_then(|(_, tail)| tail.split_once("routine r6 SetTurtle"))
        .map(|(body, _)| body)
        .expect("Forward routine is present");
    assert_eq!(forward.matches("store.b spill ").count(), 0, "{formatted}");
    assert_eq!(forward.matches("load spill ").count(), 0, "{formatted}");
    assert!(
        forward.find("call r4 args=") < forward.find("load param p0+0"),
        "{formatted}"
    );
    assert!(
        forward.find("call r3 args=") < forward.rfind("load param p0+0"),
        "{formatted}"
    );
    assert!(
        forward.contains("store.b fixed_zp $A0, a\n  store.b fixed_zp $A1, x"),
        "{formatted}"
    );
    assert!(forward.contains("y =.b a"), "{formatted}");
    assert!(
        formatted.contains("store.b fixed_zp $A0, a\n  store.b fixed_zp $A1, x"),
        "{formatted}"
    );
    assert_eq!(
        formatted.matches("cmp_i16_direct_").count(),
        0,
        "{formatted}"
    );
    assert!(
        formatted.contains("branch flag n_set ? b2 : b3"),
        "{formatted}"
    );

    let output = mir6502::generate_output(&nir_program, CODE_ORIGIN)
        .unwrap_or_else(|err| panic!("emit MIR6502 for {}: {err:?}", fixture.display()));
    assert!(
        output.bytes.len() <= 1_078,
        "expected TURTLE1 MIR6502 output no larger than 1078 bytes, got {}",
        output.bytes.len()
    );
}
