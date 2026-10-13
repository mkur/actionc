//! Final-layout, block-local record base reuse with complete rollback.
use super::*;
use crate::mir6502::analysis::record_bases::{
    MirPhysicalMemory, RecordBaseOptions, RecordBaseProof,
};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::MirRoutine;
use crate::mir6502::rewrite::context::{MirExitStateChange, MirProof};
use crate::mir6502::rewrite::plan::{MirChangeSet, MirPostHomeRewritePlan};
use crate::mir6502::rewrite::posthome::estimated_6502_cost;

fn memory(program: &MirProgram, origin: u16) -> Option<MirPhysicalMemory> {
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(origin);
    let summary = crate::mir6502::emit::emit_program_with_physical_memory(
        program,
        origin,
        &mut emitter,
        true,
    )
    .ok()?;
    emitter.finish_with_relocations().ok()?;
    Some(summary.physical_memory)
}

fn occupied(program: &MirProgram) -> BTreeSet<u8> {
    program
        .routines
        .iter()
        .flat_map(|r| {
            r.frame.fixed_zero_page.iter().map(|s| s.0).chain(
                r.frame
                    .zero_page_allocations
                    .iter()
                    .flat_map(|a| (0..a.size).map(move |n| a.start.0.wrapping_add(n))),
            )
        })
        .collect()
}

