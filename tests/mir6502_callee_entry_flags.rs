use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
#[test]
fn callee_entry_flags_baseline_matches_modes_runtimes_and_origins() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/callee_entry_flags.act");
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
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let config = Mir6502Config {
                    enable_precise_callee_entry_flags: false,
                    ..Mir6502Config::optimized()
                };
                let p = mir6502::materialize_program_with_origin_and_runtime(
                    mir6502::lower_program(nir).unwrap(),
                    &config,
                    origin,
                    rt,
                )
                .unwrap();
                mir6502::verify_program(&p, mir6502::MirPhase::PreEmission).unwrap();
                let out =
                    mir6502::generate_output_with_config_and_runtime(nir, origin, &config, rt)
                        .unwrap();
                let enabled = Mir6502Config {
                    enable_precise_callee_entry_flags: true,
                    ..config.clone()
                };
                let after =
                    mir6502::generate_output_with_config_and_runtime(nir, origin, &enabled, rt)
                        .unwrap();
                assert_eq!(
                    actionc::codegen::format_load_file(&out).len()
                        - actionc::codegen::format_load_file(&after).len(),
                    2
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
                        if old.name == "Forward" { 2 } else { 0 }
                    );
                }
                if optimized {
                    if let Some(dir) = std::env::var_os("ACTIONC_CALLEE_ENTRY_FLAGS_BASELINE") {
                        let name = if rt == Runtime::ActionCart {
                            "cart"
                        } else {
                            "standalone"
                        };
                        let stem = std::path::PathBuf::from(dir)
                            .join(format!("neutral-{name}-{origin:04X}"));
                        assert_eq!(
                            actionc::codegen::format_load_file(&out),
                            std::fs::read(stem.with_extension("xex")).unwrap()
                        );
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
fn callee_entry_flags_mads_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    use actionc::compiler::{compile_file, CompileMode, CompileOptions};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/callee_entry_flags.act");
    let dir = std::env::temp_dir().join(format!(
        "actionc-callee-entry-flags-mads-{}",
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
