// Diagnostic-only probe, temporarily included inside analysis::known_callees
// by leaf_facts.py. It reads the real private analyses without widening their API.
use super::*;
use crate::mir6502 as mir;
use serde_json::{Value, json};

fn registers(r: MirRegisterSet) -> Value {
    json!({"a": r.a, "x": r.x, "y": r.y, "flags": r.flags, "sp": r.sp})
}

fn memory_bytes(
    owner: RoutineId, bytes: &[MirMem], physical: &mir::analysis::record_bases::MirPhysicalMemory,
) -> Value {
    json!(bytes.iter().map(|m| json!({
        "identity": format!("{m:?}"), "address": physical.address(owner, m)
    })).collect::<Vec<_>>())
}

fn summary_json(
    owner: RoutineId, s: &MirKnownCalleeExitSummary,
    physical: &mir::analysis::record_bases::MirPhysicalMemory,
) -> Value {
    json!({
        "accumulator": s.accumulator.as_ref().map(|v| format!("{v:?}")),
        "zn": s.zn.as_ref().map(|v| format!("{v:?}")),
        "preserves": registers(s.preserves),
        "reads": match &s.reads {
            MirKnownMemoryReads::Exact(v) => json!({"kind":"exact", "bytes":memory_bytes(owner, v, physical)}),
            MirKnownMemoryReads::Unknown => json!({"kind":"unknown"}),
        },
        "writes": match &s.writes {
            MirKnownMemoryWrites::Exact(v) => json!({"kind":"exact", "bytes":memory_bytes(owner, v, physical)}),
            MirKnownMemoryWrites::Unknown => json!({"kind":"unknown"}),
        },
    })
}

#[test]
#[ignore = "explicit diagnostic generator; see tools/compare6502/tn/leaf_facts.py"]
fn dump_tn_leaf_facts() {
    let destination = std::path::PathBuf::from(std::env::var_os("ACTIONC_LEAF_FACTS_OUTPUT").unwrap());
    match std::env::var("ACTIONC_LEAF_FACTS_ZP_HOME_LIFETIMES").as_deref().unwrap_or("default") {
        "compare" => {
            dump_facts(destination.join("off"), Some(false));
            dump_facts(destination.join("on"), Some(true));
        }
        "off" => dump_facts(destination, Some(false)),
        "on" => dump_facts(destination, Some(true)),
        "default" => dump_facts(destination, None),
        mode => panic!("unknown ZP lifetime mode: {mode}"),
    }
}

