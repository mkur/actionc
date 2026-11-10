use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use std::path::Path;
use std::process::Command;

#[test]
fn copy_candidate_reporting_preserves_the_emitted_image() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("fixtures/runtime/register_copy_forwarding.act");
    let output =
        std::env::temp_dir().join(format!("actionc-copy-census-{}.xex", std::process::id()));
    let baseline = compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(Runtime::ActionCart),
    )
    .unwrap();
    let reported = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .env("ACTIONC_MIR6502_PEEPHOLES", "sites")
        .args(["--mode", "mir6502", "--runtime", "cart", "-o"])
        .arg(&output)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        reported.status.success(),
        "{}",
        String::from_utf8_lossy(&reported.stderr)
    );
    assert_eq!(baseline.object_bytes(), std::fs::read(&output).unwrap());
    std::fs::remove_file(output).unwrap();
    let report = String::from_utf8(reported.stderr).unwrap();
    assert!(
        report.contains("register-copy-transfer-candidate"),
        "{report}"
    );
    assert!(report.contains("register-copy-load-candidate"), "{report}");
    assert!(report.contains("available=Some(Y)"), "{report}");
}
