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

fn standalone_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("runtime")
        .join(name)
}

fn standalone_runtime_sample() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("standalone")
        .join("standalone-runtime.act")
}

fn lexical_blocks_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("lexical_blocks.act")
}

fn lexical_declarations_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir")
        .join("lexical_declarations.act")
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
fn lexical_blocks_compile_across_backends_and_runtimes() {
    let temp = TestDir::new();
    for (fixture_name, fixture) in [
        ("scalar", lexical_blocks_fixture()),
        ("declarations", lexical_declarations_fixture()),
    ] {
        for (mode, runtime) in [
            ("optimized", "cart"),
            ("optimized", "standalone"),
            ("mir6502", "cart"),
            ("mir6502", "standalone"),
        ] {
            let object = temp
                .path()
                .join(format!("lexical-{fixture_name}-{mode}-{runtime}.com"));
            let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
                .arg("--mode")
                .arg(mode)
                .arg("--runtime")
                .arg(runtime)
                .arg("--output")
                .arg(&object)
                .arg(&fixture)
                .output()
                .unwrap_or_else(|error| panic!("run {fixture_name} {mode}/{runtime}: {error}"));
            assert!(
                output.status.success(),
                "{fixture_name} {mode}/{runtime} failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(fs::metadata(object).unwrap().len() > 0);
        }
    }
}

#[test]
fn lexical_block_listings_use_readable_scope_paths() {
    let temp = TestDir::new();
    for mode in ["optimized", "mir6502"] {
        let object = temp.path().join(format!("lexical-{mode}.com"));
        let listing = temp.path().join(format!("lexical-{mode}.lst"));
        let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--mode")
            .arg(mode)
            .arg("--runtime")
            .arg("cart")
            .arg("--output")
            .arg(&object)
            .arg("--listing")
            .arg(&listing)
            .arg(lexical_blocks_fixture())
            .output()
            .unwrap_or_else(|error| panic!("run lexical listing {mode}: {error}"));
        assert!(
            output.status.success(),
            "lexical listing {mode} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let listing = fs::read_to_string(listing).expect("read lexical block listing");
        assert!(
            listing.contains("local_main_block0_block1_value"),
            "missing nested lexical path in {mode} listing:\n{listing}"
        );
        assert!(
            !listing.contains("__lex"),
            "internal projection name leaked into {mode} listing:\n{listing}"
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
    fs::write(
        &root,
        "MODULE APP\nUSE LIB.VALUE\nPROC Main() VALUE.value=1 [<VALUE.value >VALUE.value] RETURN\nENDMODULE\n",
    )
    .expect("write root module");
    fs::write(
        second.join("lib/value.act"),
        "MODULE LIB.VALUE\nPUBLIC BYTE value\nENDMODULE\n",
    )
    .expect("write imported module");

    for mode in ["compatibility", "optimized", "mir6502"] {
        let object = temp.path().join(format!("app-{mode}.com"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_actionc"));
        command
            .arg("--module-path")
            .arg(&first)
            .arg(format!("--module-path={}", second.display()))
            .arg("--mode")
            .arg(mode)
            .arg("-o")
            .arg(&object);
        let output = command.arg(&root).output().expect("run module CLI");
        assert!(
            output.status.success(),
            "{mode} named-module compilation failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(fs::metadata(object).expect("module object metadata").len() > 0);
    }

    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .arg("--module-path")
        .arg(&first)
        .arg(format!("--module-path={}", second.display()))
        .arg("--emit-semir")
        .arg(&root)
        .output()
        .expect("emit named-module SemIR");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("LIB.VALUE.value"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn named_module_semantic_diagnostics_point_into_used_sources() {
    let temp = TestDir::new();
    let root = temp.path().join("main.act");
    let library = temp.path().join("lib/bad.act");
    fs::create_dir_all(library.parent().unwrap()).expect("create module directory");
    fs::write(&root, "MODULE APP\nUSE LIB.BAD\nENDMODULE\n").expect("write root module");
    fs::write(
        &library,
        "MODULE LIB.BAD\nPUBLIC PROC Broken() Missing=1 RETURN\nENDMODULE\n",
    )
    .expect("write imported module");

    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg(&root)
        .output()
        .expect("run compiler");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("undefined symbol `Missing`"), "{stderr}");
    assert!(stderr.contains("bad.act"), "{stderr}");
    assert!(
        stderr.contains("PUBLIC PROC Broken() Missing=1 RETURN"),
        "{stderr}"
    );
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
fn explicit_cart_runtime_matches_the_default_in_every_mode() {
    let temp = TestDir::new();
    for mode in ["compatibility", "optimized", "mir6502"] {
        let implicit = temp.path().join(format!("{mode}-implicit.com"));
        let explicit = temp.path().join(format!("{mode}-cart.com"));
        for (output, runtime) in [(&implicit, None), (&explicit, Some("cart"))] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_actionc"));
            command.arg("--mode").arg(mode);
            if let Some(runtime) = runtime {
                command.arg("--runtime").arg(runtime);
            }
            let result = command
                .arg("--output")
                .arg(output)
                .arg(hello_world())
                .output()
                .expect("compile runtime-equivalence fixture");
            assert!(
                result.status.success(),
                "actionc --mode {mode} failed\nstderr:\n{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        assert_eq!(
            fs::read(&implicit).expect("read implicit-runtime object"),
            fs::read(&explicit).expect("read explicit-cart object"),
            "explicit cart changed {mode} output"
        );
    }
}

#[test]
fn classic_standalone_is_supported_without_switching_backend() {
    let temp = TestDir::new();
    for mode in ["compatibility", "optimized"] {
        let object = temp.path().join(format!("classic-{mode}.com"));
        let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .args(["--mode", mode, "--runtime", "standalone", "--output"])
            .arg(&object)
            .arg(standalone_fixture("standalone_arithmetic.act"))
            .output()
            .expect("compile classic standalone configuration");
        assert!(
            output.status.success(),
            "classic standalone {mode} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!fs::read(&object).expect("read classic object").is_empty());
    }
}

#[test]
fn classic_standalone_appends_runtime_after_source_controlled_layout() {
    let temp = TestDir::new();
    let source = temp.path().join("classic-fixed-layout.act");
    fs::write(
        &source,
        "SET $E=$2C00\n\
         SET $491=$2C00\n\
         BYTE ARRAY buffer\n\
         BYTE ARRAY allocbuf($800)=$2000\n\
         CARD left, right, result\n\
         PROC Four(BYTE a,b,c,d) RETURN\n\
         PROC Main()\n\
           result=left*right\n\
           result=left/right\n\
           result=left MOD right\n\
           Four(1,2,3,4)\n\
         RETURN\n\
         SET buffer=*\n",
    )
    .expect("write fixed-layout classic standalone source");

    for mode in ["compatibility", "optimized"] {
        let object = temp.path().join(format!("classic-fixed-layout-{mode}.com"));
        let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .args(["--mode", mode, "--runtime", "standalone", "--output"])
            .arg(&object)
            .arg(&source)
            .output()
            .expect("compile fixed-layout classic standalone source");
        assert!(
            output.status.success(),
            "classic standalone {mode} placed runtime before application layout: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = fs::read(&object).expect("read fixed-layout classic standalone object");
        assert_eq!(load_file_origin(&bytes), 0x2c00);
        let segment_end = u16::from_le_bytes([bytes[4], bytes[5]]);
        let buffer = u16::from_le_bytes([bytes[6], bytes[7]]);
        assert!(
            buffer >= segment_end,
            "classic standalone {mode} patched `SET buffer=*` before its runtime closure: ${buffer:04X} < ${segment_end:04X}"
        );
    }
}

#[test]
fn classic_standalone_links_the_same_sargs_source_closure() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/standalone_sargs.act");
    for profile in ["legacy", "modern"] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args([
                "--profile",
                profile,
                "--backend",
                "classic",
                "--runtime",
                "standalone",
                "--emit-map",
            ])
            .arg(&fixture)
            .output()
            .expect("emit classic standalone SArgs map");
        assert!(
            output.status.success(),
            "classic standalone SArgs ({profile}) failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let map = String::from_utf8_lossy(&output.stdout);
        for selected in ["ERROR", "BREAK", "SARGS"] {
            assert!(
                map.contains(&format!("M_ACTION_RUNTIME_SYSLIB_{selected}_")),
                "missing {selected}: {map}"
            );
        }
        for unused in ["LSHIFT", "RSHIFT", "MULTI", "DIVI", "REMI"] {
            assert!(
                !map.contains(&format!("M_ACTION_RUNTIME_SYSLIB_{unused}_")),
                "unexpected {unused}: {map}"
            );
        }
    }
}

#[test]
fn standalone_source_listings_omit_library_source_annotations() {
    for backend in ["classic", "mir6502"] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args([
                "--profile",
                "modern",
                "--backend",
                backend,
                "--runtime",
                "standalone",
                "--emit-source-listing",
            ])
            .arg(standalone_runtime_sample())
            .output()
            .expect("emit standalone source listing");
        assert!(
            output.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let listing = String::from_utf8_lossy(&output.stdout);
        let mut in_runtime_routine = false;
        let mut runtime_routines = 0;
        for line in listing.lines() {
            if line.starts_with("; ===== PROC ") {
                in_runtime_routine =
                    line.contains("M_ACTION_RUNTIME_") || line.contains("ACTION.RUNTIME.");
                runtime_routines += usize::from(in_runtime_routine);
            }
            if in_runtime_routine {
                assert!(
                    !line.contains(" | "),
                    "{backend} runtime source annotation was not suppressed: {line}"
                );
            }
            if line.starts_with("; ===== END PROC ") {
                in_runtime_routine = false;
            }
        }

        assert!(
            runtime_routines > 0,
            "{backend}: no runtime assembler found"
        );
        assert!(listing.contains("STA.Z $A0"), "{backend}: {listing}");
        assert!(!listing.contains("PUBLIC EXTERNAL PROC Poke"));
        assert!(
            listing.contains("proc_syslib_sargs:"),
            "{backend}: {listing}"
        );
        assert!(
            listing.contains("proc_resident_print:"),
            "{backend}: {listing}"
        );
        assert!(!listing.contains("proc_m_action_runtime_"));
        assert!(!listing.contains("proc_action_runtime_"));
        assert!(
            listing.contains("loc_resident_in_1"),
            "{backend}: {listing}"
        );
        assert!(!listing.contains("loc_m_action_runtime_"));
        assert!(!listing.contains("loc_action_runtime_"));
        assert!(listing.contains("param_syslib_error_err"));
        assert!(!listing.contains("param_m_action_runtime_"));
        assert!(!listing.contains("param_action_runtime_"));
        if backend == "classic" {
            assert!(listing.contains("| PROC SaveResults(CARD p,q,r,s)"));
        }
    }
}

#[test]
fn classic_standalone_sys_binding_is_selective() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/standalone_sys_memory.act");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "classic",
            "--runtime",
            "standalone",
            "--emit-map",
        ])
        .arg(&fixture)
        .output()
        .expect("emit classic standalone SYS map");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map = String::from_utf8_lossy(&output.stdout);
    assert!(map.contains("runtime-binding SYS.Zero"));
    assert!(map.contains("M_ACTION_RUNTIME_RESIDENT_ZERO_"));
    assert!(map.contains("M_ACTION_RUNTIME_RESIDENT_SETBLOCK_"));
    assert!(!map.contains("M_ACTION_RUNTIME_RESIDENT_MOVEBLOCK_"));
}

#[test]
fn mir6502_standalone_without_helpers_adds_no_runtime_routines() {
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--runtime",
            "standalone",
            "--emit-map",
        ])
        .arg(standalone_fixture("standalone_minimal.act"))
        .output()
        .expect("emit a standalone map for a helper-free program");
    assert!(
        output.status.success(),
        "standalone helper-free compilation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map = String::from_utf8_lossy(&output.stdout);
    assert!(map.starts_with("runtime standalone\n"));
    assert!(!map.contains("ACTION.RUNTIME.SYSLIB"));
    assert!(!map.contains("runtime-binding"));
}

#[test]
fn mir6502_standalone_links_only_the_sargs_dependency_group() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/standalone_sargs.act");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--runtime",
            "standalone",
            "--emit-map",
        ])
        .arg(&fixture)
        .output()
        .expect("emit standalone SArgs map");
    assert!(
        output.status.success(),
        "standalone SArgs compilation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map = String::from_utf8_lossy(&output.stdout);
    assert!(map.contains("runtime-binding SArgs"));
    assert!(map.contains("ACTION.RUNTIME.SYSLIB::SArgs"));
    assert!(map.contains("ACTION.RUNTIME.SYSLIB::Break"));
    assert!(map.contains("ACTION.RUNTIME.SYSLIB::Error"));
    for unused in ["LShift", "RShift", "MultI", "DivI", "RemI"] {
        assert!(!map.contains(&format!("ACTION.RUNTIME.SYSLIB::{unused}")));
    }
}

#[test]
fn standalone_sys_break_links_only_the_exception_group() {
    let temp = TestDir::new();
    let source = temp.path().join("sys-break.act");
    fs::write(
        &source,
        "PROC Main() Error(1) Error(1,2,3) Break() RETURN\n",
    )
    .expect("write SYS.Break source");

    for backend in ["classic", "mir6502"] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args([
                "--profile",
                "modern",
                "--backend",
                backend,
                "--runtime",
                "standalone",
                "--emit-map",
            ])
            .arg(&source)
            .output()
            .expect("emit standalone SYS.Break map");
        assert!(
            output.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let map = String::from_utf8_lossy(&output.stdout).to_ascii_uppercase();
        if backend == "classic" {
            assert!(
                map.contains("RUNTIME-BINDING SYS.ERROR"),
                "{backend}: {map}"
            );
            assert!(
                map.contains("RUNTIME-BINDING SYS.BREAK"),
                "{backend}: {map}"
            );
        }
        assert!(
            map.contains("RESIDENT::BREAK") || map.contains("RESIDENT_BREAK_"),
            "{backend}: {map}"
        );
        assert!(
            map.contains("RESIDENT::ERROR") || map.contains("RESIDENT_ERROR_"),
            "{backend}: {map}"
        );
        for unused in ["SARGS", "LSHIFT", "RSHIFT", "MULTI", "DIVI", "REMI"] {
            assert!(
                !map.contains(unused),
                "{backend}, unexpected {unused}: {map}"
            );
        }
    }
}

#[test]
fn mir6502_sys_memory_binding_is_runtime_selected_and_selective() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/standalone_sys_memory.act");
    let standalone = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--runtime",
            "standalone",
            "--emit-map",
        ])
        .arg(&fixture)
        .output()
        .expect("emit standalone SYS map");
    assert!(
        standalone.status.success(),
        "{}",
        String::from_utf8_lossy(&standalone.stderr)
    );
    let map = String::from_utf8_lossy(&standalone.stdout);
    assert_eq!(map.matches("ACTION.RUNTIME.RESIDENT::Zero").count(), 3);
    assert_eq!(map.matches("ACTION.RUNTIME.RESIDENT::SetBlock").count(), 3);
    assert!(!map.contains("ACTION.RUNTIME.RESIDENT::MoveBlock"));

    let cart = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--runtime",
            "cart",
            "--emit-materialized-mir6502",
        ])
        .arg(&fixture)
        .output()
        .expect("emit cart SYS MIR");
    assert!(
        cart.status.success(),
        "{}",
        String::from_utf8_lossy(&cart.stderr)
    );
    let mir = String::from_utf8_lossy(&cart.stdout);
    assert!(mir.contains("Zero@$A78A"), "{mir}");
    assert!(!mir.contains("ACTION.RUNTIME.SYSBLK"));
}

