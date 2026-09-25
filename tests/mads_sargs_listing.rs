use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, compile_file};
use actionc::runtime::Runtime;

const MODES: [CompileMode; 3] = [
    CompileMode::Compatibility,
    CompileMode::Optimized,
    CompileMode::Mir6502,
];

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/listing/mads_sargs.act")
}

fn assert_payloads(listing: &str, expected: usize) {
    let lines = listing.lines().collect::<Vec<_>>();
    let mut found = 0;
    for (i, line) in lines.iter().enumerate() {
        if !line.starts_with("; Parameter frame follows") {
            continue;
        }
        found += 1;
        assert!(lines[i - 1].trim_start().starts_with("JSR.A "));
        assert!(lines[i + 1].trim_start().starts_with(".WORD param_"));
        assert!(lines[i + 2].trim_start().starts_with(".BYTE $"));
        assert_eq!(lines[i + 1].find(';'), lines[i + 2].find(';'), "{listing}");
        assert!(lines[i + 2].contains("| frame size: "));
        assert!(!lines[i + 2].contains("minus one"));
    }
    assert_eq!(found, expected, "{listing}");
}

fn assert_parameter(listing: &str, label: &str, directive: &str, name: &str) {
    let lines = listing.lines().collect::<Vec<_>>();
    let at = lines.iter().position(|line| *line == label).expect(label);
    assert!(
        lines[at + 1].trim_start().starts_with(directive),
        "{listing}"
    );
    assert!(lines[at + 1].ends_with(&format!(" {name}")), "{listing}");
}

#[test]
fn sargs_descriptors_and_parameter_storage_are_typed_and_named() {
    for mode in MODES {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let compiled = compile_file(
                    fixture(),
                    &CompileOptions::for_mode(mode)
                        .with_runtime(runtime)
                        .with_origin(origin),
                )
                .unwrap();
                let listing = compiled.source_listing();
                assert_payloads(
                    &listing,
                    if mode == CompileMode::Compatibility {
                        3
                    } else {
                        2
                    },
                );
                assert!(listing.contains("; Parameter frame follows: first, word, last, extra"));
                assert!(listing.contains("; Parameter frame follows: tag, ptr, items"));
                assert_eq!(listing.matches("| frame size: 5 bytes").count(), 2);
                assert_parameter(&listing, "param_sumframe_first:", ".BYTE $00", "first");
                assert_parameter(&listing, "param_sumframe_word:", ".WORD $0000", "word");
                assert_parameter(&listing, "param_indirect_ptr:", ".WORD $0000", "ptr");
                assert_parameter(&listing, "param_indirect_items:", ".WORD $0000", "items");
                if mode == CompileMode::Compatibility {
                    assert!(listing.contains("; Parameter frame follows: x, y, col"));
                    assert!(listing.contains("| frame size: 3 bytes"));
                }
            }
        }
    }
}

#[test]
fn long_parameter_labels_keep_payload_comments_aligned_with_lf_and_crlf() {
    let temp = TestDir::new();
    let source = std::fs::read_to_string(fixture())
        .unwrap()
        .replace("\r\n", "\n")
        .replace("SumFrame", "SumWithALongRoutineName")
        .replace("first", "firstParameterWithALongName");
    for newline in ["\n", "\r\n"] {
        let path = temp.0.join("long-names.act");
        std::fs::write(&path, source.replace('\n', newline)).unwrap();
        for mode in MODES {
            let listing = compile_file(
                &path,
                &CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone),
            )
            .unwrap()
            .source_listing();
            assert_payloads(
                &listing,
                if mode == CompileMode::Compatibility {
                    3
                } else {
                    2
                },
            );
            assert!(listing.contains(
                "; Parameter frame follows: firstParameterWithALongName, word, last, extra"
            ));
        }
    }
}

#[test]
fn sargs_overrides_are_recognized_by_binding_instead_of_implementation_name() {
    let temp = TestDir::new();
    let source = std::fs::read_to_string(fixture()).unwrap();
    // This helper implements the SArgs descriptor ABI without a BREAK-key check.
    let helper = "PROC CopyFrame=*()\n\
        [$A085$A186$A284$18$68$8485$369$A8$68$8585$69$0$48$98$48$1A0\n\
         $84B1$8285$C8$84B1$8385$C8$84B1$A8$B9$A0$0$8291$88$F810$60]\n\
         SET $4EE=CopyFrame\n";
    let path = temp.0.join("overridden.act");
    std::fs::write(&path, format!("{helper}{source}")).unwrap();
    for mode in MODES {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let listing =
                compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime))
                    .unwrap()
                    .source_listing();
            assert_payloads(
                &listing,
                if mode == CompileMode::Compatibility {
                    3
                } else {
                    2
                },
            );
            assert!(
                listing
                    .lines()
                    .any(|line| line.trim_start().starts_with("JSR.A proc_copyframe "))
            );
            assert!(listing.contains("; Parameter frame follows: first, word, last, extra"));
        }
    }
}

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "actionc-sargs-test-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
