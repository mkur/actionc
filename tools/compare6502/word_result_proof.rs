//! Frozen baseline/current corpus for the ordinary word-result destination proof.
use actionc::{
    compiler::Runtime,
    mir6502::{
        self, Mir6502Config,
        trace::{Mir6502Trace, TraceOptions},
    },
};
use std::{fs, io::Write, path::Path};

fn main() {
    let arg = std::env::args().nth(1).expect("new output directory");
    let root = Path::new(&arg);
    fs::create_dir(root).unwrap();
    let mut cases = vec![
        ("tn", "samples/tn/modern/TN.ACT"),
        ("word", "fixtures/runtime/word_load_sharing.act"),
        ("staging", "fixtures/runtime/argument_staging.act"),
        ("loop", "fixtures/runtime/pointer_invariant_loop.act"),
        ("result", "fixtures/runtime/core_layout_census.act"),
    ];
    if std::env::args().any(|arg| arg == "--pointer-setup") {
        cases.push(("pointer", "fixtures/runtime/pointer_setup_bounds.act"));
    }
    for (name, path) in cases {
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
                let config = Mir6502Config {
                    enable_small_leaf_inlining: name != "result",
                    ..Mir6502Config::optimized()
                };
                let mut off = Mir6502Trace::new(TraceOptions {
                    max_bytes: 128 * 1024 * 1024,
                    ..Default::default()
                });
                let before =
                    mir6502::generate_output_with_trace(&nir, origin, &config, runtime, &mut off)
                        .unwrap();
                let mut on = Mir6502Trace::new(TraceOptions {
                    core_layout_census: true,
                    max_bytes: 128 * 1024 * 1024,
                    ..Default::default()
                });
                let after =
                    mir6502::generate_output_with_trace(&nir, origin, &config, runtime, &mut on)
                        .unwrap();
                assert_eq!(before, after);
                assert_eq!(off.manifest()["work"], on.manifest()["work"]);
                assert_eq!(on.manifest()["status"], "complete");
                assert_eq!(on.manifest()["core_layout_census"]["status"], "complete");
                let id = format!("{name}-{runtime:?}-{origin:04x}");
                let dir = root.join(&id);
                fs::create_dir(&dir).unwrap();
                let image = actionc::codegen::format_load_file(&after);
                fs::write(dir.join("output.xex"), &image).unwrap();
                fs::write(dir.join("output.debug"), format!("{after:#?}")).unwrap();
                fs::write(dir.join("manifest.json"), on.manifest().to_string()).unwrap();
                let mut events =
                    std::io::BufWriter::new(fs::File::create(dir.join("events.jsonl")).unwrap());
                for e in on.events() {
                    writeln!(
                        events,
                        "{{\"id\":{},\"attempt\":{},\"kind\":\"{}\",\"data\":{}}}",
                        e.id, e.attempt, e.kind, e.data
                    )
                    .unwrap();
                }
                println!(
                    "{{\"id\":\"{id}\",\"bytes\":{},\"work\":{},\"census\":{}}}",
                    image.len(),
                    on.manifest()["work"],
                    on.manifest()["core_layout_census"]
                );
            }
        }
    }
}