#[test]
fn sys_string_bindings_work_in_every_backend_and_runtime_pair() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/standalone_sys_strings.act");
    let temp = TestDir::new();

    for backend in ["classic", "mir6502"] {
        for runtime in ["cart", "standalone"] {
            let object = temp.path().join(format!("strings-{backend}-{runtime}.com"));
            let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
                .args([
                    "--profile",
                    "modern",
                    "--backend",
                    backend,
                    "--runtime",
                    runtime,
                    "-o",
                ])
                .arg(&object)
                .arg(&fixture)
                .output()
                .expect("compile SYS string fixture");
            assert!(
                output.status.success(),
                "{backend} + {runtime}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(fs::metadata(object).expect("string fixture object").len() > 0);
        }
    }

    for backend in ["classic", "mir6502"] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args([
                "--profile",
                "modern",
                "--backend",
                backend,
                "--runtime",
                "standalone",
                "--emit-map",
            ])
            .arg(&fixture)
            .output()
            .expect("emit standalone SYS string map");
        assert!(
            output.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let map = String::from_utf8_lossy(&output.stdout);
        for routine in [
            "SCompare", "SCopy", "SCopyS", "SAssign", "StrB", "StrC", "StrI",
        ] {
            let expected = if backend == "classic" {
                format!("runtime-binding SYS.{routine}")
            } else {
                format!("ACTION.RUNTIME.RESIDENT::{}", routine.to_ascii_uppercase())
            };
            assert!(
                map.to_ascii_uppercase()
                    .contains(&expected.to_ascii_uppercase()),
                "{backend}, missing {routine}: {map}"
            );
        }
        let implementation_unit = if backend == "classic" {
            "SYSSTR"
        } else {
            "ACTION.RUNTIME.RESIDENT"
        };
        assert!(
            map.to_ascii_uppercase().contains(implementation_unit),
            "{backend}: {map}"
        );
        if backend == "classic" {
            assert!(map.contains("SYSIO.ACT"), "{backend}: {map}");
        }
        assert!(!map.contains("ACTION.RUNTIME.SYSBLK"), "{backend}: {map}");
    }
}

