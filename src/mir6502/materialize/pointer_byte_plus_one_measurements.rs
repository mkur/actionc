//! Opt-in final-image measurements using the normal linked compilation path.
use super::*;
use crate::{compiler::Runtime, includes, linker, semantic};
use std::{collections::BTreeMap, path::Path};

#[test]
#[ignore = "opt-in corpus measurement; writes ignored build artifacts"]
fn pointer_byte_plus_one_measure_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let artifacts = root.join("build/pointer-byte-plus-one/measured");
    std::fs::create_dir_all(&artifacts).unwrap();
    let mut regressions = Vec::new();
    for (name, source, origin) in [
        (
            "neutral-3000",
            "fixtures/runtime/pointer_byte_plus_one.act",
            0x3000,
        ),
        (
            "neutral-41C7",
            "fixtures/runtime/pointer_byte_plus_one.act",
            0x41C7,
        ),
        ("TN", "samples/tn/modern/TN.ACT", 0x2C00),
        ("TNDBG", "samples/tn/modern/TNDBG.ACT", 0x2C00),
        (
            "word-arithmetic",
            "fixtures/runtime/direct_action_word_arithmetic_args.act",
            0x3000,
        ),
        (
            "indexed-byte",
            "fixtures/runtime/indexed_byte_fixed_action_args.act",
            0x3000,
        ),
        (
            "scalar-predicate",
            "surveys/tn/reproducers/captured-call-arguments.act",
            0x3000,
        ),
        (
            "predicate-snapshots",
            "surveys/tn/reproducers/call-predicate-snapshots.act",
            0x3000,
        ),
        (
            "string-loop",
            "fixtures/mir6502/array_param_string_length_loop.act",
            0x3000,
        ),
        ("graphics", "samples/graphics/fedora.act", 0x3000),
        (
            "strings-runtime",
            "fixtures/runtime/standalone_sys_strings_runtime.act",
            0x3000,
        ),
    ] {
        let loaded =
            includes::load_compilation(&root.join(source), &includes::ModuleLoadOptions::default())
                .unwrap();
        let model = semantic::analyze_compilation_with_options(
            &loaded,
            semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let semir = linker::select_semir(
            &semantic::ir::lower_compilation(&loaded, &model),
            linker::SemLinkPolicy::EntryReachable,
        )
        .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut before_size = None;
            for enabled in [false, true] {
                let stem = format!("{name}-{runtime:?}-{enabled}");
                eprintln!("MEASURE {stem}");
                let config = Mir6502Config {
                    enable_pointer_load_sharing: false,
                    enable_pointer_byte_plus_one_fusion: enabled,
                    peephole_report: MirPeepholeReportMode::PerRoutine,
                    ..Mir6502Config::optimized()
                };
                let mir = crate::mir6502::lower_program(&nir).unwrap();
                let started = std::time::Instant::now();
                let mir = crate::mir6502::materialize_program_with_origin_and_runtime(
                    mir, &config, origin, runtime,
                )
                .unwrap();
                let materialization_ms = started.elapsed().as_secs_f64() * 1000.0;
                crate::mir6502::verify_program(&mir, crate::mir6502::MirPhase::PreEmission)
                    .unwrap();
                let mut emitter =
                    crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
                let summary =
                    crate::mir6502::emit::emit_program(&mir, origin, &mut emitter).unwrap();
                let output = crate::mir6502::codegen_output(
                    emitter.finish_with_relocations().unwrap(),
                    origin,
                    summary,
                    runtime,
                    &mir,
                );
                let image = crate::codegen::format_load_file(&output);
                if let Some(before) = before_size {
                    if image.len() > before {
                        regressions.push(format!("{stem}: {} > {before}", image.len()));
                    }
                } else {
                    before_size = Some(image.len());
                }
                let routine_bytes: BTreeMap<_, _> = output
                    .map
                    .routine_ranges
                    .iter()
                    .map(|r| (r.name.clone(), r.end - r.start))
                    .collect();
                let mut zero_page = BTreeSet::new();
                for routine in &mir.routines {
                    for slot in &routine.frame.fixed_zero_page {
                        zero_page.insert(u16::from(slot.0));
                    }
                    for slot in &routine.frame.zero_page_allocations {
                        zero_page.extend(
                            u16::from(slot.start.0)..u16::from(slot.start.0) + u16::from(slot.size),
                        );
                    }
                }
                for symbol in &output.map.storage_symbols {
                    zero_page.extend(
                        (symbol.address..symbol.address.saturating_add(symbol.size))
                            .filter(|address| *address < 0x100),
                    );
                }
                let report = serde_json::json!({
                    "source": source, "materialization_ms": materialization_ms,
                    "pair_count": mir.routines.iter().flat_map(|r|&r.blocks).map(|b|b.ops.windows(2).filter(|ops|crate::mir6502::analysis::pointer_byte_plus_one::shape(ops).is_ok()).count()).sum::<usize>(),
                    "fused_count": mir.routines.iter().flat_map(|r|&r.blocks).flat_map(|b|&b.ops).filter(|o|matches!(o,MirOp::OffsetPointerByIndirectBytePlusOne{..})).count(),
                    "runtime": format!("{runtime:?}"), "enabled": enabled,
                    "origin": origin, "nir": "optimized", "image_bytes": image.len(),
                    "image_span_bytes": output.bytes.len(),
                    "routine_code_bytes": routine_bytes.values().map(|n| usize::from(*n)).sum::<usize>(),
                    "code_start": output.map.routine_ranges.iter().map(|r| r.start).min(),
                    "code_end": output.map.routine_ranges.iter().map(|r| r.end).max(),
                    "reserved_ranges": output.skipped_ranges.iter().map(|r| (r.start, r.len)).collect::<Vec<_>>(),
                    "storage_symbols": output.map.storage_symbols.iter().map(|s| (&s.name, s.address, s.size)).collect::<Vec<_>>(),
                    "declared_and_allocated_zero_page": zero_page,
                    "routine_bytes": routine_bytes,
                    "parameter_addresses": output.map.storage_symbols.iter().filter_map(|s| {
                        let crate::codegen::CodegenSymbolScope::Routine(routine)=&s.scope else {return None;};
                        (s.kind==crate::codegen::CodegenSymbolKind::Parameter).then(||(format!("{routine}.{}",s.name),s.address))
                    }).collect::<BTreeMap<_,_>>(),
                    "frames": mir.routines.iter().map(|r| (&r.name, format!("{:?}", r.frame))).collect::<BTreeMap<_,_>>(),
                    "direct_loads_stores": mir.routines.iter().map(|r| {
                        let ops = || r.blocks.iter().flat_map(|b| &b.ops);
                        let loads = ops().filter(|op| matches!(op, MirOp::Load {src: MirAddr::Direct(_), ..})).count();
                        let stores = ops().filter(|op| matches!(op, MirOp::Store {dst: MirAddr::Direct(_), ..})).count();
                        (&r.name, (loads, stores))
                    }).collect::<BTreeMap<_,_>>(),
                    "routine_addresses": output.map.routine_ranges.iter().map(|r| (&r.name, r.start)).collect::<BTreeMap<_, _>>(),
                });
                let parameters = report["parameter_addresses"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| format!("{key}\t{:04X}\n", value.as_u64().unwrap()))
                    .collect::<String>();
                std::fs::write(artifacts.join(format!("{stem}.params")), parameters).unwrap();
                std::fs::write(artifacts.join(format!("{stem}.xex")), image).unwrap();
                std::fs::write(
                    artifacts.join(format!("{stem}.entries")),
                    output
                        .routine_addresses
                        .iter()
                        .map(|r| format!("{}\t{:04X}\n", r.name, r.address))
                        .collect::<String>(),
                )
                .unwrap();
                std::fs::write(
                    artifacts.join(format!("{stem}.json")),
                    serde_json::to_string_pretty(&report).unwrap(),
                )
                .unwrap();
                std::fs::write(
                    artifacts.join(format!("{stem}.mir")),
                    crate::mir6502::format_program(&mir),
                )
                .unwrap();
                eprintln!("RESULT {report}");
            }
        }
    }
    assert!(regressions.is_empty(), "{regressions:?}");
}
