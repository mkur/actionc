use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config, trace::Mir6502Trace},
};

fn nir(source: &str) -> actionc::nir::NirProgram {
    let program = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&program, &model),
    ))
    .unwrap()
}

#[test]
fn wrapper_controls_have_stable_artifacts_and_real_callers() {
    let lf =
        include_str!("../fixtures/runtime/parameter_argument_handoff.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut expected: Option<actionc::codegen::CodegenOutput> = None;
            for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                let n = nir(&source);
                let config = Mir6502Config {
                    enable_small_leaf_inlining: false,
                    ..Mir6502Config::optimized()
                };
                let mut trace = Mir6502Trace::default();
                let output =
                    mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut trace)
                        .unwrap();
                assert_eq!(
                    output,
                    mir6502::generate_output_with_config_and_runtime(&n, origin, &config, runtime)
                        .unwrap()
                );
                if let Some(old) = &expected {
                    // Source offsets reflect the physical newline width.
                    assert_eq!(output.bytes, old.bytes);
                    assert_eq!(output.relocations, old.relocations);
                    assert_eq!(output.map.storage_symbols, old.map.storage_symbols);
                    assert_eq!(output.map.routine_ranges, old.map.routine_ranges);
                    assert_eq!(output.run_address, old.run_address);
                } else {
                    expected = Some(output.clone());
                }
                for name in [
                    "ForwardWords",
                    "ForwardBytes",
                    "CopyCounted",
                    "Mutated",
                    "AfterCall",
                    "Twice",
                    "Exposed",
                    "ExtraEntry",
                ] {
                    assert!(output.map.routine_ranges.iter().any(|r| r.name == name));
                }
                assert_eq!(trace.manifest()["status"], "complete");
            }
        }
    }
}

#[test]
fn explicit_source_storage_keeps_its_zero_page_address() {
    let source =
        include_str!("../fixtures/runtime/parameter_argument_handoff.act").replace("\r\n", "\n");
    let source = format!("BYTE reserved=$A2\n{source}");
    for source in [source.clone(), source.replace('\n', "\r\n")] {
        let output = mir6502::generate_output_with_config_and_runtime(
            &nir(&source),
            0x3000,
            &Mir6502Config {
                enable_small_leaf_inlining: false,
                enable_parameter_argument_handoff: true,
                ..Mir6502Config::optimized()
            },
            Runtime::ActionCart,
        )
        .unwrap();
        assert_eq!(
            output
                .map
                .storage_symbols
                .iter()
                .find(|s| s.name == "reserved")
                .unwrap()
                .address,
            0xa2
        );
        let copy: Vec<_> = output
            .map
            .storage_symbols
            .iter()
            .filter(|s| {
                s.scope == actionc::codegen::CodegenSymbolScope::Routine("CopyCounted".into())
            })
            .collect();
        assert_eq!(copy.iter().map(|s| s.size).sum::<u16>(), 4);
        assert!(copy.iter().all(|s| s.address > 255));
    }
}

#[test]
fn gated_no_candidate_adds_no_compiler_work() {
    let n = nir("BYTE FUNC Identity(BYTE value) RETURN(value)");
    for (peepholes, homes) in [(true, true), (true, false), (false, true)] {
        let mut off = Mir6502Trace::default();
        let mut on = Mir6502Trace::default();
        let mut config = Mir6502Config {
            enable_peepholes: peepholes,
            enable_parameter_home_selection: homes,
            enable_parameter_argument_handoff: false,
            ..Mir6502Config::optimized()
        };
        let expected =
            mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut off)
                .unwrap();
        config.enable_parameter_argument_handoff = true;
        let actual =
            mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut on)
                .unwrap();
        assert_eq!(expected, actual);
        assert_eq!(off.manifest()["work"], on.manifest()["work"]);
    }
}