#[test]
fn mir6502_sys_routine_addresses_follow_the_selected_runtime() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/standalone_sys_address.act");
    let cart = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--runtime",
            "cart",
            "--emit-code",
        ])
        .arg(&fixture)
        .output()
        .expect("emit cart SYS routine address");
    assert!(
        cart.status.success(),
        "{}",
        String::from_utf8_lossy(&cart.stderr)
    );
    assert!(
        String::from_utf8_lossy(&cart.stdout).starts_with("8A A7 "),
        "{}",
        String::from_utf8_lossy(&cart.stdout)
    );

    let standalone = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--runtime",
            "standalone",
            "--emit-map",
        ])
        .arg(&fixture)
        .output()
        .expect("emit standalone SYS routine address map");
    assert!(
        standalone.status.success(),
        "{}",
        String::from_utf8_lossy(&standalone.stderr)
    );
    assert!(String::from_utf8_lossy(&standalone.stdout).contains("ACTION.RUNTIME.RESIDENT::Zero"));
}

#[test]
fn unused_sys_use_adds_no_runtime_code() {
    let temp = TestDir::new();
    let source = temp.path().join("unused-sys.act");
    fs::write(
        &source,
        "MODULE APP\nUSE SYS\nPROC Main() RETURN\nENDMODULE\n",
    )
    .expect("write unused SYS source");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--runtime",
            "standalone",
            "--emit-map",
        ])
        .arg(&source)
        .output()
        .expect("emit unused SYS map");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map = String::from_utf8_lossy(&output.stdout);
    assert!(!map.contains("ACTION.RUNTIME.SYSBLK"));
}

