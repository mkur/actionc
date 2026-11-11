use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};

#[test]
fn pointer_byte_plus_one_baseline_verifies_both_nir_modes_runtimes_and_origins() {
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
                    &Mir6502Config::optimized(),
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
                    &Mir6502Config::optimized(),
                    runtime,
                )
                .unwrap();
                assert!(!actionc::codegen::format_load_file(&output).is_empty());
            }
        }
    }
}
