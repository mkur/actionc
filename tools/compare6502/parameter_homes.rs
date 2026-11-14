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
    let destination = std::env::args().nth(1).expect("output directory");
    let destination = Path::new(&destination);
    fs::create_dir_all(destination).unwrap();
    for (case, source, inline) in [
        ("tn", "samples/tn/modern/TN.ACT", true),
        (
            "registers",
            "fixtures/runtime/parameter_registers.act",
            false,
        ),
        ("controls", "fixtures/runtime/parameter_homes.act", false),
        (
            "controls-inline",
            "fixtures/runtime/parameter_homes.act",
            true,
        ),
    ] {
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
                    enable_small_leaf_inlining: inline,
                    ..Mir6502Config::optimized()
                };
                let mut trace = Mir6502Trace::new(TraceOptions {
                    routines: vec!["Fnamecmp".into(), "ComparePointers".into()],
                    ..Default::default()
                });
                let output =
                    mir6502::generate_output_with_trace(&nir, origin, &config, runtime, &mut trace)
                        .unwrap();
                let id = format!("{case}-{runtime:?}-{origin:04x}");
                let bytes = actionc::codegen::format_load_file(&output);
                fs::write(destination.join(format!("{id}.xex")), &bytes).unwrap();
                let mut rows = Vec::new();
                for name in [
                    "Fnamecmp",
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
                        let homes: u16 = symbols.iter().map(|s| s.size).sum();
                        rows.push(format!("{{\"name\":\"{name}\",\"code_bytes\":{},\"inline_home_bytes\":{homes}}}", r.end - r.start));
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
                println!(
                    "{{\"id\":\"{id}\",\"image_bytes\":{},\"routines\":[{}],\"parameter_home_outcome\":{outcome},\"work\":{}}}",
                    bytes.len(),
                    rows.join(","),
                    trace.manifest()["work"]
                );
            }
        }
    }
}
