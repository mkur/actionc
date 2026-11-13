//! Full-fixture API probe, including default/optimized output and relocation/map equality.
//! Built by phase_trace.py against the same target directory as actionc-emit.
use actionc::mir6502::{self, Mir6502Config, trace::Mir6502Trace};
use std::{fs, io::Write, path::Path};

fn main() {
    let destination = std::env::args().nth(1).expect("output directory");
    fs::create_dir(&destination).unwrap();
    for (name, source, optimized) in [
        (
            "home",
            "fixtures/runtime/home_value_invalidation.act",
            false,
        ),
        ("staging", "fixtures/runtime/argument_staging.act", false),
        (
            "home-optimized",
            "fixtures/runtime/home_value_invalidation.act",
            true,
        ),
        (
            "ranges-optimized",
            "fixtures/runtime/checked_pointer_ranges.act",
            true,
        ),
    ] {
        let config = if optimized {
            Mir6502Config::optimized()
        } else {
            Mir6502Config::default()
        };
        let source = fs::read_to_string(source).unwrap().replace("\r\n", "\n");
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &ast,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let input = actionc::nir::optimize_program(&actionc::nir::lower_program(
            &actionc::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        for runtime in [
            actionc::compiler::Runtime::ActionCart,
            actionc::compiler::Runtime::Standalone,
        ] {
            let off =
                mir6502::generate_output_with_config_and_runtime(&input, 0x3000, &config, runtime)
                    .unwrap();
            let mut trace = Mir6502Trace::default();
            let on =
                mir6502::generate_output_with_trace(&input, 0x3000, &config, runtime, &mut trace)
                    .unwrap();
            assert_eq!(on, off);
            assert_eq!(trace.manifest()["status"], "complete");
            let id = format!("{name}-{runtime:?}");
            let trace_dir = Path::new(&destination).join(format!("trace-{id}"));
            fs::create_dir(&trace_dir).unwrap();
            fs::write(
                trace_dir.join("manifest.json"),
                trace.manifest().to_string(),
            )
            .unwrap();
            fs::write(
                trace_dir.join("home-summary.json"),
                trace.home_summary().to_string(),
            )
            .unwrap();
            let mut journal = fs::File::create(trace_dir.join("events.jsonl")).unwrap();
            trace.with_events(|events| {
                for event in events {
                    // The event kind is an internal constant; data supplies JSON escaping.
                    writeln!(
                        journal,
                        "{{\"id\":{},\"attempt\":{},\"kind\":\"{}\",\"data\":{}}}",
                        event.id, event.attempt, event.kind, event.data
                    )
                    .unwrap();
                }
            });
            fs::write(
                Path::new(&destination).join(format!("{id}.xex")),
                actionc::codegen::format_load_file(&on),
            )
            .unwrap();
            fs::write(
                Path::new(&destination).join(format!("{id}.debug")),
                format!("{on:#?}"),
            )
            .unwrap();
            fs::write(
                Path::new(&destination).join(format!("{id}.json")),
                trace.manifest().to_string(),
            )
            .unwrap();
        }
    }
}
