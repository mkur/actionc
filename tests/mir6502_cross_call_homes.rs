use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config, trace::Mir6502Trace},
};

fn nir(source: &str) -> actionc::nir::NirProgram {
    nir_mode(source, true)
}

#[test]
fn acyclic_call_homes_coordinate_one_profitable_batch() {
    let source = include_str!("../fixtures/runtime/acyclic_call_homes.act").replace("\r\n", "\n");
    for observable in [false, true] {
        let source = if observable { source.replace("FUNC Middle(", "FUNC Middle=*(") } else { source.clone() };
        let n = nir(&source);
        let config = Mir6502Config { enable_small_leaf_inlining: false,
            enable_acyclic_call_homes: true, ..Mir6502Config::optimized() };
        let mut trace = Mir6502Trace::default();
        let new = mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace).unwrap();
        let old = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
            &Mir6502Config { enable_acyclic_call_homes: false, ..config.clone() }, Runtime::ActionCart).unwrap();
        let home = |routine: &str, name: &str| new.map.storage_symbols.iter().find(|s|
            s.scope == actionc::codegen::CodegenSymbolScope::Routine(routine.into()) && s.name == name).unwrap().address;
        let events = trace.events();
        if !observable {
            let default = Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() };
            assert_eq!(new, mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
                &default, Runtime::ActionCart).unwrap());
        }
        for name in ["keep", "n", "work"] { assert!(home("Hold", name) < 256, "{name} {events:#?}"); }
        if observable { assert!(home("Middle", "x") >= 256 && home("Middle", "c") >= 256); }
        else {
            assert_eq!(home("Middle", "x"), home("Sibling", "x"));
            assert_eq!(home("Middle", "c"), home("Sibling", "c"));
        }
        assert!(actionc::codegen::format_load_file(&new).len() < actionc::codegen::format_load_file(&old).len(), "{events:#?}");
        assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
        println!("acyclic observable={observable}: {} -> {} bytes", actionc::codegen::format_load_file(&old).len(), actionc::codegen::format_load_file(&new).len());
    }
}

#[test]
fn acyclic_call_homes_respect_parent_gates_and_empty_proposals() {
    assert!(!Mir6502Config::default().enable_acyclic_call_homes);
    assert!(Mir6502Config::optimized().enable_acyclic_call_homes);
    let source = include_str!("../fixtures/runtime/acyclic_call_homes.act").replace("\r\n", "\n");
    let n = nir(&source);
    for parent in 0..5 {
        let mut config = Mir6502Config { enable_small_leaf_inlining: false,
            enable_acyclic_call_homes: true, ..Mir6502Config::optimized() };
        match parent {
            0 => config.enable_peepholes = false,
            1 => config.enable_parameter_home_selection = false,
            2 => config.enable_cross_call_home_selection = false,
            3 => config.enable_zp_home_lifetimes = false,
            _ => config.enable_multiple_leaf_call_homes = false,
        }
        let enabled = mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config, Runtime::ActionCart).unwrap();
        config.enable_acyclic_call_homes = false;
        let disabled = mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config, Runtime::ActionCart).unwrap();
        assert_eq!(enabled, disabled, "parent={parent}");
    }
    let mut trace = Mir6502Trace::default();
    mir6502::generate_output_with_trace(&nir("PROC Main() RETURN"), 0x3000,
        &Mir6502Config { enable_acyclic_call_homes: true, ..Mir6502Config::optimized() }, Runtime::ActionCart, &mut trace).unwrap();
    assert_eq!(trace.manifest()["work"]["cross_call_home_trials"].as_u64().unwrap_or(0), 0);
}