fn dump_facts(destination: std::path::PathBuf, zp_lifetimes: Option<bool>) {
    let started = std::time::Instant::now();
    let loaded = crate::includes::load_program_with_expanded_source("samples/tn/modern/TN.ACT").unwrap();
    let model = crate::semantic::analyze_with_options(&loaded.program, crate::semantic::SemanticOptions::modern()).unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&loaded.program, &model))).unwrap();
    let mut config = mir::Mir6502Config::optimized();
    if let Some(enabled) = zp_lifetimes { config.enable_zp_home_lifetimes = enabled; }
    let runtime = crate::runtime::Runtime::ActionCart;
    let origin = 0x3000;
    let session = mir::trace::Mir6502Trace::new(mir::trace::TraceOptions {
        routines: ["FindItem", "PopUp", "Sort", "Handle", "Ord", "Items", "Next",
            "Key", "Instr", "Fnamecmp"].into_iter().map(String::from).collect(),
        max_bytes: 128 * 1024 * 1024,
        ..Default::default()
    });
    let trace = session.context();
    trace.event("compilation", || json!({"origin":origin,"runtime":format!("{runtime:?}"),
        "config":format!("{config:?}"),"nir_identity":mir::trace::identity(&format!("{nir:?}")),
        "nir_optimization":"caller-owned"}));
    // Follow generate_output_impl exactly, retaining its selected MIR as well.
    let (program, resume) = mir::prepare_program_for_emission(&nir, origin, &config, runtime, &trace).unwrap();
    let baseline = mir::finalize_output(&nir, program, origin, runtime, &trace).unwrap();
    let selected = mir::parameter_homes::select_output(&nir, baseline, resume, &config, origin, runtime, &trace);
    let p = &selected.program;
    let output = &selected.output;
    trace.select_final_artifact(selected.trace_snapshot);
    session.finish(&Ok(output.clone()));
    let generation_seconds = started.elapsed().as_secs_f64();
    // One process-resident sample, in KiB (ps on macOS/Linux), not peak RSS.
    // Rust compilation and the following diagnostic replay are excluded.
    let resident_kib = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok()?.trim().parse::<u64>().ok());
    let manifest = session.manifest();
    let decisions: Vec<_> = session.events().into_iter().filter(|e| matches!(e.kind,
        "cross-call-ineligible" | "cross-call-candidate" | "cross-call-home-ineligible" | "cross-call-home-result"
        | "parameter-home-result" | "parameter-argument-handoff-result"
        | "final-artifact-selection")).collect();
    let image = crate::codegen::format_load_file(output);
    if let Some(path) = std::env::var_os("ACTIONC_LEAF_FACTS_EXPECT_IMAGE") {
        assert_eq!(image, std::fs::read(path).unwrap(), "qualified TN image changed");
    }

    // One layout/emission replay resolves the summaries' logical memory IDs.
    // This is diagnostic work, outside optimization/candidate selection.
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let emitted = mir::emit::emit_program_with_reference_programs_traced(
        p, origin, &mut emitter, true, &[p], &mir::trace::Trace::disabled()).unwrap();
    let physical = emitted.physical_memory.clone();
    let emission = emitter.finish_with_relocations().unwrap();
    let replay = mir::codegen_output(emission, origin, emitted, runtime, p);
    assert_eq!(output.bytes, replay.bytes);
    assert_eq!(output.relocations, replay.relocations);
    assert_eq!(image, crate::codegen::format_load_file(&replay));

    let known = MirKnownCalleeSummaries::analyze_for_rewriting(p);
    let raw = MirKnownCalleeSummaries::analyze(p);
    let mut calls = mir::parameter_homes::ResidenceCalls::new(p, MirCalleeEntryStage::ActualEntry);
    let mut inventory = Vec::new();
    let mut machine_observations = Vec::new();
    let mut abi_captures = Vec::new();
    let mut leaves = Vec::new();
    let mut listing = String::new();
    let footprints: Vec<_> = output.map.routine_ranges.iter().map(|r| {
        let homes: Vec<_> = output.map.storage_symbols.iter().filter(|s|
            s.scope == crate::codegen::CodegenSymbolScope::Routine(r.name.clone())
            && matches!(s.kind, crate::codegen::CodegenSymbolKind::Parameter | crate::codegen::CodegenSymbolKind::Local)
        ).collect();
        json!({"name":r.name,"code_bytes":r.end-r.start,
            "inline_home_bytes":homes.iter().filter(|s|s.address>=256).map(|s|u32::from(s.size)).sum::<u32>(),
            "zp_home_bytes":homes.iter().filter(|s|s.address<256).map(|s|u32::from(s.size)).sum::<u32>(),
            "homes":homes.iter().map(|s|json!({"name":s.name,"address":s.address,"size":s.size,
                "kind":format!("{:?}",s.kind)})).collect::<Vec<_>>()})
    }).collect();
    for r in &p.routines {
        for b in &r.blocks {
            for op in &b.ops {
                if let MirOp::SArgsCapture { first_param, byte_count, effects } = op {
                    abi_captures.push(json!({"routine":r.name,"routine_id":r.id.0,"block":b.label,
                        "first_param":first_param.0,"bytes":byte_count,
                        "address":physical.assembly_address(r.id, &MirMem::Param { id:*first_param, offset:0 }),
                        "effects":format!("{effects:?}"),"kind":"sargs"}));
                }
                let MirOp::MachineBlock { id, .. } = op else { continue; };
                let machine = p.machine_blocks.iter().find(|m| m.id == *id).unwrap();
                let decoded = machine_storage_observations(machine, r, p);
                let expressions = |accesses: &[observations::StorageObservation]| accesses.iter()
                    .map(|access| json!({"identity":format!("{:?}", access.address.base),
                        "addend":access.address.addend,"requires_zero_page":access.address.requires_zero_page,
                        "extent":format!("{:?}",access.extent),"fixed_address":access.address.fixed(p)}))
                    .collect::<Vec<_>>();
                let resolve = |accesses: &[observations::StorageObservation]| accesses.iter()
                    .map(|access| access.resolve(p, r.id, Some(&physical)))
                    .collect::<Option<Vec<_>>>().map(|ranges| ranges.into_iter().flatten()
                        .map(|range| json!({"address":physical.address(r.id, &range.base),"bytes":range.bytes}))
                        .collect::<Vec<_>>());
                let reads = decoded.as_ref().and_then(|o|resolve(&o.reads));
                let writes = decoded.as_ref().and_then(|o|resolve(&o.writes));
                machine_observations.push(json!({"routine":r.name,"routine_id":r.id.0,
                    "block":b.label,"machine_block":id.0,"complete":reads.is_some() && writes.is_some(),
                    "decoded":decoded.is_some(),
                    "requires_layout":decoded.as_ref().map(|o| o.reads.iter().chain(&o.writes)
                        .any(|a| a.address.fixed(p).is_none())),
                    "resolution":if decoded.is_none() { "opaque" } else if reads.is_some() && writes.is_some() { "resolved" } else { "failed" },
                    "read_expressions":decoded.as_ref().map(|o|expressions(&o.reads)),
                    "write_expressions":decoded.as_ref().map(|o|expressions(&o.writes)),
                    "reads":reads,"writes":writes}));
            }
        }
        let ops: Vec<_> = r.blocks.iter().flat_map(|b| &b.ops).collect();
        let call_count = ops.iter().filter(|o| matches!(o, MirOp::Call {..} | MirOp::RuntimeHelper {..} | MirOp::SArgsCapture { .. })).count();
        let machine_count = ops.iter().filter(|o| matches!(o, MirOp::MachineBlock {..})).count();
        let barrier_count = ops.iter().filter(|o| matches!(o, MirOp::Barrier {..})).count();
        inventory.push(json!({"id":r.id.0,"name":r.name,"mir_calls":call_count,
            "machine_blocks":machine_count,"barriers":barrier_count}));
        if call_count != 0 { continue; }
        let s = known.get(r.id).unwrap();
        let range = output.map.routine_ranges.iter().find(|range| range.name == r.name).unwrap();
        let asm = crate::codegen::format_listing_with_origin(
            &output.bytes[usize::from(range.start-origin)..usize::from(range.end-origin)], range.start);
        listing.push_str(&format!("; {} (r{}, {} bytes)\n{}\n\n", r.name, r.id.0, range.end-range.start, asm));
        let entry = MirCalleeEntryFact::analyze(p, r.id, MirCalleeEntryStage::ActualEntry);
        let mut callers = Vec::new();
        for caller in &p.routines {
            for b in &caller.blocks {
                for (index, op) in b.ops.iter().enumerate() {
                    if matches!(op, MirOp::Call {target:MirCallTarget::Routine(id),..} if *id == r.id) {
                        callers.push(json!({"caller":caller.name,"block":b.id.0,"op":index,
                            "residence_check":calls.writes(op).err().unwrap_or("accepted")}));
                    }
                }
            }
        }
        let mut op_facts = Vec::new();
        for b in &r.blocks {
            for (index, op) in b.ops.iter().enumerate() {
                let e = classify_op(op);
                let mut writes = MirKnownMemoryWrites::exact();
                summarize_op_writes(op, r, &known, &mut writes);
                op_facts.push(json!({"block":b.id.0,"index":index,"kind":format!("{:?}",e.kind),
                    "op":format!("{op:?}"),"memory":format!("{:?}",e.memory),
                    "machine":format!("{:?}",e.machine),"homes":format!("{:?}",e.homes),
                    "addresses":format!("{:?}",e.addresses),
                    "makes_writes_unknown":matches!(writes,MirKnownMemoryWrites::Unknown),
                    "indirect_reads":e.memory.indirect_reads,"indirect_writes":e.memory.indirect_writes,
                    "opaque":e.memory.opaque,"may_write_any":e.memory.may_write_any,
                    "has_unknown_effects":e.memory.has_unknown_effects,"unknown_home_writes":e.homes.unknown_writes,
                }));
            }
        }
        leaves.push(json!({"id":r.id.0,"name":r.name,"abi":format!("{:?}",r.abi),
            "class":if machine_count != 0 {"machine-containing"} else if barrier_count != 0 {"barrier-containing"} else {"native-leaf"},
            "start":range.start,"end":range.end,"code_bytes":range.end-range.start,
            "scalar_signature":format!("{:?}",r.scalar_signature),
            "direct_parameter_abi":format!("{:?}",r.frame.direct_parameter_abi),
            "declared_effects":format!("{:?}",r.effects),
            "frame":mir::trace::homes::frame(r),
            "entry":entry.as_ref().map(|e|json!({"registers":registers(e.inputs.registers),
                "flags":format!("{:?}",e.inputs.flags),"fixed":e.inputs.fixed.iter().collect::<Vec<_>>()})),
            "rewrite_summary":summary_json(r.id,s,&physical),
            "analysis_summary":summary_json(r.id,raw.get(r.id).unwrap(),&physical),
            "native_write_scan":format!("{:?}",summarize_routine_writes(r,&known)),
            "map_signature":format!("{:?}",output.map.routine_signatures.iter().find(|s|s.name==r.name)),
            "callers":callers,"ops":op_facts,
            "mir":mir::printer::format_program_filtered(p,|id|id==r.id),"assembly":asm,
        }));
    }
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("selected.mir"), mir::format_program(p)).unwrap();
    let controls = ["Sort", "Fnamecmp", "Ord", "Items", "FindItem", "Next", "PopUp"];
    std::fs::write(destination.join("controls.mir"), mir::printer::format_program_filtered(p,
        |id| p.routines.iter().any(|r| r.id == id && controls.contains(&r.name.as_str())))).unwrap();
    let mut controls_asm = String::new();
    for name in controls {
        let range = output.map.routine_ranges.iter().find(|r| r.name == name).unwrap();
        controls_asm.push_str(&format!("; {name} ({} bytes)\n{}\n", range.end-range.start,
            crate::codegen::format_listing_with_origin(
                &output.bytes[usize::from(range.start-origin)..usize::from(range.end-origin)], range.start)));
    }
    std::fs::write(destination.join("controls.asm"), controls_asm).unwrap();
    std::fs::write(destination.join("facts.json"),serde_json::to_string_pretty(&json!({
        "schema":2,"source":"samples/tn/modern/TN.ACT","origin":origin,"runtime":"ActionCart",
        "config":format!("{config:?}"),"packaged_bytes":image.len(),
        "layout_replay_identical":true,"routine_count":p.routines.len(),
        "inventory":inventory,"footprints":footprints,"leaf_candidates":leaves,
        "machine_observations":machine_observations,"abi_captures":abi_captures,
        "machine_observation_scope":"explicit operands and pointer fetches; not a complete memory-effect summary",
        "qualification":{"generation_seconds":generation_seconds,"resident_kib":resident_kib,
            "sample_scope":"single traced source-to-executable generation; not peak RSS",
            "manifest":manifest,"decisions":decisions},
    })).unwrap()+"\n").unwrap();
    std::fs::write(destination.join("leaves.asm"),listing).unwrap();
    std::fs::write(destination.join("tn.xex"),image).unwrap();
    std::fs::write(destination.join("map.txt"),format!("{:#?}",output.map)).unwrap();
    std::fs::write(destination.join("relocations.txt"),format!("{:#?}",output.relocations)).unwrap();
    println!("TN leaf facts generated in {}",destination.display());
}
