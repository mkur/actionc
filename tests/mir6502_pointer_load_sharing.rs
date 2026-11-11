use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};

#[test]
fn pointer_load_sharing_baseline_preserves_nir_modes_runtimes_and_origins() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/pointer_load_sharing.act");
    let loaded = actionc::includes::load_program_with_expanded_source(source).unwrap();
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
    for (is_optimized, nir) in [(false, &raw), (true, &optimized)] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let config = Mir6502Config::optimized();
                let p = mir6502::materialize_program_with_origin_and_runtime(
                    mir6502::lower_program(nir).unwrap(),
                    &config,
                    origin,
                    runtime,
                )
                .unwrap();
                mir6502::verify_program(&p, mir6502::MirPhase::PreEmission).unwrap();
                let output =
                    mir6502::generate_output_with_config_and_runtime(nir, origin, &config, runtime)
                        .unwrap();
                let image = actionc::codegen::format_load_file(&output);
                assert!(!image.is_empty());
                if is_optimized {
                    if let Some(dir) = std::env::var_os("ACTIONC_POINTER_LOAD_BASELINE") {
                        let rt = if runtime == Runtime::ActionCart {
                            "cart"
                        } else {
                            "standalone"
                        };
                        let stem = std::path::PathBuf::from(dir)
                            .join(format!("neutral-{rt}-{origin:04X}"));
                        assert_eq!(image, std::fs::read(stem.with_extension("xex")).unwrap());
                        let facts = serde_json::json!({
                            "frames":p.routines.iter().map(|r|(&r.name,format!("{:?}",r.frame))).collect::<std::collections::BTreeMap<_,_>>(),
                            "storage":output.map.storage_symbols.iter().map(|s|(&s.name,s.address,s.size)).collect::<Vec<_>>(),
                            "reserved":output.skipped_ranges.iter().map(|r|(r.start,r.len)).collect::<Vec<_>>(),
                        });
                        std::fs::write(
                            stem.with_extension("facts.json"),
                            serde_json::to_string_pretty(&facts).unwrap(),
                        )
                        .unwrap();
                    }
                }
                let forward = p.routines.iter().find(|r| r.name == "Forward").unwrap();
                let text = mir6502::format_program(&mir6502::MirProgram {
                    routines: vec![forward.clone()],
                    ..p.clone()
                });
                assert_eq!(text.matches("load param p1+0").count(), 2);
                assert_eq!(text.matches("load param p1+1").count(), 2); // Both setup reads survive.
            }
        }
    }
}