#[test]
fn printh_is_bound_in_both_runtimes_and_backends() {
    let temp = TestDir::new();
    let source = temp.path().join("resident-print.act");
    fs::write(&source, "PROC Main() PrintH($1234) RETURN\n").expect("write resident-call source");

    for runtime in ["cart", "standalone"] {
        for backend in ["classic", "mir6502"] {
            let output_kind = if runtime == "cart" {
                "--emit-listing"
            } else {
                "--emit-map"
            };
            let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
                .args([
                    "--profile",
                    "modern",
                    "--backend",
                    backend,
                    "--runtime",
                    runtime,
                    output_kind,
                ])
                .arg(&source)
                .output()
                .expect("emit bound PrintH call");
            assert!(
                output.status.success(),
                "{runtime}/{backend}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let map = String::from_utf8_lossy(&output.stdout);
            if runtime == "cart" {
                assert!(map.contains("$B8C2"), "{runtime}/{backend}: {map}");
            } else {
                assert!(
                    map.to_ascii_uppercase().contains("RESIDENT::PRINTH")
                        || map.to_ascii_uppercase().contains("RESIDENT_PRINTH_"),
                    "{runtime}/{backend}: {map}"
                );
            }
        }
    }
}

#[test]
fn referenced_external_without_selected_binding_fails_closed() {
    let temp = TestDir::new();
    let source = temp.path().join("missing-binding.act");
    fs::write(
        &source,
        "MODULE APP\nPUBLIC EXTERNAL PROC Missing()\n\
         PROC Main() Missing() RETURN\nENDMODULE\n",
    )
    .expect("write missing binding source");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .args(["--mode", "mir6502", "--runtime", "standalone"])
        .arg(&source)
        .output()
        .expect("compile missing binding source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("E-BINDING-MISSING-FOR-RUNTIME"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn standalone_sargs_local_override_suppresses_the_embedded_default() {
    let temp = TestDir::new();
    let source = temp.path().join("local-sargs.act");
    fs::write(
        &source,
        "PROC LocalSArgs=*() [$60]\nSET $4EE=LocalSArgs\nPROC Four(BYTE a,b,c,d) RETURN\nPROC Main() Four(1,2,3,4) RETURN\n",
    )
    .expect("write local SArgs override source");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--runtime",
            "standalone",
            "--emit-map",
        ])
        .arg(&source)
        .output()
        .expect("emit local SArgs override map");
    assert!(
        output.status.success(),
        "local SArgs override failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map = String::from_utf8_lossy(&output.stdout);
    assert!(map.contains("runtime-binding SArgs"));
    assert!(map.contains("LocalSArgs"));
    assert!(map.contains("suppressed-default=ACTION.RUNTIME.SYSLIB::SArgs"));
    assert!(!map.contains("$A0F5"));
}

#[test]
fn classic_standalone_honors_a_local_sargs_override() {
    let temp = TestDir::new();
    let source = temp.path().join("classic-local-sargs.act");
    fs::write(
        &source,
        "PROC LocalSArgs=*() [$60]\nSET $4EE=LocalSArgs\nPROC Four(BYTE a,b,c,d) RETURN\nPROC Main() Four(1,2,3,4) RETURN\n",
    )
    .expect("write classic local SArgs override source");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
        .args([
            "--profile",
            "modern",
            "--backend",
            "classic",
            "--runtime",
            "standalone",
            "--emit-map",
        ])
        .arg(&source)
        .output()
        .expect("emit classic local SArgs override map");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let map = String::from_utf8_lossy(&output.stdout);
    assert!(map.contains("runtime-binding SArgs"));
    assert!(map.contains("LocalSArgs"));
    assert!(map.contains("suppressed-default=ACTION.RUNTIME.SYSLIB::SArgs"));
    assert!(!map.contains("M_ACTION_RUNTIME_SYSLIB_SARGS_"));
}

#[test]
fn standalone_rejects_absolute_sargs_override() {
    let temp = TestDir::new();
    let source = temp.path().join("absolute-sargs.act");
    fs::write(
        &source,
        "SET $4EE=$A0F5\nPROC Four(BYTE a,b,c,d) RETURN\nPROC Main() Four(1,2,3,4) RETURN\n",
    )
    .expect("write absolute SArgs override source");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .args(["--mode", "mir6502", "--runtime", "standalone"])
        .arg(&source)
        .output()
        .expect("compile absolute SArgs override");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("standalone runtime rejects absolute override $A0F5 for `SArgs`")
    );
}