#[test]
fn native_current_location_callees_allow_caller_homes_and_spills() {
    let source = include_str!("../fixtures/runtime/native_current_location_homes.act").replace("\r\n", "\n");
    for marker in ["=*", ""] {
        let n = nir(&source.replace("Touch=*", &format!("Touch{marker}")));
        let config = Mir6502Config { enable_small_leaf_inlining: false, enable_acyclic_call_homes: false, ..Mir6502Config::optimized() };
        let mut trace = Mir6502Trace::default();
        let new = mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace).unwrap();
        let old = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
            &Mir6502Config { enable_cross_call_home_selection: false, ..config }, Runtime::ActionCart).unwrap();
        let home = |routine: &str, name: &str| new.map.storage_symbols.iter().find(|s|
            s.scope == actionc::codegen::CodegenSymbolScope::Routine(routine.into()) && s.name == name).unwrap().address;
        for name in ["keep", "n", "work"] { assert!(home("Hold", name) < 256, "{marker} {name}"); }
        if marker == "=*" {
            assert!(home("Touch", "x") >= 256 && home("Touch", "b") >= 256);
            let lowered = mir6502::lower_program(&n).unwrap();
            assert_eq!(lowered.routines.iter().find(|r| r.name == "Touch").unwrap().abi,
                mir6502::MirRoutineAbi::ActionObservable);
        }
        assert!(actionc::codegen::format_load_file(&new).len() < actionc::codegen::format_load_file(&old).len());
        assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
        println!("native current-location {marker}: {} -> {} bytes", actionc::codegen::format_load_file(&old).len(), actionc::codegen::format_load_file(&new).len());
    }
    let spills = include_str!("../fixtures/runtime/cross_call_spills.act").replace("\r\n", "\n")
        .replace("FUNC TouchByte(", "FUNC TouchByte=*(");
    let mut trace = Mir6502Trace::default();
    mir6502::generate_output_with_trace(&nir(&spills), 0x3000,
        &Mir6502Config { enable_small_leaf_inlining: false, enable_acyclic_call_homes: false, ..Mir6502Config::optimized() },
        Runtime::ActionCart, &mut trace).unwrap();
    assert!(trace.events().iter().any(|e| e.kind == "cross-call-spill-home" && e.data["routine"] == "Hold"));

    for (callee, reason) in [
        ("CARD FUNC Leaf(CARD x) RETURN(x+1) CARD FUNC Native=*(CARD x) RETURN(Leaf(x))",
            "nonleaf observable residence entry"),
        ("PROC Native=*(CARD x)\nASM\n rts\nENDASM", "unsupported residence callee"),
    ] {
        let n = nir(&format!("{callee}\nCARD FUNC Hold(CARD keep) Native(keep) RETURN(keep+1)"));
        let config = Mir6502Config { enable_small_leaf_inlining: false, enable_acyclic_call_homes: false, ..Mir6502Config::optimized() };
        let mut trace = Mir6502Trace::default();
        let new = mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace).unwrap();
        let old = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
            &Mir6502Config { enable_cross_call_home_selection: false, ..config }, Runtime::ActionCart).unwrap();
        assert_eq!(new, old);
        assert!(trace.events().iter().any(|e| e.kind == "cross-call-candidate"
            && e.data["routine"] == "Hold" && e.data["reason"] == reason));
    }
}

#[test]
fn value_lifetimes_allow_dead_call_boundaries_and_sibling_reuse() {
    let source = include_str!("../fixtures/runtime/live_call_homes.act").replace("\r\n", "\n");
    let n = nir_mode(&source, false);
    let config = Mir6502Config { enable_small_leaf_inlining: false, enable_acyclic_call_homes: false,
        enable_zp_home_lifetimes: true, ..Mir6502Config::optimized() };
    let mut trace = Mir6502Trace::default();
    let new = mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace).unwrap();
    let old = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
        &Mir6502Config { enable_zp_home_lifetimes: false, ..config }, Runtime::ActionCart).unwrap();
    let events = trace.events();
    assert!(actionc::codegen::format_load_file(&new).len() < actionc::codegen::format_load_file(&old).len(), "{events:#?}");
    println!("live-call control: {} -> {} bytes", actionc::codegen::format_load_file(&old).len(), actionc::codegen::format_load_file(&new).len());
    let home = |routine: &str, name: &str| new.map.storage_symbols.iter().find(|s|
        s.scope == actionc::codegen::CodegenSymbolScope::Routine(routine.into()) && s.name == name).unwrap();
    assert!(home("Short", "keep").address >= 256, "live across unsupported Nested");
    assert!(home("Short", "n").address < 256, "defined after Nested");
    assert!(home("Gone", "keep").address < 256, "dead at Nested");
    assert_eq!(home("Short", "n").address, home("Sibling", "n").address);
    assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
}

#[test]
fn zp_home_lifetimes_default_and_parent_gates_match_explicit_selection() {
    let source = include_str!("../fixtures/runtime/mixed_zp_homes.act").replace("\r\n", "\n");
    let n = nir(&source);
    let default = Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() };
    assert!(default.enable_zp_home_lifetimes);
    let compile = |c: &Mir6502Config| mir6502::generate_output_with_config_and_runtime(&n, 0x3000, c, Runtime::ActionCart).unwrap();
    assert_eq!(compile(&default), compile(&Mir6502Config { enable_zp_home_lifetimes: true, ..default.clone() }));
    for parent in 0..3 {
        let mut config = default.clone();
        match parent {
            0 => config.enable_peepholes = false,
            1 => config.enable_parameter_home_selection = false,
            _ => config.enable_cross_call_home_selection = false,
        }
        let enabled = compile(&config);
        config.enable_zp_home_lifetimes = false;
        assert_eq!(enabled, compile(&config));
    }
}

#[test]
fn loop_carried_spills_join_the_existing_placement_trial() {
    let lf = include_str!("../fixtures/runtime/cross_call_spills.act").replace("\r\n", "\n");
    for (runtime, origin) in [(Runtime::ActionCart, 0x3000), (Runtime::Standalone, 0x41c7)] {
        let mut image = None;
        for source in [lf.clone(), lf.replace('\n', "\r\n")] {
            let n = nir(&source);
            let config = Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() };
            let mut trace = Mir6502Trace::default();
            let new = mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut trace).unwrap();
            let events = trace.events();
            assert!(events.iter().any(|e| e.kind == "cross-call-spill-home" && e.data["routine"] == "Hold"), "{events:#?}");
            let result = events.iter().find(|e| e.kind == "cross-call-home-result").unwrap();
            assert_eq!(result.data["accepted"], true, "{result:?}");
            assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
            let old = mir6502::generate_output_with_config_and_runtime(&n, origin,
                &Mir6502Config { enable_cross_call_home_selection: false, ..config }, runtime).unwrap();
            assert!(new.bytes.len() < old.bytes.len());
            let bytes = actionc::codegen::format_load_file(&new);
            if let Some(previous) = &image { assert_eq!(previous, &bytes); } else { image = Some(bytes); }
        }
    }
}

