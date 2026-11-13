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
fn late_cleanup_shrinks_argument_staging_and_preserves_trace_equivalence() {
    let input = nir(include_str!("../fixtures/runtime/argument_staging.act"));
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let off = Mir6502Config {
                enable_late_private_store_cleanup: false,
                ..Mir6502Config::optimized()
            };
            let on = Mir6502Config::optimized();
            let mut before_trace = Mir6502Trace::default();
            let before = mir6502::generate_output_with_trace(
                &input,
                origin,
                &off,
                runtime,
                &mut before_trace,
            )
            .unwrap();
            let mut trace = Mir6502Trace::default();
            let after =
                mir6502::generate_output_with_trace(&input, origin, &on, runtime, &mut trace)
                    .unwrap();
            let events = trace.events();
            let result = &events
                .iter()
                .find(|e| e.kind == "late-cleanup-result")
                .unwrap()
                .data;
            assert_eq!(result["accepted"], true, "{runtime:?} {origin:x}: {result}");
            assert!(
                actionc::codegen::format_load_file(&after).len()
                    < actionc::codegen::format_load_file(&before).len()
            );
            assert_eq!(
                after,
                mir6502::generate_output_with_config_and_runtime(&input, origin, &on, runtime)
                    .unwrap()
            );
            assert_eq!(trace.manifest()["status"], "complete");
            let old_work = before_trace.manifest()["work"].clone();
            let new_work = trace.manifest()["work"].clone();
            for (key, count) in old_work.as_object().unwrap() {
                if key.contains("materializ") {
                    assert_eq!(new_work[key], *count, "{key}");
                }
            }
            assert_eq!(
                new_work["emission_invocations"].as_u64().unwrap(),
                old_work["emission_invocations"].as_u64().unwrap() + 2
            );
        }
    }
}

#[test]
fn late_cleanup_master_gate_and_historical_materialized_dump_are_independent() {
    let input = nir(include_str!("../fixtures/runtime/argument_staging.act"));
    let on = Mir6502Config::optimized();
    let off = Mir6502Config {
        enable_late_private_store_cleanup: false,
        ..on.clone()
    };
    let lower = mir6502::lower_program(&input).unwrap();
    assert_eq!(
        mir6502::materialize_program_with_origin(lower.clone(), &on, 0x3000).unwrap(),
        mir6502::materialize_program_with_origin(lower, &off, 0x3000).unwrap()
    );
    let disabled = Mir6502Config {
        enable_peepholes: false,
        ..on
    };
    let mut trace = Mir6502Trace::default();
    mir6502::generate_output_with_trace(&input, 0x3000, &disabled, Runtime::ActionCart, &mut trace)
        .unwrap();
    assert!(
        !trace
            .events()
            .iter()
            .any(|e| e.kind == "late-cleanup-result")
    );
}

#[test]
fn tn_late_cleanup_removes_six_stores_without_extra_materialization() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tn/modern/TN.ACT");
    let loaded = actionc::includes::load_program_with_expanded_source(path).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let input = actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&loaded.program, &model),
    ))
    .unwrap();
    let mut trace = Mir6502Trace::new(mir6502::trace::TraceOptions {
        routines: vec!["FormatDirectoryRow".into(), "SetWin".into()],
        ..Default::default()
    });
    mir6502::generate_output_with_trace(
        &input,
        0x2c00,
        &Mir6502Config::optimized(),
        Runtime::ActionCart,
        &mut trace,
    )
    .unwrap();
    let mut off_trace = Mir6502Trace::new(mir6502::trace::TraceOptions {
        routines: vec!["FormatDirectoryRow".into(), "SetWin".into()],
        ..Default::default()
    });
    mir6502::generate_output_with_trace(
        &input,
        0x2c00,
        &Mir6502Config {
            enable_late_private_store_cleanup: false,
            ..Mir6502Config::optimized()
        },
        Runtime::ActionCart,
        &mut off_trace,
    )
    .unwrap();
    assert_eq!(trace.manifest()["status"], "complete");
    assert_eq!(
        trace.manifest()["work"]["materializations"],
        off_trace.manifest()["work"]["materializations"]
    );
    assert_eq!(
        trace.manifest()["work"]["emission_invocations"]
            .as_u64()
            .unwrap(),
        off_trace.manifest()["work"]["emission_invocations"]
            .as_u64()
            .unwrap()
            + 2
    );
    let events = trace.events();
    let result = &events
        .iter()
        .find(|e| e.kind == "late-cleanup-result")
        .unwrap()
        .data;
    assert_eq!(result["accepted"], true, "{result}");
    assert!(
        result["instruction_bytes_before"].as_u64().unwrap()
            - result["instruction_bytes_after"].as_u64().unwrap()
            >= 18
    );
}
