use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::codegen::{CodegenProfile, format_load_file, generate_profile_with_origin};
use actionc::compiler::{
    CompileErrorKind, CompileMode, CompileOptions, CompilerPhase, DiagnosticSite, compile_file,
};
use actionc::includes::load_program_with_expanded_source;

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
