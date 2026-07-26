use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn circle_uses_direct_binary_call_arg_materialization() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpora")
        .join("toolkit")
        .join("original")
        .join("extracted")
        .join("CIRCLE.ACT");
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
    let abs_end = formatted
        .find("\nroutine r1 Circle")
        .expect("Circle routine");
    let abs = &formatted[..abs_end];
    assert!(!abs.contains("param p0"), "{abs}");
    assert!(
        abs.contains(
            "store.b fixed_zp $A1, x\n  store.b fixed_zp $A0, a\n  a =.b x\n  branch flag n_set"
        ),
        "{abs}"
    );
    assert!(
        !formatted.contains("store.b spill sp34+0") && !formatted.contains("store.b spill sp35+0"),
        "{formatted}"
    );
    assert!(formatted.contains("store.b fixed_zp $A0, a"), "{formatted}");
    assert!(
        formatted.contains(
            "a =.b load param p1+0\n  a =.b a add *local l4+0 carry_in=clear carry_out=produce\n  y =.b a"
        ),
        "{formatted}"
    );
    assert!(
        formatted.contains(
            "a =.b load param p0+0\n  a =.b a add *local l3+0 carry_in=clear carry_out=produce\n  store.b fixed_zp $A0, a\n  a =.b load param p0+1\n  a =.b a add *local l3+1 carry_in=previous carry_out=ignore\n  x =.b a"
        ),
        "{formatted}"
    );
    for staged_rhs in [
        "load local l3+0\n  store.b fixed_zp $A0",
        "load local l3+0\n  store.b fixed_zp $A1",
        "load local l4+0\n  store.b fixed_zp $A0",
        "load local l4+0\n  store.b fixed_zp $A1",
    ] {
        assert!(!formatted.contains(staged_rhs), "{formatted}");
    }
    assert!(
        formatted.contains(
            "a =.b load local l0+0\n  a =.b a add *local l4+0 carry_in=clear carry_out=produce\n  store.b zp0, a"
        ),
        "{formatted}"
    );
    assert!(
        formatted
            .contains("a =.b a add #$01 carry_in=clear carry_out=produce\n  store.b local l1+0, a"),
        "{formatted}"
    );
    assert!(
        !formatted.contains("a =.b load local l0+0\n  store.b zp0, a\n  a =.b load local l0+1"),
        "{formatted}"
    );
    assert_eq!(
        formatted.matches("call Plot@$A6C3").count(),
        8,
        "{formatted}"
    );
    assert_eq!(
        formatted.matches("cmp_i16_direct_").count(),
        2,
        "{formatted}"
    );
    assert_eq!(
        formatted.matches("cmp_i16_v_correct_").count(),
        2,
        "{formatted}"
    );
    assert!(!formatted.contains("spill sp"), "{formatted}");
    let abs_calls = formatted
        .match_indices("call r0 args=[a.b -> a, x.b -> x]")
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    assert_eq!(abs_calls.len(), 2, "{formatted}");
    let preserved_result = &formatted[abs_calls[0]..abs_calls[1]];
    assert!(
        preserved_result.contains("a =.b load fixed_zp $A0\n  store.b zp0, a")
            && preserved_result.contains("a =.b load fixed_zp $A1\n  store.b zp1, a"),
        "{formatted}"
    );
    let latest_result = &formatted[abs_calls[1]..];
    assert!(
        latest_result.contains("cmp_i16_direct_5:\n  a =.b load zp0\n  a =.b a sub *fixed_zp $A0"),
        "{formatted}"
    );
    for obsolete in [
        "cmp_i16_left_sign_",
        "cmp_i16_right_sign_pos_",
        "cmp_i16_right_sign_neg_",
        "cmp_i16_v_set_",
        "cmp_i16_v_clear_",
    ] {
        assert!(!formatted.contains(obsolete), "{formatted}");
    }

    let output = mir6502::generate_output(&nir_program, CODE_ORIGIN)
        .unwrap_or_else(|err| panic!("emit MIR6502 for {}: {err:?}", fixture.display()));
    assert!(
        output.bytes.len() <= 527,
        "expected CIRCLE.ACT MIR6502 output no larger than 527 bytes, got {}",
        output.bytes.len()
    );
}