fn nir_mode(source: &str, optimize: bool) -> actionc::nir::NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let n = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
    if optimize {
        actionc::nir::optimize_program(&n).unwrap()
    } else {
        n
    }
}

#[test]
fn object_storage_indexed_callers_select_resident_homes_in_one_trial() {
    let lf = include_str!("../fixtures/runtime/object_storage.act").replace("\r\n", "\n");
    let mut reference = None;
    for source in [lf.clone(), lf.replace('\n', "\r\n")] {
        let n = nir(&source);
        assert_eq!(n.statics.iter().filter(|s| s.name == "menu").count(), 1);
        assert!(!n.routines.iter().any(|r| r.name == "menu"));
        let p = mir6502::lower_program(&n).unwrap();
        let observed = p.globals.iter().find(|g| g.name == "observed").unwrap();
        assert_eq!(observed.backing, mir6502::MirGlobalBacking::Absolute(0xe6));
        assert!(n.routines.iter().filter(|r| r.name == "ReadFixed")
            .flat_map(|r| &r.blocks).flat_map(|b| &b.ops).any(|op| {
                let actionc::nir::NirOp::ForeignCode { code, .. } = op else { return false; };
                let actionc::nir::NirForeignCodePayload::Bytes { relocations, .. } = &code.payload
                    else { return false; };
                relocations.iter().any(|r| r.required_address_bits == Some(8)
                    && r.target == actionc::nir::NirForeignCodeTarget::Storage(
                        actionc::nir::NirStorageId::Global(observed.id)))
            }), "must exercise the typed fixed-global scanner path");
        let relocs: Vec<_> = n.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter_map(|op| if let actionc::nir::NirOp::ForeignCode { code, .. } = op {
                if let actionc::nir::NirForeignCodePayload::Bytes { relocations, .. } = &code.payload {
                    Some(relocations.as_slice())
                } else { None }
            } else { None }).flatten().collect();
        assert!(relocs.iter().any(|r| r.addend == -1 && matches!(r.target,
            actionc::nir::NirForeignCodeTarget::Storage(actionc::nir::NirStorageId::Local(_)))));
        assert!(relocs.iter().any(|r| r.addend == -2 && matches!(r.target,
            actionc::nir::NirForeignCodeTarget::Storage(actionc::nir::NirStorageId::Global(_)))));
        let vector = p.globals.iter().find(|g| g.name == "vector").unwrap();
        assert!(matches!(vector.backing, mir6502::MirGlobalBacking::Ordinary { .. }));
        assert!(relocs.iter().any(|r| r.target == actionc::nir::NirForeignCodeTarget::Storage(
            actionc::nir::NirStorageId::Global(vector.id))));
        let config = Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() };
        let mut trace = Mir6502Trace::default();
        let output = mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace).unwrap();
        let events: Vec<_> = trace.events().into_iter().filter(|e| e.kind.starts_with("cross-call")).collect();
        if let Ok(path) = std::env::var("ACTIONC_OBJECT_STORAGE_TRACE") {
            std::fs::write(path, serde_json::to_vec_pretty(&trace.events()).unwrap()).unwrap();
        }
        assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1, "{events:?}");
        for name in ["SwapWords", "SwapBytes"] {
            let homes: Vec<_> = output.map.storage_symbols.iter().filter(|s|
                s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                && s.kind == actionc::codegen::CodegenSymbolKind::Local).collect();
            assert!(homes.iter().any(|s| s.address < 256), "{name}: {homes:?}; {events:?}");
            assert!(homes.iter().all(|s| !(s.address..s.address+s.size).any(|a| (0xe4..=0xea).contains(&a))));
        }
        let image = actionc::codegen::format_load_file(&output);
        if let Some(old) = &reference { assert_eq!(old, &image); } else { reference = Some(image); }
        let mut disabled = config;
        disabled.enable_cross_call_home_selection = false;
        let old = mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &disabled, Runtime::ActionCart).unwrap();
        assert!(output.bytes.len() < old.bytes.len());
    }
}

