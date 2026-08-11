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

    #[cfg(unix)]
    fn executable(&self, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = self.0.join(name);
        fs::write(&path, script).expect("write fake emulator");
        let mut permissions = fs::metadata(&path)
            .expect("read fake emulator metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("make fake emulator executable");
        path
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

#[cfg(unix)]
#[test]
fn launches_atari800_with_embedded_assets_and_the_prepared_atr() {
    let temp = TestDir::new();
    let record = temp.path().join("atari800-args.txt");
    let output_atr = temp.path().join("program.atr");
    let emulator = temp.executable(
        "atari800",
        "#!/bin/sh\nset -eu\ntest -s \"$3\"\ntest -s \"$5\"\ntest -s \"$6\"\nprintf '%s\\n' \"$@\" > \"$ACTIONC_TEST_RECORD\"\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--emulator")
        .arg("atari800")
        .arg("--emulator-path")
        .arg(&emulator)
        .arg("--out-atr")
        .arg(&output_atr)
        .arg(hello_world())
        .env("ACTIONC_TEST_RECORD", &record)
        .output()
        .expect("run actionc-run with fake Atari800");

    assert!(
        output.status.success(),
        "actionc-run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let args = fs::read_to_string(&record).expect("read Atari800 arguments");
    let args = args.lines().collect::<Vec<_>>();
    assert_eq!(args.len(), 6);
    assert_eq!(args[0], "-xl");
    assert_eq!(args[1], "-xlxe_rom");
    assert_eq!(args[3], "-cart");
    assert_eq!(Path::new(args[5]), output_atr);
    assert!(output_atr.is_file());
    assert!(
        !Path::new(args[2]).exists(),
        "run directory should be removed"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Launching Atari800"));
}

#[cfg(unix)]
#[test]
fn launches_altirra_with_separate_switch_and_media_arguments() {
    let temp = TestDir::new();
    let record = temp.path().join("altirra-args.txt");
    let output_atr = temp.path().join("program.atr");
    let emulator = temp.executable(
        "Altirra64.exe",
        "#!/bin/sh\nset -eu\ntest -s \"$6\"\ntest -s \"$8\"\nprintf '%s\\n' \"$@\" > \"$ACTIONC_TEST_RECORD\"\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--emulator=altirra")
        .arg("--emulator-path")
        .arg(&emulator)
        .arg("--out-atr")
        .arg(&output_atr)
        .arg(hello_world())
        .env("ACTIONC_TEST_RECORD", &record)
        .output()
        .expect("run actionc-run with fake Altirra");

    assert!(
        output.status.success(),
        "actionc-run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let args = fs::read_to_string(&record).expect("read Altirra arguments");
    let args = args.lines().collect::<Vec<_>>();
    assert_eq!(
        &args[..5],
        &[
            "/tempprofile",
            "/hardware:800xl",
            "/kernel:llexl",
            "/nobasic",
            "/cart",
        ]
    );
    assert_eq!(args[6], "/disk");
    assert_eq!(Path::new(args[7]), output_atr);
    assert!(
        !Path::new(args[5]).exists(),
        "run directory should be removed"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Launching Altirra"));
}
