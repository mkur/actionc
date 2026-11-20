use actionc::{
    compiler::Runtime,
    mir6502::{
        self, Mir6502Config,
        trace::{Mir6502Trace, TraceOptions},
    },
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
fn reviewed_asm_parameters_use_the_abi_without_reserving_inline_bytes() {
    use actionc::codegen::CodegenSymbolScope;
    let source = format!(
        "{}\nPROC Main() Store($7100,3) RETURN",
        include_str!("../fixtures/runtime/reviewed_fixed_store.act")
    );
    let stale = source.replace("99c57fc17b23abe6", "0000000000000000");
    let no_entry_fact = source
        .replace(";@actionc abi-inputs-only", "")
        .replace(";@actionc entry-reads abi", "");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let mut image_sizes = Vec::new();
        for (src, expected_bytes) in [(&source, 0), (&stale, 3), (&no_entry_fact, 3)] {
            let output = mir6502::generate_output_with_config_and_runtime(
                &nir(src),
                0x3000,
                &Mir6502Config::optimized(),
                runtime,
            )
            .unwrap();
            let scope = CodegenSymbolScope::Routine("Store".into());
            let home_bytes: u16 = output
                .map
                .storage_symbols
                .iter()
                .filter(|s| s.scope == scope)
                .map(|s| s.size)
                .sum();
            assert_eq!(home_bytes, expected_bytes);
            let signature = output
                .map
                .routine_signatures
                .iter()
                .find(|r| r.name == "Store")
                .unwrap();
            assert_eq!(
                signature.params.iter().map(|p| p.width).collect::<Vec<_>>(),
                [2, 1]
            );
            let code = output
                .map
                .routine_ranges
                .iter()
                .find(|r| r.name == "Store")
                .unwrap();
            assert_eq!(
                &output.bytes[usize::from(code.start - output.origin)
                    ..usize::from(code.end - output.origin)],
                &[0x85, 0xa0, 0x86, 0xa1, 0x98, 0xa0, 0x00, 0x91, 0xa0, 0x60]
            );
            image_sizes.push(output.bytes.len());
        }
        assert_eq!(image_sizes[0] + 3, image_sizes[1]);
    }
}

#[test]
fn reviewed_asm_parameter_with_a_data_relocation_keeps_its_home() {
    use mir6502::{
        MirDataImage, MirDataRelocation, MirDataRelocationKind, MirDataRelocationTarget, MirStatic,
        MirStorageBase,
    };
    let n = nir(include_str!(
        "../fixtures/runtime/reviewed_conversion_home.act"
    ));
    let mut program = mir6502::lower_program(&n).unwrap();
    let leaf = program
        .routines
        .iter()
        .find(|r| r.name == "DoubleByte")
        .unwrap();
    let owner = leaf.id;
    let MirStorageBase::Param(id) = leaf.frame.params[0].base else {
        panic!("missing ch")
    };
    program.statics.push(MirStatic {
        id: actionc::nir::SymbolId(1000),
        name: "address".into(),
        ty: "Card".into(),
        image: MirDataImage {
            bytes: vec![0; 2],
            relocations: vec![MirDataRelocation {
                offset: 0,
                kind: MirDataRelocationKind::Word16,
                target: MirDataRelocationTarget::Param { routine: owner, id },
                addend: 0,
                span: actionc::source::Span { start: 0, end: 0 },
            }],
        },
        display: "address".into(),
        alignment: 1,
        mutable: false,
        section: "data".into(),
    });
    let result = mir6502::materialize_program(program, &Mir6502Config::optimized()).unwrap();
    let leaf = result.routines.iter().find(|r| r.id == owner).unwrap();
    assert_eq!(leaf.frame.params[0].base, MirStorageBase::Param(id));
}