#[test]
fn object_storage_numeric_dispatch_conflict_is_avoided_or_rolls_back() {
    let source = include_str!("../fixtures/runtime/object_storage.act").replace("\r\n", "\n")
        .replace("sta dispatchVector+1", "sta $A1")
        .replace("sta dispatchVector", "sta $A0")
        .replace("jmp (dispatchVector)", "jmp ($A0)");
    let n = nir(&source);
    for lifetimes in [false, true] {
        let config = Mir6502Config { enable_small_leaf_inlining: false,
            enable_zp_home_lifetimes: lifetimes, ..Mir6502Config::optimized() };
        let mut trace = Mir6502Trace::default();
        let selected = mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace).unwrap();
        let events = trace.events();
        let result = events.iter().find(|e| e.kind == "cross-call-home-result").unwrap();
        let baseline = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
            &Mir6502Config { enable_cross_call_home_selection: false, ..config }, Runtime::ActionCart).unwrap();
        if lifetimes {
            assert_eq!(result.data["accepted"], true);
            assert!(selected.bytes.len() < baseline.bytes.len());
            for name in ["SwapWords", "SwapBytes"] {
                let homes: Vec<_> = selected.map.storage_symbols.iter().filter(|s|
                    s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into()) && s.address < 256).collect();
                assert!(!homes.is_empty());
                assert!(homes.iter().all(|s| !(s.address..s.address+s.size).any(|b| [0xa0, 0xa1].contains(&b))));
            }
        } else {
            assert_eq!(result.data["accepted"], false);
            assert_eq!(result.data["reason"], "invalid-trial");
            assert!(result.data["diagnostics"].as_str().unwrap().contains("numeric observation overlaps"));
            assert_eq!(selected, baseline);
        }
        assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
    }
}

#[test]
fn object_storage_missing_numeric_inventory_keeps_independent_callers() {
    let source = include_str!("../fixtures/runtime/object_storage.act").replace("\r\n", "\n")
        .replace("PROC Main()", "CARD FUNC Hold(CARD keep) Touch(7) RETURN(keep) PROC Main()");
    let n = nir(&format!("PROC Opaque() [$02] RETURN\n{source}"));
    let mut trace = Mir6502Trace::default();
    let output = mir6502::generate_output_with_trace(&n, 0x3000,
        &Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() },
        Runtime::ActionCart, &mut trace).unwrap();
    let events = trace.events();
    let observation = &events.iter().find(|e| e.kind == "cross-call-ineligible"
        && e.data["reason"] == "unresolved explicit observation").unwrap().data["observation"];
    assert_eq!(observation["routine"], "Opaque");
    assert!(observation["machine_block"].is_number());
    for name in ["SwapWords", "SwapBytes"] {
        assert!(events.iter().any(|e| e.kind == "cross-call-ineligible"
            && e.data["routine"] == name && e.data["reason"] == "unresolved explicit observation"));
        assert!(output.map.storage_symbols.iter().filter(|s|
            s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into()))
            .all(|s| s.address >= 256));
    }
    assert!(output.map.storage_symbols.iter().any(|s|
        s.scope == actionc::codegen::CodegenSymbolScope::Routine("Hold".into()) && s.address < 256));
    assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
}

#[test]
fn ordinary_call_baseline_has_inline_homes_and_stable_artifacts() {
    let lf = include_str!("../fixtures/runtime/cross_call_homes.act").replace("\r\n", "\n");
    for (runtime, origin) in [(Runtime::ActionCart, 0x3000), (Runtime::Standalone, 0x41c7)] {
        let mut expected: Option<actionc::codegen::CodegenOutput> = None;
        for source in [lf.clone(), lf.replace('\n', "\r\n")] {
            let n = nir(&source);
            let config = Mir6502Config {
                enable_small_leaf_inlining: false,
                enable_cross_call_home_selection: false,
                ..Mir6502Config::optimized()
            };
            let mut trace = Mir6502Trace::default();
            let output =
                mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut trace)
                    .unwrap();
            if let Ok(directory) = std::env::var("ACTIONC_CROSS_CALL_BASELINE") {
                let path = std::path::Path::new(&directory)
                    .join(format!("cross-call-{runtime:?}-{origin:04x}.xex"));
                assert_eq!(
                    actionc::codegen::format_load_file(&output),
                    std::fs::read(path).unwrap()
                );
            }
            assert_eq!(
                output,
                mir6502::generate_output_with_config_and_runtime(&n, origin, &config, runtime)
                    .unwrap()
            );
            if let Some(old) = &expected {
                assert_eq!(output.bytes, old.bytes);
                assert_eq!(output.relocations, old.relocations);
                assert_eq!(output.map.storage_symbols, old.map.storage_symbols);
            } else {
                expected = Some(output.clone());
            }
            for (name, bytes) in [("FindRecord", 5), ("CountRecords", 3), ("KeepWord", 4)] {
                let homes: Vec<_> = output
                    .map
                    .storage_symbols
                    .iter()
                    .filter(|s| {
                        s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                            && matches!(
                                s.kind,
                                actionc::codegen::CodegenSymbolKind::Parameter
                                    | actionc::codegen::CodegenSymbolKind::Local
                            )
                    })
                    .collect();
                assert_eq!(homes.iter().map(|s| s.size).sum::<u16>(), bytes, "{name}");
                assert!(homes.iter().all(|s| s.address >= 256), "{name}");
            }
            assert_eq!(trace.manifest()["status"], "complete");
        }
    }
}

