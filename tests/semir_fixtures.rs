use std::fs;
use std::path::{Path, PathBuf};

use actionc::includes::load_program_with_expanded_source;
use actionc::semantic::{analyze, ir};

#[test]
fn semir_fixtures_match_snapshots() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("semir");
    let mut sources = collect_action_fixtures(&fixture_dir);
    sources.sort();

    assert!(!sources.is_empty(), "expected SemIR fixtures");

    for source_path in sources {
        let expected_path = source_path.with_extension("semir");
        let actual = emit_semir(&source_path);
        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", expected_path.display()));

        assert_eq!(
            actual,
            expected,
            "SemIR fixture changed for {}\n\nrefresh with:\n  cargo run --bin actionc-emit -- --emit-semir {} > {}",
            source_path.display(),
            source_path.display(),
            expected_path.display()
        );
    }
}

fn emit_semir(path: &Path) -> String {
    let loaded = load_program_with_expanded_source(path)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", path.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", path.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    ir::format_program(&semir)
}

fn collect_action_fixtures(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        .map(|entry| entry.expect("read SemIR fixture entry").path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("act"))
        })
        .collect()
}