#[test]
fn classic_standalone_rejects_an_absolute_sargs_override() {
    let temp = TestDir::new();
    let source = temp.path().join("classic-absolute-sargs.act");
    fs::write(
        &source,
        "SET $4EE=$A0F5\nPROC Four(BYTE a,b,c,d) RETURN\nPROC Main() Four(1,2,3,4) RETURN\n",
    )
    .expect("write classic absolute SArgs override source");
    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .args(["--mode", "optimized", "--runtime", "standalone"])
        .arg(&source)
        .output()
        .expect("compile classic absolute SArgs override");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("standalone runtime rejects absolute override $A0F5 for `SArgs`")
    );
}

#[test]
fn standalone_arithmetic_selects_deterministic_minimal_dependency_closures() {
    let cases: [(&str, &str, &[&str], &[&str]); 5] = [
        (
            "lsh",
            "LSH",
            &["LShift"],
            &["RShift", "MultI", "DivI", "RemI"],
        ),
        (
            "rsh",
            "RSH",
            &["RShift"],
            &["LShift", "MultI", "DivI", "RemI"],
        ),
        (
            "mul",
            "*",
            &["SetSign", "SS1", "SMOps", "MultB", "MultI"],
            &["LShift", "RShift", "DivI", "RemI"],
        ),
        (
            "div",
            "/",
            &["SetSign", "SS1", "SMOps", "DivI"],
            &["LShift", "RShift", "MultB", "MultI", "RemI"],
        ),
        (
            "rem",
            "MOD",
            &["SetSign", "SS1", "SMOps", "DivI", "RemI"],
            &["LShift", "RShift", "MultB", "MultI"],
        ),
    ];
    let temp = TestDir::new();
    for (label, operator, expected, absent) in cases {
        let source = temp.path().join(format!("{label}.act"));
        fs::write(
            &source,
            format!(
                "CARD result=$600\nCARD left=$610\nCARD right=$612\nPROC Main() left=17 right=3 result=left {operator} right result=left {operator} right RETURN\n"
            ),
        )
        .expect("write standalone arithmetic selection source");
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args([
                "--profile",
                "modern",
                "--backend",
                "mir6502",
                "--runtime",
                "standalone",
                "--emit-map",
            ])
            .arg(&source)
            .output()
            .expect("emit standalone arithmetic map");
        assert!(
            output.status.success(),
            "standalone {label} compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let map = String::from_utf8_lossy(&output.stdout);
        for routine in expected {
            let address_rows = map
                .lines()
                .filter(|line| {
                    line.starts_with('$')
                        && line.ends_with(&format!("ACTION.RUNTIME.SYSLIB::{routine}"))
                })
                .count();
            assert_eq!(address_rows, 1, "{label} selected {routine} more than once");
        }
        for routine in absent {
            assert!(
                !map.contains(&format!("ACTION.RUNTIME.SYSLIB::{routine}")),
                "{label} unexpectedly selected {routine}"
            );
        }
        assert!(!map.contains("copy_right"));
    }
}

#[test]
fn runtime_option_uses_the_space_separated_cli_form() {
    let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .arg("--runtime=cart")
        .arg(hello_world())
        .output()
        .expect("run invalid equals-form runtime option");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unexpected argument: --runtime=cart")
    );
}

#[test]
fn advanced_classic_codegen_sources_survive_the_api_migration() {
    let temp = TestDir::new();
    let source = temp.path().join("minimal.act");
    fs::write(&source, "PROC Main()\nRETURN\n").expect("write advanced-codegen source");
    for codegen_source in ["ast", "semir"] {
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
fn removed_semir_native_selectors_fail_as_invalid_values() {
    for codegen_source in [
        "native",
        "semir-native",
        "sem-ir-native",
        "native-ir",
        "modern-ir",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--codegen-source")
            .arg(codegen_source)
            .arg(hello_world())
            .output()
            .expect("run actionc with a removed codegen source");
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains(&format!("invalid codegen source: {codegen_source}")),
            "unexpected diagnostic for {codegen_source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for profile in ["semir-native", "sem-ir-native", "native-ir", "modern-ir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc"))
            .arg("--profile")
            .arg(profile)
            .arg(hello_world())
            .output()
            .expect("run actionc with a removed profile alias");
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains(&format!("invalid codegen profile: {profile}")),
            "unexpected diagnostic for {profile}: {}",
            String::from_utf8_lossy(&output.stderr)
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
