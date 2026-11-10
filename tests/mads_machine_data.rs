use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TestDir(PathBuf);
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_symbols_resolve(listing: &str) {
    let statements: Vec<_> = listing
        .lines()
        .map(|line| line.split(';').next().unwrap().trim())
        .collect();
    let definitions: BTreeSet<_> = statements
        .iter()
        .filter_map(|line| {
            line.strip_suffix(':')
                .or_else(|| line.split_once('=').map(|(name, _)| name.trim()))
        })
        .collect();
    for line in &statements {
        for symbol in line.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_') {
            if ["loc_", "data_generated_", "proc_"]
                .iter()
                .any(|prefix| symbol.starts_with(prefix))
            {
                assert!(
                    definitions.contains(symbol),
                    "undefined {symbol}: {line}\n{listing}"
                );
            }
        }
    }
    let menu = listing
        .split_once("proc_Menu:\n")
        .unwrap()
        .1
        .split_once("; ===== END PROC Menu")
        .unwrap()
        .0;
    for statement in menu
        .lines()
        .map(|line| line.split(';').next().unwrap().trim())
    {
        assert!(
            statement.is_empty()
                || statement.ends_with(':')
                || statement.starts_with(".BYTE ")
                || statement.starts_with("DTA "),
            "{statement}"
        );
    }
    assert!(
        menu.contains("DTA A("),
        "menu pointers must remain relocatable\n{menu}"
    );
    assert!(
        listing
            .lines()
            .any(|line| line.starts_with("loc_OperandReference_")
                && line.contains(" = ACTIONC_ORIGIN + ")),
        "{listing}"
    );
}

#[test]
fn literal_machine_data_and_interior_symbols_have_reassemblable_listings() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/listing/mads_machine_data.act");
    let source = std::fs::read_to_string(fixture)
        .unwrap()
        .replace("\r\n", "\n");
    let dir = TestDir(
        std::env::temp_dir().join(format!("actionc-mads-machine-data-{}", std::process::id())),
    );
    std::fs::create_dir_all(&dir.0).unwrap();
    for (mode, profile, backend) in [
        (CompileMode::Compatibility, "legacy", "classic"),
        (CompileMode::Optimized, "modern", "classic"),
        (CompileMode::Mir6502, "modern", "mir6502"),
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut previous = None;
                for newline in ["\n", "\r\n"] {
                    let path = dir.0.join("input.act");
                    std::fs::write(&path, source.replace('\n', newline)).unwrap();
                    let compiled = compile_file(
                        &path,
                        &CompileOptions::for_mode(mode)
                            .with_runtime(runtime)
                            .with_origin(origin),
                    )
                    .unwrap();
                    assert_symbols_resolve(&compiled.source_listing());
                    let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
                        .args([
                            "--profile",
                            profile,
                            "--backend",
                            backend,
                            "--runtime",
                            if runtime == Runtime::ActionCart {
                                "cart"
                            } else {
                                "standalone"
                            },
                            "--origin",
                            &origin.to_string(),
                            "--emit-listing",
                        ])
                        .arg(&path)
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    assert_symbols_resolve(&String::from_utf8(output.stdout).unwrap());
                    if let Some(previous) = previous {
                        assert_eq!(compiled.object_bytes(), previous);
                    }
                    previous = Some(compiled.object_bytes().to_vec());
                }
            }
        }
    }
}
