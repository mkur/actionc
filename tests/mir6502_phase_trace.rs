use actionc::compiler::Runtime;
use actionc::mir6502::{
    self, Mir6502Config,
    trace::{Mir6502Trace, TraceOptions},
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
fn traced_generation_preserves_artifacts_and_captures_the_actual_final_mir() {
    let source =
        include_str!("../fixtures/runtime/checked_pointer_ranges.act").replace("\r\n", "\n");
    for source in [source.clone(), source.replace('\n', "\r\n")] {
        let input = nir(&source);
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut trace = Mir6502Trace::default();
            let mut final_mirs = Vec::new();
            for enabled in [false, true] {
                let config = Mir6502Config {
                    enable_checked_range_pointer_retention: enabled,
                    ..Mir6502Config::optimized()
                };
                let old = mir6502::generate_output_with_config_and_runtime(
                    &input, 0x3000, &config, runtime,
                )
                .unwrap();
                let output = mir6502::generate_output_with_trace(
                    &input, 0x3000, &config, runtime, &mut trace,
                )
                .unwrap();
                assert_eq!(old, output);
                assert_eq!(
                    actionc::codegen::format_load_file(&old),
                    actionc::codegen::format_load_file(&output)
                );
                assert_eq!(trace.manifest()["status"], "complete");
                let events = trace.events();
                assert_eq!(events.iter().filter(|e| e.kind == "compilation").count(), 1);
                let final_mir = events
                    .iter()
                    .find(|e| e.kind == "snapshot" && e.data["stage"] == "pre-emission")
                    .unwrap()
                    .data["mir"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                if !enabled {
                    let expected = mir6502::materialize_program_with_origin_and_runtime(
                        mir6502::lower_program(&input).unwrap(),
                        &config,
                        0x3000,
                        runtime,
                    )
                    .unwrap();
                    assert_eq!(final_mir, mir6502::format_program(&expected));
                }
                final_mirs.push(final_mir);
            }
            assert_ne!(
                final_mirs[0], final_mirs[1],
                "the snapshot must include checked-range retention"
            );
        }
    }
}

#[test]
fn sessions_are_isolated_and_failures_and_truncation_are_explicit() {
    let input = nir("BYTE result\nPROC Main()\n result=7\nRETURN\n");
    let config = Mir6502Config::default();
    let mut full = Mir6502Trace::default();
    let mut limited = Mir6502Trace::new(TraceOptions {
        max_bytes: 1,
        ..Default::default()
    });
    let a = mir6502::generate_output_with_trace(
        &input,
        0x3000,
        &config,
        Runtime::ActionCart,
        &mut full,
    )
    .unwrap();
    let saved = full.manifest();
    let b = mir6502::generate_output_with_trace(
        &input,
        0x3000,
        &config,
        Runtime::ActionCart,
        &mut limited,
    )
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(full.manifest(), saved);
    assert_eq!(limited.manifest()["status"], "truncated");
    let mut invalid = input.clone();
    invalid.target_layout.address_bits = 0;
    let old = mir6502::generate_output_with_config_and_runtime(
        &invalid,
        0x3000,
        &config,
        Runtime::ActionCart,
    );
    let new = mir6502::generate_output_with_trace(
        &invalid,
        0x3000,
        &config,
        Runtime::ActionCart,
        &mut full,
    );
    assert_eq!(old, new);
    assert_eq!(full.manifest()["status"], "compilation-failed");
    assert!(full.manifest()["final_snapshot"].is_null());
}

#[test]
fn nested_trials_have_scoped_results_and_selected_inlining_is_replayed() {
    let input = nir(
        "BYTE input,output BYTE FUNC Map(BYTE value) IF (value AND $80)#0 THEN RETURN((value LSH 1) XOR $1B) FI RETURN(value LSH 1) PROC Main() output=Map(input) RETURN",
    );
    let mut trace = Mir6502Trace::default();
    let config = Mir6502Config::optimized();
    let output = mir6502::generate_output_with_trace(
        &input,
        0x3000,
        &config,
        Runtime::ActionCart,
        &mut trace,
    )
    .unwrap();
    assert_eq!(
        output,
        mir6502::generate_output_with_config_and_runtime(
            &input,
            0x3000,
            &config,
            Runtime::ActionCart
        )
        .unwrap()
    );
    let events = trace.events();
    assert_eq!(trace.manifest()["status"], "complete");
    assert!(
        events
            .iter()
            .any(|e| e.kind == "attempt-start"
                && e.data["stage"] == "inline-final-rematerialization")
    );
    assert!(events.iter().any(|e| e.kind == "inline-selection"));
    for end in events
        .iter()
        .filter(|e| e.kind == "attempt-end" && e.data["returned_snapshot"].is_number())
    {
        let snapshot = events
            .iter()
            .find(|e| Some(e.id) == end.data["returned_snapshot"].as_u64())
            .unwrap();
        assert_eq!(snapshot.attempt, end.attempt);
        assert_eq!(snapshot.data["stage"], "attempt-return");
        for id in end.data["equivalent_source_snapshots"].as_array().unwrap() {
            let source = events.iter().find(|e| Some(e.id) == id.as_u64()).unwrap();
            assert_ne!(source.attempt, end.attempt);
            assert!(source.id < snapshot.id);
            assert_eq!(
                source.data["program_identity"],
                snapshot.data["program_identity"]
            );
        }
    }
}

#[test]
fn home_history_has_typed_scoped_plans_remaps_and_explicit_observations() {
    let mut kinds = std::collections::BTreeSet::new();
    let mut zp = false;
    let mut reused = false;
    let mut repeated_definition = false;
    for source in [
        include_str!("../fixtures/runtime/home_value_invalidation.act"),
        include_str!("../fixtures/runtime/argument_staging.act"),
        include_str!("../fixtures/runtime/word_load_sharing.act"),
        include_str!("../fixtures/runtime/pointer_invariant_loop.act"),
        include_str!("../fixtures/runtime/mir6502_phase_trace_homes.act"),
    ] {
        let input = nir(&source.replace("\r\n", "\n"));
        let mut trace = Mir6502Trace::default();
        mir6502::generate_output_with_trace(
            &input,
            0x3000,
            &Mir6502Config::optimized(),
            Runtime::ActionCart,
            &mut trace,
        )
        .unwrap();
        assert_eq!(trace.manifest()["status"], "complete");
        let events = trace.events();
        for event in &events {
            kinds.insert(event.kind);
            if event.kind == "home-plan" {
                let snapshot = events
                    .iter()
                    .find(|s| Some(s.id) == event.data["snapshot"].as_u64())
                    .unwrap();
                assert_eq!(snapshot.attempt, event.attempt);
                assert_eq!(snapshot.data["stage"], "before-home-assignment");
                let r = snapshot.data["routines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["id"] == event.data["routine"])
                    .unwrap();
                for definition in event.data["definitions"].as_array().unwrap() {
                    let b = r["blocks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|b| b["id"] == definition["block"])
                        .unwrap();
                    assert!(
                        definition["block_parameter"] == true
                            || definition["op"].as_u64().unwrap() < b["ops"].as_u64().unwrap()
                    );
                }
            }
            if event.kind == "home-remap" {
                for field in ["from_snapshot", "to_snapshot"] {
                    let snapshot = events
                        .iter()
                        .find(|s| Some(s.id) == event.data[field].as_u64())
                        .unwrap();
                    assert_eq!(snapshot.attempt, event.attempt);
                    assert!(
                        snapshot.data["routines"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|r| r["id"] == event.data["routine"])
                    );
                }
                let mut destinations = std::collections::BTreeSet::new();
                for entry in event.data["entries"].as_array().unwrap() {
                    assert_eq!(entry["from"]["routine"], event.data["routine"]);
                    assert_eq!(entry["to"]["routine"], event.data["routine"]);
                    zp |= entry["to"]["kind"] == "virtual-zp";
                    reused |= !destinations.insert(entry["to"].to_string());
                    let before = events
                        .iter()
                        .find(|s| Some(s.id) == event.data["from_snapshot"].as_u64())
                        .unwrap();
                    reused |= entry["from"] != entry["to"]
                        && entry["to"]["kind"] == "spill"
                        && before.data["frames"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|f| f["routine"] == event.data["routine"])
                            .any(|f| f["spills"].as_array().unwrap().contains(&entry["to"]));
                }
            }
            if event.kind == "snapshot" {
                for r in event.data["accesses"].as_array().unwrap() {
                    assert_eq!(
                        r["interpretation"],
                        "zero explicit reads is not a deadness proof"
                    );
                    for h in r["homes"].as_array().unwrap() {
                        repeated_definition |= h["home"]["kind"] == "spill"
                            && h["counts"]["writes"].as_u64().unwrap() > 1;
                    }
                }
            }
        }
    }
    for kind in [
        "home-plan",
        "home-assignment",
        "home-elision",
        "home-fate",
        "home-remap",
        "home-prune",
        "abi-home-elision",
    ] {
        assert!(kinds.contains(kind), "missing coverage for {kind}");
    }
    assert!(zp, "zero-page remapping must be observed");
    assert!(
        reused,
        "distinct homes must be observed sharing a colored home"
    );
    assert!(
        repeated_definition,
        "stores to the same home remain distinct points"
    );
}

#[test]
fn final_ranges_join_the_exact_mir_and_expose_zero_byte_consumers() {
    let mut empty_reads = 0;
    for source in [
        include_str!("../fixtures/runtime/argument_staging.act"),
        include_str!("../fixtures/runtime/word_load_sharing.act"),
    ] {
        let mut trace = Mir6502Trace::default();
        mir6502::generate_output_with_trace(
            &nir(&source.replace("\r\n", "\n")),
            0x3000,
            &Mir6502Config::optimized(),
            Runtime::ActionCart,
            &mut trace,
        )
        .unwrap();
        assert_eq!(trace.manifest()["status"], "complete");
        assert!(trace.manifest()["final_layout"].is_number());
        let events = trace.events();
        let final_id = trace.manifest()["final_snapshot"].as_u64().unwrap();
        let snapshot = events.iter().find(|e| e.id == final_id).unwrap();
        let expected: usize = snapshot.data["routines"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r["blocks"].as_array().unwrap())
            .map(|b| b["ops"].as_u64().unwrap() as usize + 1)
            .sum();
        let ranges: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "emitted-range")
            .collect();
        assert_eq!(ranges.len(), expected);
        for range in ranges {
            assert_eq!(range.data["snapshot"], final_id);
            assert!(
                range.data["offset_start"].as_u64().unwrap()
                    <= range.data["offset_end"].as_u64().unwrap()
            );
        }
        let summary = trace.home_summary();
        for r in summary["routines"].as_array().unwrap() {
            for h in r["homes"].as_array().unwrap() {
                empty_reads += h["zero_byte_read_events"].as_array().unwrap().len();
            }
        }
        assert!(events.iter().any(|e| e.kind == "compat-spill-stop"));
        assert!(
            events
                .iter()
                .any(|e| e.kind == "rewrite" && e.data["outcome"] == "applied")
        );
    }
    assert!(empty_reads > 0, "final MIR consumers can emit no bytes");
}

mod cli {
    use serde_json::Value;
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        process::{Command, Output},
    };
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("actionc-phase-trace-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn emit(source: &Path, args: &[&str], trace: Option<&Path>, report: bool) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_actionc-emit"));
        cmd.args(["--profile", "modern", "--backend", "mir6502"]);
        if report {
            cmd.env("ACTIONC_MIR6502_PEEPHOLES", "sites");
        } else {
            cmd.env_remove("ACTIONC_MIR6502_PEEPHOLES");
        }
        if let Some(trace) = trace {
            cmd.arg("--trace-mir6502").arg(trace);
        }
        cmd.args(args).arg(source).output().unwrap()
    }
    fn bundle(path: &Path) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(path)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                (
                    p.file_name().unwrap().to_str().unwrap().to_owned(),
                    fs::read(p).unwrap(),
                )
            })
            .collect()
    }
    fn manifest(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap()
    }
    #[test]
    fn bundles_are_deterministic_and_real_lf_crlf_paths_preserve_outputs() {
        let dir = Scratch::new("determinism");
        let source = "BYTE input,output\nPROC Main()\n output=input+1\nRETURN\n";
        for (label, source) in [
            ("lf", source.to_owned()),
            ("crlf", source.replace('\n', "\r\n")),
        ] {
            let input = dir.path(&format!("{label}.act"));
            fs::write(&input, source).unwrap();
            for (mode, report) in [
                ("--emit-load", false),
                ("--emit-listing", false),
                ("--emit-map", true),
            ] {
                let a = dir.path(&format!("{label}-{mode}-a"));
                let b = dir.path(&format!("{label}-{mode}-b"));
                let off = emit(&input, &[mode], None, report);
                let on = emit(&input, &[mode, "--trace-routine", "Main"], Some(&a), report);
                let again = emit(&input, &[mode, "--trace-routine", "Main"], Some(&b), report);
                assert!(
                    on.status.success(),
                    "{}",
                    String::from_utf8_lossy(&on.stderr)
                );
                assert_eq!(on.stdout, off.stdout);
                assert_eq!(on.stderr, off.stderr);
                assert_eq!(on.stdout, again.stdout);
                assert_eq!(bundle(&a), bundle(&b));
                assert_eq!(manifest(&a)["status"], "complete");
                let journal = fs::read_to_string(a.join("events.jsonl"))
                    .unwrap()
                    .replace("\r\n", "\n");
                for line in journal.lines() {
                    let event: Value = serde_json::from_str(line).unwrap();
                    if let Some(file) = event["data"]["mir_file"].as_str() {
                        assert!(a.join(file).is_file());
                    }
                }
            }
        }
    }
    #[test]
    fn filters_limits_and_frontend_failures_have_explicit_status() {
        let dir = Scratch::new("statuses");
        let source = dir.path("input.act");
        fs::write(&source, "BYTE output\nPROC Main()\n output=7\nRETURN\n").unwrap();
        let off = emit(&source, &["--emit-load"], None, false);
        let limited = dir.path("limited");
        let out = emit(
            &source,
            &["--emit-load", "--trace-max-bytes", "0"],
            Some(&limited),
            false,
        );
        assert!(out.status.success());
        assert_eq!(out.stdout, off.stdout);
        assert_eq!(manifest(&limited)["status"], "truncated");
        let unknown = dir.path("unknown");
        assert!(
            !emit(
                &source,
                &["--trace-routine", "absent"],
                Some(&unknown),
                false
            )
            .status
            .success()
        );
        assert_eq!(manifest(&unknown)["status"], "filter-error");
        let existing = dir.path("existing");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("sentinel"), "keep").unwrap();
        assert!(!emit(&source, &[], Some(&existing), false).status.success());
        assert_eq!(fs::read(existing.join("sentinel")).unwrap(), b"keep");
        for args in [
            vec!["--emit-nir"],
            vec!["--emit-materialized-mir6502"],
            vec!["--backend", "classic"],
        ] {
            let rejected = dir.path("rejected");
            assert!(
                !emit(&source, &args, Some(&rejected), false)
                    .status
                    .success()
            );
            assert!(!rejected.exists());
        }
        assert!(
            !emit(&source, &["--trace-routine", "Main"], None, false)
                .status
                .success()
        );
        fs::write(&source, "PROC Main(???").unwrap();
        let failed = dir.path("failed");
        assert!(!emit(&source, &[], Some(&failed), false).status.success());
        assert_eq!(manifest(&failed)["status"], "compilation-failed");
    }
}
