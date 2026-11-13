use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config, trace::Mir6502Trace},
};

fn nir(source: &str) -> actionc::nir::NirProgram {
    let source = source.replace("\r\n", "\n");
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap()
}

#[test]
fn spill_pruning_is_opt_in_and_respects_terminal_and_master_gates() {
    assert!(!Mir6502Config::default().enable_late_spill_pruning);
    assert!(!Mir6502Config::optimized().enable_late_spill_pruning);
    let input = nir(include_str!("../fixtures/runtime/late_spill_pruning.act"));
    let on = Mir6502Config {
        enable_late_spill_pruning: true,
        ..Mir6502Config::optimized()
    };
    let lower = mir6502::lower_program(&input).unwrap();
    assert_eq!(
        mir6502::materialize_program_with_origin(lower.clone(), &on, 0x3000).unwrap(),
        mir6502::materialize_program_with_origin(lower, &Mir6502Config::optimized(), 0x3000)
            .unwrap()
    );
    for config in [
        Mir6502Config::optimized(),
        Mir6502Config {
            enable_peepholes: false,
            ..on.clone()
        },
        Mir6502Config {
            enable_late_private_store_cleanup: false,
            ..on
        },
    ] {
        let mut trace = Mir6502Trace::default();
        mir6502::generate_output_with_trace(
            &input,
            0x3000,
            &config,
            Runtime::ActionCart,
            &mut trace,
        )
        .unwrap();
        assert!(
            !trace
                .events()
                .iter()
                .any(|e| e.kind == "late-spill-pruning-result")
        );
    }
}

#[test]
fn source_tail_proof_blocker_preserves_output_and_prior_cleanup() {
    let input = nir(include_str!("../fixtures/runtime/late_spill_pruning.act"));
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let off = Mir6502Config::optimized();
            let on = Mir6502Config {
                enable_late_spill_pruning: true,
                ..off.clone()
            };
            let before =
                mir6502::generate_output_with_config_and_runtime(&input, origin, &off, runtime)
                    .unwrap();
            let mut trace = Mir6502Trace::default();
            let after =
                mir6502::generate_output_with_trace(&input, origin, &on, runtime, &mut trace)
                    .unwrap();
            assert_eq!(before, after);
            assert_eq!(
                after,
                mir6502::generate_output_with_config_and_runtime(&input, origin, &on, runtime)
                    .unwrap()
            );
            assert_eq!(trace.manifest()["status"], "complete");
            let events = trace.events();
            let result = &events
                .iter()
                .find(|e| e.kind == "late-spill-pruning-result")
                .unwrap()
                .data;
            assert_eq!(result["candidate_spills"], 1);
            assert_eq!(result["reason"], "ReferencedOrExposed", "{result}");
            assert_eq!(result["removed_spills"], 0);
        }
    }
}
