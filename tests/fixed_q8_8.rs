use actionc::ast::{FundType, Visibility};
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::includes::{ModuleLoadOptions, load_compilation};
use actionc::semantic::{SymbolClass, SymbolId, ValueTypeBase, analyze_compilation};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Source(PathBuf);
impl Source {
    fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-q8-8-{}-{}.act",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, text).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn fixed_q8_8_imports_and_conversions_compile_in_all_modes_and_runtimes() {
    for (import, alias) in [("USE MATH.Q8_8", "Q8_8"), ("USE MATH.Q8_8 AS Q", "Q")] {
        let source = Source::new(&format!(
            "MODULE TEST {import}\nINT input=$6E0,result=$601\nPROC Main()\n\
             result={alias}.FromInt(input) result={alias}.Trunc(result)\n\
             result={alias}.One result={alias}.Half result={alias}.Epsilon\n\
             result={alias}.MinValue result={alias}.MaxValue\nRETURN ENDMODULE"
        ));
        for mode in [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                compile_file(
                    &source.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap_or_else(|error| panic!("{import}, {mode:?}/{runtime:?}: {error}"));
            }
        }
    }
}

#[test]
fn fixed_q8_8_exports_typed_constants_without_loading_real() {
    let source = Source::new("MODULE TEST USE MATH.Q8_8 AS Q PROC Main() RETURN ENDMODULE");
    let loaded = load_compilation(&source.0, &ModuleLoadOptions::default()).unwrap();
    let origins: Vec<_> = loaded
        .modules
        .iter()
        .map(|m| m.origin.to_string())
        .collect();
    assert!(
        origins.iter().any(|s| s == "<embedded:MATH.Q8_8>"),
        "{origins:?}"
    );
    assert!(
        !origins
            .iter()
            .any(|s| s == "<embedded:MATH>" || s == "<embedded:ATARI.REAL>"),
        "{origins:?}"
    );
    let model = analyze_compilation(&loaded).unwrap();
    for (name, bits) in [
        ("One", 256),
        ("Half", 128),
        ("Epsilon", 1),
        ("MinValue", 32768),
        ("MaxValue", 32767),
    ] {
        let qualified = format!("MATH.Q8_8.{name}");
        let (index, symbol) = model
            .symbols
            .symbols
            .iter()
            .enumerate()
            .find(|(_, s)| s.qualified_name.eq_ignore_ascii_case(&qualified))
            .unwrap();
        assert_eq!(symbol.class, SymbolClass::Const);
        assert_eq!(symbol.visibility, Visibility::Public);
        assert_eq!(
            symbol.ty.as_ref().unwrap().base,
            ValueTypeBase::Fund(FundType::Int)
        );
        assert_eq!(model.constants[&SymbolId(index)].bits, bits);
    }
}
