use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn word_store_consumer_materializes_directly_without_spills() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("word_arithmetic.act");

    assert!(!formatted.contains("spill sp"));
    assert!(formatted.contains("a =.b load global g0+0"));
    assert!(formatted.contains("a =.b a add #$02 carry_in=clear carry_out=produce"));
    assert!(formatted.contains("store.b global g1+0, a"));
    assert!(formatted.contains("a =.b a add #$01 carry_in=previous carry_out=ignore"));
    assert!(formatted.contains("store.b global g1+1, a"));
    assert!(formatted.contains("dec.w global g1+0"));
    assert!(!bytes.is_empty());
}

#[test]
fn int_store_consumer_materializes_like_word_without_spills() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("int_arithmetic.act");

    assert!(!formatted.contains("spill sp"));
    assert!(formatted.contains("a =.b load global g0+0"));
    assert!(formatted.contains("a =.b a add #$02 carry_in=clear carry_out=produce"));
    assert!(formatted.contains("store.b global g1+0, a"));
    assert!(formatted.contains("a =.b a add #$00 carry_in=previous carry_out=ignore"));
    assert!(formatted.contains("store.b global g1+1, a"));
    assert!(formatted.contains("dec.w global g1+0"));
    assert!(!bytes.is_empty());
}

#[test]
fn byte_store_consumer_materializes_directly_without_spills() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("byte_arithmetic.act");

    assert!(!formatted.contains("spill sp"));
    assert!(
        formatted.contains("a =.b load global g0+0") || formatted.contains("a =.b #1"),
        "{formatted}"
    );
    assert!(formatted.contains("a =.b a add #$02 carry_in=clear carry_out=ignore"));
    assert!(formatted.contains("store.b global g1+0, a"));
    assert!(
        formatted.contains("a =.b a sub #$01 carry_in=set carry_out=ignore")
            || formatted.contains("dec.b global g1+0"),
        "{formatted}"
    );
    assert!(!bytes.is_empty());
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