#[test]
fn register_parameters_save_final_bytes_without_upstream_search() {
    let n = nir(include_str!("../fixtures/runtime/parameter_registers.act"));
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut config = Mir6502Config {
                enable_parameter_home_selection: false,
                enable_small_leaf_inlining: false,
                ..Mir6502Config::optimized()
            };
            let mut old_trace = Mir6502Trace::new(TraceOptions::default());
            let old =
                mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut old_trace)
                    .unwrap();
            config.enable_parameter_home_selection = true;
            let mut trace = Mir6502Trace::new(TraceOptions::default());
            let new = mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut trace)
                .unwrap();
            assert!(
                actionc::codegen::format_load_file(&new).len()
                    < actionc::codegen::format_load_file(&old).len(),
                "{:?}",
                trace
                    .events()
                    .iter()
                    .filter(|e| e.kind == "parameter-home-result")
                    .collect::<Vec<_>>()
            );
            assert_eq!(trace.manifest()["work"]["parameter_home_trials"], 1);
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
                mir6502::generate_output_with_config_and_runtime(&n, origin, &config, runtime)
                    .unwrap()
            );
            for name in ["First", "Second", "Third", "WordXY"] {
                assert!(!new.map.storage_symbols.iter().any(|s| s.scope
                    == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                    && s.kind == actionc::codegen::CodegenSymbolKind::Parameter));
            }
        }
    }
}

#[test]
fn unsupported_register_lifetimes_keep_complete_fallback() {
    for body in [
        "CARD FUNC Subject(BYTE a,b,c) RETURN(@a)",
        "CARD FUNC Subject(BYTE a,b,c,d,e) RETURN(@e)",
        "CARD FUNC Subject(BYTE a,b,c) RETURN(a*b)",
    ] {
        let n = nir(&format!("{body}\nPROC Main() DO OD RETURN"));
        let off = Mir6502Config {
            enable_parameter_home_selection: false,
            // Exercise leaf selection in isolation. Cross-call selection can
            // place operands across a contracted multiply.
            enable_cross_call_home_selection: false,
            enable_small_leaf_inlining: false,
            ..Mir6502Config::optimized()
        };
        let on = Mir6502Config {
            enable_parameter_home_selection: true,
            ..off.clone()
        };
        let compile = |config: &Mir6502Config| {
            mir6502::generate_output_with_config_and_runtime(
                &n,
                0x3000,
                config,
                Runtime::ActionCart,
            )
            .unwrap()
        };
        assert_eq!(compile(&off), compile(&on), "{body}");
    }
}

#[test]
fn mutable_word_uses_a_reserved_pair_without_inline_storage() {
    let n = nir(include_str!("../fixtures/runtime/parameter_scalars.act"));
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut config = Mir6502Config {
                enable_parameter_home_selection: false,
                enable_small_leaf_inlining: false,
                ..Mir6502Config::optimized()
            };
            let old =
                mir6502::generate_output_with_config_and_runtime(&n, origin, &config, runtime)
                    .unwrap();
            config.enable_parameter_home_selection = true;
            let mut trace = Mir6502Trace::new(TraceOptions::default());
            let new = mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut trace)
                .unwrap();
            assert!(
                actionc::codegen::format_load_file(&new).len()
                    < actionc::codegen::format_load_file(&old).len(),
                "{:?}",
                trace
                    .events()
                    .iter()
                    .filter(|e| e.kind == "parameter-home-result")
                    .collect::<Vec<_>>()
            );
            let homes: Vec<_> = new
                .map
                .storage_symbols
                .iter()
                .filter(|s| s.kind == actionc::codegen::CodegenSymbolKind::Parameter)
                .collect();
            assert!(homes.iter().any(|s| s.scope
                == actionc::codegen::CodegenSymbolScope::Routine("StepWord".into())
                && s.address == 0xa0
                && s.size == 2));
        }
    }
}

