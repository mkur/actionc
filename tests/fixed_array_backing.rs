use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::compiler::{CompileMode, CompileOptions, compile_file};
use actionc::includes::{ModuleLoadOptions, load_compilation};
use actionc::nir::{self, NirGlobalBacking};
use actionc::semantic::{analyze_compilation, ir};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "actionc-fixed-array-backing-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(path.join("lib")).expect("create fixed-array fixture directory");
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

fn write_fixture(temp: &TestDir) -> PathBuf {
    let application = temp.path().join("app.act");
    fs::write(
        &application,
        "MODULE APP\n\
         USE LIB.ADDRESS\n\
         BYTE ARRAY literal(256)=$83F1,\n\
                    symbolic(256)=ADDRESS.FIRE_ANCHOR-31\n\
         PROC Main()\n\
           literal(1)=1\n\
           symbolic(2)=2\n\
           DO OD\n\
         RETURN\n\
         ENDMODULE\n",
    )
    .expect("write fixed-array application");
    fs::write(
        temp.path().join("lib/address.act"),
        "MODULE LIB.ADDRESS\n\
         PUBLIC CONST CARD FIRE_ANCHOR=$8410\n\
         ENDMODULE\n",
    )
    .expect("write fixed-array address module");
    application
}

#[test]
fn qualified_constant_fixed_arrays_share_exact_storage_facts_in_all_modes() {
    let temp = TestDir::new();
    let application = write_fixture(&temp);
    let module_options = ModuleLoadOptions {
        project_root: None,
        module_paths: vec![temp.path().to_path_buf()],
    };
    let compilation = load_compilation(&application, &module_options)
        .expect("load qualified fixed-array fixture");
    let model = analyze_compilation(&compilation).expect("analyze qualified fixed-array fixture");
    let semir = ir::lower_compilation(&compilation, &model);
    let nir = nir::lower_program(&semir);
    nir::verify_program(&nir).expect("verify qualified fixed-array NIR");

    for name in ["LITERAL", "SYMBOLIC"] {
        let global = nir
            .globals
            .iter()
            .find(|global| global.name.to_ascii_uppercase().contains(name))
            .unwrap_or_else(|| panic!("find {name} fixed array"));
        assert_eq!(global.backing, NirGlobalBacking::Absolute(0x83F1));
        assert_eq!(
            global.array.as_ref().and_then(|array| array.address_initializer),
            Some(0x83F1)
        );
    }

    let symbolic_index = nir
        .globals
        .iter()
        .position(|global| global.name.to_ascii_uppercase().contains("SYMBOLIC"))
        .expect("find symbolic fixed array index");
    let mut missing_backing = nir.clone();
    missing_backing.globals[symbolic_index].backing = NirGlobalBacking::Ordinary;
    let diagnostics = nir::verify_program(&missing_backing)
        .expect_err("direct fixed array without absolute backing must fail verification");
    assert!(diagnostics.iter().any(|diagnostic| diagnostic
        .message
        .contains("must have absolute backing $83F1")));

    let mut mismatched_backing = nir.clone();
    mismatched_backing.globals[symbolic_index].backing = NirGlobalBacking::Absolute(0x83F2);
    let diagnostics = nir::verify_program(&mismatched_backing)
        .expect_err("direct fixed array with mismatched backing must fail verification");
    assert!(diagnostics.iter().any(|diagnostic| diagnostic
        .message
        .contains("backing $83F2 but address initializer $83F1")));

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(
            &application,
            &CompileOptions::for_mode(mode).with_module_path(temp.path()),
        )
        .unwrap_or_else(|error| panic!("compile fixed arrays in {mode:?}: {error}"));
        let listing = compiled.source_listing();
        assert!(
            listing.matches("= $83F1").count() >= 2,
            "both fixed arrays should resolve to $83F1 in {mode:?}:\n{listing}"
        );
    }
}
