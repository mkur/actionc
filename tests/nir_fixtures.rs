use std::fs;
use std::path::{Path, PathBuf};

use actionc::includes::load_program_with_expanded_source;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn nir_fixtures_match_snapshots() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("nir");
    let mut sources = collect_action_fixtures(&fixture_dir);
    sources.sort();

    assert!(!sources.is_empty(), "expected NIR fixtures");

    for source_path in sources {
        let expected_path = source_path.with_extension("nir");
        let actual = emit_nir(&source_path);
        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", expected_path.display()));

        assert_eq!(
            actual,
            expected,
            "NIR fixture changed for {}\n\nrefresh with:\n  cargo run --bin actionc-emit -- --emit-nir {} > {}",
            source_path.display(),
            source_path.display(),
            expected_path.display()
        );
    }
}

fn emit_nir(path: &Path) -> String {
    let loaded = load_program_with_expanded_source(path)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", path.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", path.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let program = nir::lower_program(&semir);
    nir::verify_program(&program)
        .unwrap_or_else(|err| panic!("verify NIR for {}: {err:?}", path.display()));
    nir::format_program(&program)
}

fn collect_action_fixtures(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        .map(|entry| entry.expect("read NIR fixture entry").path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("act"))
        })
        .collect()
}
