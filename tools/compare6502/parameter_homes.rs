//! Final executable measurements; compile with the current actionc rlib.
use actionc::{
    compiler::Runtime,
    mir6502::{
        self, Mir6502Config,
        trace::{Mir6502Trace, TraceOptions},
    },
};
use std::{fs, path::Path};

fn main() {
    let enabled = std::env::args().any(|a| a == "--enabled");
    let handoff = std::env::args().any(|a| a == "--handoff");
    let destination = std::env::args().nth(1).expect("output directory");
    let destination = Path::new(&destination);
    fs::create_dir_all(destination).unwrap();
    let handoff_suite = std::env::args().any(|a| a == "--suite-handoff");
    let cases: &[(&str, &str, bool)] = if handoff_suite {
        &[
            ("tn", "samples/tn/modern/TN.ACT", true),
            (
                "handoff",
                "fixtures/runtime/parameter_argument_handoff.act",
                false,
            ),
            (
                "handoff-inline",
                "fixtures/runtime/parameter_argument_handoff.act",
                true,
            ),
            (
                "handoff-direct",
                "fixtures/runtime/parameter_argument_handoff_direct.act",
                true,
            ),
        ]
    } else {
        &[
            ("tn", "samples/tn/modern/TN.ACT", true),
            ("scalars", "fixtures/runtime/parameter_scalars.act", false),
            (
                "registers",
                "fixtures/runtime/parameter_registers.act",
                false,
            ),
            ("controls", "fixtures/runtime/parameter_homes.act", false),
            (
                "trace-controls",
                "fixtures/runtime/mir6502_phase_trace_homes.act",
                true,
            ),
            (
                "cleanup-controls",
                "fixtures/runtime/late_producer_cleanup.act",
                true,
            ),
            (
                "controls-inline",
                "fixtures/runtime/parameter_homes.act",
                true,
            ),
        ]
    };
    for &(case, source, inline) in cases {
        let loaded = actionc::includes::load_program_with_expanded_source(source).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &loaded.program,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(
            &actionc::semantic::ir::lower_program(&loaded.program, &model),
        ))
        .unwrap();
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41c7] {
                let config = Mir6502Config {
                    enable_parameter_home_selection: enabled,
                    enable_parameter_argument_handoff: handoff,
                    enable_small_leaf_inlining: inline,
                    ..Mir6502Config::optimized()
                };
                let mut trace = Mir6502Trace::new(TraceOptions {
                    routines: nir
                        .routines
                        .iter()
                        .filter(|r| {
                            matches!(
                                r.name.as_str(),
                                "Fnamecmp"
                                    | "ComparePointers"
                                    | "AdvanceBoth"
                                    | "First"
                                    | "CopyCounted"
                                    | "ForwardWords"
                                    | "Strcpy"
                            )
                        })
                        .map(|r| r.name.clone())
                        .collect(),
                    ..Default::default()
                });
                let started = std::time::Instant::now();
                let output =
                    mir6502::generate_output_with_trace(&nir, origin, &config, runtime, &mut trace)
                        .unwrap();
                let traced_micros = started.elapsed().as_micros();
                assert_eq!(trace.manifest()["status"], "complete");
                let started = std::time::Instant::now();
                let untraced = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                let untraced_micros = started.elapsed().as_micros();
                assert_eq!(output, untraced, "tracing changed the executable");
                let id = format!("{case}-{runtime:?}-{origin:04x}");
                let bytes = actionc::codegen::format_load_file(&output);
                fs::write(destination.join(format!("{id}.xex")), &bytes).unwrap();
                fs::write(
                    destination.join(format!("{id}.map.txt")),
                    format!("{:#?}", output.map),
                )
                .unwrap();
                fs::write(
                    destination.join(format!("{id}.relocations.txt")),
                    format!("{:#?}", output.relocations),
                )
                .unwrap();
                let mut rows = Vec::new();
                for name in [
                    "Fnamecmp",
                    "Strcpy",
                    "Strcat",
                    "Key",
                    "Next",
                    "Swapped",
                    "Repeated",
                    "Computed",
                    "ForwardWords",
                    "ForwardBytes",
                    "CopyCounted",
                    "Mutated",
                    "AfterCall",
                    "Twice",
                    "Exposed",
                    "ExtraEntry",
                    "AdvanceBoth",
                    "ReadWord",
                    "CountDown",
                    "Edges",
                    "First",
                    "Second",
                    "Third",
                    "WordXY",
                    "KeepByte",
                    "StepWord",
                    "ComparePointers",
                    "ExposeWord",
                    "MultiplyWords",
                ] {
                    if let Some(r) = output.map.routine_ranges.iter().find(|r| r.name == name) {
                        let symbols: Vec<_> = output
                            .map
                            .storage_symbols
                            .iter()
                            .filter(|s| {
                                s.scope
                                    == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                                    && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                            })
                            .collect();
                        let homes: u16 = symbols
                            .iter()
                            .filter(|s| s.address >= 256)
                            .map(|s| s.size)
                            .sum();
                        let zp: u16 = symbols
                            .iter()
                            .filter(|s| s.address < 256)
                            .map(|s| s.size)
                            .sum();
                        rows.push(format!("{{\"name\":\"{name}\",\"code_bytes\":{},\"inline_home_bytes\":{homes},\"zp_home_bytes\":{zp}}}", r.end - r.start));
                    }
                }
                let events = trace.events();
                let outcome = events
                    .iter()
                    .rev()
                    .find(|e| {
                        e.kind == "parameter-home-result"
                            || (e.kind == "stage" && e.data["stage"] == "parameter-homes")
                    })
                    .map(|e| e.data.to_string())
                    .unwrap_or("null".into());
                let handoff_outcome = events
                    .iter()
                    .rev()
                    .find(|e| {
                        e.kind == "parameter-argument-handoff-result"
                            || (e.kind == "stage"
                                && e.data["stage"] == "parameter-argument-handoff")
                    })
                    .map(|e| e.data.to_string())
                    .unwrap_or("null".into());
                let upstream = [
                    "inlining",
                    "loop_motion",
                    "comparison_loops",
                    "call_predicates",
                    "address_values",
                    "copy_values",
                    "scalar_captures",
                    "captured_comparisons",
                ]
                .iter()
                .map(|stage| {
                    let starts = events
                        .iter()
                        .filter(|e| e.kind == "attempt-start" && e.data["stage"] == *stage)
                        .count();
                    let choices: Vec<_> = events
                        .iter()
                        .filter(|e| {
                            (e.kind == "selection" && e.data["stage"] == *stage)
                                || (*stage == "inlining" && e.kind == "inline-selection")
                        })
                        .map(|e| e.data.to_string())
                        .collect();
                    format!(
                        "\"{stage}\":{{\"attempts\":{starts},\"choices\":[{}]}}",
                        choices.join(",")
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
                println!(
                    "{{\"id\":\"{id}\",\"image_bytes\":{},\"routines\":[{}],\"parameter_home_outcome\":{outcome},\"handoff_outcome\":{handoff_outcome},\"work\":{},\"upstream\":{{{upstream}}},\"traced_micros\":{traced_micros},\"untraced_micros\":{untraced_micros},\"trace_matches_untraced\":true}}",
                    bytes.len(),
                    rows.join(","),
                    trace.manifest()["work"]
                );
            }
        }
    }
}
