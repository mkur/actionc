use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("actionc-cli-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).expect("create CLI test directory");
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

fn load_file_origin(bytes: &[u8]) -> u16 {
    assert!(bytes.len() >= 4, "load file is too short");
    assert_eq!(&bytes[..2], &[0xff, 0xff], "missing load-file header");
    u16::from_le_bytes([bytes[2], bytes[3]])
}

#[test]
fn help_describes_the_existing_listing_options_as_mads_assembly() {
    for binary in [
        env!("CARGO_BIN_EXE_actionc"),
        env!("CARGO_BIN_EXE_actionc-emit"),
    ] {
        let output = Command::new(binary)
            .arg("--help")
            .output()
            .unwrap_or_else(|error| panic!("run {binary} --help: {error}"));
        assert!(output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("re-originable"),
            "unexpected help:\n{stderr}"
        );
        assert!(
            stderr.contains("ACTIONC_ORIGIN"),
            "unexpected help:\n{stderr}"
        );
        assert!(
            stderr.contains("MADS assembly"),
            "unexpected help:\n{stderr}"
        );
    }
}

#[test]
fn repeatable_module_paths_are_used_by_compile_and_emit_commands() {
    let temp = TestDir::new();
    let root = temp.path().join("app.act");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(second.join("lib")).expect("create module directory");
    fs::write(&root, "MODULE APP\nIMPORT LIB.VALUE\nENDMODULE\n").expect("write root module");
    fs::write(
        second.join("lib/value.act"),
        "MODULE LIB.VALUE\nPUBLIC BYTE value\nENDMODULE\n",
    )
    .expect("write imported module");

    for (binary, emit_arg) in [
        (env!("CARGO_BIN_EXE_actionc"), None),
        (env!("CARGO_BIN_EXE_actionc-emit"), Some("--emit-semir")),
    ] {
        let mut command = Command::new(binary);
        command
            .arg("--module-path")
            .arg(&first)
            .arg(format!("--module-path={}", second.display()));
        if let Some(emit_arg) = emit_arg {
            command.arg(emit_arg);
        }
        let output = command.arg(&root).output().expect("run module CLI");
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("named-module semantic resolution is not implemented yet"),
            "module was not found through the ordered paths:\n{stderr}"
        );
        assert!(!stderr.contains("cannot find module"));
    }
}

#[test]
fn compiles_object_and_listing_in_one_invocation() {
    let temp = TestDir::new();
    let object = temp.path().join("nested/hello.com");
    let listing = temp.path().join("listings/hello.lst");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg(hello_world())
        .arg("-o")
        .arg(&object)
        .arg("--listing")
        .arg(&listing)
        .output()
        .expect("run actionc");

    assert!(
        output.status.success(),
        "actionc failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(fs::metadata(&object).expect("object metadata").len() > 0);
    let listing_text = fs::read_to_string(&listing).expect("read listing");
    assert!(listing_text.contains("PROC Main"));
    assert!(listing_text.contains("JSR.A $A46C"));
    assert!(listing_text.contains("ORG $02E2"));

    let emitted = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg("--emit-load")
        .arg(hello_world())
        .output()
        .expect("run actionc-emit");
    assert!(emitted.status.success());
    assert_eq!(fs::read(object).expect("read object"), emitted.stdout);
}

#[test]
fn bare_invocation_uses_source_stem_in_current_directory() {
    let temp = TestDir::new();
    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .current_dir(temp.path())
        .arg(hello_world())
        .output()
        .expect("run actionc");

    assert!(output.status.success());
    assert!(temp.path().join("hello-world.com").is_file());
}

#[test]
fn modes_match_their_profile_and_backend_presets() {
    let temp = TestDir::new();
    for (mode, profile, backend) in [
        ("compatibility", "legacy", "classic"),
        ("optimized", "modern", "classic"),
        ("mir6502", "modern", "mir6502"),
    ] {
        let object = temp.path().join(format!("{mode}.com"));
        let compiled = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--mode")
            .arg(mode)
            .arg("--output")
            .arg(&object)
            .arg(hello_world())
            .output()
            .expect("run actionc mode");
        assert!(
            compiled.status.success(),
            "actionc --mode {mode} failed\nstderr:\n{}",
            String::from_utf8_lossy(&compiled.stderr)
        );

        let emitted = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .arg("--profile")
            .arg(profile)
            .arg("--backend")
            .arg(backend)
            .arg("--emit-load")
            .arg(hello_world())
            .output()
            .expect("run matching actionc-emit configuration");
        assert!(emitted.status.success());
        assert_eq!(
            fs::read(&object).expect("read mode object"),
            emitted.stdout,
            "--mode {mode} selected the wrong compiler configuration"
        );
    }
}

#[test]
fn mode_rejects_low_level_profile_and_backend_overrides() {
    for option in ["--profile", "--backend"] {
        let value = if option == "--profile" {
            "modern"
        } else {
            "classic"
        };
        let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--mode")
            .arg("optimized")
            .arg(option)
            .arg(value)
            .arg(hello_world())
            .output()
            .expect("run actionc with conflicting configuration options");

        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("--mode cannot be combined with --profile or --backend")
        );
    }
}

