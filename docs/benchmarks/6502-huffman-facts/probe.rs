// Audit only, temporarily included inside analysis::known_callees.
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/benchmarks/6502-call-facts-audit/probe.rs"));

fn extra_inventory(p: &MirProgram) -> Value {
    json!(p.routines.iter().map(|r| json!({
        "id": r.id.0, "name": r.name, "frame": format!("{:#?}", r.frame),
        "ops": r.blocks.iter().flat_map(|b| b.ops.iter().enumerate().map(move |(i, op)| json!({
            "block": b.id.0, "op": i, "instruction": format!("{op:?}"),
            "effects": format!("{:#?}", classify_op(op)),
        }))).collect::<Vec<_>>(),
    })).collect::<Vec<_>>())
}

#[test]
#[ignore = "diagnostic audit, temporary test-only include"]
fn audit_huffman() {
    let root = std::path::Path::new("build/huffman-facts");
    let portable = "fixtures/runtime/tacle/huff_dec/huff_dec.act";
    for (label, path, origin, inlining) in [
        ("portable", portable, 0x2000, true),
        ("driver", "build/huffman-facts/driver/huff_dec.act", 0x8000, true),
        ("portable-no-inline", portable, 0x2000, false),
    ] {
        let loaded = crate::includes::load_program_with_expanded_source(path).unwrap();
        let model = crate::semantic::analyze_with_options(&loaded.program,
            crate::semantic::SemanticOptions::modern()).unwrap();
        let n = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&loaded.program, &model))).unwrap();
        let config = mir::Mir6502Config { enable_small_leaf_inlining: inlining,
            ..mir::Mir6502Config::optimized() };
        let session = mir::trace::Mir6502Trace::new(mir::trace::TraceOptions {
            routines: vec!["ReadByte".into(), "ReadBit".into(), "Decode".into()],
            max_bytes: 64 * 1024 * 1024, ..Default::default()
        });
        let t = session.context();
        let started = std::time::Instant::now();
        let (p, resume) = mir::prepare_program_for_emission(&n, origin, &config,
            crate::runtime::Runtime::ActionCart, &t).unwrap();
        if let Some(resume) = &resume {
            let retained = &resume.core.checkpoint.input.program;
            std::fs::write(root.join(format!("{label}-retained.mir")), mir::format_program(retained)).unwrap();
            std::fs::write(root.join(format!("{label}-retained.json")),
                serde_json::to_string_pretty(&extra_inventory(retained)).unwrap()).unwrap();
        }
        let base = mir::finalize_output(&n, p, origin, crate::runtime::Runtime::ActionCart, &t).unwrap();
        let before_bytes = crate::codegen::format_load_file(&base.output).len();
        let e = mir::parameter_homes::select_output(&n, base, resume, &config, origin,
            crate::runtime::Runtime::ActionCart, &t);
        let seconds = started.elapsed().as_secs_f64();
        let facts = inventory(&e.program, Some(&e.output));
        std::fs::write(root.join(format!("{label}.json")), serde_json::to_string_pretty(&facts).unwrap()).unwrap();
        std::fs::write(root.join(format!("{label}-selected.json")),
            serde_json::to_string_pretty(&extra_inventory(&e.program)).unwrap()).unwrap();
        std::fs::write(root.join(format!("{label}.mir")), mir::format_program(&e.program)).unwrap();
        std::fs::write(root.join(format!("{label}.xex")), crate::codegen::format_load_file(&e.output)).unwrap();
        std::fs::write(root.join(format!("{label}.asm")),
            crate::codegen::format_listing_with_origin(&e.output.bytes, e.output.origin)).unwrap();
        let events: Vec<_> = session.events().into_iter().filter(|event|
            event.kind.starts_with("cross-call") || event.kind.starts_with("core-callee")
                || event.kind.contains("parameter-home") || event.kind.contains("parameter-argument")
                || event.kind == "skipped").map(|event|
                    json!({"id":event.id,"attempt":event.attempt,"kind":event.kind,"data":event.data})).collect();
        std::fs::write(root.join(format!("{label}-events.json")), serde_json::to_string_pretty(&events).unwrap()).unwrap();
        std::fs::write(root.join(format!("{label}-run.json")), serde_json::to_string_pretty(&json!({
            "source":path,"origin":origin,"config":format!("{config:?}"),"seconds":seconds,
            "before_home_selection_bytes":before_bytes,"packaged_bytes":facts["packaged_bytes"],
            "trace":session.manifest(),
        })).unwrap()).unwrap();
        println!("{label}: {} bytes, {seconds:.3}s", facts["packaged_bytes"]);
    }
}
