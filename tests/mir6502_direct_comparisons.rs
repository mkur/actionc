use actionc::compiler::Runtime;
use actionc::mir6502::{self, Mir6502Config};
use std::path::Path;

#[test]
fn captured_byte_comparisons_reduce_neutral_images_and_routines() {
    assert!(!Mir6502Config::default().enable_captured_byte_comparisons);
    assert!(Mir6502Config::optimized().enable_captured_byte_comparisons);
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/direct_comparisons.act");
    let loaded = actionc::includes::load_program_with_expanded_source(path).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let semir = actionc::semantic::ir::lower_program(&loaded.program, &model);
    let raw = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&raw).unwrap();
    let optimized = actionc::nir::optimize_program(&raw).unwrap();
    for nir in [&raw, &optimized] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut config = Mir6502Config::optimized();
                config.enable_captured_byte_comparisons = false;
                let before =
                    mir6502::generate_output_with_config_and_runtime(nir, origin, &config, runtime)
                        .unwrap();
                config.enable_captured_byte_comparisons = true;
                let after =
                    mir6502::generate_output_with_config_and_runtime(nir, origin, &config, runtime)
                        .unwrap();
                assert!(
                    after.bytes.len() < before.bytes.len(),
                    "{runtime:?}/{origin:04X}"
                );
                for name in ["AtMost", "Greater", "NamedAtMost"] {
                    let size = |output: &actionc::codegen::CodegenOutput| {
                        let range = output
                            .map
                            .routine_ranges
                            .iter()
                            .find(|r| r.name.eq_ignore_ascii_case(name))
                            .unwrap();
                        range.end - range.start
                    };
                    assert!(
                        size(&after) < size(&before),
                        "{name}/{runtime:?}/{origin:04X}"
                    );
                }
            }
        }
    }
}
