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
    let scoped = std::env::args().any(|a| a == "--scoped-observation");
    let scoped_suite = std::env::args().any(|a| a == "--suite-storage-observation");
    let cross = std::env::args().any(|a| a == "--cross-call");
    let destination = std::env::args().nth(1).expect("output directory");
    let destination = Path::new(&destination);
    fs::create_dir_all(destination).unwrap();
    let handoff_suite = std::env::args().any(|a| a == "--suite-handoff");
    let capture_suite = std::env::args().any(|a| a == "--suite-capture");
    let cross_suite = std::env::args().any(|a| a == "--suite-cross-call");
    let cases: &[(&str, &str, bool)] = if scoped_suite {
        &[
            ("tn", "samples/tn/modern/TN.ACT", true),
            ("storage-observation", "fixtures/runtime/storage_observation.act", false),
            ("storage-observation-inline", "fixtures/runtime/storage_observation.act", true),
        ]
    } else if cross_suite {
        &[
            ("tn", "samples/tn/modern/TN.ACT", true),
            ("cross-call", "fixtures/runtime/cross_call_homes.act", false),
            (
                "cross-call-inline",
                "fixtures/runtime/cross_call_homes.act",
                true,
            ),
            (
                "cross-call-locals",
                "fixtures/runtime/cross_call_locals.act",
                false,
            ),
            (
                "cross-call-locals-inline",
                "fixtures/runtime/cross_call_locals.act",
                true,
            ),
        ]
    } else if capture_suite {
        &[
            ("tn", "samples/tn/modern/TN.ACT", true),
            (
                "capture",
                "fixtures/runtime/callee_entry_capture.act",
                false,
            ),
            (
                "capture-inline",
                "fixtures/runtime/callee_entry_capture.act",
                true,
            ),
        ]
    } else if handoff_suite {
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
        if case == "tn" && std::env::args().any(|a| a == "--skip-tn") {
            continue;
        }
        if case != "tn" && std::env::args().any(|a| a == "--tn-only") { continue; }
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
                // The focused suite is one A/B, not a full configuration matrix.
                if scoped_suite && (runtime != Runtime::ActionCart || origin != 0x3000) { continue; }
                let config = Mir6502Config {
                    enable_parameter_home_selection: enabled,
                    enable_parameter_argument_handoff: handoff,
                    enable_cross_call_home_selection: cross,
                    enable_scoped_storage_observation: scoped,
                    enable_small_leaf_inlining: inline,
                    ..Mir6502Config::optimized()
                };
                let mut trace = Mir6502Trace::new(TraceOptions {
                    // This suite includes the four interacting TN routines and
                    // observation provenance in addition to residence snapshots.
                    max_bytes: if scoped_suite { 64 * 1024 * 1024 } else { actionc::mir6502::trace::DEFAULT_MAX_BYTES },
                    routines: nir
                        .routines
                        .iter()
                        .filter(|r| {
                            matches!(
                                r.name.as_str(),
                                "CaptureWords"
                                    | "ReadPointers"
                                    | "ForwardPointers"
                                    | "Fnamecmp"
                                    | "ComparePointers"
                                    | "AdvanceBoth"
                                    | "First"
                                    | "CopyCounted"
                                    | "ForwardWords"
                                    | "Strcpy"
                                    | "MoveMenuBar" | "Draw" | "Tag" | "TagAll" | "Walk" | "Next"
                                    | "Ord"
                                    | "Items"
                                    | "FindRecord"
                                    | "CountRecords"
                                    | "KeepWord"
                                    | "Persistent"
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
                assert_eq!(trace.manifest()["status"], "complete", "{}", trace.manifest());
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
                if cross_suite || scoped_suite {
                    let events = trace.events();
                    let decisions: Vec<_> = events
                        .iter()
                        .filter(|e| e.kind.ends_with("-result") || e.kind == "cross-call-candidate" || (scoped_suite && e.kind.starts_with("late-cleanup-store-")))
                        .collect();
                    fs::write(
                        destination.join(format!("{id}.decisions.txt")),
                        format!("{decisions:#?}"),
                    )
                    .unwrap();
                    fs::write(
                        destination.join(format!("{id}.manifest.json")),
                        trace.manifest().to_string(),
                    )
                    .unwrap();
                }
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
                    "MoveMenuBar", "Draw", "Tag", "TagAll", "Walk", "Observe",
                    "CaptureWord",
                    "ForwardWord",
                    "CaptureWords",
                    "ReadPointers",
                    "ForwardPointers",
                    "ExtraFixed",
                    "ForwardExtra",
                    "Fnamecmp",
                    "Strcpy",
                    "Strcat",
                    "Key",
                    "Next",
                    "NoArgs",
                    "KeptLocal",
                    "SumSteps",
                    "WalkLocal",
                    "EntryLocal",
                    "AddressLocal",
                    "Ord",
                    "Items",
                    "AdvanceRecord",
                    "FindRecord",
                    "CountRecords",
                    "KeepWord",
                    "Conditional",
                    "Persistent",
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
                                    && (s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                                        || ((cross_suite || scoped_suite)
                                            && s.kind
                                                == actionc::codegen::CodegenSymbolKind::Local))
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
                        let fragments = symbols
                            .iter()
                            .map(|s| format!("{{\"address\":{},\"bytes\":{}}}", s.address, s.size))
                            .collect::<Vec<_>>()
                            .join(",");
                        rows.push(format!("{{\"name\":\"{name}\",\"code_bytes\":{},\"inline_home_bytes\":{homes},\"zp_home_bytes\":{zp},\"home_fragments\":[{fragments}]}}", r.end - r.start));
                    }
                }
                if scoped_suite {
                    let mut listings = String::new();
                    for r in &output.map.routine_ranges {
                        listings.push_str(&format!("; {}\n", r.name));
                        listings.push_str(&actionc::codegen::format_listing_with_origin(
                            &output.bytes[usize::from(r.start-origin)..usize::from(r.end-origin)], r.start));
                    }
                    fs::write(destination.join(format!("{id}.listing.txt")), listings).unwrap();
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
                let cross_outcome = events
                    .iter()
                    .rev()
                    .find(|e| e.kind == "cross-call-home-result")
                    .map(|e| e.data.to_string())
                    .unwrap_or("null".into());
                let config_debug = format!("{config:?}");
                fs::write(destination.join(format!("{id}.config.txt")), &config_debug).unwrap();
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
                    "{{\"id\":\"{id}\",\"image_bytes\":{},\"routines\":[{}],\"parameter_home_outcome\":{outcome},\"handoff_outcome\":{handoff_outcome},\"cross_call_outcome\":{cross_outcome},\"work\":{},\"upstream\":{{{upstream}}},\"traced_micros\":{traced_micros},\"untraced_micros\":{untraced_micros},\"trace_matches_untraced\":true}}",
                    bytes.len(),
                    rows.join(","),
                    trace.manifest()["work"]
                );
            }
        }
    }
}
