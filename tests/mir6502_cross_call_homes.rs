use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config, trace::Mir6502Trace},
};

fn nir(source: &str) -> actionc::nir::NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap()
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
                ..Mir6502Config::optimized()
            };
            let mut trace = Mir6502Trace::default();
            let output =
                mir6502::generate_output_with_trace(&n, origin, &config, runtime, &mut trace)
                    .unwrap();
            if let Ok(directory) = std::env::var("ACTIONC_CROSS_CALL_BASELINE") {
                let path = std::path::Path::new(&directory).join(format!("cross-call-{runtime:?}-{origin:04x}.xex"));
                assert_eq!(actionc::codegen::format_load_file(&output), std::fs::read(path).unwrap());
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