#[test]
fn parameter_residence_saves_final_bytes_in_one_bounded_trial() {
    let n = nir(&include_str!("../fixtures/runtime/cross_call_homes.act").replace("\r\n", "\n"));
    for (runtime, origin) in [(Runtime::ActionCart, 0x3000), (Runtime::Standalone, 0x41c7)] {
        let mut config = Mir6502Config {
            enable_small_leaf_inlining: false,
            enable_cross_call_home_selection: false,
            ..Mir6502Config::optimized()
        };
        let mut old_trace = Mir6502Trace::default();
        let old = mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut old_trace)
            .unwrap();
        config.enable_cross_call_home_selection = true;
        let mut trace = Mir6502Trace::default();
        let new =
            mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut trace).unwrap();
        assert!(
            actionc::codegen::format_load_file(&new).len()
                < actionc::codegen::format_load_file(&old).len(),
            "{:?}",
            trace
                .events()
                .iter()
                .filter(|e| e.kind.starts_with("cross-call"))
                .collect::<Vec<_>>()
        );
        assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
        assert_eq!(
            trace.manifest()["work"]["materializations"]
                .as_u64()
                .unwrap(),
            old_trace.manifest()["work"]["materializations"]
                .as_u64()
                .unwrap()
                + 1
        );
        assert_eq!(
            new,
            mir6502::generate_output_with_config_and_runtime(&n, origin, &config, runtime).unwrap()
        );
        for name in ["FindRecord", "CountRecords", "KeepWord"] {
            let homes: Vec<_> = new
                .map
                .storage_symbols
                .iter()
                .filter(|s| {
                    s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                        && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                })
                .collect();
            assert!(!homes.is_empty(), "{name}");
            assert!(homes.iter().all(|s| s.address < 256), "{name}: {homes:?}");
            if name != "KeepWord" {
                let counter = new
                    .map
                    .storage_symbols
                    .iter()
                    .find(|s| {
                        s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                            && s.kind == actionc::codegen::CodegenSymbolKind::Local
                    })
                    .unwrap();
                assert!(counter.address < 256, "{counter:?}");
            }
        }
    }
}

#[test]
fn unsupported_calls_exposed_objects_and_exhaustion_retain_the_fallback() {
    for source in [
        "CARD FUNC Leaf(CARD x) RETURN(x+1) CARD FUNC Exposed(CARD x) x=Leaf(x) RETURN(@x)",
        "BYTE ARRAY window(4)=$A0,pool(16)=$E0 CARD FUNC Leaf(CARD x) RETURN(x+1) CARD FUNC Subject(CARD x,y) RETURN(Leaf(x)+y)",
        "CARD FUNC Leaf(CARD x) RETURN(x+1)",
        "CARD FUNC Recurse(CARD x) IF x THEN RETURN(Recurse(x-1)+x) FI RETURN(x)",
        "CARD FUNC Helper(CARD x,y) RETURN(x*y) CARD FUNC Caller(CARD x,y) RETURN(Helper(x,3)+x+y)",
        "PROC Machine(BYTE x)\nASM\n rts\nENDASM\nBYTE FUNC Caller(BYTE x) Machine(x) RETURN(x)",
    ] {
        let n = nir(source);
        let config = Mir6502Config {
            enable_small_leaf_inlining: false,
            enable_cross_call_home_selection: false,
            ..Mir6502Config::optimized()
        };
        let mut old_trace = Mir6502Trace::default();
        let old = mir6502::generate_output_with_trace(
            &n,
            0x3000,
            &config,
            Runtime::ActionCart,
            &mut old_trace,
        )
        .unwrap();
        let mut trace = Mir6502Trace::default();
        let new = mir6502::generate_output_with_trace(
            &n,
            0x3000,
            &Mir6502Config {
                enable_cross_call_home_selection: true,
                ..config
            },
            Runtime::ActionCart,
            &mut trace,
        )
        .unwrap();
        assert_eq!(old, new, "{source}");
        assert_eq!(
            trace.manifest()["work"]["materializations"],
            old_trace.manifest()["work"]["materializations"]
        );
    }
}

#[test]
fn raw_nir_keeps_the_same_bounded_proof_and_output_contract() {
    let n = nir_mode(
        &include_str!("../fixtures/runtime/cross_call_homes.act").replace("\r\n", "\n"),
        false,
    );
    let config = Mir6502Config {
        enable_small_leaf_inlining: false,
        enable_cross_call_home_selection: true,
        ..Mir6502Config::optimized()
    };
    let mut trace = Mir6502Trace::default();
    let output =
        mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace)
            .unwrap();
    assert_eq!(
        output,
        mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config, Runtime::ActionCart)
            .unwrap()
    );
    assert!(
        trace.manifest()["work"]["cross_call_home_trials"]
            .as_u64()
            .unwrap_or(0)
            <= 1
    );
}

#[test]
fn nested_callee_keeps_only_the_callers_live_argument_inline() {
    let n = nir(
        "CARD FUNC Leaf(CARD x) RETURN(x+1) CARD FUNC Nested(CARD x) x=Leaf(x) RETURN(x) CARD FUNC Caller(CARD x,y) RETURN(Nested(x)+y)",
    );
    let config = Mir6502Config {
        enable_small_leaf_inlining: false, enable_acyclic_call_homes: false,
        enable_cross_call_home_selection: true,
        ..Mir6502Config::optimized()
    };
    let mut trace = Mir6502Trace::default();
    let output =
        mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace)
            .unwrap();
    let homes: Vec<_> = output
        .map
        .storage_symbols
        .iter()
        .filter(|s| s.scope == actionc::codegen::CodegenSymbolScope::Routine("Caller".into()))
        .collect();
    assert_eq!(homes.len(), 2);
    assert!(homes.iter().find(|s| s.name == "x").unwrap().address < 256);
    assert!(homes.iter().find(|s| s.name == "y").unwrap().address >= 256);
    assert!(
        trace
            .events()
            .iter()
            .any(|e| e.kind == "cross-call-home-ineligible"
                && e.data["routine"] == "Caller"
                && e.data["reason"] == "unsupported residence callee")
    );
}

