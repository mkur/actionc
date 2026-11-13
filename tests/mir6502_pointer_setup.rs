use actionc::{
    compiler::Runtime,
    mir6502::{
        self, Mir6502Config,
        trace::{Mir6502Trace, TraceOptions},
    },
};

fn nir(text: &str) -> actionc::nir::NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap()
}

#[test]
fn pointer_candidates_keep_exact_outcomes_and_trace_off_on_outputs_with_lf_crlf() {
    let lf = include_str!("../fixtures/runtime/pointer_setup_bounds.act").replace("\r\n", "\n");
    let mut applied = 0;
    let mut rejected = 0;
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut reference = None;
            for text in [lf.clone(), lf.replace('\n', "\r\n")] {
                let input = nir(&text);
                let config = Mir6502Config::optimized();
                let mut off = Mir6502Trace::default();
                let before =
                    mir6502::generate_output_with_trace(&input, origin, &config, runtime, &mut off)
                        .unwrap();
                let mut on = Mir6502Trace::new(TraceOptions {
                    core_layout_census: true,
                    ..Default::default()
                });
                let after =
                    mir6502::generate_output_with_trace(&input, origin, &config, runtime, &mut on)
                        .unwrap();
                assert_eq!(before, after);
                assert_eq!(off.manifest()["work"], on.manifest()["work"]);
                assert_eq!(on.manifest()["status"], "complete");
                assert_eq!(on.manifest()["core_layout_census"]["status"], "complete");
                let image = actionc::codegen::format_load_file(&after);
                if let Some(old) = &reference {
                    assert_eq!(old, &image);
                } else {
                    reference = Some(image);
                }
                for event in on.events().iter().filter(|e| e.kind == "core-layout-round") {
                    for c in event.data["candidates"].as_array().unwrap() {
                        let observations: Vec<_> = event.data["observations"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|o| o["observation"]["candidate"] == c["key"])
                            .collect();
                        if c["key"]["family"] == "direct-pointer-temp-rematerialization"
                            && c["outcome"] == "applied"
                        {
                            applied += 1;
                            assert!(
                                observations.is_empty(),
                                "this selector has no layout inputs"
                            );
                        }
                        if c["key"]["family"] == "pointer-temp-deref"
                            && c["outcome"] == "overlap-rejected"
                        {
                            rejected += 1;
                            assert!(!observations.is_empty());
                        }
                    }
                }
            }
        }
    }
    assert!(applied > 0 && rejected > 0);
}
