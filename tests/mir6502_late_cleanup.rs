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
                ..Mir6502Config {
                    enable_parameter_home_selection: false,
                    ..Mir6502Config::optimized()
                }
            };
            let on = Mir6502Config {
                enable_parameter_home_selection: false,
                ..Mir6502Config::optimized()
            };
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
    assert!(!Mir6502Config::default().enable_late_dead_register_cleanup);
    assert!(Mir6502Config::optimized().enable_late_dead_register_cleanup);
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
        &Mir6502Config {
            enable_parameter_home_selection: false,
            ..Mir6502Config::optimized()
        },
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
            ..Mir6502Config {
                enable_parameter_home_selection: false,
                ..Mir6502Config::optimized()
            }
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

#[test]
fn tn_producer_extension_saves_beyond_store_only_cleanup() {
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
    let off = Mir6502Config {
        enable_late_dead_register_cleanup: false,
        ..Mir6502Config::optimized()
    };
    let on = Mir6502Config {
        enable_late_dead_register_cleanup: true,
        ..off.clone()
    };
    let before =
        mir6502::generate_output_with_config_and_runtime(&input, 0x2c00, &off, Runtime::ActionCart)
            .unwrap();
    let mut trace = Mir6502Trace::new(mir6502::trace::TraceOptions {
        routines: vec!["FormatDirectoryRow".into(), "SetWin".into()],
        ..Default::default()
    });
    let after =
        mir6502::generate_output_with_trace(&input, 0x2c00, &on, Runtime::ActionCart, &mut trace)
            .unwrap();
    for (name, address) in [
        ("i", 0xe0),
        ("j", 0xe1),
        ("screen", 0xe6),
        ("scl", 0xe6),
        ("sch", 0xe7),
        ("allocp", 0xe8),
    ] {
        assert!(
            after
                .map
                .storage_symbols
                .iter()
                .any(|s| s.name == name && s.address == address),
            "source ZP home {name} at ${address:02X}"
        );
    }
    let events = trace.events();
    let result = &events
        .iter()
        .find(|e| e.kind == "late-cleanup-result")
        .unwrap()
        .data;
    assert_eq!(result["accepted"], true, "{result}");
    assert!(
        result["removed_producers"].as_u64().unwrap() >= 4,
        "{result}"
    );
    assert!(
        actionc::codegen::format_load_file(&before).len()
            - actionc::codegen::format_load_file(&after).len()
            >= 12
    );
}

#[test]
fn late_producers_remove_independent_pointer_capture_loads() {
    let input = nir(include_str!(
        "../fixtures/runtime/late_producer_cleanup.act"
    ));
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let off = Mir6502Config {
                enable_late_dead_register_cleanup: false,
                ..Mir6502Config::optimized()
            };
            let on = Mir6502Config {
                enable_late_dead_register_cleanup: true,
                ..off.clone()
            };
            let mut old_trace = Mir6502Trace::default();
            let before =
                mir6502::generate_output_with_trace(&input, origin, &off, runtime, &mut old_trace)
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
            assert_eq!(result["accepted"], true, "{result}");
            assert!(
                result["removed_producers"].as_u64().unwrap() >= 2,
                "{result}"
            );
            assert!(
                actionc::codegen::format_load_file(&after).len()
                    < actionc::codegen::format_load_file(&before).len()
            );
            assert_eq!(
                after,
                mir6502::generate_output_with_config_and_runtime(&input, origin, &on, runtime)
                    .unwrap()
            );
            assert_eq!(
                trace.manifest()["work"]["materializations"],
                old_trace.manifest()["work"]["materializations"]
            );
            assert_eq!(trace.manifest()["status"], "complete");
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn late_producer_listings_relocate_with_lf_and_crlf() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/late_producer_cleanup.act");
    let directory = std::env::temp_dir().join(format!(
        "actionc-late-producers-mads-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let outputs = [0x3000, 0x41c7].map(|origin| {
            compile_file(
                &source,
                &CompileOptions::for_mode(CompileMode::Mir6502)
                    .with_origin(origin)
                    .with_runtime(runtime),
            )
            .unwrap()
        });
        for (n, origin) in [0x3000, 0x41c7].into_iter().enumerate() {
            let listing = outputs[0].source_listing().replace("\r\n", "\n").replace(
                "ACTIONC_ORIGIN = $3000",
                &format!("ACTIONC_ORIGIN = ${origin:04X}"),
            );
            for crlf in [false, true] {
                let name = format!("{runtime:?}-{origin:04x}-{crlf}");
                let asm = directory.join(format!("{name}.asm"));
                let image = directory.join(format!("{name}.xex"));
                std::fs::write(
                    &asm,
                    if crlf {
                        listing.replace('\n', "\r\n")
                    } else {
                        listing.clone()
                    },
                )
                .unwrap();
                let result = std::process::Command::new(
                    std::env::var_os("ACTIONC_MADS").unwrap_or_else(|| "mads".into()),
                )
                .arg(&asm)
                .arg(format!("-o:{}", image.display()))
                .arg("-s")
                .output()
                .unwrap();
                assert!(
                    result.status.success(),
                    "{} {}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    std::fs::read(image).unwrap(),
                    outputs[n].object_bytes(),
                    "{name}"
                );
            }
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
