use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};

#[test]
fn checked_range_regions_save_twenty_bytes_each_and_respect_parent_gating() {
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
                        if !parent {
                            assert_eq!(old, new);
                            continue;
                        }
                        assert_eq!(old.bytes.len() - new.bytes.len(), 40);
                        for name in ["Guarded", "FromReturn", "Unrestricted", "CopyBytes"] {
                            let size = |o: &actionc::codegen::CodegenOutput| {
                                let r = o
                                    .map
                                    .routine_ranges
                                    .iter()
                                    .find(|r| r.name == name)
                                    .unwrap();
                                r.end - r.start
                            };
                            assert_eq!(
                                size(&old) - size(&new),
                                if name == "Guarded" || name == "FromReturn" {
                                    20
                                } else {
                                    0
                                },
                                "{name}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn scratch_observations_hardware_wrap_and_overlapping_code_keep_the_baseline() {
    let lf = include_str!("../fixtures/runtime/checked_pointer_ranges.act").replace("\r\n", "\n");
    let scratch = lf
        .replace("BYTE before, after", "BYTE before, after\nBYTE scratch=$AC")
        .replace("  dst^==+src^", "  dst^==+src^\n  before=scratch");
    for (source, origin) in [
        (scratch, 0x3000),
        (
            lf.replace("FLOOR=$2103, LIMIT=$270B", "FLOOR=$D003, LIMIT=$D70B"),
            0x3000,
        ),
        (
            lf.replace("FLOOR=$2103, LIMIT=$270B", "FLOOR=$FF03, LIMIT=$FF0B"),
            0x3000,
        ),
        (lf, 0x2100),
    ] {
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &ast,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
        for nir in [raw.clone(), actionc::nir::optimize_program(&raw).unwrap()] {
            let mut config = Mir6502Config::optimized();
            config.enable_checked_range_pointer_retention = false;
            let old = mir6502::generate_output_with_config_and_runtime(
                &nir,
                origin,
                &config,
                Runtime::Standalone,
            )
            .unwrap();
            config.enable_checked_range_pointer_retention = true;
            let new = mir6502::generate_output_with_config_and_runtime(
                &nir,
                origin,
                &config,
                Runtime::Standalone,
            )
            .unwrap();
            assert_eq!(old, new, "origin {origin:04X}");
        }
    }
}
