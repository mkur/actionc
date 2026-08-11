use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::codegen::{
    CODE_ORIGIN, CodegenProfile, format_load_file, generate_profile_with_origin,
};
use actionc::compiler::{
    CompileErrorKind, CompileMode, CompileOptions, CompilerPhase, DiagnosticSite, compile_file,
};
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502::{self, Mir6502Config};
use actionc::nir;
use actionc::semantic::{analyze, ir};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("actionc-api-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).expect("create compiler API test directory");
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

fn write_source(temp: &TestDir, name: &str, source: &str) -> PathBuf {
    let path = temp.path().join(name);
    fs::write(&path, source).expect("write Action source");
    path
}

fn baseline_object(path: &Path, mode: CompileMode) -> Vec<u8> {
    let loaded = load_program_with_expanded_source(path).expect("load baseline program");
    let model = analyze(&loaded.program).expect("analyze baseline program");
    let output = match mode {
        CompileMode::Compatibility => {
            generate_profile_with_origin(&loaded.program, CODE_ORIGIN, CodegenProfile::Compat)
                .expect("compile compatibility baseline")
        }
        CompileMode::Optimized => {
            generate_profile_with_origin(&loaded.program, CODE_ORIGIN, CodegenProfile::Modern)
                .expect("compile optimized baseline")
        }
        CompileMode::Mir6502 => {
            let semir = ir::lower_program(&loaded.program, &model);
            let lowered = nir::lower_program(&semir);
            let optimized = nir::optimize_program(&lowered).expect("optimize NIR baseline");
            mir6502::generate_output_with_config(
                &optimized,
                CODE_ORIGIN,
                &Mir6502Config::optimized(),
            )
            .expect("compile MIR6502 baseline")
        }
    };
    format_load_file(&output)
}

#[test]
fn compatibility_api_matches_the_existing_classic_pipeline() {
    let source = hello_world();
    let compiled = compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Compatibility),
    )
    .expect("compile through reusable API");

    let loaded = load_program_with_expanded_source(&source).expect("load baseline program");
    let baseline = generate_profile_with_origin(&loaded.program, 0x3000, CodegenProfile::Compat)
        .expect("compile through existing classic pipeline");

    assert_eq!(compiled.object_bytes(), format_load_file(&baseline));
    assert_eq!(compiled.origin(), baseline.origin);
    assert_eq!(compiled.run_address(), baseline.run_address);
}

#[test]
fn compiled_program_formats_the_existing_source_listing() {
    let compiled = compile_file(
        hello_world(),
        &CompileOptions::for_mode(CompileMode::Compatibility),
    )
    .expect("compile source listing input");

    let listing = compiled.source_listing();
    assert!(listing.contains("; ===== PROC Main"));
    assert!(listing.contains("JSR $A46C"));
    assert!(listing.contains("| PrintE(\"Hello, world!\")"));
}

#[test]
fn all_public_modes_match_the_existing_pipelines() {
    let source = hello_world();
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile {mode:?} through reusable API: {error}"));

        assert_eq!(compiled.object_bytes(), baseline_object(&source, mode));
    }
}

#[test]
fn compatibility_api_honors_an_explicit_nondefault_origin() {
    let compiled = compile_file(
        hello_world(),
        &CompileOptions::for_mode(CompileMode::Compatibility).with_origin(0x3a00),
    )
    .expect("compile at an explicit origin");

    assert_eq!(compiled.origin(), 0x3a00);
    assert_eq!(&compiled.object_bytes()[2..4], &0x3a00u16.to_le_bytes());
}

#[test]
fn origin_precedence_is_consistent_in_all_modes() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "origin.act",
        "SET $E=$4000\nSET $491=$4000\nPROC Main()\nRETURN\n",
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let implicit = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile implicit-origin {mode:?}: {error}"));
        assert_eq!(implicit.origin(), 0x4000);

        let explicit = compile_file(
            &source,
            &CompileOptions::for_mode(mode).with_origin(CODE_ORIGIN),
        )
        .unwrap_or_else(|error| panic!("compile explicit-origin {mode:?}: {error}"));
        assert_eq!(explicit.origin(), CODE_ORIGIN);
    }
}

