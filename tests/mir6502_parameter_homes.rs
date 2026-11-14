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
fn register_parameters_save_final_bytes_without_upstream_search() {
    let n = nir(include_str!("../fixtures/runtime/parameter_registers.act"));
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut config = Mir6502Config {
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
        "BYTE FUNC Subject(BYTE a,b,c) a==+1 RETURN(a)",
        "BYTE FUNC Subject(BYTE a,b,c) IF b THEN RETURN(a) FI RETURN(c)",
        "CARD FUNC Subject(BYTE a,b,c) RETURN(@a)",
        "BYTE FUNC Subject(BYTE a,b,c,d,e) RETURN(e)",
        "CARD FUNC Subject(BYTE a,b,c) RETURN(a*b)",
    ] {
        let n = nir(&format!("{body}\nPROC Main() DO OD RETURN"));
        let off = Mir6502Config {
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
