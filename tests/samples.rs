use std::fs;
use std::path::Path;

use actionc::includes::load_program_with_includes;
use actionc::semantic::analyze;

#[test]
fn parses_all_sample_programs() {
    let samples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
    let entries = collect_sample_files(&samples_dir, &["act", "lib"]);
    let mut sample_count = 0usize;

    for path in entries {
        if is_known_action_macro_expansion_sample(&path) {
            continue;
        }
        check_sample(&path);
        sample_count += 1;
    }

    assert!(sample_count > 0, "expected at least one Action! sample");
}

fn check_sample(path: &Path) {
    if is_action_source(path) {
        let program = load_program_with_includes(path)
            .unwrap_or_else(|err| panic!("load {} with includes: {err:?}", path.display()));
        analyze(&program).unwrap_or_else(|err| panic!("analyze {}: {err:?}", path.display()));
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
