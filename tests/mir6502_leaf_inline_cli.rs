use std::process::Command;

#[test]
fn requested_outcomes_explain_selection_and_safe_fallbacks() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/inline_requested.act");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--backend",
            "mir6502",
            "--profile",
            "modern",
            "--emit-materialized-mir6502",
        ])
        .env("ACTIONC_MIR6502_PEEPHOLES", "sites")
        .arg(source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8(output.stderr).unwrap();
    for (callee, outcome) in [
        ("Map", "applied"),
        ("Count", "declined-operation-or-effects"),
        ("Recur", "declined-recursion"),
        ("Ping", "declined-non-improvement"),
    ] {
        assert!(
            report
                .lines()
                .any(|line| line.contains(&format!("callee={callee} "))
                    && line.contains("preference=prefer")
                    && line.contains(&format!("outcome={outcome}"))),
            "{report}"
        );
    }
    assert_eq!(report.matches("mir6502 peepholes:").count(), 2, "{report}");
    assert!(report.contains("group-cycles-saved="));
}

#[test]
fn materialized_dump_and_codegen_use_the_same_profile() {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/leaf_inline.act");
    for (mode, call, inlined) in [
        ("--emit-mir6502", "call r0 ", false),
        ("--emit-materialized-mir6502", "call r0 ", true),
        ("--emit-listing", "JSR proc_map", true),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args(["--backend", "mir6502", "--profile", "modern", mode])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let normalized = text.replace("JSR.A", "JSR");
        assert_eq!(normalized.contains(call), !inlined, "{mode}: {text}");
    }
}

#[test]
fn trial_reports_do_not_escape_even_with_environment_reporting() {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/leaf_inline.act");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--backend",
            "mir6502",
            "--profile",
            "modern",
            "--emit-materialized-mir6502",
        ])
        .env("ACTIONC_MIR6502_PEEPHOLES", "aggregate")
        .arg(source)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        report.matches("mir6502 peepholes:").count(),
        2,
        "one selection report and one final materialization report, never trial reports: {report}"
    );
    assert_eq!(
        report.matches("leaf-inline-applied: 1").count(),
        1,
        "{report}"
    );
    assert_eq!(
        report.matches("leaf-inline-trials: 1").count(),
        1,
        "{report}"
    );
}
