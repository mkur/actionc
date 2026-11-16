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
fn config(enabled: bool) -> Mir6502Config {
    Mir6502Config {
        enable_parameter_home_selection: enabled,
        // Isolate leaf placement work; separate targets cover call composition.
        enable_parameter_argument_handoff: false,
        enable_cross_call_home_selection: false,
        ..Mir6502Config::optimized()
    }
}
fn output(
    n: &actionc::nir::NirProgram,
    enabled: bool,
    runtime: Runtime,
    origin: u16,
) -> (actionc::codegen::CodegenOutput, Mir6502Trace) {
    let mut trace = Mir6502Trace::new(TraceOptions {
        routines: n
            .routines
            .iter()
            .filter(|r| ["Fnamecmp", "Pair", "Subject"].contains(&r.name.as_str()))
            .map(|r| r.name.clone())
            .collect(),
        ..Default::default()
    });
    let output =
        mir6502::generate_output_with_trace(n, origin, &config(enabled), runtime, &mut trace)
            .unwrap();
    (output, trace)
}
fn size(output: &actionc::codegen::CodegenOutput) -> usize {
    actionc::codegen::format_load_file(output).len()
}
#[test]
fn pointer_bindings_preserve_upstream_selections_and_search_counts() {
    let n = nir(
        "BYTE result=$0680\nBYTE FUNC Pair(BYTE POINTER p,q)\n IF p^>q^ THEN RETURN(17) ELSE RETURN(29) FI\nPROC Main() result=Pair($7000,$7100) DO OD RETURN",
    );
    let (old, off) = output(&n, false, Runtime::ActionCart, 0x3000);
    let (new, on) = output(&n, true, Runtime::ActionCart, 0x3000);
    let events = on.events();
    assert!(
        size(&new) < size(&old),
        "{:?}",
        events
            .iter()
            .filter(|e| e.kind == "parameter-home-result")
            .collect::<Vec<_>>()
    );
    assert_eq!(
        on.manifest()["work"]["materializations"].as_u64().unwrap(),
        off.manifest()["work"]["materializations"].as_u64().unwrap() + 1
    );
    for stage in [
        "inlining",
        "loop_motion",
        "comparison_loops",
        "call_predicates",
        "address_values",
        "copy_values",
        "scalar_captures",
        "captured_comparisons",
    ] {
        let count = |trace: &Mir6502Trace| {
            trace
                .events()
                .iter()
                .filter(|e| e.kind == "attempt-start" && e.data["stage"] == stage)
                .count()
        };
        assert_eq!(count(&on), count(&off), "upstream search repeated: {stage}");
    }
    assert_eq!(
        new,
        mir6502::generate_output_with_config_and_runtime(
            &n,
            0x3000,
            &config(true),
            Runtime::ActionCart
        )
        .unwrap()
    );
}
#[test]
fn actual_tn_uses_resident_pointer_pairs_and_saves_final_bytes() {
    let loaded =
        actionc::includes::load_program_with_expanded_source("samples/tn/modern/TN.ACT").unwrap();
    let n = nir(&loaded.source);
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let (old, off) = output(&n, false, runtime, origin);
            let (new, trace) = output(&n, true, runtime, origin);
            assert!(
                size(&new) < size(&old),
                "{:?}",
                trace
                    .events()
                    .iter()
                    .filter(|e| e.kind == "parameter-home-result"
                        || (e.kind == "stage" && e.data["stage"] == "parameter-homes"))
                    .collect::<Vec<_>>()
            );
            let bytes = |p: &actionc::codegen::CodegenOutput| {
                let r = p
                    .map
                    .routine_ranges
                    .iter()
                    .find(|r| r.name == "Fnamecmp")
                    .unwrap();
                r.end - r.start
            };
            assert!(bytes(&new) < bytes(&old));
            assert!(trace.events().iter().any(|e| {
                e.kind == "selection"
                    && e.data["stage"] == "captured_comparisons"
                    && e.data["accepted_routines"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty())
            }));
            let homes: Vec<_> = new
                .map
                .storage_symbols
                .iter()
                .filter(|s| {
                    s.scope == actionc::codegen::CodegenSymbolScope::Routine("Fnamecmp".into())
                        && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                })
                .map(|s| (s.address, s.size))
                .collect();
            assert_eq!(homes, vec![(0xa0, 2), (0xa2, 2)]);
            assert_eq!(trace.manifest()["work"]["parameter_home_trials"], 1);
            let inline_choices = |t: &Mir6502Trace| {
                t.events()
                    .into_iter()
                    .filter(|e| e.kind == "inline-selection")
                    .map(|e| e.data)
                    .collect::<Vec<_>>()
            };

            assert_eq!(inline_choices(&off), inline_choices(&trace));
        }
    }
}
#[test]
fn pointer_scratch_pairs_are_available_only_with_proved_lifetimes() {
    let source = "BYTE ARRAY window(4)=$A0,pool(16)=$E0\nBYTE result=$0680\nBYTE FUNC Pair(CHAR POINTER left,right)\n WHILE left^=right^ DO left==+1 right==+1 OD\nRETURN(left^>right^)\nPROC Main() result=Pair($7000,$7500) DO OD RETURN";
    let n = nir(source);
    let (old, _) = output(&n, false, Runtime::ActionCart, 0x3000);
    let (new, trace) = output(&n, true, Runtime::ActionCart, 0x3000);
    assert!(
        size(&new) < size(&old),
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
        .filter(|s| {
            s.scope == actionc::codegen::CodegenSymbolScope::Routine("Pair".into())
                && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
        })
        .map(|s| (s.address, s.size))
        .collect();
    assert_eq!(homes, vec![(0xaa, 2), (0xae, 2)]);
}
#[test]
fn ordinary_pointee_write_uses_the_existing_object_storage_contract() {
    let n = nir("PROC Subject(BYTE POINTER p BYTE value) p^=value RETURN PROC Main() DO OD RETURN");
    let old = output(&n, false, Runtime::Standalone, 0x3000).0;
    let new = output(&n, true, Runtime::Standalone, 0x3000).0;
    assert!(size(&new) < size(&old));
    // This admission predates value-lifetime placement: native pointee writes
    // already use object-specific address control and physical observation.
    let prior = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
        &Mir6502Config { enable_zp_home_lifetimes: false, ..config(true) }, Runtime::Standalone).unwrap();
    assert_eq!(new, prior);
}

