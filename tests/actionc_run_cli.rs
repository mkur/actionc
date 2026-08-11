use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::compiler::{CompileMode, CompileOptions, compile_file};
use atrcopy_rs::AtrImage;

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("actionc-run-cli-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).expect("create actionc-run test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn hello_world() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("hello-world.act")
}

#[test]
fn no_run_writes_an_atr_with_the_compiler_object() {
    let temp = TestDir::new();
    let output_atr = temp.path().join("nested/hello world.atr");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--mode")
        .arg("optimized")
        .arg("--no-run")
        .arg("--out-atr")
        .arg(&output_atr)
        .arg(hello_world())
        .output()
        .expect("run actionc-run --no-run");

    assert!(
        output.status.success(),
        "actionc-run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let image = AtrImage::from_bytes(fs::read(&output_atr).expect("read output ATR"))
        .expect("parse output ATR");
    let autorun = image
        .read_file_named("AUTORUN.AR0")
        .expect("read output directory")
        .expect("find AUTORUN.AR0");
    let compiled = compile_file(
        hello_world(),
        &CompileOptions::for_mode(CompileMode::Optimized),
    )
    .expect("compile expected object");

    assert_eq!(autorun, compiled.object_bytes());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(output_atr.to_string_lossy().as_ref())
    );
}

#[test]
fn failed_compilation_does_not_create_an_atr() {
    let temp = TestDir::new();
    let source = temp.path().join("broken.act");
    let output_atr = temp.path().join("broken.atr");
    fs::write(&source, "PROC Broken( RETURN").expect("write broken source");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--no-run")
        .arg("--out-atr")
        .arg(&output_atr)
        .arg(&source)
        .output()
        .expect("run actionc-run with invalid source");

    assert_eq!(output.status.code(), Some(1));
    assert!(!output_atr.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected"));
}
