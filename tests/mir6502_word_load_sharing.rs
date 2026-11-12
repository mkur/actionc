use actionc::{
    codegen,
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};

#[test]
fn word_load_sharing_disabled_matches_baseline_and_enabled_shares_both_destinations() {
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
                    &Mir6502Config {
                        enable_word_load_sharing: false,
                        ..Mir6502Config::optimized()
                    },
                    origin,
                    runtime,
                )
                .unwrap();
                mir6502::verify_program(&p, mir6502::MirPhase::PreEmission).unwrap();
                let output = mir6502::generate_output_with_config_and_runtime(
                    &nir,
                    origin,
                    &Mir6502Config {
                        enable_word_load_sharing: false,
                        ..Mir6502Config::optimized()
                    },
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
                    let frozen: serde_json::Value = serde_json::from_slice(
                        &std::fs::read(
                            std::path::PathBuf::from(directory)
                                .join(format!("{name}-{rt}.facts.json")),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                    assert_eq!(
                        facts, frozen,
                        "frozen frame/storage/reservation facts changed"
                    );
                }
                let config = Mir6502Config {
                    enable_word_load_sharing: true,
                    ..Mir6502Config::optimized()
                };
                let selected = mir6502::materialize_program_with_origin_and_runtime(
                    mir6502::lower_program(&nir).unwrap(),
                    &config,
                    origin,
                    runtime,
                )
                .unwrap();
                mir6502::verify_program(&selected, mir6502::MirPhase::PreEmission).unwrap();
                for (a, b) in p.routines.iter().zip(&selected.routines) {
                    assert_eq!(a.frame, b.frame);
                    assert_eq!(
                        a.blocks.iter().map(|b| b.id).collect::<Vec<_>>(),
                        b.blocks.iter().map(|b| b.id).collect::<Vec<_>>()
                    );
                    for (a, b) in a.blocks.iter().zip(&b.blocks) {
                        assert_eq!(a.terminator, b.terminator);
                    }
                }
                assert_eq!(p.globals, selected.globals);
                assert_eq!(p.statics, selected.statics);
                let after = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                assert_eq!(output.skipped_ranges, after.skipped_ranges);
                if let Some(directory) = std::env::var_os("ACTIONC_WORD_LOAD_AFTER") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let rt = if runtime == Runtime::ActionCart {
                        "cart"
                    } else {
                        "standalone"
                    };
                    std::fs::write(
                        directory.join(format!("{stem}-{rt}-{origin:04X}.mir")),
                        mir6502::format_program(&selected),
                    )
                    .unwrap();
                    std::fs::write(
                        directory.join(format!("{stem}-{rt}-{origin:04X}.xex")),
                        codegen::format_load_file(&after),
                    )
                    .unwrap();
                    std::fs::write(
                        directory.join(format!("{stem}-{rt}-{origin:04X}.lst")),
                        codegen::format_listing_with_origin(&after.bytes, after.origin),
                    )
                    .unwrap();
                }
                let expected_saving = if stem == "neutral" { 12 } else { 24 };
                assert_eq!(
                    image.len() - codegen::format_load_file(&after).len(),
                    expected_saving,
                    "{stem}/{runtime:?}/{origin:04X}"
                );
                for before in &output.map.routine_ranges {
                    if matches!(
                        before.name.as_str(),
                        "Unlink" | "Reset" | "Alloc" | "Free" | "AllocInit"
                    ) {
                        let changed = after
                            .map
                            .routine_ranges
                            .iter()
                            .find(|r| r.name == before.name)
                            .unwrap();
                        assert_eq!(
                            (before.end - before.start) - (changed.end - changed.start),
                            if before.name == "Free" { 12 } else { 6 },
                            "{}",
                            before.name
                        );
                    }
                }
            }
        }
    }
}
