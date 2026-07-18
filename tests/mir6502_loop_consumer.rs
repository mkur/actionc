use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn while_loop_condition_and_increment_avoid_spills() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("while_loop.act");

    assert!(!formatted.contains("spill sp"));
    assert!(formatted.contains("flags = cmp.b a lt #$03"));
    assert!(formatted.contains("branch fused b1:1 c_clear ? b2 : b3"));
    assert!(formatted.contains("inc.b global g0+0"));
    assert!(
        bytes
            .windows(3)
            .any(|bytes| matches!(bytes, [0xC9, 0x03, 0x90 | 0xB0]))
    );
    assert!(bytes.windows(3).any(|bytes| bytes == [0xEE, 0x00, 0x30]));
}

#[test]
fn byte_for_loop_bound_and_body_consumers_avoid_spills() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("for_loop_byte.act");

    assert!(!formatted.contains("spill sp"));
    assert!(formatted.contains("flags = cmp.b a le #$03"));
    assert!(formatted.contains("a =.b a add *global g0+0 carry_in=clear carry_out=ignore"));
    assert!(formatted.contains("store.b global g1+0, a"));
    assert!(formatted.contains("inc.b global g0+0"));
    assert!(
        bytes
            .windows(3)
            .any(|bytes| matches!(bytes, [0xC9, 0x03, 0x90 | 0xB0]))
    );
    assert!(bytes.windows(3).any(|bytes| bytes == [0x6D, 0x00, 0x30]));
}

#[test]
fn complex_while_and_until_conditions_materialize_bool_values() {
    for fixture in [
        "while_complex_bool_array_func.act",
        "until_complex_bool_array_func.act",
    ] {
        let (formatted, bytes) = compile_materialized_mir6502_fixture(fixture);

        assert!(
            formatted.contains("call r"),
            "{fixture} should keep the function call in the loop condition:\n{formatted}"
        );
        assert!(
            formatted.contains("load computed")
                || formatted.contains("load *")
                || formatted.contains("load (fixed_zp $AC),y")
                || formatted.contains("[y]"),
            "{fixture} should exercise an array read in the loop condition:\n{formatted}"
        );
        let materialized_bool =
            formatted.contains("a =.b a or") || formatted.contains("a =.b a and");
        let short_circuit_flags =
            formatted.contains("cmp_sc_") && formatted.contains("branch flag");
        assert!(
            materialized_bool || short_circuit_flags,
            "{fixture} should lower a compound boolean loop condition:\n{formatted}"
        );
        assert!(
            formatted.contains("= cmp.b") || formatted.contains("flags = cmp.b"),
            "{fixture} should exercise byte compares:\n{formatted}"
        );
        assert!(
            !formatted.contains("branch bool"),
            "{fixture} leaked a pre-emission branch bool:\n{formatted}"
        );
        let emitted_bool_materialization = bytes
            .windows(5)
            .any(|bytes| matches!(bytes, [0xF0 | 0xD0, 0x05, 0xA9, 0x00, 0x4C]));
        assert!(
            emitted_bool_materialization || short_circuit_flags,
            "{fixture} did not emit a materialized or short-circuit boolean condition"
        );
        assert!(!bytes.is_empty(), "{fixture} should emit object bytes");
    }
}

fn compile_materialized_mir6502_fixture(name: &str) -> (String, Vec<u8>) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("mir6502")
        .join(name);
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
    let output = mir6502::generate_output(&nir_program, CODE_ORIGIN)
        .unwrap_or_else(|err| panic!("emit MIR6502 for {}: {err:?}", fixture.display()));
    (formatted, output.bytes)
}
