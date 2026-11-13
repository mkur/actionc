//! Full-fixture comparisons with only terminal cleanup toggled, including T10.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config, trace::Mir6502Trace},
};
use std::{fs, path::Path};

fn main() {
    let destination = std::env::args().nth(1).expect("output directory");
    let destination = Path::new(&destination);
    fs::create_dir(destination).unwrap();
    for (name, source, optimized) in [
        (
            "home-default",
            "fixtures/runtime/home_value_invalidation.act",
            false,
        ),
        ("home", "fixtures/runtime/home_value_invalidation.act", true),
        ("staging", "fixtures/runtime/argument_staging.act", true),
        (
            "ranges",
            "fixtures/runtime/checked_pointer_ranges.act",
            true,
        ),
        ("word", "fixtures/runtime/word_load_sharing.act", true),
    ] {
        let loaded = actionc::includes::load_program_with_expanded_source(source).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &loaded.program,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
            &loaded.program,
            &model,
        ));
        let input = if optimized {
            actionc::nir::optimize_program(&raw).unwrap()
        } else {
            raw
        };
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41c7] {
                let mut off = if optimized {
                    Mir6502Config::optimized()
                } else {
                    Mir6502Config::default()
                };
                off.enable_late_private_store_cleanup = false;
                let on = Mir6502Config {
                    enable_late_private_store_cleanup: true,
                    ..off.clone()
                };
                let mut old_trace = Mir6502Trace::default();
                let before = mir6502::generate_output_with_trace(
                    &input,
                    origin,
                    &off,
                    runtime,
                    &mut old_trace,
                )
                .unwrap();
                let mut trace = Mir6502Trace::default();
                let after =
                    mir6502::generate_output_with_trace(&input, origin, &on, runtime, &mut trace)
                        .unwrap();
                assert_eq!(
                    after,
                    mir6502::generate_output_with_config_and_runtime(&input, origin, &on, runtime)
                        .unwrap()
                );
                assert_eq!(trace.manifest()["status"], "complete");
                let old_image = actionc::codegen::format_load_file(&before);
                let new_image = actionc::codegen::format_load_file(&after);
                assert!(new_image.len() <= old_image.len());
                if new_image.len() == old_image.len() {
                    assert_eq!(before, after);
                }
                if name == "staging" {
                    assert!(new_image.len() < old_image.len());
                }
                let old_work = old_trace.manifest()["work"].clone();
                let new_work = trace.manifest()["work"].clone();
                assert_eq!(old_work["materializations"], new_work["materializations"]);
                assert!(
                    new_work["emission_invocations"].as_u64().unwrap()
                        <= old_work["emission_invocations"].as_u64().unwrap() + 2
                );
                let id = format!("{name}-{runtime:?}-{origin:04x}");
                fs::write(destination.join(format!("{id}-before.xex")), old_image).unwrap();
                fs::write(destination.join(format!("{id}-after.xex")), &new_image).unwrap();
                fs::write(
                    destination.join(format!("{id}-before.debug")),
                    format!("{before:#?}"),
                )
                .unwrap();
                fs::write(
                    destination.join(format!("{id}-after.debug")),
                    format!("{after:#?}"),
                )
                .unwrap();
                let events = trace.events();
                let result = events
                    .iter()
                    .find(|e| e.kind == "late-cleanup-result")
                    .map(|e| e.data.to_string())
                    .unwrap_or("null".into());
                println!(
                    "{{\"id\":\"{id}\",\"before_bytes\":{},\"after_bytes\":{},\"before_work\":{old_work},\"after_work\":{new_work},\"result\":{result}}}",
                    actionc::codegen::format_load_file(&before).len(),
                    new_image.len()
                );
            }
        }
    }
}
