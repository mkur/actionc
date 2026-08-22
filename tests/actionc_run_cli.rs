use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
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

fn standalone_minimal() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("runtime")
        .join("standalone_minimal.act")
}

#[test]
fn help_documents_the_canonical_runtime_selector_and_no_cart_convenience() {
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--help")
        .output()
        .expect("run actionc-run --help");

    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stderr);
    assert!(help.contains("[--runtime cart|standalone]"));
    assert!(help.contains("[--cart <path>|--no-cart]"));
}

#[test]
fn no_run_writes_an_atr_with_the_bootstrap_and_compiler_object() {
    let temp = TestDir::new();
    let output_atr = temp.path().join("nested/hello world.atr");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--mode")
        .arg("optimized")
        .arg("--no-run")
        .arg("--out-atr")
        .arg(&output_atr)
        .arg(hello_world())
        .env(
            "ACTIONC_EMULATOR",
            temp.path().join("must-not-be-discovered"),
        )
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
    let bootstrap = image
        .read_file_named("BOOT.AR0")
        .expect("read output directory")
        .expect("find BOOT.AR0");
    let program = image
        .read_file_named("PROGRAM.AR1")
        .expect("read output directory")
        .expect("find PROGRAM.AR1");
    let compiled = compile_file(
        hello_world(),
        &CompileOptions::for_mode(CompileMode::Optimized),
    )
    .expect("compile expected object");

    assert_eq!(
        bootstrap,
        [
            0xFF, 0xFF, 0x00, 0x30, 0x2D, 0x30, 0xA0, 0x00, 0x8C, 0xC9, 0x04, 0x8C, 0x00, 0xD5,
            0xA9, 0x06, 0x85, 0xB7, 0xA2, 0x60, 0xA9, 0x03, 0x9D, 0x42, 0x03, 0xA9, 0x2B, 0x9D,
            0x44, 0x03, 0xA9, 0x30, 0x9D, 0x45, 0x03, 0xA9, 0x0C, 0x9D, 0x4A, 0x03, 0xA9, 0x00,
            0x9D, 0x4B, 0x03, 0x20, 0x56, 0xE4, 0x60, 0x45, 0x3A, 0x9B, 0xE2, 0x02, 0xE3, 0x02,
            0x00, 0x30,
        ]
    );
    assert_eq!(program, compiled.object_bytes());
    assert!(
        image
            .read_file_named("AUTORUN.AR0")
            .expect("read output directory")
            .is_none()
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(output_atr.to_string_lossy().as_ref())
    );
}

