use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
#[test]
fn argument_x_forwarding_baseline_verifies_modes_runtimes_and_origins() {
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
                let config = Mir6502Config::optimized();
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
                for name in ["Forward", "ForwardByte", "ForwardPrivate"] {
                    let r = p.routines.iter().find(|r| r.name == name).unwrap();
                    assert!(
                        mir6502::format_program(&mir6502::MirProgram {
                            routines: vec![r.clone()],
                            ..p.clone()
                        })
                        .contains("x =.b load ")
                    );
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