#[test]
fn explicit_mode_overrides_source_configuration_annotations() {
    let temp = TestDir::new();
    let source = temp.path().join("annotated.act");
    let object = temp.path().join("compatibility.com");
    fs::write(
        &source,
        ";@actionc profile modern\n;@actionc backend mir6502\nPROC Main()\nRETURN\n",
    )
    .expect("write annotated source");

    let compiled = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg("--mode")
        .arg("compatibility")
        .arg("--output")
        .arg(&object)
        .arg(&source)
        .output()
        .expect("run actionc with explicit mode");
    assert!(compiled.status.success());

    let emitted = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg("--profile")
        .arg("legacy")
        .arg("--backend")
        .arg("classic")
        .arg("--emit-load")
        .arg(&source)
        .output()
        .expect("run explicit compatibility configuration");
    assert!(emitted.status.success());
    assert_eq!(fs::read(object).expect("read object"), emitted.stdout);
}

#[test]
fn source_configuration_annotations_drive_bare_actionc() {
    let temp = TestDir::new();
    let source = temp.path().join("annotated-default.act");
    let object = temp.path().join("annotated-default.com");
    fs::write(
        &source,
        ";@actionc profile modern\n;@actionc backend mir6502\nPROC Main()\nRETURN\n",
    )
    .expect("write annotated source");

    let compiled = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg(&source)
        .arg("--output")
        .arg(&object)
        .output()
        .expect("run actionc with source-selected defaults");
    assert!(
        compiled.status.success(),
        "actionc failed\nstderr:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    let emitted = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg("--profile")
        .arg("modern")
        .arg("--backend")
        .arg("mir6502")
        .arg("--emit-load")
        .arg(&source)
        .output()
        .expect("run matching actionc-emit configuration");
    assert!(emitted.status.success());
    assert_eq!(fs::read(object).expect("read object"), emitted.stdout);
}

#[test]
fn source_origin_and_explicit_origin_precedence_are_stable() {
    let temp = TestDir::new();
    let source = temp.path().join("origin.act");
    fs::write(
        &source,
        "SET $E=$4000\nSET $491=$4000\nPROC Main()\nRETURN\n",
    )
    .expect("write origin source");

    for mode in ["compatibility", "optimized", "mir6502"] {
        let implicit = temp.path().join(format!("{mode}-implicit.com"));
        let implicit_output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--mode")
            .arg(mode)
            .arg("--output")
            .arg(&implicit)
            .arg(&source)
            .output()
            .expect("compile with source origin");
        assert!(
            implicit_output.status.success(),
            "actionc --mode {mode} failed\nstderr:\n{}",
            String::from_utf8_lossy(&implicit_output.stderr)
        );
        assert_eq!(
            load_file_origin(&fs::read(&implicit).expect("read source-origin object")),
            0x4000,
            "--mode {mode} ignored the source origin"
        );

        let explicit = temp.path().join(format!("{mode}-explicit.com"));
        let explicit_output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--mode")
            .arg(mode)
            .arg("--origin")
            .arg("$3A00")
            .arg("--output")
            .arg(&explicit)
            .arg(&source)
            .output()
            .expect("compile with explicit origin");
        assert!(
            explicit_output.status.success(),
            "actionc --mode {mode} --origin failed\nstderr:\n{}",
            String::from_utf8_lossy(&explicit_output.stderr)
        );
        assert_eq!(
            load_file_origin(&fs::read(&explicit).expect("read explicit-origin object")),
            0x3A00,
            "--mode {mode} ignored the explicit origin"
        );

        let explicit_default = temp.path().join(format!("{mode}-explicit-default.com"));
        let explicit_default_output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--mode")
            .arg(mode)
            .arg("--origin")
            .arg("$3000")
            .arg("--output")
            .arg(&explicit_default)
            .arg(&source)
            .output()
            .expect("compile with explicit default origin");
        assert!(
            explicit_default_output.status.success(),
            "actionc --mode {mode} --origin $3000 failed\nstderr:\n{}",
            String::from_utf8_lossy(&explicit_default_output.stderr)
        );
        assert_eq!(
            load_file_origin(
                &fs::read(&explicit_default).expect("read explicit-default-origin object")
            ),
            0x3000,
            "--mode {mode} treated explicit $3000 as an implicit origin"
        );
    }
}

#[test]
fn invalid_profile_backend_combination_remains_a_configuration_error() {
    let temp = TestDir::new();
    let object = temp.path().join("invalid.com");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg("--backend")
        .arg("mir6502")
        .arg("--output")
        .arg(&object)
        .arg(hello_world())
        .output()
        .expect("run actionc with an invalid compiler configuration");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--backend mir6502 requires --profile modern")
    );
    assert!(!object.exists());
}