#[test]
fn no_cart_writes_the_compiler_object_directly_as_ar0() {
    let temp = TestDir::new();
    let output_atr = temp.path().join("no-cart.atr");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--no-cart")
        .arg("--no-run")
        .arg("--out-atr")
        .arg(&output_atr)
        .arg(standalone_minimal())
        .output()
        .expect("run actionc-run --no-cart --no-run");

    assert!(
        output.status.success(),
        "actionc-run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let image = AtrImage::from_bytes(fs::read(&output_atr).expect("read output ATR"))
        .expect("parse output ATR");
    let program = image
        .read_file_named("PROGRAM.AR0")
        .expect("read output directory")
        .expect("find PROGRAM.AR0");
    let compiled = compile_file(
        standalone_minimal(),
        &CompileOptions::default().with_runtime(Runtime::Standalone),
    )
    .expect("compile expected standalone object");

    assert_eq!(program, compiled.object_bytes());
    assert!(
        image
            .read_file_named("BOOT.AR0")
            .expect("read output directory")
            .is_none()
    );
    assert!(
        image
            .read_file_named("PROGRAM.AR1")
            .expect("read output directory")
            .is_none()
    );
}

#[test]
fn runtime_standalone_writes_the_compiler_object_directly_as_ar0() {
    let temp = TestDir::new();
    let output_atr = temp.path().join("standalone-runtime.atr");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .args(["--runtime", "standalone", "--no-run", "--out-atr"])
        .arg(&output_atr)
        .arg(standalone_minimal())
        .output()
        .expect("run actionc-run --runtime standalone --no-run");

    assert!(
        output.status.success(),
        "actionc-run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let image = AtrImage::from_bytes(fs::read(&output_atr).expect("read output ATR"))
        .expect("parse output ATR");
    let program = image
        .read_file_named("PROGRAM.AR0")
        .expect("read output directory")
        .expect("find PROGRAM.AR0");
    let compiled = compile_file(
        standalone_minimal(),
        &CompileOptions::default().with_runtime(Runtime::Standalone),
    )
    .expect("compile expected standalone object");

    assert_eq!(program, compiled.object_bytes());
    assert!(
        image
            .read_file_named("BOOT.AR0")
            .expect("read output directory")
            .is_none()
    );
    assert!(
        image
            .read_file_named("PROGRAM.AR1")
            .expect("read output directory")
            .is_none()
    );
}

#[test]
fn runtime_standalone_rejects_an_explicit_cartridge() {
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .args(["--runtime=standalone", "--cart=action.car"])
        .arg(standalone_minimal())
        .output()
        .expect("run actionc-run with conflicting runtime and cartridge");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--runtime standalone conflicts with --cart")
    );
}

#[cfg(unix)]
#[test]
fn no_cart_launches_atari800_with_standalone_program_and_no_cart_argument() {
    let temp = TestDir::new();
    let record = temp.path().join("atari800-no-cart-args.txt");
    let output_atr = temp.path().join("standalone.atr");
    let emulator = temp.executable(
        "atari800",
        "#!/bin/sh\nset -eu\ntest -s \"$2\"\ntest -s \"$6\"\ntest -s \"$7\"\nprintf '%s\\n' \"$@\" > \"$ACTIONC_TEST_RECORD\"\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--no-cart")
        .arg("--mode=optimized")
        .arg("--emulator=atari800")
        .arg("--emulator-path")
        .arg(&emulator)
        .arg("--out-atr")
        .arg(&output_atr)
        .arg(standalone_minimal())
        .env("ACTIONC_TEST_RECORD", &record)
        .output()
        .expect("run standalone program with fake Atari800");

    assert!(
        output.status.success(),
        "actionc-run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let args = fs::read_to_string(&record).expect("read Atari800 arguments");
    let args = args.lines().collect::<Vec<_>>();
    assert_eq!(args.len(), 7);
    assert_eq!(args[0], "-config");
    assert_eq!(args[2], "-no-autosave-config");
    assert_eq!(args[3], "-xl");
    assert_eq!(args[4], "-xlxe_rom");
    assert!(!args.contains(&"-cart"));
    assert_eq!(Path::new(args[6]), output_atr);

    let image = AtrImage::from_bytes(fs::read(&output_atr).expect("read output ATR"))
        .expect("parse standalone ATR");
    assert!(
        image
            .read_file_named("PROGRAM.AR0")
            .expect("read output directory")
            .is_some()
    );
    assert!(
        image
            .read_file_named("BOOT.AR0")
            .expect("read output directory")
            .is_none()
    );
}

#[test]
fn no_run_replaces_an_existing_atr() {
    let temp = TestDir::new();
    let output_atr = temp.path().join("existing.atr");
    fs::write(&output_atr, b"old contents").expect("write old ATR");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--no-run")
        .arg("--out-atr")
        .arg(&output_atr)
        .arg(hello_world())
        .output()
        .expect("replace existing ATR");

    assert!(
        output.status.success(),
        "actionc-run failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    AtrImage::from_bytes(fs::read(output_atr).expect("read replacement ATR"))
        .expect("replacement is an ATR");
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

#[cfg(unix)]
#[test]
fn unsuccessful_emulator_exit_is_reported_and_the_run_directory_is_removed() {
    let temp = TestDir::new();
    let record = temp.path().join("failed-run-args.txt");
    let emulator = temp.executable(
        "atari800",
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ACTIONC_TEST_RECORD\"\nexit 7\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--emulator=atari800")
        .arg("--emulator-path")
        .arg(&emulator)
        .arg(hello_world())
        .env("ACTIONC_TEST_RECORD", &record)
        .output()
        .expect("run failing fake emulator");

    assert_eq!(output.status.code(), Some(1));
    let args = fs::read_to_string(record).expect("read failed launch arguments");
    let os_rom = Path::new(args.lines().nth(2).expect("find OS ROM argument"));
    assert!(!os_rom.exists(), "failed run directory should be removed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("exited unsuccessfully"));
    assert!(stderr.contains("7"));
}

#[cfg(unix)]
#[test]
fn keep_retains_and_reports_the_complete_run_directory() {
    let temp = TestDir::new();
    let record = temp.path().join("kept-run-args.txt");
    let emulator = temp.executable(
        "atari800",
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ACTIONC_TEST_RECORD\"\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-run"))
        .arg("--emulator=atari800")
        .arg("--emulator-path")
        .arg(&emulator)
        .arg("--keep")
        .arg(hello_world())
        .env("ACTIONC_TEST_RECORD", &record)
        .output()
        .expect("run fake emulator with --keep");

    assert!(output.status.success());
    let args = fs::read_to_string(record).expect("read kept launch arguments");
    let args = args.lines().collect::<Vec<_>>();
    let run_directory = Path::new(args[2])
        .parent()
        .expect("OS ROM has a parent")
        .to_path_buf();
    assert_eq!(run_directory.parent(), Some(std::env::temp_dir().as_path()));
    assert!(
        run_directory
            .file_name()
            .expect("run directory name")
            .to_string_lossy()
            .starts_with("actionc-run-")
    );
    let metadata = fs::symlink_metadata(&run_directory).expect("read kept directory metadata");
    assert!(metadata.is_dir());
    assert!(!metadata.file_type().is_symlink());
    assert!(run_directory.join("program.atr").is_file());
    assert!(run_directory.join("action.car").is_file());
    assert!(run_directory.join("altirraos-xl.rom").is_file());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("Run directory: {}", run_directory.display())));

    fs::remove_dir_all(&run_directory).expect("remove validated kept test directory");
}
