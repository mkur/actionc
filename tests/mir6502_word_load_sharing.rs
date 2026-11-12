use actionc::{
    codegen,
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};

#[test]
fn word_load_sharing_dormant_proof_preserves_frozen_images() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (stem, path, origins, sizes) in [
        (
            "neutral",
            "fixtures/runtime/word_load_sharing.act",
            vec![0x3000, 0x41C7],
            [313, 313],
        ),
        (
            "allocate",
            "fixtures/runtime/allocate_word_load_sharing.act",
            vec![0x3000],
            [952, 1427],
        ),
    ] {
        let loaded =
            actionc::includes::load_program_with_expanded_source(&root.join(path)).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &loaded.program,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let semir = actionc::semantic::ir::lower_program(&loaded.program, &model);
        let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(&semir)).unwrap();
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in origins.iter().copied() {
                let p = mir6502::materialize_program_with_origin_and_runtime(
                    mir6502::lower_program(&nir).unwrap(),
                    &Mir6502Config::optimized(),
                    origin,
                    runtime,
                )
                .unwrap();
                mir6502::verify_program(&p, mir6502::MirPhase::PreEmission).unwrap();
                let output = mir6502::generate_output_with_config_and_runtime(
                    &nir,
                    origin,
                    &Mir6502Config::optimized(),
                    runtime,
                )
                .unwrap();
                let image = codegen::format_load_file(&output);
                assert_eq!(
                    image.len(),
                    sizes[usize::from(runtime == Runtime::Standalone)]
                );
                if let Some(directory) = std::env::var_os("ACTIONC_WORD_LOAD_BASELINE") {
                    let rt = if runtime == Runtime::ActionCart {
                        "cart"
                    } else {
                        "standalone"
                    };
                    let name = if origin == 0x3000 {
                        stem.to_string()
                    } else {
                        format!("{stem}-41C7")
                    };
                    assert_eq!(
                        image,
                        std::fs::read(
                            std::path::PathBuf::from(&directory).join(format!("{name}-{rt}.xex"))
                        )
                        .unwrap()
                    );
                    let facts = serde_json::json!({
                        "frames":p.routines.iter().map(|r|(&r.name,format!("{:?}",r.frame))).collect::<std::collections::BTreeMap<_,_>>(),
                        "storage":output.map.storage_symbols.iter().map(|s|(&s.name,s.address,s.size)).collect::<Vec<_>>(),
                        "reserved":output.skipped_ranges.iter().map(|r|(r.start,r.len)).collect::<Vec<_>>(),
                    });
                    std::fs::write(
                        std::path::PathBuf::from(directory).join(format!("{name}-{rt}.facts.json")),
                        serde_json::to_string_pretty(&facts).unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }
}
