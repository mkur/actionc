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

    assert!(output.contains("routine Main"));
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

    assert!(lowered.contains("%t0:Byte = Neg 1"));
    assert!(!optimized.contains("%t0:Byte = Neg 1"));
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
    assert!(first.contains("operations.removed=7\n"));
    assert!(first.contains("temp_definitions.removed=7\n"));
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