#[test]
fn initialized_word_and_pointer_locals_share_one_trial_but_entry_reads_keep_storage() {
    let n = nir(&include_str!("../fixtures/runtime/cross_call_locals.act").replace("\r\n", "\n"));
    let config = Mir6502Config {
        enable_small_leaf_inlining: false,
        enable_cross_call_home_selection: true,
        ..Mir6502Config::optimized()
    };
    let mut trace = Mir6502Trace::default();
    let output =
        mir6502::generate_output_with_trace(&n, 0x3000, &config, Runtime::ActionCart, &mut trace)
            .unwrap();
    for (name, relocated) in [
        ("KeptLocal", true),
        ("NoArgs", true),
        ("SumSteps", true),
        ("WalkLocal", true),
        ("EntryLocal", false),
        ("AddressLocal", false),
    ] {
        let local = output
            .map
            .storage_symbols
            .iter()
            .find(|s| {
                s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                    && s.kind == actionc::codegen::CodegenSymbolKind::Local
            })
            .unwrap();
        assert_eq!(
            local.address < 256,
            relocated,
            "{name}: {local:?}; {:?}",
            trace
                .events()
                .iter()
                .filter(|e| e.kind.starts_with("cross-call"))
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
    let old = mir6502::generate_output_with_config_and_runtime(
        &n,
        0x3000,
        &Mir6502Config {
            enable_cross_call_home_selection: false,
            ..config
        },
        Runtime::ActionCart,
    )
    .unwrap();
    assert!(
        actionc::codegen::format_load_file(&output).len()
            < actionc::codegen::format_load_file(&old).len()
    );
}

#[test]
fn optimized_default_saves_bytes_and_disabled_parent_gates_keep_output() {
    let n = nir(&include_str!("../fixtures/runtime/cross_call_homes.act").replace("\r\n", "\n"));
    let config = Mir6502Config::optimized();
    let output =
        mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config, Runtime::ActionCart)
            .unwrap();
    let off = Mir6502Config {
        enable_cross_call_home_selection: false,
        ..config.clone()
    };
    let old =
        mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &off, Runtime::ActionCart)
            .unwrap();
    assert!(
        actionc::codegen::format_load_file(&output).len()
            < actionc::codegen::format_load_file(&old).len()
    );
    // The no-peephole path supports this branch-free call control; abstract
    // Boolean branches require the existing lowering optimizations.
    let guard_n =
        nir("CARD FUNC Leaf(CARD x) RETURN(x+1) CARD FUNC Guard(CARD x,y) RETURN(Leaf(x)+y)");
    for (peepholes, placement) in [(false, true), (true, false)] {
        let config = Mir6502Config {
            enable_peepholes: peepholes,
            enable_small_leaf_inlining: false,
            enable_parameter_home_selection: placement,
            ..config.clone()
        };
        let output = mir6502::generate_output_with_config_and_runtime(
            &guard_n,
            0x3000,
            &config,
            Runtime::ActionCart,
        )
        .unwrap();
        let off = Mir6502Config {
            enable_cross_call_home_selection: false,
            ..config
        };
        assert_eq!(
            output,
            mir6502::generate_output_with_config_and_runtime(
                &guard_n,
                0x3000,
                &off,
                Runtime::ActionCart
            )
            .unwrap()
        );
    }
}

