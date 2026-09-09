use actionc::compiler::{CompileMode, CompileOptions, CompilerPhase, Runtime, compile_file};
use std::path::{Path, PathBuf};

struct Work(PathBuf);
impl Work {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "actionc-unions-public-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, source).unwrap();
        path
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn union_sample_is_modern_only_in_both_public_backends_and_runtimes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/union-views.act");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        }
        let errors = compile_file(
            &path,
            &CompileOptions::for_mode(CompileMode::Compatibility).with_runtime(runtime),
        )
        .unwrap_err();
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|d| d.phase == CompilerPhase::Semantic && d.message.contains("UNION")),
            "{errors}"
        );
    }
}

#[test]
fn public_classic_rejects_wide_field_writes_but_accepts_narrow_views() {
    let work = Work::new();
    for declaration in [
        "TYPE View=UNION [LONGCARD wide CARD word]",
        "TYPE View=[LONGCARD wide CARD word]",
    ] {
        for body in ["value.wide=1", "value.wide==+1"] {
            let path = work.write(
                "wide.act",
                &format!("{declaration} View value PROC Main() {body} RETURN"),
            );
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let errors = compile_file(
                    &path,
                    &CompileOptions::for_mode(CompileMode::Optimized).with_runtime(runtime),
                )
                .unwrap_err();
                assert!(
                    errors
                        .diagnostics()
                        .iter()
                        .any(|d| d.phase == CompilerPhase::Codegen
                            && d.message.contains("requires the MIR6502 backend")),
                    "{errors}"
                );
                compile_file(
                    &path,
                    &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
                )
                .unwrap();
            }
        }
    }
    let path = work.write(
        "narrow.act",
        "TYPE View=UNION [LONGCARD wide CARD word] View value PROC Main() value.word=1 RETURN",
    );
    compile_file(&path, &CompileOptions::for_mode(CompileMode::Optimized)).unwrap();
}

#[test]
fn public_module_only_union_types_and_callbacks_compile_through_the_facade() {
    let work = Work::new();
    work.write("lib.act","MODULE Lib PUBLIC TYPE View<T>=UNION [T value BYTE ARRAY bytes(2)] PUBLIC View<CARD> FUNC Copy(View<CARD> input) RETURN(input) ENDMODULE");
    let path=work.write("main.act","MODULE App USE Lib AS A USE Lib AS B A.View<CARD> original B.View<CARD> FUNC POINTER callback(B.View<CARD> input) PROC Main() original.value=7 callback=A.Copy original=callback(original) RETURN ENDMODULE");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        }
    }
}

#[test]
fn inspection_cli_preserves_union_profile_and_classic_width_diagnostics() {
    let work = Work::new();
    let path = work.write(
        "view.act",
        "TYPE View=UNION [LONGCARD wide CARD word] View value PROC Main() value.wide=1 RETURN",
    );
    for (profile, backend, success) in [
        ("legacy", "classic", false),
        ("modern", "classic", false),
        ("modern", "mir6502", true),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args(["--profile", profile, "--backend", backend, "--emit-listing"])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if !success {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(if profile == "legacy" {
                    "UNION"
                } else {
                    "requires the MIR6502 backend"
                })
            );
        }
    }
}
