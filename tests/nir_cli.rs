use std::path::Path;
use std::process::Command;

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