#[test]
#[ignore = "requires MADS assembler on PATH or ACTIONC_MADS"]
fn cross_call_homes_mads_listings_reconstruct_and_relocate_with_lf_and_crlf() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    for fixture in ["cross_call_homes", "cross_call_locals"] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("fixtures/runtime/{fixture}.act"));
        let dir = std::env::temp_dir().join(format!(
            "actionc-cross-call-homes-mads-{}",
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
fn whole_image_cost_accepts_resident_homes_despite_another_routines_growth() {
    let source = include_str!("../fixtures/runtime/storage_observation.act").replace("\r\n", "\n");
    let n = nir(&source);
    // Keep the extra Walk staging store: this isolates cost acceptance from
    // scoped observation's separate proof for deleting that store.
    let baseline_config = Mir6502Config {
        enable_small_leaf_inlining: false,
        enable_scoped_storage_observation: false,
        enable_cross_call_home_selection: false,
        ..Mir6502Config::optimized()
    };
    let baseline = mir6502::generate_output_with_config_and_runtime(
        &n, 0x3000, &baseline_config, Runtime::ActionCart,
    ).unwrap();
    let config = Mir6502Config {
        enable_cross_call_home_selection: true,
        ..baseline_config
    };
    let mut trace = Mir6502Trace::default();
    let selected = mir6502::generate_output_with_trace(
        &n, 0x3000, &config, Runtime::ActionCart, &mut trace,
    ).unwrap();
    let before_bytes = actionc::codegen::format_load_file(&baseline).len();
    let after_bytes = actionc::codegen::format_load_file(&selected).len();
    assert!(after_bytes < before_bytes);

    let code_bytes = |output: &actionc::codegen::CodegenOutput, name: &str| {
        let range = output.map.routine_ranges.iter().find(|r| r.name == name).unwrap();
        range.end - range.start
    };
    let before_walk = code_bytes(&baseline, "Walk");
    let after_walk = code_bytes(&selected, "Walk");
    assert!(after_walk > before_walk, "the control must exercise actual local growth");
    let resident: Vec<_> = selected.map.storage_symbols.iter().filter(|s| {
        s.scope == actionc::codegen::CodegenSymbolScope::Routine("FindRecord".into())
            && matches!(s.kind, actionc::codegen::CodegenSymbolKind::Parameter
                | actionc::codegen::CodegenSymbolKind::Local)
    }).collect();
    assert!(!resident.is_empty());
    assert!(resident.iter().all(|s| s.address < 256), "{resident:?}");

    let events = trace.events();
    let decision = &events.iter().find(|e| e.kind == "cross-call-home-result").unwrap().data;
    assert_eq!(decision["accepted"], true);
    assert_eq!(decision["reason"], "smaller-image");
    assert_eq!(decision["before_bytes"], before_bytes);
    assert_eq!(decision["after_bytes"], after_bytes);
    let walk = decision["routine_cost_changes"].as_array().unwrap().iter()
        .find(|r| r["routine"] == "Walk").unwrap();
    assert_eq!(walk["after"].as_u64().unwrap() - walk["before"].as_u64().unwrap(),
        u64::from(after_walk - before_walk));
    assert_eq!(trace.manifest()["work"]["cross_call_home_trials"], 1);
    assert_eq!(selected, mir6502::generate_output_with_config_and_runtime(
        &n, 0x3000, &config, Runtime::ActionCart,
    ).unwrap());
}

#[test]
fn storage_observation_removes_staging_without_growing_any_routine() {
    let source = include_str!("../fixtures/runtime/storage_observation.act").replace("\r\n", "\n");
    let n = nir(&source);
    for inline in [false, true] {
        let off = Mir6502Config { enable_small_leaf_inlining: inline,
            enable_scoped_storage_observation: false, ..Mir6502Config::optimized() };
        let on = Mir6502Config { enable_scoped_storage_observation: true, ..off.clone() };
        let mut before_trace = Mir6502Trace::default();
        let mut trace = Mir6502Trace::default();
        let before = mir6502::generate_output_with_trace(&n, 0x3000, &off, Runtime::ActionCart, &mut before_trace).unwrap();
        let after = mir6502::generate_output_with_trace(&n, 0x3000, &on, Runtime::ActionCart, &mut trace).unwrap();
        assert!(actionc::codegen::format_load_file(&after).len() < actionc::codegen::format_load_file(&before).len());
        for old in &before.map.routine_ranges {
            let new = after.map.routine_ranges.iter().find(|r| r.name == old.name).unwrap();
            assert!(new.end-new.start <= old.end-old.start, "{} grew", old.name);
        }
        assert_eq!(trace.manifest()["work"]["materializations"], before_trace.manifest()["work"]["materializations"]);
        assert!(trace.manifest()["work"]["storage_observation_snapshot_builds"].as_u64().unwrap_or(0) > 0);
        assert!(after.map.storage_symbols.iter().any(|s| s.name == "n" && s.address < 256
            && s.scope == actionc::codegen::CodegenSymbolScope::Routine("FindRecord".into())));
        assert_eq!(after, mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &on, Runtime::ActionCart).unwrap());
    }
    // A declared source window remains protected, whether it is actively read or not.
    let protected = nir(&source.replace("BYTE reserved0=$E0,reserved1=$E1,sourceOwned=$E6", "BYTE ARRAY ownedPool(16)=$E0"));
    let off = Mir6502Config { enable_small_leaf_inlining: false,
        enable_scoped_storage_observation: false, ..Mir6502Config::optimized() };
    assert_eq!(
        mir6502::generate_output_with_config_and_runtime(&protected, 0x3000, &off, Runtime::ActionCart).unwrap(),
        mir6502::generate_output_with_config_and_runtime(&protected, 0x3000,
            &Mir6502Config { enable_scoped_storage_observation: true, ..off }, Runtime::ActionCart).unwrap());
}

#[test]
fn storage_observation_optimized_default_matches_explicit_gate_and_respects_parents() {
    assert!(!Mir6502Config::default().enable_scoped_storage_observation);
    assert!(Mir6502Config::optimized().enable_scoped_storage_observation);
    let source = include_str!("../fixtures/runtime/storage_observation.act").replace("\r\n", "\n");
    // The public frontend removes uncalled routines. Keep every control reachable.
    let source = source.replace("PROC Main()", "PROC Main() FindRecord($7000,$7000) Walk($7000,0)");
    let n = nir(&source);
    let generate = |n: &actionc::nir::NirProgram, config: &Mir6502Config| {
        mir6502::generate_output_with_config_and_runtime(n, 0x3000, config, Runtime::ActionCart).unwrap()
    };
    let explicit = generate(&n, &Mir6502Config { enable_scoped_storage_observation: true,
        ..Mir6502Config::optimized() });
    let path = std::env::temp_dir().join(format!("actionc-scoped-default-{}.act", std::process::id()));
    std::fs::write(&path, source).unwrap();
    let public = actionc::compiler::compile_file(&path,
        &actionc::compiler::CompileOptions::for_mode(actionc::compiler::CompileMode::Mir6502)
            .with_origin(0x3000).with_runtime(Runtime::ActionCart));
    std::fs::remove_file(path).unwrap();
    let public = public.unwrap();
    assert_eq!(public.object_bytes(), actionc::codegen::format_load_file(&explicit));
    let n = nir("BYTE FUNC Add(BYTE a,b) RETURN(a+b) PROC Main() RETURN");
    for config in [
        Mir6502Config { enable_peepholes: false, ..Mir6502Config::optimized() },
        Mir6502Config { enable_late_private_store_cleanup: false, ..Mir6502Config::optimized() },
    ] {
        assert_eq!(generate(&n, &config), generate(&n,
            &Mir6502Config { enable_scoped_storage_observation: false, ..config }));
    }
}

#[test]
fn multiple_leaf_calls_share_one_batch_and_qualify_the_optimized_default() {
    assert!(!Mir6502Config::default().enable_multiple_leaf_call_homes);
    assert!(Mir6502Config::optimized().enable_multiple_leaf_call_homes);
    let source = include_str!("../fixtures/runtime/multiple_leaf_calls.act").replace("\r\n", "\n");
    let n = nir(&source);
    let config = Mir6502Config {
        enable_small_leaf_inlining: false,
        ..Mir6502Config::optimized()
    };
    let mut old_trace = Mir6502Trace::default();
    let before = mir6502::generate_output_with_trace(&n, 0x3000,
        &Mir6502Config { enable_multiple_leaf_call_homes: false, ..config.clone() },
        Runtime::ActionCart, &mut old_trace).unwrap();
    let mut trace = Mir6502Trace::default();
    let after = mir6502::generate_output_with_trace(&n, 0x3000, &config,
        Runtime::ActionCart, &mut trace).unwrap();
    let decisions: Vec<_> = trace.events().into_iter()
        .filter(|e| e.kind.starts_with("cross-call")).collect();
    assert!(actionc::codegen::format_load_file(&after).len()
        < actionc::codegen::format_load_file(&before).len(), "{decisions:?}");
    for name in ["SumWalk", "Twice", "Once"] {
        for (output, enabled) in [(&before, false), (&after, true)] {
            let homes: Vec<_> = output.map.storage_symbols.iter().filter(|s|
                s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                    && matches!(s.kind, actionc::codegen::CodegenSymbolKind::Parameter
                        | actionc::codegen::CodegenSymbolKind::Local)).collect();
            assert!(!homes.is_empty(), "{name}");
            assert!(homes.iter().all(|s| (s.address < 256) == (enabled || name == "Once")),
                "{name}, enabled={enabled}: {homes:?}; {decisions:?}");
        }
        let count = if name == "Once" { 1 } else { 2 };
        assert!(decisions.iter().any(|e| e.kind == "cross-call-candidate"
            && e.data["routine"] == name && e.data["call_count"] == count));
    }
    for session in [&old_trace, &trace] {
        assert_eq!(session.manifest()["work"]["cross_call_home_trials"], 1);
        assert!(session.events().iter().any(|e| e.kind == "cross-call-home-result"
            && e.data["accepted"] == true));
    }
    assert_eq!(old_trace.manifest()["work"]["materializations"],
        trace.manifest()["work"]["materializations"]);
    assert_eq!(after, mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
        &Mir6502Config { enable_multiple_leaf_call_homes: true, ..config },
        Runtime::ActionCart).unwrap());
}

#[test]
fn multiple_leaf_calls_respect_all_parent_gates() {
    let n = nir("CARD FUNC Leaf(CARD x) RETURN(x+1) CARD FUNC Twice(CARD x) x=Leaf(x) x=Leaf(x) RETURN(x)");
    for parent in 0..3 {
        let mut config = Mir6502Config { enable_small_leaf_inlining: false,
            ..Mir6502Config::optimized() };
        match parent {
            0 => config.enable_peepholes = false,
            1 => config.enable_parameter_home_selection = false,
            _ => config.enable_cross_call_home_selection = false,
        }
        let mut trace = Mir6502Trace::default();
        let enabled = mir6502::generate_output_with_trace(&n, 0x3000, &config,
            Runtime::ActionCart, &mut trace).unwrap();
        config.enable_multiple_leaf_call_homes = false;
        let disabled = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
            &config, Runtime::ActionCart).unwrap();
        assert_eq!(enabled, disabled);
        assert_eq!(trace.manifest()["work"]["cross_call_home_trials"].as_u64().unwrap_or(0), 0);
    }
}
