use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};

#[test]
fn inert_checked_range_projection_preserves_images_and_parent_gating() {
    assert!(!Mir6502Config::default().enable_checked_range_pointer_retention);
    assert!(!Mir6502Config::optimized().enable_checked_range_pointer_retention);
    let lf = include_str!("../fixtures/runtime/checked_pointer_ranges.act").replace("\r\n", "\n");
    for source in [lf.clone(), lf.replace('\n', "\r\n")] {
        let tokens = actionc::lexer::tokenize(&source).unwrap();
        let ast = actionc::parser::parse(&tokens).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &ast,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
        let optimized = actionc::nir::optimize_program(&raw).unwrap();
        for nir in [&raw, &optimized] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                for origin in [0x3000, 0x41C7] {
                    for parent in [false, true] {
                        let mut config = Mir6502Config {
                            enable_pointer_call_retention: parent,
                            ..Mir6502Config::optimized()
                        };
                        let old = mir6502::generate_output_with_config_and_runtime(
                            nir, origin, &config, runtime,
                        )
                        .unwrap();
                        config.enable_checked_range_pointer_retention = true;
                        let new = mir6502::generate_output_with_config_and_runtime(
                            nir, origin, &config, runtime,
                        )
                        .unwrap();
                        assert_eq!(old, new);
                    }
                }
            }
        }
    }
}
