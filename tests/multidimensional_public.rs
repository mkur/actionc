use actionc::compiler::native::{self, NativeCompileOptions};
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use std::path::{Path, PathBuf};

struct Work(PathBuf);
impl Work {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "actionc-multidimensional-public-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, source: &str, crlf: bool) -> PathBuf {
        let path = self.0.join(name);
        let source = source.replace("\r\n", "\n");
        std::fs::write(
            &path,
            if crlf {
                source.replace('\n', "\r\n")
            } else {
                source
            },
        )
        .unwrap();
        path
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn modern_public_profiles_compile_imported_dimensions_and_includes_in_both_line_endings() {
    let work = Work::new();
    for crlf in [false, true] {
        work.write(
            "limits.act",
            "MODULE Limits\nPUBLIC CONST Rows=3,Columns=129\nENDMODULE\n",
            crlf,
        );
        work.write("arrays.inc","CARD ARRAY grid(Limits.Rows,Limits.Columns)\nTYPE Tile=[BYTE tag CARD ARRAY pixels(2,3)]\nTile image\n",crlf);
        let path=work.write("main.act","MODULE App\nUSE Limits\nINCLUDE \"arrays.inc\"\nPROC Main()\n  BYTE row,column\n  row=2 column=128\n  grid(row,column)=42\n  image.pixels(1,2)=grid(row,column)\nRETURN\nENDMODULE\n",crlf);
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
            }
        }
        let compatibility = work.write("compat.act", "CARD ARRAY grid(2,3) PROC Main() RETURN", crlf);
        let errors = compile_file(&compatibility, &CompileOptions::for_mode(CompileMode::Compatibility)).unwrap_err();
        assert!(
            errors
                .to_string()
                .contains("multidimensional arrays require the modern profile"), "{errors}"
        );
        for optimize in [false, true] {
            let image = native::compile_file(
                &path,
                &NativeCompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap()
            .image;
            let array = image.symbol("App.grid").unwrap().array.as_ref().unwrap();
            assert_eq!((array.count, array.stride), (Some(387), 2));
        }
        for backend in ["classic", "mir6502"] {
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
                .args([
                    "--profile",
                    "modern",
                    "--backend",
                    backend,
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
        }
    }
}

#[test]
fn documented_rectangular_and_inline_example_builds_on_every_backend() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/multidimensional/example.act");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        compile_file(&path, &CompileOptions::for_mode(mode)).unwrap();
    }
    native::compile_file(&path, &NativeCompileOptions::default()).unwrap();
}
