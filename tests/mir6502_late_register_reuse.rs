use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config, trace::Mir6502Trace},
};

fn input(source: &str, optimized: bool) -> actionc::nir::NirProgram {
    let source = source.replace("\r\n", "\n");
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
    actionc::nir::verify_program(&nir).unwrap();
    if optimized {
        actionc::nir::optimize_program(&nir).unwrap()
    } else {
        nir
    }
}

#[test]
fn late_reuse_is_costed_traced_and_independent_of_source_newlines() {
    assert!(!Mir6502Config::default().enable_late_register_reuse);
    assert!(Mir6502Config::optimized().enable_late_register_reuse);
    let lf = include_str!("../fixtures/runtime/late_register_reuse.act").replace("\r\n", "\n");
    for optimized in [false, true] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41c7] {
                let nir = input(&lf, optimized);
                let config = Mir6502Config {
                    enable_small_leaf_inlining: false,
                    enable_late_register_reuse: false,
                    ..Mir6502Config::optimized()
                };
                let before = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                let on = Mir6502Config {
                    enable_late_register_reuse: true,
                    ..config
                };
                let mut trace = Mir6502Trace::default();
                let after =
                    mir6502::generate_output_with_trace(&nir, origin, &on, runtime, &mut trace)
                        .unwrap();
                assert_eq!(
                    after,
                    mir6502::generate_output_with_config_and_runtime(
                        &input(&lf.replace('\n', "\r\n"), optimized),
                        origin,
                        &on,
                        runtime
                    )
                    .unwrap()
                );
                let size = |out: &actionc::codegen::CodegenOutput, name| {
                    let r = out
                        .map
                        .routine_ranges
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap();
                    r.end - r.start
                };
                for name in ["Advance", "Retreat"] {
                    assert_eq!(
                        size(&before, name) - size(&after, name),
                        if optimized { 3 } else { 0 },
                        "{name} {runtime:?}"
                    );
                }
                assert_eq!(size(&before, "Plain"), size(&after, "Plain"));
                assert_eq!(
                    actionc::codegen::format_load_file(&before).len()
                        - actionc::codegen::format_load_file(&after).len(),
                    if optimized { 6 } else { 0 }
                );
                assert_eq!(trace.manifest()["status"], "complete");
                if optimized {
                    assert!(
                        trace
                            .events()
                            .iter()
                            .any(|e| e.kind == "late-cleanup-result"
                                && e.data["accepted"] == true
                                && e.data["removed_reloads"] == 2
                                && e.data["register_transfers"] == 2)
                    );
                } else {
                    // Raw NIR retains staged word arithmetic without this compound
                    // operation; it deliberately remains outside the first slice.
                    assert_eq!(before, after);
                }
            }
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn late_register_reuse_mads_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/late_register_reuse.act");
    let dir = std::env::temp_dir().join(format!(
        "actionc-late-register-reuse-mads-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let outputs = [0x3000, 0x41C7].map(|origin| {
            compile_file(
                &source,
                &CompileOptions::for_mode(CompileMode::Mir6502)
                    .with_origin(origin)
                    .with_runtime(runtime),
            )
            .unwrap()
        });
        for (i, origin) in [0x3000, 0x41C7].into_iter().enumerate() {
            let listing = outputs[0].source_listing().replace("\r\n", "\n").replace(
                "ACTIONC_ORIGIN = $3000",
                &format!("ACTIONC_ORIGIN = ${origin:04X}"),
            );
            for crlf in [false, true] {
                let stem = format!("{runtime:?}-{origin:04X}-{crlf}");
                let asm = dir.join(format!("{stem}.asm"));
                let image = dir.join(format!("{stem}.xex"));
                std::fs::write(
                    &asm,
                    if crlf {
                        listing.replace('\n', "\r\n")
                    } else {
                        listing.clone()
                    },
                )
                .unwrap();
                let result = std::process::Command::new(
                    std::env::var_os("ACTIONC_MADS").unwrap_or_else(|| "mads".into()),
                )
                .arg(&asm)
                .arg(format!("-o:{}", image.display()))
                .arg("-s")
                .output()
                .unwrap();
                assert!(
                    result.status.success(),
                    "{} {}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    std::fs::read(image).unwrap(),
                    outputs[i].object_bytes(),
                    "{stem}"
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
