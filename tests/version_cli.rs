use std::process::Command;

fn assert_version(executable: &str, path: &str, option: &str) {
    let output = Command::new(path)
        .arg(option)
        .output()
        .unwrap_or_else(|error| panic!("run {executable} {option}: {error}"));

    assert!(
        output.status.success(),
        "{executable} {option} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("version output is UTF-8");
    assert!(
        stdout.starts_with(&format!("{executable} {}", env!("CARGO_PKG_VERSION"))),
        "unexpected version output: {stdout:?}"
    );
    assert_eq!(stdout.lines().count(), 1);
    assert!(output.stderr.is_empty());
}

#[test]
fn public_executables_report_the_package_version() {
    for (name, path) in [
        ("actionc", env!("CARGO_BIN_EXE_actionc")),
        ("actionc-run", env!("CARGO_BIN_EXE_actionc-run")),
        ("actionc-emit", env!("CARGO_BIN_EXE_actionc-emit")),
    ] {
        assert_version(name, path, "--version");
        assert_version(name, path, "-V");
    }
}
