//! Reproducible typed MIR and physical budgets with explicit reuse controls.
use actionc::{includes, mir6502, nir, semantic};
use std::{env, fs, path::PathBuf};

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    assert!(
        args.len() >= 2,
        "SOURCE OUTPUT_PREFIX [standalone] [raw] [off|reads|writes|parameters] [sites]"
    );
    let prefix = PathBuf::from(&args[1]);
    let runtime = if args.iter().any(|a| a == "standalone") {
        actionc::runtime::Runtime::Standalone
    } else {
        actionc::runtime::Runtime::ActionCart
    };
    let loaded = includes::load_program_with_expanded_source(&args[0]).unwrap();
    let model =
        semantic::analyze_with_options(&loaded.program, semantic::SemanticOptions::modern())
            .unwrap();
    let sem = semantic::ir::lower_program(&loaded.program, &model);
    let raw = nir::lower_program(&sem);
    let normalized = if args.iter().any(|a| a == "raw") {
        raw
    } else {
        nir::optimize_program(&raw).unwrap()
    };
    let pre = mir6502::lower_program(&normalized).unwrap();
    let mut config = mir6502::Mir6502Config::optimized();
    if args
        .iter()
        .any(|a| a == "reads" || a == "writes" || a == "parameters")
    {
        config.enable_record_base_reuse = true;
        config.enable_record_base_writes = args.iter().any(|a| a == "writes" || a == "parameters");
        config.enable_record_base_parameter_origins = args.iter().any(|a| a == "parameters");
    }
    if args.iter().any(|a| a == "off") {
        config.enable_record_base_reuse = false;
    }
    if args.iter().any(|a| a == "sites") {
        config.peephole_report = mir6502::MirPeepholeReportMode::Sites;
    }
    let post =
        mir6502::materialize_program_with_origin_and_runtime(pre.clone(), &config, 0x2C00, runtime)
            .unwrap();
    fs::write(
        prefix.with_extension("pre.mir"),
        mir6502::format_program(&pre),
    )
    .unwrap();
    fs::write(
        prefix.with_extension("post.mir"),
        mir6502::format_program(&post),
    )
    .unwrap();
    let output =
        mir6502::generate_output_with_config_and_runtime(&normalized, 0x2C00, &config, runtime)
            .unwrap();
    let routines = output.map.routine_ranges.iter().map(|r| serde_json::json!({"name":r.name,"start":r.start,"end":r.end,"bytes":r.end-r.start})).collect::<Vec<_>>();
    let loaded_size = |slots: &[mir6502::MirStorageSlot]| {
        slots
            .iter()
            .filter(|s| {
                matches!(
                    s.base,
                    mir6502::MirStorageBase::Local(_) | mir6502::MirStorageBase::Param(_)
                )
            })
            .map(|s| s.storage_size as usize)
            .sum::<usize>()
    };
    let homes = post.routines.iter().map(|r| serde_json::json!({"routine":r.name,"spill_bytes":r.frame.spills.len(),"parameter_bytes":loaded_size(&r.frame.params),"local_bytes":loaded_size(&r.frame.locals),"fixed_zero_page":r.frame.fixed_zero_page.iter().map(|s|s.0).collect::<Vec<_>>()})).collect::<Vec<_>>();
    fs::write(prefix.with_extension("bin"), &output.bytes).unwrap();
    fs::write(prefix.with_extension("json"), serde_json::to_string_pretty(&serde_json::json!({"payload_bytes":output.bytes.len(),"routines":routines,"homes":homes})).unwrap()+"\n").unwrap();
}
