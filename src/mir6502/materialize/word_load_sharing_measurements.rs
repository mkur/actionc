//! Opt-in corpus export through the normal linked compiler path.
use super::*;
use crate::mir6502::MirPeepholeReportMode;
use crate::{compiler::Runtime, includes, linker, semantic};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[test]
#[ignore = "opt-in corpus export; writes ignored build artifacts"]
fn word_load_sharing_measure_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let artifacts = root.join("build/word-load-sharing/measured");
    std::fs::create_dir_all(&artifacts).unwrap();
    let mut rows = Vec::new();
    for (name, source, origin) in [
        ("neutral", "fixtures/runtime/word_load_sharing.act", 0x3000),
        (
            "neutral-41C7",
            "fixtures/runtime/word_load_sharing.act",
            0x41C7,
        ),
        (
            "sources",
            "fixtures/runtime/word_load_sharing_sources.act",
            0x3000,
        ),
        (
            "allocate",
            "fixtures/runtime/allocate_word_load_sharing.act",
            0x3000,
        ),
        ("TN", "samples/tn/modern/TN.ACT", 0x2C00),
        ("TNDBG", "samples/tn/modern/TNDBG.ACT", 0x2C00),
        (
            "pointer-load-sharing",
            "fixtures/runtime/pointer_load_sharing.act",
            0x3000,
        ),
        (
            "word-move-store",
            "fixtures/runtime/word_move_store.act",
            0x3000,
        ),
        (
            "word-arguments",
            "fixtures/runtime/direct_action_word_arithmetic_args.act",
            0x3000,
        ),
        (
            "indexed-byte",
            "fixtures/runtime/indexed_byte_fixed_action_args.act",
            0x3000,
        ),
        (
            "predicate-captures",
            "surveys/tn/reproducers/captured-call-arguments.act",
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
            let mut outputs = Vec::new();
            let mut before: Option<MirProgram> = None;
            for enabled in [false, true] {
                eprintln!("MEASURE word-load-sharing {name}/{runtime:?}/{enabled}");
                let config = Mir6502Config {
                    enable_word_load_sharing: enabled,
                    peephole_report: MirPeepholeReportMode::PerRoutine,
                    ..Mir6502Config::optimized()
                };
                WORK.with(|c| c.borrow_mut().clear());
                let started = std::time::Instant::now();
                let mir = crate::mir6502::materialize_program_with_origin_and_runtime(
                    crate::mir6502::lower_program(&nir).unwrap(),
                    &config,
                    origin,
                    runtime,
                )
                .unwrap();
                let seconds = started.elapsed().as_secs_f64();
                let work = WORK.with(|c| c.borrow().clone());
                assert!(work.iter().all(|&n| n <= 7));
                crate::mir6502::verify_program(&mir, MirPhase::PreEmission).unwrap();
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
                let stem = format!("{name}-{runtime:?}-{enabled}");
                std::fs::write(artifacts.join(format!("{stem}.xex")), &image).unwrap();
                std::fs::write(
                    artifacts.join(format!("{stem}.asm")),
                    crate::compiler::artifacts::format_listing_with_boundaries(&output),
                )
                .unwrap();
                std::fs::write(
                    artifacts.join(format!("{stem}.mir")),
                    crate::mir6502::format_program(&mir),
                )
                .unwrap();
                let routines = output
                    .map
                    .routine_ranges
                    .iter()
                    .map(|r| (r.name.clone(), r.end - r.start))
                    .collect::<BTreeMap<_, _>>();
                let frames = mir
                    .routines
                    .iter()
                    .map(|r| (r.name.clone(), format!("{:?}", r.frame)))
                    .collect::<BTreeMap<_, _>>();
                let loads = mir
                    .routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .filter(|op| {
                        matches!(
                            op,
                            MirOp::Load {
                                src: MirAddr::Direct(_),
                                width: MirWidth::Byte,
                                ..
                            }
                        )
                    })
                    .count();
                let mut sites = BTreeMap::new();
                if let Some(baseline) = &before {
                    assert_eq!(baseline.routines.len(), mir.routines.len());
                    for (old, new) in baseline.routines.iter().zip(&mir.routines) {
                        assert_eq!(old.frame, new.frame);
                        assert_eq!(old.blocks.len(), new.blocks.len());
                        for (old_block, new_block) in old.blocks.iter().zip(&new.blocks) {
                            assert_eq!(old_block.id, new_block.id);
                            let mut terminator = new_block.terminator.clone();
                            if let (
                                MirTerminator::Branch {
                                    cond: MirCond::FusedCompare { producer: a, .. },
                                    ..
                                },
                                MirTerminator::Branch {
                                    cond: MirCond::FusedCompare { producer: b, .. },
                                    ..
                                },
                            ) = (&old_block.terminator, &mut terminator)
                            {
                                // Earlier materialization may leave historical
                                // producer indices. Emission consumes flag_test;
                                // check CFG/flags and the bounded splice rebase.
                                assert_eq!(a.block, b.block);
                                assert!(b.op_index <= a.op_index);
                                let removed = old_block.ops.len() - new_block.ops.len();
                                assert!(a.op_index - b.op_index <= removed);
                                assert_eq!((a.op_index - b.op_index) % 2, 0);
                                *b = *a;
                            }
                            assert_eq!(old_block.terminator, terminator);
                        }
                        let old_ops = old.blocks.iter().map(|b| b.ops.len()).sum::<usize>();
                        let new_ops = new.blocks.iter().map(|b| b.ops.len()).sum::<usize>();
                        assert!(new_ops <= old_ops);
                        assert_eq!((old_ops - new_ops) % 2, 0);
                        if new_ops < old_ops {
                            sites.insert(old.name.clone(), (old_ops - new_ops) / 2);
                        }
                    }
                } else {
                    before = Some(mir.clone());
                }
                let stable = serde_json::json!({"enabled":enabled,"xex_bytes":image.len(),"routine_bytes":routines,
                    "direct_byte_loads":loads,"frames":frames,"reserved":output.skipped_ranges.iter().map(|r|(r.start,r.len)).collect::<Vec<_>>(),
                    "sites":sites,"selector_emission_batches":work});
                std::fs::write(artifacts.join(format!("{stem}.timing.json")),serde_json::to_string_pretty(&serde_json::json!({"materialization_seconds":seconds,"outer_materializations":1})).unwrap()).unwrap();
                outputs.push(stable);
            }
            let old = &outputs[0];
            let new = &outputs[1];
            assert!(new["xex_bytes"].as_u64().unwrap() <= old["xex_bytes"].as_u64().unwrap());
            assert_eq!(old["frames"], new["frames"]);
            // Uninitialized buffers follow code and can relocate when it
            // shrinks. Their capacities and all frame/ZP allocations stay equal;
            // the selector separately replays proofs at the final addresses.
            let capacities = |row: &serde_json::Value| {
                row["reserved"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|range| range[1].clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(capacities(old), capacities(new));
            let sites = new["sites"]
                .as_object()
                .unwrap()
                .values()
                .map(|n| n.as_u64().unwrap())
                .sum::<u64>();
            assert_eq!(
                old["direct_byte_loads"].as_u64().unwrap()
                    - new["direct_byte_loads"].as_u64().unwrap(),
                2 * sites
            );
            for (routine, size) in old["routine_bytes"].as_object().unwrap() {
                assert!(new["routine_bytes"][routine].as_u64().unwrap() <= size.as_u64().unwrap());
            }
            rows.push(serde_json::json!({"name":name,"source":source,"origin":origin,"runtime":format!("{runtime:?}"),"before":old,"after":new}));
        }
    }
    assert_eq!(rows.len(), 28);
    let path = std::env::var_os("ACTIONC_WORD_LOAD_RESULTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| artifacts.join("current.json"));
    std::fs::write(
        path,
        serde_json::to_string_pretty(&serde_json::json!({"schema":1,"measurements":rows})).unwrap()
            + "\n",
    )
    .unwrap();
}