#[test]
fn helper_and_machine_leaves_retain_exact_fallback() {
    for source in [
        "CARD FUNC Subject(CARD value,factor) RETURN(value*factor)",
        "PROC Subject=*(CARD value) [$60]",
    ] {
        let n = nir(&format!("{source}\nPROC Main() DO OD RETURN"));
        assert_eq!(
            output(&n, false, Runtime::Standalone, 0x3000).0,
            output(&n, true, Runtime::Standalone, 0x3000).0
        );
    }
}

#[test]
fn data_dependent_pointer_adjustments_keep_the_existing_fused_strategy() {
    for body in [
        "BYTE FUNC Subject(CHAR POINTER p) p==+p^+3 RETURN(p^)",
        "CARD FUNC Subject(CHAR POINTER p) RETURN(p+p^+5)",
    ] {
        let n = nir(&format!(
            "CARD result=$0680\n{body}\nPROC Main() result=Subject($7000) DO OD RETURN"
        ));
        let compile = |enabled| {
            mir6502::generate_output_with_config_and_runtime(
                &n,
                0x3000,
                &Mir6502Config {
                    enable_small_leaf_inlining: false,
                    ..config(enabled)
                },
                Runtime::ActionCart,
            )
            .unwrap()
        };
        let old = compile(false);
        assert!(old.map.routine_ranges.iter().any(|r| r.name == "Subject"));
        assert_eq!(compile(true), old);
    }
}
