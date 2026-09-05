use std::path::Path;
use std::process::Command;

const CORPUS_ROOTS: &[&str] = &[
    "fixtures/nir",
    "fixtures/semir",
    "fixtures/mir6502",
    "fixtures/runtime",
];

const DECLARED_NON_ENTRYPOINTS: &[&str] = &[
    "fixtures/runtime/card_loop_above_byte_range.act",
    "fixtures/runtime/native_real_library.act",
    "fixtures/runtime/native_real_trig.act",
    "fixtures/runtime/resident_console_input.act",
    "fixtures/runtime/resident_numeric_output.act",
];

#[test]
fn broad_fixture_corpus_verifies_lowered_and_optimized_nir() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-nir-sweep"))
        .args(CORPUS_ROOTS)
        .current_dir(repo_root)
        .output()
        .expect("run the NIR corpus sweep");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 NIR sweep output");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 NIR sweep diagnostics");

    // The five declarations below are library-style named modules, not
    // standalone compilation roots. Keep this debt visible until the sweep
    // itself becomes module-aware.
    assert_eq!(
        output.status.code(),
        Some(1),
        "unexpected sweep status\n{stdout}\n{stderr}"
    );
    let semantic_failures = stdout
        .lines()
        .filter(|line| line.starts_with("SEMFAIL"))
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect::<Vec<_>>();
    assert_eq!(semantic_failures, DECLARED_NON_ENTRYPOINTS, "{stdout}");

    for unexpected in ["LOADFAIL", "LOWERFAIL", "VERIFYFAIL", "OPTFAIL"] {
        assert!(
            !stdout.lines().any(|line| line.starts_with(unexpected)),
            "unexpected {unexpected} in NIR corpus sweep:\n{stdout}"
        );
    }
    assert!(
        stdout.contains(
            "NIR sweep summary: ok=316 load_failed=0 sem_failed=5 lower_failed=0 verify_failed=0 optimize_failed=0"
        ),
        "unexpected NIR corpus totals:\n{stdout}"
    );
}
