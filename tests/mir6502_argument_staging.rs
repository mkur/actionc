use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use std::path::Path;

#[test]
fn argument_staging_neutral_fixture_verifies_and_emits_in_both_runtimes() {
    assert!(!Mir6502Config::default().enable_argument_staging_reduction);
    assert!(Mir6502Config::optimized().enable_argument_staging_reduction);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/argument_staging.act");
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
    actionc::nir::verify_program(&raw).unwrap();
    let optimized = actionc::nir::optimize_program(&raw).unwrap();
    for nir in [&raw, &optimized] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let off = Mir6502Config {
                    enable_argument_staging_reduction: false,
                    ..Mir6502Config::optimized()
                };
                let output =
                    mir6502::generate_output_with_config_and_runtime(nir, origin, &off, runtime)
                        .unwrap();
                for name in ["ForwardAdd", "ForwardSub", "FourByte", "FivePlain"] {
                    let range = output
                        .map
                        .routine_ranges
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap();
                    assert!(range.end > range.start, "{name}");
                }
                assert_eq!(output.origin, origin);
                let mut on = Mir6502Config::optimized();
                on.enable_argument_staging_reduction = true;
                let after =
                    mir6502::generate_output_with_config_and_runtime(nir, origin, &on, runtime)
                        .unwrap();
                assert!(after.bytes.len() < output.bytes.len());
                for name in ["ForwardAdd", "ForwardSub"] {
                    let size = |o: &actionc::codegen::CodegenOutput| {
                        let range = o
                            .map
                            .routine_ranges
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap();
                        range.end - range.start
                    };
                    assert!(size(&after) < size(&output));
                }
            }
        }
    }
}
