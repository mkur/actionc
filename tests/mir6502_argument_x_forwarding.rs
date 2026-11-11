use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
#[test]
fn argument_x_forwarding_baseline_verifies_modes_runtimes_and_origins() {
    assert!(!Mir6502Config::default().enable_argument_x_forwarding);
    assert!(Mir6502Config::optimized().enable_argument_x_forwarding);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/argument_x_forwarding.act");
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
    let opt = actionc::nir::optimize_program(&raw).unwrap();
    for (optimized, nir) in [(false, &raw), (true, &opt)] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let config = Mir6502Config {
                    enable_argument_x_bitwise_forwarding: false,
                    enable_precise_callee_entry_flags: false,
                    enable_argument_x_forwarding: false,
                    ..Mir6502Config::optimized()
                };
                let p = mir6502::materialize_program_with_origin_and_runtime(
                    mir6502::lower_program(nir).unwrap(),
                    &config,
                    origin,
                    runtime,
                )
                .unwrap();
                mir6502::verify_program(&p, mir6502::MirPhase::PreEmission).unwrap();
                let out =
                    mir6502::generate_output_with_config_and_runtime(nir, origin, &config, runtime)
                        .unwrap();
                let image = actionc::codegen::format_load_file(&out);
                let enabled = Mir6502Config {
                    enable_argument_x_bitwise_forwarding: false,
                    enable_precise_callee_entry_flags: false,
                    enable_argument_x_forwarding: true,
                    ..config.clone()
                };
                let after = mir6502::generate_output_with_config_and_runtime(
                    nir, origin, &enabled, runtime,
                )
                .unwrap();
                assert_eq!(
                    image.len() - actionc::codegen::format_load_file(&after).len(),
                    4
                );
                for old in &out.map.routine_ranges {
                    let new = after
                        .map
                        .routine_ranges
                        .iter()
                        .find(|r| r.name == old.name)
                        .unwrap();
                    assert_eq!(
                        (old.end - old.start) - (new.end - new.start),
                        match old.name.as_str() {
                            "Forward" | "ForwardPrivate" => 1,
                            "ForwardByte" => 2,
                            _ => 0,
                        }
                    );
                }
                for name in ["Forward", "ForwardByte", "ForwardPrivate"] {
                    let r = p.routines.iter().find(|r| r.name == name).unwrap();
                    assert!(mir6502::format_program(&mir6502::MirProgram {
                        routines: vec![r.clone()],
                        ..p.clone()
                    })
                    .contains("x =.b load "));
                }
                if optimized {
                    if let Some(dir) = std::env::var_os("ACTIONC_ARGUMENT_X_BASELINE") {
                        let rt = if runtime == Runtime::ActionCart {
                            "cart"
                        } else {
                            "standalone"
                        };
                        let stem = std::path::PathBuf::from(dir)
                            .join(format!("neutral-{rt}-{origin:04X}"));
                        assert_eq!(image, std::fs::read(stem.with_extension("xex")).unwrap());
                        let facts = serde_json::json!({"frames":p.routines.iter().map(|r|(&r.name,format!("{:?}",r.frame))).collect::<std::collections::BTreeMap<_,_>>(),"storage":out.map.storage_symbols.iter().map(|s|(&s.name,s.address,s.size)).collect::<Vec<_>>(),"reserved":out.skipped_ranges.iter().map(|r|(r.start,r.len)).collect::<Vec<_>>()});
                        std::fs::write(
                            stem.with_extension("facts.json"),
                            serde_json::to_string_pretty(&facts).unwrap(),
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn argument_x_forwarding_mads_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    use actionc::compiler::{compile_file, CompileMode, CompileOptions};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/argument_x_forwarding.act");
    let dir = std::env::temp_dir().join(format!(
        "actionc-argument-x-forwarding-mads-{}",
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
