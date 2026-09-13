use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::includes::{ModuleLoadOptions, load_compilation};
use actionc::semantic::{self, SemanticOptions};
use actionc::target::TargetId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Source(PathBuf);
impl Source {
    fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-integer-shifts-{}-{}.act",
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
fn integer_shifts_import_and_compile_in_all_modes_and_runtimes() {
    for (import, alias) in [
        ("USE MATH.INTEGER", "INTEGER."),
        ("USE MATH.INTEGER AS BITS", "BITS."),
        ("USE ALL FROM MATH.INTEGER", ""),
    ] {
        let source = Source::new(&format!(
            "MODULE TEST {import}\nINT value,result LONGINT wide,wideResult BYTE count\n\
             PROC Main() result={alias}AsrI(value,count) wideResult={alias}AsrLI(wide,count)\n\
             result={alias}AsrI(-3,1) wideResult={alias}AsrLI(wide,255) RETURN ENDMODULE"
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
fn integer_shifts_load_without_real_and_lower_for_all_target_layouts() {
    let source = Source::new(
        "MODULE TEST USE MATH.INTEGER AS BITS INT value,result LONGINT wide,wideResult BYTE count\n\
         PROC Main() result=BITS.AsrI(value,count) wideResult=BITS.AsrLI(wide,count) RETURN ENDMODULE",
    );
    let loaded = load_compilation(&source.0, &ModuleLoadOptions::default()).unwrap();
    let origins: Vec<_> = loaded
        .modules
        .iter()
        .map(|m| m.origin.to_string())
        .collect();
    assert!(
        origins.iter().any(|s| s == "<embedded:MATH.INTEGER>"),
        "{origins:?}"
    );
    assert!(
        !origins
            .iter()
            .any(|s| s == "<embedded:MATH>" || s == "<embedded:ATARI.REAL>"),
        "{origins:?}"
    );
    for target in [
        TargetId::Atari6502,
        TargetId::Wdc65816Small,
        TargetId::Wdc65816Native,
        TargetId::Motorola68000,
    ] {
        let model = semantic::analyze_compilation_with_options(
            &loaded,
            SemanticOptions::modern().with_target(target),
        )
        .unwrap_or_else(|error| panic!("{target:?}: {error:?}"));
        let raw = actionc::nir::lower_program(&semantic::ir::lower_compilation(&loaded, &model));
        actionc::nir::verify_program(&raw).unwrap();
        let optimized = actionc::nir::optimize_program(&raw).unwrap();
        actionc::nir::verify_program(&optimized).unwrap();
        for program in [&raw, &optimized] {
            match target {
                TargetId::Atari6502 => {
                    actionc::mir6502::lower_program(program).unwrap();
                }
                TargetId::Motorola68000 => {
                    actionc::mir68k::lower_program(program).unwrap();
                }
                _ => {
                    actionc::mir65816::lower_program(program).unwrap();
                }
            }
        }
    }
}
