use std::fs;
use std::path::Path;

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::includes::{ModuleLoadOptions, load_compilation};
use actionc::nir;
use actionc::semantic::{SemanticOptions, analyze_compilation_with_options, ir};

#[test]
fn parses_all_sample_programs() {
    let samples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
    let entries = collect_sample_files(&samples_dir, &["act", "lib"]);
    let mut sample_count = 0usize;

    for path in entries {
        if is_known_action_macro_expansion_sample(&path)
            || is_support_module_without_entry_point(&path, &samples_dir)
        {
            continue;
        }
        check_sample(&path, &samples_dir);
        sample_count += 1;
    }

    assert!(sample_count > 0, "expected at least one Action! sample");
}

#[test]
fn unqualified_standalone_runtime_sample_compiles_with_both_backends() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("standalone")
        .join("standalone-runtime.act");

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let options = CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone);
        compile_file(&source, &options).unwrap_or_else(|error| {
            panic!("compile standalone runtime sample in {mode:?} mode: {error}")
        });
    }
}

#[test]
fn native_real_tutorial_sample_compiles_with_both_backends() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("real-basics.act");

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let options = CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone);
        compile_file(&source, &options).unwrap_or_else(|error| {
            panic!("compile native REAL tutorial sample in {mode:?} mode: {error}")
        });
    }
}

fn is_support_module_without_entry_point(path: &Path, samples_dir: &Path) -> bool {
    path.strip_prefix(samples_dir)
        .is_ok_and(|relative| relative == Path::new("modules/project/demo/color.act"))
}

fn check_sample(path: &Path, samples_dir: &Path) {
    if is_action_source(path) {
        let raytracer_root = samples_dir.join("vbxe/raytracer");
        let benchmark_root = samples_dir.join("benchmarks");
        let options = if path.starts_with(&benchmark_root) {
            ModuleLoadOptions {
                project_root: Some(benchmark_root),
                ..ModuleLoadOptions::default()
            }
        } else if path.starts_with(&raytracer_root) {
            ModuleLoadOptions {
                module_paths: vec![samples_dir.join("vbxe")],
                ..ModuleLoadOptions::default()
            }
        } else {
            ModuleLoadOptions::default()
        };
        let compilation = load_compilation(path, &options)
            .unwrap_or_else(|err| panic!("load compilation {}: {err:?}", path.display()));
        let model = analyze_compilation_with_options(&compilation, SemanticOptions::modern())
            .unwrap_or_else(|err| panic!("analyze {}: {err:?}", path.display()));
        let semir = ir::lower_compilation(&compilation, &model);
        let lowered = nir::lower_program(&semir);
        nir::verify_program(&lowered)
            .unwrap_or_else(|err| panic!("verify lowered NIR for {}: {err:?}", path.display()));
        nir::optimize_program(&lowered)
            .unwrap_or_else(|err| panic!("verify optimized NIR for {}: {err:?}", path.display()));
    }
}

fn is_action_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("act") || extension.eq_ignore_ascii_case("lib")
        })
}

fn collect_sample_files(dir: &Path, extensions: &[&str]) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    collect_sample_files_into(dir, extensions, &mut files);
    files.sort();
    files
}

fn collect_sample_files_into(dir: &Path, extensions: &[&str], files: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display())) {
        let path = entry.expect("read sample entry").path();
        if path.is_dir() {
            collect_sample_files_into(&path, extensions, files);
            continue;
        }

        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if extension
            .as_deref()
            .is_some_and(|extension| extensions.contains(&extension))
        {
            files.push(path);
        }
    }
}

fn is_known_action_macro_expansion_sample(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("KALROM.ACT" | "ST.ACT")
    )
}
