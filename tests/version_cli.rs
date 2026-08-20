use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("actionc-version-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).expect("create version test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

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
    let digest = stdout
        .split("vfs=")
        .nth(1)
        .and_then(|suffix| suffix.strip_suffix(")\n"))
        .unwrap_or_else(|| panic!("version output has no VFS digest: {stdout:?}"));
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
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

#[test]
fn copied_compiler_reports_the_same_vfs_digest_without_extracting_files() {
    let directory = TempDir::new();
    let source = Path::new(env!("CARGO_BIN_EXE_actionc"));
    let copied = directory.path().join(
        source
            .file_name()
            .expect("compiler executable has a file name"),
    );
    fs::copy(source, &copied).expect("copy compiler executable");

    let original = Command::new(source)
        .arg("--version")
        .output()
        .expect("run original compiler");
    let copied_output = Command::new(&copied)
        .current_dir(directory.path())
        .arg("--version")
        .output()
        .expect("run copied compiler");

    assert!(original.status.success());
    assert!(copied_output.status.success());
    assert_eq!(copied_output.stdout, original.stdout);
    assert!(copied_output.stderr.is_empty());
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("read copied compiler directory")
            .count(),
        1,
        "the copied compiler must not extract its embedded VFS"
    );
}