#[test]
fn direct_handoff_saves_bytes_and_preserves_fallback_shapes() {
    let source =
        include_str!("../fixtures/runtime/parameter_argument_handoff.act").replace("\r\n", "\n");
    let n = nir(&source);
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            for inline in [false, true] {
                let mut config = Mir6502Config {
                    enable_small_leaf_inlining: inline,
                    enable_parameter_argument_handoff: false,
                    ..Mir6502Config::optimized()
                };
                let mut before = Mir6502Trace::default();
                let old =
                    mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut before)
                        .unwrap();
                config.enable_parameter_argument_handoff = true;
                let mut after = Mir6502Trace::default();
                let new =
                    mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut after)
                        .unwrap();
                assert!(new.bytes.len() < old.bytes.len(), "{:?}", after.manifest());
                assert_eq!(
                    new,
                    mir6502::generate_output_with_config_and_runtime(&n, origin, &config, runtime)
                        .unwrap()
                );
                for name in ["Mutated", "AfterCall", "Twice", "Exposed", "ExtraEntry"] {
                    let a = old
                        .map
                        .routine_ranges
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap();
                    let b = new
                        .map
                        .routine_ranges
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap();
                    assert_eq!(a.end - a.start, b.end - b.start, "{name}");
                }
                let copy = new
                    .map
                    .routine_ranges
                    .iter()
                    .find(|r| r.name == "CopyCounted")
                    .unwrap();
                assert_eq!(copy.end - copy.start, 20);
                assert!(
                    new.map
                        .storage_symbols
                        .iter()
                        .filter(|s| s.scope
                            == actionc::codegen::CodegenSymbolScope::Routine("CopyCounted".into()))
                        .all(|s| s.address < 256)
                );
                assert_eq!(
                    after.manifest()["work"]["parameter_argument_handoff_trials"],
                    1
                );
            }
        }
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn handoff_mads_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    for path in [
        "fixtures/runtime/parameter_argument_handoff.act",
        "fixtures/runtime/parameter_argument_handoff_direct.act",
        "fixtures/runtime/callee_entry_capture.act",
        "samples/tn/modern/TN.ACT",
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        let dir = std::env::temp_dir().join(format!(
            "actionc-parameter-handoff-mads-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let outputs = [0x3000, 0x41C7].map(|origin| {
                compile_file(
                    &source,
                    &CompileOptions::for_mode(CompileMode::Mir6502)
                        .with_origin(origin)
                        .with_runtime(runtime),
                )
                .unwrap()
            });
            for (i, origin) in [0x3000, 0x41C7].into_iter().enumerate() {
                let listing = outputs[0].source_listing().replace("\r\n", "\n").replace(
                    "ACTIONC_ORIGIN = $3000",
                    &format!("ACTIONC_ORIGIN = ${origin:04X}"),
                );
                for crlf in [false, true] {
                    let stem = format!("{runtime:?}-{origin:04X}-{crlf}");
                    let asm = dir.join(format!("{stem}.asm"));
                    let image = dir.join(format!("{stem}.xex"));
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
                        outputs[i].object_bytes(),
                        "{stem}"
                    );
                }
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn optimized_default_and_parent_gates_apply_to_productive_wrappers() {
    assert!(Mir6502Config::optimized().enable_parameter_argument_handoff);
    assert!(!Mir6502Config::default().enable_parameter_argument_handoff);
    let n = nir(
        &include_str!("../fixtures/runtime/parameter_argument_handoff.act").replace("\r\n", "\n"),
    );
    for (peepholes, homes) in [(true, false), (false, true)] {
        let mut config = Mir6502Config {
            enable_peepholes: peepholes,
            enable_parameter_home_selection: homes,
            enable_parameter_argument_handoff: false,
            ..Mir6502Config::optimized()
        };
        let mut before = Mir6502Trace::default();
        let old = mir6502::generate_output_with_trace(
            &n,
            0x3000,
            &config,
            Runtime::ActionCart,
            &mut before,
        )
        .unwrap();
        config.enable_parameter_argument_handoff = true;
        let mut after = Mir6502Trace::default();
        let new = mir6502::generate_output_with_trace(
            &n,
            0x3000,
            &config,
            Runtime::ActionCart,
            &mut after,
        )
        .unwrap();
        assert_eq!(old, new);
        assert_eq!(before.manifest()["work"], after.manifest()["work"]);
    }
}

#[test]
fn ordinary_action_capture_saves_final_bytes_and_cross_newline_artifacts() {
    let lf = include_str!("../fixtures/runtime/callee_entry_capture.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            for inline in [false, true] {
                let mut expected: Option<actionc::codegen::CodegenOutput> = None;
                for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                    let n = nir(&source);
                    let config = Mir6502Config {
                        enable_small_leaf_inlining: inline,
                        ..Mir6502Config::optimized()
                    };
                    let mut trace = Mir6502Trace::default();
                    let out = mir6502::generate_output_with_trace(
                        &n, origin, &config, runtime, &mut trace,
                    )
                    .unwrap();
                    assert_eq!(
                        out,
                        mir6502::generate_output_with_config_and_runtime(
                            &n, origin, &config, runtime
                        )
                        .unwrap()
                    );
                    if let Some(old) = &expected {
                        assert_eq!(out.bytes, old.bytes);
                        assert_eq!(out.map.storage_symbols, old.map.storage_symbols);
                        assert_eq!(out.relocations, old.relocations);
                    } else {
                        expected = Some(out.clone());
                    }
                    assert!(actionc::codegen::format_load_file(&out).len() < 375);
                    assert!(trace.manifest()["work"]["parameter_argument_handoff_trials"] == 1);
                    for name in ["ForwardWords", "ForwardPointers", "ForwardExtra"] {
                        let r = out
                            .map
                            .routine_ranges
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap();
                        if name == "ForwardExtra" {
                            assert_eq!(r.end - r.start, 22);
                        } else {
                            assert!(r.end - r.start < 22, "{name}");
                        }
                        let homes: u16 = out
                            .map
                            .storage_symbols
                            .iter()
                            .filter(|s| {
                                s.scope
                                    == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                                    && s.address >= 256
                            })
                            .map(|s| s.size)
                            .sum();
                        assert_eq!(homes, if name == "ForwardExtra" { 4 } else { 0 }, "{name}");
                    }
                }
            }
        }
    }
}