#[test]
fn advanced_classic_codegen_sources_survive_the_api_migration() {
    let temp = TestDir::new();
    let source = temp.path().join("minimal.act");
    fs::write(&source, "PROC Main()\nRETURN\n").expect("write advanced-codegen source");
    for codegen_source in ["ast", "semir", "semir-native"] {
        let object = temp.path().join(format!("{codegen_source}.com"));
        let compiled = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--profile")
            .arg("modern")
            .arg("--backend")
            .arg("classic")
            .arg("--codegen-source")
            .arg(codegen_source)
            .arg("--output")
            .arg(&object)
            .arg(&source)
            .output()
            .expect("run actionc with an advanced codegen source");
        assert!(
            compiled.status.success(),
            "actionc --codegen-source {codegen_source} failed\nstderr:\n{}",
            String::from_utf8_lossy(&compiled.stderr)
        );

        let emitted = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .arg("--profile")
            .arg("modern")
            .arg("--backend")
            .arg("classic")
            .arg("--codegen-source")
            .arg(codegen_source)
            .arg("--emit-load")
            .arg(&source)
            .output()
            .expect("run actionc-emit with the matching codegen source");
        assert!(emitted.status.success());
        assert_eq!(
            fs::read(&object).expect("read advanced-codegen object"),
            emitted.stdout,
            "actionc changed --codegen-source {codegen_source}"
        );
    }
}

#[test]
fn semantic_diagnostics_point_into_included_sources() {
    let temp = TestDir::new();
    let source = temp.path().join("main.act");
    let included = temp.path().join("lib.act");
    let object = temp.path().join("main.com");
    fs::write(&source, "INCLUDE \"lib.act\"\nPROC Main() RETURN\n").expect("write root source");
    fs::write(&included, "BYTE x\nPROC Broken()\n  missing=1\nRETURN\n")
        .expect("write included source");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg("--output")
        .arg(&object)
        .arg(&source)
        .output()
        .expect("compile source with included semantic error");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("{}:3:3", included.display())),
        "included diagnostic lost its source location:\n{stderr}"
    );
    assert!(stderr.contains("undefined symbol `missing`"));
    assert!(!object.exists());
}

#[test]
fn actionc_rejects_emit_options_with_migration_guidance() {
    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg("--emit-nir")
        .arg(hello_world())
        .output()
        .expect("run actionc");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--emit-* options belong to actionc-emit")
    );
}

#[test]
fn failed_compilation_does_not_create_outputs() {
    let temp = TestDir::new();
    let source = temp.path().join("broken.act");
    let object = temp.path().join("out/broken.com");
    let listing = temp.path().join("out/broken.lst");
    fs::write(&source, "PROC Main( RETURN").expect("write broken source");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg(&source)
        .arg("--output")
        .arg(&object)
        .arg("--listing")
        .arg(&listing)
        .output()
        .expect("run actionc");

    assert!(!output.status.success());
    assert!(!object.exists());
    assert!(!listing.exists());
}
