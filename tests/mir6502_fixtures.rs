use std::fs;
use std::path::{Path, PathBuf};

use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn mir6502_fixtures_match_snapshots() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("mir6502");
    let mut sources = collect_action_fixtures(&fixture_dir);
    sources.sort();

    assert!(!sources.is_empty(), "expected MIR6502 fixtures");

    for source_path in sources {
        let expected_path = source_path.with_extension("mir6502");
        let actual = emit_mir6502(&source_path);
        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", expected_path.display()));

        assert_eq!(
            actual,
            expected,
            "MIR6502 fixture changed for {}\n\nrefresh with:\n  cargo run --bin actionc-emit -- --emit-mir6502 {} > {}",
            source_path.display(),
            source_path.display(),
            expected_path.display()
        );
    }
}

fn emit_mir6502(path: &Path) -> String {
    let loaded = load_program_with_expanded_source(path)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", path.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", path.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let nir_program = nir::optimize_program(&nir::lower_program(&semir))
        .unwrap_or_else(|err| panic!("optimize NIR for {}: {err:?}", path.display()));
    let mir = mir6502::lower_program(&nir_program)
        .unwrap_or_else(|err| panic!("lower MIR6502 for {}: {err:?}", path.display()));
    mir6502::verify_program(&mir, mir6502::MirPhase::PreMaterialization)
        .unwrap_or_else(|err| panic!("verify MIR6502 for {}: {err:?}", path.display()));
    mir6502::format_program(&mir)
}

fn collect_action_fixtures(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        .map(|entry| entry.expect("read MIR6502 fixture entry").path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("act"))
        })
        .collect()
}
