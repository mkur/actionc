use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};

#[test]
fn pointer_byte_plus_one_baseline_verifies_both_nir_modes_runtimes_and_origins() {
    assert!(!Mir6502Config::default().enable_pointer_byte_plus_one_fusion);
    assert!(Mir6502Config::optimized().enable_pointer_byte_plus_one_fusion);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/pointer_byte_plus_one.act");
    let loaded = actionc::includes::load_program_with_expanded_source(path).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
        &loaded.program,
        &model,
    ));
    let optimized = actionc::nir::optimize_program(&raw).unwrap();
    for nir in [&raw, &optimized] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mir = mir6502::materialize_program_with_origin_and_runtime(
                    mir6502::lower_program(nir).unwrap(),
                    &Mir6502Config {
                        enable_argument_x_forwarding: false,
                        enable_pointer_load_sharing: false,
                        enable_pointer_byte_plus_one_fusion: false,
                        ..Mir6502Config::optimized()
                    },
                    origin,
                    runtime,
                )
                .unwrap();
                mir6502::verify_program(&mir, mir6502::MirPhase::PreEmission).unwrap();
                let r = mir.routines.iter().find(|r| r.name == "Forward").unwrap();
                assert!(r.blocks.iter().any(|b| b.ops.windows(2).any(|ops| matches!(
                    ops,
                    [
                        mir6502::MirOp::OffsetPointerByIndirectByte { .. },
                        mir6502::MirOp::AddByteToWordMem { .. }
                    ]
                ))));
                let output = mir6502::generate_output_with_config_and_runtime(
                    nir,
                    origin,
                    &Mir6502Config {
                        enable_argument_x_forwarding: false,
                        enable_pointer_load_sharing: false,
                        enable_pointer_byte_plus_one_fusion: false,
                        ..Mir6502Config::optimized()
                    },
                    runtime,
                )
                .unwrap();
                assert!(!actionc::codegen::format_load_file(&output).is_empty());
                let after = mir6502::generate_output_with_config_and_runtime(
                    nir,
                    origin,
                    &Mir6502Config {
                        enable_argument_x_forwarding: false,
                        enable_pointer_load_sharing: false,
                        enable_pointer_byte_plus_one_fusion: true,
                        ..Mir6502Config::optimized()
                    },
                    runtime,
                )
                .unwrap();
                assert_eq!(
                    actionc::codegen::format_load_file(&output).len()
                        - actionc::codegen::format_load_file(&after).len(),
                    5
                );
                for name in [
                    "Forward", "PlusZero", "PlusTwo", "Subtract", "Plain", "Address",
                ] {
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
                        size(&output) - size(&after),
                        if name == "Forward" { 5 } else { 0 }
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn pointer_byte_plus_one_mads_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/pointer_byte_plus_one.act");
    let dir = std::env::temp_dir().join(format!(
        "actionc-pointer-byte-plus-one-mads-{}",
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