#[test]
fn source_annotations_fill_defaults_but_do_not_override_an_explicit_mode() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "annotated.act",
        ";@actionc profile modern\n;@actionc backend mir6502\nPROC Main()\nRETURN\n",
    );

    let implicit = compile_file(&source, &CompileOptions::default()).expect("compile annotations");
    let explicit_mir = compile_file(&source, &CompileOptions::for_mode(CompileMode::Mir6502))
        .expect("compile explicit MIR6502");
    assert_eq!(implicit.object_bytes(), explicit_mir.object_bytes());

    let explicit_compatibility = compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Compatibility),
    )
    .expect("compile explicit compatibility");
    assert_eq!(
        explicit_compatibility.object_bytes(),
        baseline_object(&source, CompileMode::Compatibility)
    );
}

#[test]
fn nir_preflight_failure_is_returned_as_a_source_diagnostic() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "retarget.act",
        "PROC A() RETURN PROC T() PROC Main() T=A T() RETURN",
    );

    let error = compile_file(&source, &CompileOptions::for_mode(CompileMode::Mir6502)).unwrap_err();

    assert_eq!(error.kind(), CompileErrorKind::Compilation);
    assert_eq!(error.diagnostics()[0].phase, CompilerPhase::Nir);
    assert!(
        error.diagnostics()[0]
            .message
            .contains("legacy routine-name retargeting")
    );
    assert!(matches!(
        &error.diagnostics()[0].site,
        DiagnosticSite::Source { path, .. } if path == &source
    ));
}

#[test]
fn include_diagnostics_keep_the_included_path() {
    let temp = TestDir::new();
    let source_dir = temp.path().join("source tree");
    fs::create_dir_all(&source_dir).expect("create spaced source directory");
    let source = source_dir.join("main.act");
    let included = source_dir.join("library.act");
    fs::write(&source, "INCLUDE \"library.act\"\nPROC Main() RETURN\n")
        .expect("write including source");
    fs::write(&included, "PROC Broken()\n  missing_symbol=1\nRETURN\n")
        .expect("write broken include");

    let error = compile_file(&source, &CompileOptions::default()).unwrap_err();

    assert_eq!(error.diagnostics()[0].phase, CompilerPhase::Semantic);
    assert!(matches!(
        &error.diagnostics()[0].site,
        DiagnosticSite::Source { path, .. } if path == &included
    ));
}

#[test]
fn repeated_and_concurrent_compilations_are_independent() {
    let source = hello_world();
    let first = compile_file(&source, &CompileOptions::for_mode(CompileMode::Optimized))
        .expect("first repeated compilation");
    let second = compile_file(&source, &CompileOptions::for_mode(CompileMode::Optimized))
        .expect("second repeated compilation");
    assert_eq!(first.object_bytes(), second.object_bytes());

    std::thread::scope(|scope| {
        let jobs = [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ]
        .map(|mode| {
            let source = source.clone();
            scope.spawn(move || {
                compile_file(&source, &CompileOptions::for_mode(mode))
                    .unwrap_or_else(|error| panic!("concurrent {mode:?} compilation: {error}"))
                    .object_bytes()
                    .to_vec()
            })
        });

        for (job, mode) in jobs.into_iter().zip([
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ]) {
            assert_eq!(
                job.join().expect("join compilation"),
                baseline_object(&source, mode)
            );
        }
    });
}

#[test]
fn invalid_source_returns_diagnostics_without_creating_outputs() {
    let temp = TestDir::new();
    let source = temp.path().join("broken.act");
    fs::write(&source, "PROC Main( RETURN").expect("write invalid source");

    let error = compile_file(&source, &CompileOptions::default()).unwrap_err();

    assert_eq!(error.kind(), CompileErrorKind::Compilation);
    assert_eq!(error.diagnostics()[0].phase, CompilerPhase::Frontend);
    assert!(error.diagnostics()[0].message.contains("expected"));
    assert!(matches!(
        &error.diagnostics()[0].site,
        DiagnosticSite::Source { path, .. } if path == &source
    ));
    assert_eq!(
        fs::read_dir(temp.path()).unwrap().count(),
        1,
        "the compiler API wrote an output file"
    );
}
