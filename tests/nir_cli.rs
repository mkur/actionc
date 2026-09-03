use std::path::Path;
use std::process::Command;

use actionc::includes::load_program_with_expanded_source;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn emit_nir_prints_nir_output() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("scalar_assignments.act");

    let output = run_actionc("--emit-nir", &fixture);

    assert!(output.contains("routine r0 Main"));
}

#[test]
fn candidate_targets_reach_verified_nir_with_their_layout_contract() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("scalar_assignments.act");

    for (target, expected) in [
        (
            "wdc-65816-native",
            "target wdc-65816-native cpu=Wdc65816 endian=Little address_bits=24 data_pointer=3 code_pointer=3",
        ),
        (
            "wdc-65816-small",
            "target wdc-65816-small cpu=Wdc65816 endian=Little address_bits=24 data_pointer=2 code_pointer=2",
        ),
        (
            "motorola-68000",
            "target motorola-68000 cpu=Motorola68000 endian=Big address_bits=32 data_pointer=4 code_pointer=4",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args(["--profile", "modern", "--target", target, "--emit-nir"])
            .arg(&fixture)
            .output()
            .unwrap_or_else(|error| panic!("emit NIR for {target}: {error}"));
        assert!(
            output.status.success(),
            "{target} NIR inspection failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("UTF-8 NIR");
        assert!(
            stdout.contains(expected),
            "unexpected {target} NIR:\n{stdout}"
        );
        assert!(
            stdout.contains("activation=native-reentrant"),
            "unexpected {target} activation:\n{stdout}"
        );
    }
}

#[test]
fn candidate_target_codegen_reports_the_missing_backend() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("scalar_assignments.act");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--target",
            "motorola-68000",
            "--emit-code",
        ])
        .arg(&fixture)
        .output()
        .expect("request unavailable 68k backend");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("code generation backend for target `motorola-68000` is not implemented")
    );
}

#[test]
fn emit_nir_honors_the_modern_semantic_profile_for_native_real() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("native_real.act");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args(["--profile", "modern", "--emit-nir"])
        .arg(&fixture)
        .output()
        .unwrap_or_else(|err| panic!("run actionc-emit for {}: {err}", fixture.display()));

    assert!(
        output.status.success(),
        "actionc-emit failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 NIR output");
    assert!(stdout.contains("static __nir_real_Main_0:REAL"));
    assert!(stdout.contains("real.mul"));
    assert!(stdout.contains("real.cmp.gt"));
}

#[test]
fn emit_optimized_nir_prints_the_post_optimizer_program() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("unary_cast.act");

    let lowered = run_actionc("--emit-nir", &fixture);
    let optimized = run_actionc("--emit-optimized-nir", &fixture);
    let expected = optimized_nir_from_library(&fixture);

    assert!(lowered.contains("%t1:Int = Neg %t0"));
    assert!(!optimized.contains("%t1:Int = Neg %t0"));
    assert!(optimized.contains("store b = 255"));
    assert_eq!(optimized, expected);
}

#[test]
fn emit_nir_stats_compares_lowered_and_optimized_programs() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("unary_cast.act");

    let first = run_actionc("--emit-nir-stats", &fixture);
    let second = run_actionc("--emit-nir-stats", &fixture);

    assert_eq!(first, second, "NIR statistics must be deterministic");
    assert!(first.starts_with("nir statistics\nstage lowered\n"));
    assert!(first.contains("stage optimized\n"));
    assert!(first.contains("optimizer_total\n"));
    assert!(first.contains("block_parameters=0\n"));
    assert!(first.contains("edge_arguments=0\n"));
    assert!(first.contains("operations.removed=9\n"));
    assert!(first.contains("temp_definitions.removed=9\n"));
    assert!(first.contains("loads.removed=2\n"));
}

#[test]
fn emit_tac_flag_is_rejected() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("scalar_assignments.act");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg("--emit-tac")
        .arg(&fixture)
        .output()
        .unwrap_or_else(|err| panic!("run actionc-emit --emit-tac {}: {err}", fixture.display()));

    assert!(
        !output.status.success(),
        "actionc-emit --emit-tac {} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        fixture.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unexpected argument: --emit-tac"),
        "unexpected stderr for --emit-tac\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_actionc(flag: &str, fixture: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg(flag)
        .arg(fixture)
        .output()
        .unwrap_or_else(|err| panic!("run actionc {flag} {}: {err}", fixture.display()));

    assert!(
        output.status.success(),
        "actionc {flag} {} failed\nstdout:\n{}\nstderr:\n{}",
        fixture.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .unwrap_or_else(|err| panic!("actionc {flag} output was not UTF-8: {err}"))
}

fn optimized_nir_from_library(fixture: &Path) -> String {
    let loaded = load_program_with_expanded_source(fixture)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", fixture.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", fixture.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let lowered = nir::lower_program(&semir);
    let optimized = nir::optimize_program(&lowered)
        .unwrap_or_else(|err| panic!("optimize NIR for {}: {err:?}", fixture.display()));
    nir::format_program(&optimized)
}
