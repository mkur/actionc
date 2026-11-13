use actionc::{
    compiler::Runtime,
    mir6502::{
        self, Mir6502Config,
        trace::{Mir6502Trace, TraceOptions},
    },
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
fn census_preserves_artifacts_work_and_real_lf_crlf_parsing() {
    let source = include_str!("../fixtures/runtime/core_layout_census.act").replace("\r\n", "\n");
    let mut observed_applied = false;
    let mut observed_predicate = false;
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let input = nir(&source);
            let config = Mir6502Config {
                enable_small_leaf_inlining: false,
                ..Mir6502Config::optimized()
            };
            let mut off = Mir6502Trace::default();
            let before =
                mir6502::generate_output_with_trace(&input, origin, &config, runtime, &mut off)
                    .unwrap();
            for text in [source.clone(), source.replace('\n', "\r\n")] {
                let input = nir(&text);
                // Host line endings legitimately change source byte spans.
                // Compare full metadata within the same source, and executable
                // bytes across LF/CRLF inputs through the actual parser.
                let mut same_source_trace = Mir6502Trace::default();
                let same_source = mir6502::generate_output_with_trace(
                    &input,
                    origin,
                    &config,
                    runtime,
                    &mut same_source_trace,
                )
                .unwrap();
                let mut on = Mir6502Trace::new(TraceOptions {
                    core_layout_census: true,
                    ..Default::default()
                });
                let after =
                    mir6502::generate_output_with_trace(&input, origin, &config, runtime, &mut on)
                        .unwrap();
                assert_eq!(same_source, after);
                assert_eq!(
                    actionc::codegen::format_load_file(&before),
                    actionc::codegen::format_load_file(&after)
                );
                assert_eq!(off.manifest()["work"], on.manifest()["work"]);
                assert_eq!(on.manifest()["status"], "complete");
                assert_eq!(on.manifest()["core_layout_census"]["status"], "complete");
                for event in on.events().iter().filter(|e| e.kind == "core-layout-round") {
                    if event.data["applied"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|a| a["family"] == "known-callee-word-result-placement")
                    {
                        observed_applied = true;
                        observed_predicate |= event.data["observations"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|x| {
                                x["observation"]["predicate"] == "equal-address"
                                    && x["observation"]["result"] == false
                                    && x["observation"]["inputs"].as_array().unwrap().iter().any(
                                        |i| {
                                            i["planning_address"] == true
                                                && i["home"]["kind"] == "global"
                                        },
                                    )
                            });
                    }
                }
            }
            assert!(
                !off.events()
                    .iter()
                    .any(|e| e.kind.starts_with("core-layout-"))
            );
        }
    }
    assert!(observed_applied && observed_predicate);
}

#[test]
fn truncation_and_filters_do_not_change_codegen_or_silently_certify_coverage() {
    let input = nir("BYTE a\nCARD b\nPROC Main()\nb=a LSH 1\nRETURN\n");
    let config = Mir6502Config::optimized();
    let before = mir6502::generate_output_with_config_and_runtime(
        &input,
        0x3000,
        &config,
        Runtime::ActionCart,
    )
    .unwrap();
    for max_bytes in [1, 32 * 1024 * 1024] {
        let mut trace = Mir6502Trace::new(TraceOptions {
            core_layout_census: true,
            routines: vec!["Main".into()],
            max_bytes,
        });
        let after = mir6502::generate_output_with_trace(
            &input,
            0x3000,
            &config,
            Runtime::ActionCart,
            &mut trace,
        )
        .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            trace.manifest()["status"],
            if max_bytes == 1 {
                "truncated"
            } else {
                "complete"
            }
        );
    }
}

#[test]
fn cli_requires_trace_destination_and_emits_census_without_changing_the_image() {
    use std::process::Command;
    let binary = env!("CARGO_BIN_EXE_actionc-emit");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/word_load_sharing.act");
    let bad = Command::new(binary)
        .args([
            "--trace-core-layout",
            "--profile",
            "modern",
            "--backend",
            "mir6502",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("require --trace-mir6502"));
    let dir = std::env::temp_dir().join(format!("actionc-layout-census-{}", std::process::id()));
    let plain = Command::new(binary)
        .args(["--profile", "modern", "--backend", "mir6502", "--emit-load"])
        .arg(&source)
        .output()
        .unwrap();
    let traced = Command::new(binary)
        .args([
            "--profile",
            "modern",
            "--backend",
            "mir6502",
            "--emit-load",
            "--trace-core-layout",
            "--trace-mir6502",
        ])
        .arg(&dir)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        plain.status.success() && traced.status.success(),
        "{}",
        String::from_utf8_lossy(&traced.stderr)
    );
    assert_eq!(plain.stdout, traced.stdout);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["core_layout_census"]["enabled"], true);
    assert_eq!(manifest["status"], "complete");
    assert!(
        std::fs::read_to_string(dir.join("events.jsonl"))
            .unwrap()
            .contains("core-layout-round")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
