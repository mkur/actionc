use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config, trace::Mir6502Trace},
};

fn nir(source: &str) -> actionc::nir::NirProgram {
    let program = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&program, &model),
    ))
    .unwrap()
}

#[test]
fn wrapper_controls_have_stable_artifacts_and_real_callers() {
    let lf =
        include_str!("../fixtures/runtime/parameter_argument_handoff.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut expected: Option<actionc::codegen::CodegenOutput> = None;
            for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                let n = nir(&source);
                let config = Mir6502Config {
                    enable_small_leaf_inlining: false,
                    ..Mir6502Config::optimized()
                };
                let mut trace = Mir6502Trace::default();
                let output =
                    mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut trace)
                        .unwrap();
                assert_eq!(
                    output,
                    mir6502::generate_output_with_config_and_runtime(&n, origin, &config, runtime)
                        .unwrap()
                );
                if let Some(old) = &expected {
                    // Source offsets reflect the physical newline width.
                    assert_eq!(output.bytes, old.bytes);
                    assert_eq!(output.relocations, old.relocations);
                    assert_eq!(output.map.storage_symbols, old.map.storage_symbols);
                    assert_eq!(output.map.routine_ranges, old.map.routine_ranges);
                    assert_eq!(output.run_address, old.run_address);
                } else {
                    expected = Some(output.clone());
                }
                for name in [
                    "ForwardWords",
                    "ForwardBytes",
                    "CopyCounted",
                    "Mutated",
                    "AfterCall",
                    "Twice",
                    "Exposed",
                    "ExtraEntry",
                ] {
                    assert!(output.map.routine_ranges.iter().any(|r| r.name == name));
                }
                assert_eq!(trace.manifest()["status"], "complete");
            }
        }
    }
}

#[test]
fn explicit_source_storage_keeps_its_zero_page_address() {
    let source =
        include_str!("../fixtures/runtime/parameter_argument_handoff.act").replace("\r\n", "\n");
    let source = format!("BYTE reserved=$A2\n{source}");
    for source in [source.clone(), source.replace('\n', "\r\n")] {
        let output = mir6502::generate_output_with_config_and_runtime(
            &nir(&source),
            0x3000,
            &Mir6502Config {
                enable_small_leaf_inlining: false,
                ..Mir6502Config::optimized()
            },
            Runtime::ActionCart,
        )
        .unwrap();
        assert_eq!(
            output
                .map
                .storage_symbols
                .iter()
                .find(|s| s.name == "reserved")
                .unwrap()
                .address,
            0xa2
        );
        let copy: Vec<_> = output
            .map
            .storage_symbols
            .iter()
            .filter(|s| {
                s.scope == actionc::codegen::CodegenSymbolScope::Routine("CopyCounted".into())
            })
            .collect();
        assert_eq!(copy.iter().map(|s| s.size).sum::<u16>(), 4);
        assert!(copy.iter().all(|s| s.address > 255));
    }
}

#[test]
fn gated_no_candidate_adds_no_compiler_work() {
    let n = nir("BYTE FUNC Identity(BYTE value) RETURN(value)");
    for (peepholes, homes) in [(true, true), (true, false), (false, true)] {
        let mut off = Mir6502Trace::default();
        let mut on = Mir6502Trace::default();
        let mut config = Mir6502Config {
            enable_peepholes: peepholes,
            enable_parameter_home_selection: homes,
            enable_parameter_argument_handoff: false,
            ..Mir6502Config::optimized()
        };
        let expected =
            mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut off)
                .unwrap();
        config.enable_parameter_argument_handoff = true;
        let actual =
            mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut on)
                .unwrap();
        assert_eq!(expected, actual);
        assert_eq!(off.manifest()["work"], on.manifest()["work"]);
    }
}
