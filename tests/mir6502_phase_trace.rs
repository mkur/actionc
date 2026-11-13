use actionc::compiler::Runtime;
use actionc::mir6502::{
    self, Mir6502Config,
    trace::{Mir6502Trace, TraceOptions},
};

fn nir(source: &str) -> actionc::nir::NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap()
}

#[test]
fn traced_generation_preserves_artifacts_and_captures_the_actual_final_mir() {
    let source =
        include_str!("../fixtures/runtime/checked_pointer_ranges.act").replace("\r\n", "\n");
    for source in [source.clone(), source.replace('\n', "\r\n")] {
        let input = nir(&source);
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut trace = Mir6502Trace::default();
            let mut final_mirs = Vec::new();
            for enabled in [false, true] {
                let config = Mir6502Config {
                    enable_checked_range_pointer_retention: enabled,
                    ..Mir6502Config::optimized()
                };
                let old = mir6502::generate_output_with_config_and_runtime(
                    &input, 0x3000, &config, runtime,
                )
                .unwrap();
                let output = mir6502::generate_output_with_trace(
                    &input, 0x3000, &config, runtime, &mut trace,
                )
                .unwrap();
                assert_eq!(old, output);
                assert_eq!(
                    actionc::codegen::format_load_file(&old),
                    actionc::codegen::format_load_file(&output)
                );
                assert_eq!(trace.manifest()["status"], "complete");
                let events = trace.events();
                assert_eq!(events.iter().filter(|e| e.kind == "compilation").count(), 1);
                let final_mir = events
                    .iter()
                    .find(|e| e.kind == "snapshot" && e.data["stage"] == "pre-emission")
                    .unwrap()
                    .data["mir"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                if !enabled {
                    let expected = mir6502::materialize_program_with_origin_and_runtime(
                        mir6502::lower_program(&input).unwrap(),
                        &config,
                        0x3000,
                        runtime,
                    )
                    .unwrap();
                    assert_eq!(final_mir, mir6502::format_program(&expected));
                }
                final_mirs.push(final_mir);
            }
            assert_ne!(
                final_mirs[0], final_mirs[1],
                "the snapshot must include checked-range retention"
            );
        }
    }
}

#[test]
fn sessions_are_isolated_and_failures_and_truncation_are_explicit() {
    let input = nir("BYTE result\nPROC Main()\n result=7\nRETURN\n");
    let config = Mir6502Config::default();
    let mut full = Mir6502Trace::default();
    let mut limited = Mir6502Trace::new(TraceOptions {
        max_bytes: 1,
        ..Default::default()
    });
    let a = mir6502::generate_output_with_trace(
        &input,
        0x3000,
        &config,
        Runtime::ActionCart,
        &mut full,
    )
    .unwrap();
    let saved = full.manifest();
    let b = mir6502::generate_output_with_trace(
        &input,
        0x3000,
        &config,
        Runtime::ActionCart,
        &mut limited,
    )
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(full.manifest(), saved);
    assert_eq!(limited.manifest()["status"], "truncated");
    let mut invalid = input.clone();
    invalid.target_layout.address_bits = 0;
    let old = mir6502::generate_output_with_config_and_runtime(
        &invalid,
        0x3000,
        &config,
        Runtime::ActionCart,
    );
    let new = mir6502::generate_output_with_trace(
        &invalid,
        0x3000,
        &config,
        Runtime::ActionCart,
        &mut full,
    );
    assert_eq!(old, new);
    assert_eq!(full.manifest()["status"], "compilation-failed");
    assert!(full.manifest()["final_snapshot"].is_null());
}

#[test]
fn nested_trials_have_scoped_results_and_selected_inlining_is_replayed() {
    let input = nir(
        "BYTE input,output BYTE FUNC Map(BYTE value) IF (value AND $80)#0 THEN RETURN((value LSH 1) XOR $1B) FI RETURN(value LSH 1) PROC Main() output=Map(input) RETURN",
    );
    let mut trace = Mir6502Trace::default();
    let config = Mir6502Config::optimized();
    let output = mir6502::generate_output_with_trace(
        &input,
        0x3000,
        &config,
        Runtime::ActionCart,
        &mut trace,
    )
    .unwrap();
    assert_eq!(
        output,
        mir6502::generate_output_with_config_and_runtime(
            &input,
            0x3000,
            &config,
            Runtime::ActionCart
        )
        .unwrap()
    );
    let events = trace.events();
    assert_eq!(trace.manifest()["status"], "complete");
    assert!(
        events
            .iter()
            .any(|e| e.kind == "attempt-start"
                && e.data["stage"] == "inline-final-rematerialization")
    );
    assert!(events.iter().any(|e| e.kind == "inline-selection"));
    for end in events.iter().filter(|e| e.kind == "attempt-end") {
        let snapshot = events
            .iter()
            .find(|e| Some(e.id) == end.data["returned_snapshot"].as_u64())
            .unwrap();
        assert_eq!(snapshot.attempt, end.attempt);
        assert_eq!(snapshot.data["stage"], "attempt-return");
        for id in end.data["equivalent_source_snapshots"].as_array().unwrap() {
            let source = events.iter().find(|e| Some(e.id) == id.as_u64()).unwrap();
            assert_ne!(source.attempt, end.attempt);
            assert!(source.id < snapshot.id);
            assert_eq!(
                source.data["program_identity"],
                snapshot.data["program_identity"]
            );
        }
    }
}
