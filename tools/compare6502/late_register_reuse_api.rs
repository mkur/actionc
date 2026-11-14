//! Compare only late register reuse; keep allocation and upstream selection fixed.
use actionc::{
    compiler::Runtime,
    mir6502::{
        self, Mir6502Config,
        trace::{Mir6502Trace, TraceOptions},
    },
};
use std::{fs, path::PathBuf};
fn main() {
    let destination = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    fs::create_dir_all(&destination).unwrap();
    let mut rows = Vec::new();
    for (name, path) in [
        ("tn", "samples/tn/modern/TN.ACT"),
        ("arithmetic", "fixtures/runtime/late_register_reuse.act"),
        ("pointer", "fixtures/runtime/pointer_byte_plus_one.act"),
        ("staging", "fixtures/runtime/argument_staging.act"),
        ("word", "fixtures/runtime/word_load_sharing.act"),
    ] {
        let loaded = actionc::includes::load_program_with_expanded_source(path).unwrap();
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
                let mut results = Vec::new();
                let mut outputs = Vec::new();
                for enabled in [false, true] {
                    let mut trace = Mir6502Trace::new(TraceOptions {
                        routines: if name == "tn" {
                            vec!["Next".into()]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    });
                    let config = Mir6502Config {
                        enable_late_register_reuse: enabled,
                        enable_small_leaf_inlining: name != "arithmetic",
                        ..Mir6502Config::optimized()
                    };
                    let out = mir6502::generate_output_with_trace(
                        &nir, origin, &config, runtime, &mut trace,
                    )
                    .unwrap();
                    assert_eq!(trace.manifest()["status"], "complete");
                    let image = actionc::codegen::format_load_file(&out);
                    let id = format!("{name}-{runtime:?}-{origin:04x}-{enabled}");
                    fs::write(destination.join(format!("{id}.xex")), &image).unwrap();
                    let routines = out
                        .map
                        .routine_ranges
                        .iter()
                        .map(|r| (r.name.clone(), r.end - r.start))
                        .collect::<std::collections::BTreeMap<_, _>>();
                    results.push(serde_json::json!({ "enabled": enabled, "bytes": image.len(), "routines": routines, "work": trace.manifest()["work"],
                        "decisions": trace.events().iter().filter(|e| e.kind == "late-cleanup-result").map(|e| &e.data).collect::<Vec<_>>() }));
                    outputs.push(out);
                }
                assert!(results[1]["bytes"].as_u64() <= results[0]["bytes"].as_u64());
                assert_eq!(
                    results[0]["work"]["materializations"],
                    results[1]["work"]["materializations"]
                );
                if results[0]["bytes"] == results[1]["bytes"] {
                    assert_eq!(outputs[0], outputs[1]);
                }
                println!(
                    "{name} {runtime:?} {origin:04x}: {} -> {}",
                    results[0]["bytes"], results[1]["bytes"]
                );
                rows.push(serde_json::json!({"case":name,"runtime":format!("{runtime:?}"),"origin":origin,"results":results}));
                fs::write(
                    destination.join("results.json"),
                    serde_json::to_string_pretty(&rows).unwrap(),
                )
                .unwrap();
            }
        }
    }
}
