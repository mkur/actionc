use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use std::path::Path;

#[test]
fn bounded_call_pointer_retention_saves_ten_bytes_with_unchanged_unknown_calls() {
    assert!(!Mir6502Config::default().enable_bounded_call_pointer_retention);
    assert!(Mir6502Config::optimized().enable_bounded_call_pointer_retention);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/bounded_call_pointer_retention.act");
    let loaded = actionc::includes::load_program_with_expanded_source(&source).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
        &loaded.program,
        &model,
    ));
    actionc::nir::verify_program(&raw).unwrap();
    for opt in [false, true] {
        let nir = if opt {
            actionc::nir::optimize_program(&raw).unwrap()
        } else {
            raw.clone()
        };
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut config = Mir6502Config::optimized();
                config.enable_bounded_call_pointer_retention = false;
                let old = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                config.enable_bounded_call_pointer_retention = true;
                let new = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                assert_eq!(old.bytes.len() - new.bytes.len(), 10);
                if let Some(directory) = std::env::var_os("ACTIONC_BOUNDED_CALL_IMAGES") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    for (enabled, out) in [(false, &old), (true, &new)] {
                        let stem = format!("neutral-{opt}-{runtime:?}-{origin:04X}-{enabled}");
                        let image = actionc::codegen::format_load_file(out);
                        let facts = serde_json::json!({
                            "emitted_bytes":out.bytes.len(), "xex_bytes":image.len(),
                            "routines":out.map.routine_ranges.iter().map(|r|(&r.name,r.start,r.end)).collect::<Vec<_>>(),
                            "storage":out.map.storage_symbols.iter().map(|s|(format!("{:?}",s.scope),&s.name,s.address,s.size)).collect::<Vec<_>>(),
                            "reservations":out.skipped_ranges.iter().map(|s|(s.start,s.len)).collect::<Vec<_>>()
                        });
                        std::fs::write(directory.join(format!("{stem}.xex")), image).unwrap();
                        std::fs::write(
                            directory.join(format!("{stem}.json")),
                            serde_json::to_string_pretty(&facts).unwrap() + "\n",
                        )
                        .unwrap();
                    }
                }
                for name in ["CopyBytes", "ReadAcrossCopy", "UnknownCopy", "Main"] {
                    let size = |o: &actionc::codegen::CodegenOutput| {
                        let r = o
                            .map
                            .routine_ranges
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap();
                        r.end - r.start
                    };
                    assert_eq!(
                        size(&old) - size(&new),
                        if name == "ReadAcrossCopy" { 10 } else { 0 }
                    );
                }
                config.enable_pointer_call_retention = false;
                let disabled = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                config.enable_bounded_call_pointer_retention = false;
                let control = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                assert_eq!(disabled.bytes, control.bytes);
            }
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn bounded_call_pointer_retention_mads_relocation_with_lf_and_crlf() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/bounded_call_pointer_retention.act");
    let directory =
        std::env::temp_dir().join(format!("actionc-bounded-call-mads-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
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
        for (index, origin) in [0x3000, 0x41C7].into_iter().enumerate() {
            let listing = outputs[0].source_listing().replace("\r\n", "\n").replace(
                "ACTIONC_ORIGIN = $3000",
                &format!("ACTIONC_ORIGIN = ${origin:04X}"),
            );
            for crlf in [false, true] {
                let stem = format!("{runtime:?}-{origin:04X}-{crlf}");
                let asm = directory.join(format!("{stem}.asm"));
                let image = directory.join(format!("{stem}.xex"));
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
                assert_eq!(std::fs::read(image).unwrap(), outputs[index].object_bytes());
            }
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires the qualified argument-x-bitwise TN images"]
fn bounded_call_pointer_retention_keeps_unknown_tn_destinations_unchanged() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = root.join("build/bounded-call/tn-qualified");
    std::fs::create_dir_all(&output).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for name in ["TN", "TNDBG"] {
            let compiled = compile_file(
                root.join(format!("samples/tn/modern/{name}.ACT")),
                &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
            )
            .unwrap();
            let stem = format!("{name}-{runtime:?}-true");
            assert_eq!(
                compiled.object_bytes(),
                std::fs::read(root.join(format!("build/argument-x-bitwise/measured/{stem}.xex")))
                    .unwrap()
            );
            std::fs::write(output.join(format!("{stem}.xex")), compiled.object_bytes()).unwrap();
            std::fs::write(
                output.join(format!("{stem}.asm")),
                compiled.source_listing(),
            )
            .unwrap();
        }
    }
}