pub(in crate::mir6502) fn select_costed(
    baseline: MirProgram,
    config: &Mir6502Config,
    origin: u16,
    report: bool,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    // Avoid an extra empty report (and emission probe) for programs with no
    // complete setup. Inliner trial/final report counts stay unchanged there.
    if !baseline.routines.iter().flat_map(|r| &r.blocks).any(|b| {
        b.ops
            .windows(4)
            .any(|w| crate::mir6502::analysis::record_bases::setup_pair(w).is_some())
    }) {
        return Ok(baseline);
    }
    let Some(map) = memory(&baseline, origin) else {
        return Ok(baseline);
    };
    let options = RecordBaseOptions {
        writes: config.enable_record_base_writes,
        parameter_origins: config.enable_record_base_parameter_origins,
    };
    let mut proposed = baseline.clone();
    let mut replays: Vec<(MirRoutine, RecordBaseProof)> = Vec::new();
    let mut accepted = BTreeSet::new();
    let mut stats = MirPeepholeStats::default();
    for routine in &mut proposed.routines {
        if !routine.blocks.iter().any(|b| {
            b.ops
                .windows(4)
                .any(|w| crate::mir6502::analysis::record_bases::setup_pair(w).is_some())
        }) {
            continue;
        }
        let mut driver = MirPostHomeRewriteDriver::default();
        let result = driver.run_fixed_point(routine, |routine, context| {
            let mut plans = Vec::new();
            for block in &routine.blocks {
                for index in 0..block.ops.len().saturating_sub(3) {
                    if crate::mir6502::analysis::record_bases::setup_pair(
                        &block.ops[index..index + 4],
                    )
                    .is_none()
                    {
                        continue;
                    }
                    let site = MirSite::Op {
                        block: block.id,
                        op_index: index,
                    };
                    let proof = match context.record_base_reuse(site, &map, options) {
                        MirProof::Proven(proof) => proof,
                        MirProof::Blocked(blocker) => {
                            context.record_blocker("record-base-reuse", block.id, index, &blocker);
                            continue;
                        }
                    };
                    if !context.validate_record_base_reuse(&proof, &map) {
                        continue;
                    }
                    // Keeping the original high load reproduces A and Z/N
                    // exactly, including any exit value already consumed by
                    // caller transformations using known-callee summaries.
                    let replacement = vec![block.ops[index + 2].clone()];
                    let before = estimated_6502_cost(&block.ops[index..index + 4]);
                    let after = estimated_6502_cost(&replacement);
                    plans.push(MirPostHomeRewritePlan {
                        generation: context.generation(),
                        block: block.id,
                        range: index..index + 4,
                        replacement,
                        removed_homes: Vec::new(),
                        exit_state_change: MirExitStateChange::default(),
                        change_set: MirChangeSet::posthome_operation_change(),
                        stat: "record-base-reuse",
                        observations: vec![("record-base-pair-preserved", 1)],
                        family_priority: 0,
                        estimated_byte_saving: before.0.saturating_sub(after.0),
                        estimated_cycle_saving: before.1.saturating_sub(after.1),
                    });
                    replays.push((routine.clone(), proof));
                }
            }
            plans
        });
        let Ok(result) = result else {
            return Ok(baseline);
        };
        if result.applied > 0 {
            accepted.insert(routine.id);
        }
        record_prehome_rewrite_result(routine.id, result, &mut stats);
    }
    if accepted.is_empty() {
        if report {
            maybe_report_peepholes(&baseline, &stats, config);
        }
        return Ok(baseline);
    };
    if crate::mir6502::verify::verify_program(&proposed, MirPhase::PreEmission).is_err() {
        return Ok(baseline);
    };
    let (Some(old), Some(new), Some(final_map)) = (
        address_values::Cost::measure(&baseline, origin),
        address_values::Cost::measure(&proposed, origin),
        memory(&proposed, origin),
    ) else {
        return Ok(baseline);
    };
    if new.bytes >= old.bytes
        || accepted.iter().any(|id| !new.does_not_grow(&old, *id))
        || proposed
            .routines
            .iter()
            .zip(&baseline.routines)
            .any(|(a, b)| a.frame != b.frame)
        || !occupied(&proposed).is_subset(&occupied(&baseline))
        || replays.iter().any(|(routine, proof)| {
            !crate::mir6502::analysis::record_bases::validate_placement(proof, routine, &final_map)
        })
    {
        if report {
            for id in accepted {
                stats.record(id, "record-base-declined-final-proof-or-cost");
            }
            maybe_report_peepholes(&baseline, &stats, config);
        }
        return Ok(baseline);
    }
    if report {
        maybe_report_peepholes(&proposed, &stats, config);
    }
    Ok(proposed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(raw: bool) -> MirProgram {
        let text = include_str!("../../../surveys/tn/reproducers/record-base-reuse.act")
            .replace("\r\n", "\n");
        let ast = crate::parser::parse(&crate::lexer::tokenize(&text).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let sem = crate::semantic::ir::lower_program(&ast, &model);
        let nir = crate::nir::lower_program(&sem);
        crate::mir6502::lower_program(&if raw {
            nir.clone()
        } else {
            crate::nir::optimize_program(&nir).unwrap()
        })
        .unwrap()
    }
    #[test]
    fn read_reuse_preserves_homes_and_unknown_field_write_fallbacks() {
        for raw in [false, true] {
            for runtime in [
                crate::runtime::Runtime::ActionCart,
                crate::runtime::Runtime::Standalone,
            ] {
                let input = fixture(raw);
                let mut off = Mir6502Config::optimized();
                off.enable_record_base_reuse = false;
                off.enable_record_base_writes = false;
                off.enable_record_base_parameter_origins = false;
                let mut on = off.clone();
                on.enable_record_base_reuse = true;
                let old = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input.clone(),
                    &off,
                    0x2C00,
                    runtime,
                )
                .unwrap();
                let new = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input.clone(),
                    &on,
                    0x2C00,
                    runtime,
                )
                .unwrap();
                let a = address_values::Cost::measure(&old, 0x2C00).unwrap();
                let b = address_values::Cost::measure(&new, 0x2C00).unwrap();
                assert!(
                    b.bytes < a.bytes,
                    "{raw}/{runtime:?}: {} -> {}\n{}",
                    a.bytes,
                    b.bytes,
                    crate::mir6502::format_program(&old)
                );
                for routine in &input.routines {
                    assert!(b.does_not_grow(&a, routine.id));
                }
                for name in [
                    "WriteFields",
                    "MutateSource",
                    "KnownFields",
                    "RebindParameter",
                ] {
                    let id = input.routines.iter().find(|r| r.name == name).unwrap().id;
                    assert_eq!(a.routine(id), b.routine(id), "{raw}/{runtime:?}/{name}");
                }
            }
        }
    }

    #[test]
    fn proven_data_writes_reuse_bases_and_keep_unbounded_parameters_generic() {
        for runtime in [
            crate::runtime::Runtime::ActionCart,
            crate::runtime::Runtime::Standalone,
        ] {
            let input = fixture(false);
            let mut reads = Mir6502Config::optimized();
            reads.enable_record_base_reuse = true;
            reads.enable_record_base_writes = false;
            reads.enable_record_base_parameter_origins = false;
            let mut writes = reads.clone();
            writes.enable_record_base_writes = true;
            let old = crate::mir6502::materialize_program_with_origin_and_runtime(
                input.clone(),
                &reads,
                0x2C00,
                runtime,
            )
            .unwrap();
            let new = crate::mir6502::materialize_program_with_origin_and_runtime(
                input.clone(),
                &writes,
                0x2C00,
                runtime,
            )
            .unwrap();
            let a = address_values::Cost::measure(&old, 0x2C00).unwrap();
            let b = address_values::Cost::measure(&new, 0x2C00).unwrap();
            assert!(b.bytes < a.bytes);
            for name in ["KnownFields", "KnownWord"] {
                let id = input.routines.iter().find(|r| r.name == name).unwrap().id;
                assert!(b.routine(id) < a.routine(id), "{runtime:?}/{name}");
            }
            for name in ["WriteFields", "MutateSource", "RebindParameter"] {
                let id = input.routines.iter().find(|r| r.name == name).unwrap().id;
                assert_eq!(b.routine(id), a.routine(id), "{runtime:?}/{name}");
            }
        }
    }

    #[test]
    fn parameter_origins_require_complete_assignments_within_the_routine() {
        for raw in [false, true] {
            for runtime in [
                crate::runtime::Runtime::ActionCart,
                crate::runtime::Runtime::Standalone,
            ] {
                let input = fixture(raw);
                let mut writes = Mir6502Config::optimized();
                writes.enable_record_base_reuse = true;
                writes.enable_record_base_writes = true;
                writes.enable_record_base_parameter_origins = false;
                let mut origins = writes.clone();
                origins.enable_record_base_parameter_origins = true;
                let old = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input.clone(),
                    &writes,
                    0x2C00,
                    runtime,
                )
                .unwrap();
                let new = crate::mir6502::materialize_program_with_origin_and_runtime(
                    input.clone(),
                    &origins,
                    0x2C00,
                    runtime,
                )
                .unwrap();
                let a = address_values::Cost::measure(&old, 0x2C00).unwrap();
                let b = address_values::Cost::measure(&new, 0x2C00).unwrap();
                let id = input
                    .routines
                    .iter()
                    .find(|r| r.name == "RebindParameter")
                    .unwrap()
                    .id;
                assert!(b.routine(id) < a.routine(id), "{raw}/{runtime:?}");
                for name in ["WriteFields", "MixedFields", "NestedFields", "MutateSource"] {
                    let id = input.routines.iter().find(|r| r.name == name).unwrap().id;
                    assert_eq!(b.routine(id), a.routine(id), "{raw}/{runtime:?}/{name}");
                }
                assert!(occupied(&new).is_subset(&occupied(&old)));
            }
        }
    }
}
