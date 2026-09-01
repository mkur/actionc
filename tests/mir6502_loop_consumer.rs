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
fn byte_compound_increment_reloads_for_a_following_compare() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("byte_compound_increment_compare.act");

    assert!(
        formatted.contains("inc.b global g0+0"),
        "expected direct increment:\n{formatted}"
    );
    assert!(!formatted.contains(" add #$01"));
    assert!(
        bytes
            .windows(6)
            .any(|bytes| bytes == [0xE6, 0xCF, 0xA5, 0xCF, 0xC9, 0x0A]),
        "expected INC/LDA/CMP for a==+1 followed by a=10: {bytes:02X?}"
    );
}

#[test]
fn byte_for_loop_bound_and_body_consumers_avoid_spills() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("for_loop_byte.act");

    assert!(!formatted.contains("spill sp"));
    assert!(formatted.contains("flags = cmp.b a lt #$04"));
    assert!(formatted.contains("a =.b a add *global g0+0 carry_in=clear carry_out=ignore"));
    assert!(formatted.contains("store.b global g1+0, a"));
    assert!(formatted.contains("inc.b global g0+0"));
    assert!(
        bytes
            .windows(3)
            .any(|bytes| matches!(bytes, [0xC9, 0x04, 0x90 | 0xB0]))
    );
    assert!(bytes.windows(3).any(|bytes| bytes == [0x6D, 0x00, 0x30]));
}

#[test]
fn full_range_static_byte_sum_uses_one_y_carrier_and_no_hot_loop_spills() {
    let (formatted, bytes) = compile_materialized_mir6502_path(Path::new(
        "fixtures/runtime/full_range_static_array_sum.act",
    ));

    assert!(formatted.contains("y =.b #0"), "{formatted}");
    assert!(formatted.contains("a =.b load $801E[y]"), "{formatted}");
    for base in ["$801F", "$8020", "$803F"] {
        assert!(
            formatted.contains(&format!("a =.b a add {base}[y]")),
            "missing indexed accumulation from {base}:\n{formatted}"
        );
    }
    assert!(
        formatted.contains("store.b global g1+0[y], a"),
        "{formatted}"
    );
    assert!(formatted.contains("inc y"), "{formatted}");
    assert!(formatted.contains("branch flag z_clear"), "{formatted}");
    assert!(!formatted.contains("spill sp"), "{formatted}");
    assert!(!formatted.contains("load global g2+0"), "{formatted}");
    assert!(
        bytes.windows(2).any(|window| window == [0xC8, 0xD0]),
        "expected INY/BNE latch: {bytes:02X?}"
    );
    assert!(bytes.contains(&0xB9), "expected absolute,Y LDA");
    assert_eq!(
        bytes.iter().filter(|byte| **byte == 0x79).count(),
        3,
        "expected three absolute,Y ADCs: {bytes:02X?}"
    );
    assert!(bytes.contains(&0x99), "expected absolute,Y STA");
}

#[test]
fn complex_while_and_until_conditions_use_short_circuit_cfg() {
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
        let direct_condition_branches = formatted
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with("branch fused") || line.starts_with("branch flag")
            })
            .count();
        assert!(
            direct_condition_branches >= 4,
            "{fixture} should branch directly from compares or proven incoming flags for three condition leaves and the helper:\n{formatted}"
        );
        assert!(!formatted.contains("a =.b a or"), "{formatted}");
        assert!(!formatted.contains("a =.b a and"), "{formatted}");
        assert!(
            formatted.contains("= cmp.b") || formatted.contains("flags = cmp.b"),
            "{fixture} should exercise byte compares:\n{formatted}"
        );
        assert!(
            !formatted.contains("branch bool"),
            "{fixture} leaked a pre-emission branch bool:\n{formatted}"
        );
        assert!(
            bytes
                .windows(2)
                .any(|bytes| matches!(bytes[0], 0x90 | 0xB0 | 0xD0 | 0xF0)),
            "{fixture} did not emit conditional branches"
        );
        assert!(!bytes.is_empty(), "{fixture} should emit object bytes");
    }
}

fn compile_materialized_mir6502_fixture(name: &str) -> (String, Vec<u8>) {
    compile_materialized_mir6502_path(&Path::new("fixtures").join("mir6502").join(name))
}

fn compile_materialized_mir6502_path(relative: &Path) -> (String, Vec<u8>) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
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
