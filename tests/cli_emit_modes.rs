use std::path::Path;
use std::process::Command;

#[test]
fn multiple_emit_modes_are_rejected_by_cli() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("scalar_assignments.act");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg("--emit-load")
        .arg("--emit-listing")
        .arg("--backend")
        .arg("classic")
        .arg(&fixture)
        .output()
        .unwrap_or_else(|err| panic!("run actionc with multiple emit modes: {err}"));

    assert!(
        !output.status.success(),
        "actionc unexpectedly accepted multiple emit modes\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("multiple emit modes selected: --emit-listing, --emit-load"),
        "unexpected stderr for multiple emit modes\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn lowered_and_optimized_nir_modes_are_mutually_exclusive() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("scalar_assignments.act");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg("--emit-nir")
        .arg("--emit-optimized-nir")
        .arg(&fixture)
        .output()
        .unwrap_or_else(|err| panic!("run actionc with multiple NIR emit modes: {err}"));

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("multiple emit modes selected: --emit-nir, --emit-optimized-nir"),
        "unexpected stderr for multiple NIR emit modes\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn source_codegen_settings_drive_default_backend() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("toolkit")
        .join("modern")
        .join("KALSCOPE.DEM");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg(&fixture)
        .output()
        .unwrap_or_else(|err| panic!("run actionc with source defaults: {err}"));

    assert!(
        output.status.success(),
        "actionc ignored source defaults\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).starts_with("02 4B 3A"),
        "unexpected default output\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