#[test]
fn source_exclusions_and_exhaustion_choose_safe_pairs_or_fallback() {
    let fixture = include_str!("../fixtures/runtime/parameter_scalars.act").replace("\r\n", "\n");
    for reservation in [
        "",
        "BYTE reserved=$A2",
        "local",
        "BYTE ARRAY window(4)=$A0,pool(16)=$E0",
    ] {
        let source = if reservation == "local" {
            fixture.replacen(
                "  left==+1",
                "  BYTE reserved=$A2\n  reserved==+1\n  left==+1",
                1,
            )
        } else {
            fixture.replacen("MODULE\n", &format!("MODULE\n{reservation}\n"), 1)
        };
        let n = nir(&source);
        let off = Mir6502Config {
            enable_parameter_home_selection: false,
            enable_small_leaf_inlining: false,
            ..Mir6502Config::optimized()
        };
        let on = Mir6502Config {
            enable_parameter_home_selection: true,
            ..off.clone()
        };
        let mut trace = Mir6502Trace::new(TraceOptions::default());
        let new =
            mir6502::generate_output_with_trace(&n, 0x3000, &on, Runtime::ActionCart, &mut trace)
                .unwrap();
        let old =
            mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &off, Runtime::ActionCart)
                .unwrap();
        if reservation.contains("ARRAY") {
            assert_eq!(new, old);
        } else {
            assert!(
                actionc::codegen::format_load_file(&new).len()
                    < actionc::codegen::format_load_file(&old).len(),
                "{reservation}: {:?}",
                trace
                    .events()
                    .iter()
                    .filter(|e| e.kind == "parameter-home-result")
                    .collect::<Vec<_>>()
            );
            if !reservation.is_empty() {
                assert!(
                    !new.map
                        .storage_symbols
                        .iter()
                        .any(|s| s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                            && s.address <= 0xa2
                            && s.address + s.size > 0xa2)
                );
            }
        }
    }
}

#[test]
fn optimized_policy_respects_the_master_gate_and_historical_dump_boundary() {
    assert!(!Mir6502Config::default().enable_parameter_home_selection);
    assert!(Mir6502Config::optimized().enable_parameter_home_selection);
    let n = nir(include_str!("../fixtures/runtime/parameter_registers.act"));
    let on = Mir6502Config {
        enable_small_leaf_inlining: false,
        ..Mir6502Config::optimized()
    };
    let off = Mir6502Config {
        enable_parameter_home_selection: false,
        ..on.clone()
    };
    let lower = mir6502::lower_program(&n).unwrap();
    assert_eq!(
        mir6502::materialize_program_with_origin(lower.clone(), &on, 0x3000).unwrap(),
        mir6502::materialize_program_with_origin(lower, &off, 0x3000).unwrap(),
    );
    let compile = |enabled| {
        let mut trace = Mir6502Trace::default();
        let output = mir6502::generate_output_with_trace(
            &n,
            0x3000,
            &Mir6502Config {
                enable_peepholes: false,
                enable_parameter_home_selection: enabled,
                ..on.clone()
            },
            Runtime::ActionCart,
            &mut trace,
        )
        .unwrap();
        assert!(trace.manifest()["work"]["parameter_home_trials"].is_null());
        output
    };
    assert_eq!(compile(true), compile(false));
}

#[test]
fn sargs_homes_require_room_for_the_entire_abi_frame() {
    for (count, reservations, fits) in [
        (5, "", true),
        (16, "", true),
        (17, "", false),
        (5, "BYTE a=$E4,b=$E9,c=$EE", false),
    ] {
        let names = (0..count)
            .map(|i| format!("p{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let n = nir(&format!(
            "{reservations}\nBYTE out\nPROC Wide(BYTE {names}) out=p0 RETURN\nPROC Main() RETURN"
        ));
        let output = mir6502::generate_output_with_config_and_runtime(
            &n,
            0x3000,
            &Mir6502Config {
                enable_small_leaf_inlining: false,
                ..Mir6502Config::optimized()
            },
            Runtime::ActionCart,
        )
        .unwrap();
        let entry = output
            .routine_addresses
            .iter()
            .find(|r| r.name == "Wide")
            .unwrap()
            .address;
        let offset = usize::from(entry - output.origin);
        let frame = u16::from_le_bytes([output.bytes[offset + 3], output.bytes[offset + 4]]);
        assert_eq!(output.bytes[offset], 0x20);
        assert_eq!(output.bytes[offset + 5], count - 1);
        assert_eq!(
            (0xe0..=0xf0 - u16::from(count)).contains(&frame),
            fits,
            "{count} {reservations}"
        );
        for i in 0..count {
            assert!(output.map.storage_symbols.iter().any(|s| s.scope
                == actionc::codegen::CodegenSymbolScope::Routine("Wide".into())
                && s.name == format!("p{i}")
                && s.address == frame + u16::from(i)));
        }
    }
}
