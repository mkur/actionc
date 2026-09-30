use std::path::Path;
use std::process::Command;

const CORPUS_ROOTS: &[&str] = &[
    "fixtures/nir",
    "fixtures/semir",
    "fixtures/mir6502",
    "fixtures/runtime",
];

const MODULE_AWARE_FIXTURES: &[&str] = &[
    "fixtures/runtime/card_loop_above_byte_range.act",
    "fixtures/runtime/fixed_q4_12.act",
    "fixtures/runtime/fixed_q8_8.act",
    "fixtures/runtime/fixed_q8_8_composition.act",
    "fixtures/runtime/long_integer_conversions.act",
    "fixtures/runtime/long_integer_input.act",
    "fixtures/runtime/long_integer_output.act",
    "fixtures/runtime/native_real_library.act",
    "fixtures/runtime/native_real_trig.act",
    "fixtures/runtime/oscar64/mbfixed.act",
    "fixtures/runtime/resident_console_input.act",
    "fixtures/runtime/resident_numeric_output.act",
    "fixtures/runtime/tacle/adpcm_dec/adpcm_dec.act",
    "fixtures/runtime/tacle/adpcm_enc/adpcm_enc.act",
];

fn corpus_sources(repo_root: &Path) -> Vec<String> {
    let mut directories = CORPUS_ROOTS
        .iter()
        .map(|root| repo_root.join(root))
        .collect::<Vec<_>>();
    let mut sources = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).unwrap_or_else(|error| {
            panic!("read corpus directory {}: {error}", directory.display())
        }) {
            let path = entry.expect("read corpus directory entry").path();
            if path.is_dir() {
                // Match the sweep's exclusion of generated outputs.
                if !matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("outputs" | "target" | ".git")
                ) {
                    directories.push(path);
                }
            } else if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("act"))
            {
                sources.push(
                    path.strip_prefix(repo_root)
                        .expect("fixture inside repository")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    sources.sort();
    sources
}

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

    // These named-module fixtures require the module-aware public compiler,
    // which their VM tests exercise. Keep the legacy source-only sweep's
    // limitation explicit until the sweep itself becomes module-aware.
    assert_eq!(
        output.status.code(),
        Some(1),
        "unexpected sweep status\n{stdout}\n{stderr}"
    );
    let expected_sources = corpus_sources(repo_root);
    // Exercise report parsing with both Unix and Windows line endings.
    let stdout = stdout.replace("\r\n", "\n");
    check_corpus_report(&stdout, &expected_sources);
    check_corpus_report(&stdout.replace('\n', "\r\n"), &expected_sources);
}

fn check_corpus_report(stdout: &str, expected_sources: &[String]) {
    let semantic_failures = stdout
        .lines()
        .filter(|line| line.starts_with("SEMFAIL"))
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect::<Vec<_>>();
    assert_eq!(semantic_failures, MODULE_AWARE_FIXTURES, "{stdout}");

    for unexpected in ["LOADFAIL", "LOWERFAIL", "VERIFYFAIL", "OPTFAIL"] {
        assert!(
            !stdout.lines().any(|line| line.starts_with(unexpected)),
            "unexpected {unexpected} in NIR corpus sweep:\n{stdout}"
        );
    }
    // Require one result for every fixture, including newly added sources.
    // A count derived only from stdout could silently accept skipped fixtures.
    let mut reported_sources = stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            matches!(fields.next(), Some("OK" | "SEMFAIL"))
                .then(|| fields.next().expect("fixture path in sweep result"))
        })
        .collect::<Vec<_>>();
    reported_sources.sort();
    assert_eq!(reported_sources, expected_sources, "NIR corpus coverage");

    let expected_summary = format!(
        "NIR sweep summary: ok={} load_failed=0 sem_failed={} lower_failed=0 verify_failed=0 optimize_failed=0",
        expected_sources.len() - MODULE_AWARE_FIXTURES.len(),
        MODULE_AWARE_FIXTURES.len(),
    );
    assert!(
        stdout.lines().any(|line| line == expected_summary),
        "unexpected NIR corpus totals:\n{stdout}"
    );
}
