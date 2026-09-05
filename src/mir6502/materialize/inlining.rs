//! Program transactions for costed scalar leaf expansion, before ABI lowering.
use std::collections::BTreeMap;

use super::inlining_cost::{Image, saving};
use super::small_loops::remap_block_temps;
use super::stats::{MirPeepholeStats, maybe_report_peepholes};
use crate::mir6502::analysis::leaf_routines::{
    LeafCensus, LeafRoutine, caller_supported, return_store,
};
use crate::mir6502::diagnostics::MirDiagnostic;
use crate::mir6502::ir::*;
use crate::mir6502::passes::Mir6502Config;
use crate::runtime::Runtime;

const MAX_TRIALS: usize = 16;
const MAX_SITES_PER_GROUP: usize = 8;
const MAX_BYTES_PER_SITE: usize = 32;
const MAX_BYTES_PER_CALLER: usize = 128;
const MAX_BYTES_PER_PROGRAM: usize = 256;

fn within_growth_budget(
    sites: usize,
    charged: usize,
    caller: usize,
    program: usize,
    net: usize,
) -> bool {
    charged <= MAX_BYTES_PER_SITE * sites
        && caller.saturating_add(charged) <= MAX_BYTES_PER_CALLER
        && program.saturating_add(charged) <= MAX_BYTES_PER_PROGRAM
        && net <= MAX_BYTES_PER_PROGRAM
}

#[derive(Debug)]
struct Expansion {
    sites: usize,
    sites_by_block: BTreeMap<MirBlockId, usize>,
    /// Every cloned block and continuation belongs to an original caller
    /// block. Used to compare full materialized regions, including new spills.
    origins: BTreeMap<MirBlockId, MirBlockId>,
}

fn id_error(routine: &MirRoutine) -> Vec<MirDiagnostic> {
    vec![MirDiagnostic::routine(
        &routine.name,
        "leaf inlining exhausted MIR IDs",
    )]
}

fn fresh(next: &mut u32, routine: &MirRoutine) -> Result<u32, Vec<MirDiagnostic>> {
    let result = *next;
    *next = next.checked_add(1).ok_or_else(|| id_error(routine))?;
    Ok(result)
}

fn expand_group(
    caller: &mut MirRoutine,
    leaf: &LeafRoutine,
) -> Result<Expansion, Vec<MirDiagnostic>> {
    let mut origins = caller
        .blocks
        .iter()
        .map(|b| (b.id, b.id))
        .collect::<BTreeMap<_, _>>();
    let mut next_block = caller
        .blocks
        .iter()
        .map(|b| b.id.0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| id_error(caller))?;
    let mut next_temp = caller
        .temps
        .iter()
        .chain(&leaf.routine.temps)
        .map(|t| t.id.0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| id_error(caller))?;
    let mut sites = 0;
    let mut sites_by_block = BTreeMap::new();
    loop {
        let site = caller.blocks.iter().enumerate().find_map(|(bi, b)| {
            b.ops.iter().enumerate().find_map(|(oi, op)| {
                matches!(op, MirOp::Call { target: MirCallTarget::Routine(id), .. } if *id == leaf.routine.id)
                    .then_some((bi, oi))
            })
        });
        let Some((block_index, op_index)) = site else {
            break;
        };
        let MirOp::Call { args, result, .. } = caller.blocks[block_index].ops[op_index].clone()
        else {
            unreachable!()
        };
        let parent = origins[&caller.blocks[block_index].id];
        *sites_by_block.entry(parent).or_default() += 1;
        let continuation_id = MirBlockId(fresh(&mut next_block, caller)?);
        let blocks = leaf
            .routine
            .blocks
            .iter()
            .map(|b| Ok((b.id, MirBlockId(fresh(&mut next_block, caller)?))))
            .collect::<Result<BTreeMap<_, _>, Vec<MirDiagnostic>>>()?;
        let temps = leaf
            .routine
            .temps
            .iter()
            .map(|t| Ok((t.id, MirTempId(fresh(&mut next_temp, caller)?))))
            .collect::<Result<BTreeMap<_, _>, Vec<MirDiagnostic>>>()?;
        let captures = args
            .iter()
            .map(|_| fresh(&mut next_temp, caller).map(MirTempId))
            .collect::<Result<Vec<_>, _>>()?;
        let mut cloned = Vec::new();
        for original in &leaf.routine.blocks {
            let mut block = original.clone();
            block.id = blocks[&original.id];
            block.label = format!(
                "inline_r{}_b{}_{}",
                leaf.routine.id.0, original.id.0, block.id.0
            );
            remap_block_temps(&mut block, &temps);
            if original.id == leaf.routine.blocks[0].id {
                block
                    .params
                    .extend(captures.iter().map(|dest| MirBlockParam {
                        dest: *dest,
                        width: MirWidth::Byte,
                    }));
            }
            for op in &mut block.ops {
                if let MirOp::Load {
                    dst,
                    src: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
                    width: MirWidth::Byte,
                } = op
                {
                    let index = leaf
                        .params
                        .iter()
                        .position(|p| p == id)
                        .expect("classified leaf parameter");
                    *op = MirOp::Move {
                        dst: dst.clone(),
                        src: MirValue::Def(MirDef::VTemp(captures[index])),
                        width: MirWidth::Byte,
                    };
                }
            }
            match &mut block.terminator {
                MirTerminator::Return => {
                    let mut edge = MirEdge::plain(continuation_id);
                    if result.is_some() {
                        let value = block
                            .ops
                            .last()
                            .and_then(return_store)
                            .expect("classified value return")
                            .clone();
                        edge.args.push(MirEdgeArg {
                            value,
                            width: MirWidth::Byte,
                        });
                    }
                    block.terminator = MirTerminator::Jump(edge);
                }
                MirTerminator::Jump(edge) => edge.target = blocks[&edge.target],
                MirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => {
                    then_edge.target = blocks[&then_edge.target];
                    else_edge.target = blocks[&else_edge.target];
                }
                _ => unreachable!("classified leaf control"),
            }
            origins.insert(block.id, parent);
            cloned.push(block);
        }
        let block = &mut caller.blocks[block_index];
        let suffix = block.ops.split_off(op_index + 1);
        block.ops.pop();
        let old_terminator = std::mem::replace(
            &mut block.terminator,
            MirTerminator::Jump(MirEdge {
                target: blocks[&leaf.routine.blocks[0].id],
                args: args
                    .iter()
                    .map(|a| MirEdgeArg {
                        value: a.value.clone(),
                        width: MirWidth::Byte,
                    })
                    .collect(),
            }),
        );
        let params = result
            .map(|result| {
                let MirDef::VTemp(dest) = result.dst else {
                    unreachable!("classified call result")
                };
                MirBlockParam {
                    dest,
                    width: result.width,
                }
            })
            .into_iter()
            .collect();
        cloned.push(MirBlock {
            id: continuation_id,
            label: format!("inline_continue_{}", continuation_id.0),
            params,
            ops: suffix,
            terminator: old_terminator,
        });
        origins.insert(continuation_id, parent);
        caller
            .blocks
            .splice(block_index + 1..block_index + 1, cloned);
        caller
            .temps
            .extend(temps.values().map(|id| MirTemp { id: *id }));
        caller
            .temps
            .extend(captures.into_iter().map(|id| MirTemp { id }));
        sites += 1;
    }
    if leaf.returns_value {
        let slot = MirFixedZpSlot(crate::codegen::runtime_zp::ARGS.address());
        if !caller.frame.fixed_zero_page.contains(&slot) {
            caller.frame.fixed_zero_page.push(slot);
        }
    }
    Ok(Expansion {
        sites,
        origins,
        sites_by_block,
    })
}

pub(in crate::mir6502) fn materialize(
    program: MirProgram,
    census: LeafCensus,
    config: &Mir6502Config,
    origin: u16,
    runtime: Runtime,
) -> Result<MirProgram, Vec<MirDiagnostic>> {
    let mut stats = MirPeepholeStats::default();
    for (id, reason) in census.rejected {
        stats.record_dynamic(id, format!("leaf-inline-blocked-{reason}"));
    }
    let mut current = program;
    let mut groups = current
        .routines
        .iter()
        .flat_map(|r| {
            let mut counts = BTreeMap::<RoutineId, usize>::new();
            for op in r.blocks.iter().flat_map(|b| &b.ops) {
                if let MirOp::Call {
                    target: MirCallTarget::Routine(id),
                    ..
                } = op
                {
                    if census.leaves.contains_key(id) {
                        *counts.entry(*id).or_default() += 1;
                    }
                }
            }
            counts
                .into_iter()
                .map(move |(callee, count)| (r.id, callee, count))
        })
        .collect::<Vec<_>>();
    // Cheap ranking only: actual target costs below decide acceptance.
    groups.sort_by_key(|(caller, callee, count)| {
        (
            std::cmp::Reverse(count * (12 + 4 * census.leaves[callee].params.len())),
            *caller,
            *callee,
        )
    });
    if groups.is_empty() {
        maybe_report_peepholes(&current, &stats, config);
        return crate::mir6502::materialize_resolved_program(
            current, config, origin, runtime, true,
        );
    }
    let mut trials = 0;
    let mut baseline = crate::mir6502::materialize_resolved_program(
        current.clone(),
        config,
        origin,
        runtime,
        false,
    )?;
    let mut baseline_image = Image::build(&baseline, origin);
    let initial_bytes = baseline_image.as_ref().map_or(0, |i| i.bytes.len());
    let mut caller_growth = BTreeMap::<RoutineId, usize>::new();
    let mut program_growth = 0;
    for (caller_id, callee_id, count) in groups {
        stats.record_many(caller_id, "leaf-inline-candidate", count);
        let caller_index = current
            .routines
            .iter()
            .position(|r| r.id == caller_id)
            .unwrap();
        if count > MAX_SITES_PER_GROUP || trials >= MAX_TRIALS {
            stats.record_many(caller_id, "leaf-inline-blocked-budget", count);
            continue;
        }
        if !caller_supported(&current.routines[caller_index]) {
            stats.record_many(caller_id, "leaf-inline-blocked-caller", count);
            continue;
        }
        let mut candidate = current.clone();
        let expansion = expand_group(
            &mut candidate.routines[caller_index],
            &census.leaves[&callee_id],
        )?;
        crate::mir6502::verify_program(&candidate, MirPhase::PreMaterialization)?;
        trials += 1;
        stats.record(caller_id, "leaf-inline-trials");
        let trial = crate::mir6502::materialize_resolved_program(
            candidate.clone(),
            config,
            origin,
            runtime,
            false,
        );
        let Some((trial, image)) = trial
            .ok()
            .and_then(|p| Image::build(&p, origin).map(|image| (p, image)))
        else {
            stats.record_many(caller_id, "leaf-inline-blocked-unknown", count);
            continue;
        };
        let Some(before) = &baseline_image else {
            stats.record_many(caller_id, "leaf-inline-blocked-unknown", count);
            continue;
        };
        // All bodies and storage remain in the emitted image: no speculative
        // credit for deleting the original callee, even after its last call.
        let growth = image.bytes.len().saturating_sub(before.bytes.len());
        let code_growth = image
            .routine_bytes(caller_id)
            .saturating_sub(before.routine_bytes(caller_id));
        let charged = growth.max(code_growth);
        if !within_growth_budget(
            expansion.sites,
            charged,
            caller_growth.get(&caller_id).copied().unwrap_or(0),
            program_growth,
            image.bytes.len().saturating_sub(initial_bytes),
        ) {
            stats.record_many(caller_id, "leaf-inline-blocked-budget", count);
            continue;
        }
        if baseline
            .routines
            .iter()
            .filter(|r| r.id != caller_id)
            .ne(trial.routines.iter().filter(|r| r.id != caller_id))
        {
            stats.record_many(caller_id, "leaf-inline-blocked-surrounding-code", count);
            continue;
        }
        let old_caller = baseline
            .routines
            .iter()
            .find(|r| r.id == caller_id)
            .unwrap();
        let new_caller = trial.routines.iter().find(|r| r.id == caller_id).unwrap();
        let Some(cycles) = saving(
            before,
            &image,
            old_caller,
            new_caller,
            callee_id,
            &expansion.origins,
            &expansion.sites_by_block,
        ) else {
            stats.record_many(caller_id, "leaf-inline-blocked-cost", count);
            continue;
        };
        stats.record_many(caller_id, "leaf-inline-applied", expansion.sites);
        stats.record_many(caller_id, "leaf-inline-growth-bytes", charged);
        stats.record_many(
            caller_id,
            "leaf-inline-estimated-cycles-saved",
            cycles as usize,
        );
        *caller_growth.entry(caller_id).or_default() += charged;
        program_growth += charged;
        current = candidate;
        baseline = trial;
        baseline_image = Some(image);
    }
    maybe_report_peepholes(&current, &stats, config);
    crate::mir6502::materialize_resolved_program(current, config, origin, runtime, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::leaf_routines::analyze;

    fn lower(source: &str) -> MirProgram {
        let tokens = crate::lexer::tokenize(source).unwrap();
        let ast = crate::parser::parse(&tokens).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
        crate::mir6502::lower_program(&nir).unwrap()
    }

    const SOURCE: &str = "BYTE input,output BYTE FUNC Map(BYTE value) IF (value AND $80)#0 THEN RETURN((value LSH 1) XOR $1B) FI RETURN(value LSH 1) PROC Main() output=Map(input) RETURN";

    #[test]
    fn costed_pipeline_accepts_small_leaf_and_keeps_original_body() {
        let program = lower(SOURCE);
        let mut config = Mir6502Config::optimized();
        config.enable_small_leaf_inlining = true;
        let materialized = crate::mir6502::materialize_program(program, &config).unwrap();
        assert_eq!(materialized.routines.len(), 2);
        assert!(
            !materialized.routines[1]
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(
                    op,
                    MirOp::Call {
                        target: MirCallTarget::Routine(RoutineId(0)),
                        ..
                    }
                )),
            "{}",
            crate::mir6502::format_program(&materialized)
        );
    }

    #[test]
    fn branching_leaf_expansion_preserves_public_returns_and_verifies() {
        let mut program = lower(SOURCE);
        let census = analyze(&program);
        assert_eq!(census.leaves.len(), 1, "{census:?}");
        let leaf = census.leaves.values().next().unwrap();
        let expanded = expand_group(&mut program.routines[1], leaf).unwrap();
        assert_eq!(expanded.sites, 1);
        crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
        let caller = &program.routines[1];
        assert_eq!(
            caller
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|op| return_store(op).is_some())
                .count(),
            2
        );
        assert!(
            !caller
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(op, MirOp::Call { .. }))
        );
        let materialized =
            crate::mir6502::materialize_program(program, &Mir6502Config::default()).unwrap();
        crate::mir6502::verify_program(&materialized, MirPhase::PreEmission).unwrap();
    }

    #[test]
    fn omitted_arguments_anywhere_reject_the_whole_callee() {
        let program = lower(&SOURCE.replace("output=Map(input)", "output=Map(input) output=Map()"));
        assert!(analyze(&program).leaves.is_empty());
    }

    #[test]
    fn repeated_sites_keep_distinct_clone_definitions() {
        let mut program =
            lower(&SOURCE.replace("output=Map(input)", "output=Map(input) output=Map(output)"));
        let census = analyze(&program);
        let leaf = census.leaves.values().next().unwrap();
        assert_eq!(
            expand_group(&mut program.routines[1], leaf).unwrap().sites,
            2
        );
        crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
        crate::mir6502::materialize_program(program, &Mir6502Config::default()).unwrap();
    }

    fn compile_image(source: &str, inline: bool) -> (MirProgram, Image) {
        let mut config = Mir6502Config::optimized();
        config.enable_small_leaf_inlining = inline;
        let program = crate::mir6502::materialize_program(lower(source), &config).unwrap();
        let image = Image::build(&program, 0x2000).unwrap();
        (program, image)
    }

    fn calls_to(program: &MirProgram, callee: RoutineId) -> usize {
        program.routines.iter().flat_map(|r| &r.blocks).flat_map(|b| &b.ops)
            .filter(|op| matches!(op, MirOp::Call { target: MirCallTarget::Routine(id), .. } if *id == callee)).count()
    }

    #[test]
    fn emitted_branches_and_public_returns_match_for_all_bytes() {
        let source = SOURCE.replace("BYTE input,output", "BYTE input=$600,output=$601");
        for source in [
            source.clone(),
            source.replace("output=Map(input)", "output=Map(input) output=Map(output)"),
            source.replace("Map(input)", "Map(input XOR $5A)"),
        ] {
            let (before, old) = compile_image(&source, false);
            let (after, new) = compile_image(&source, true);
            assert!(calls_to(&before, RoutineId(0)) > 0);
            assert_eq!(calls_to(&after, RoutineId(0)), 0);
            let entry = |image: &Image| {
                image
                    .blocks
                    .iter()
                    .find(|(r, _, _)| *r == RoutineId(1))
                    .unwrap()
                    .2
                    .start
            };
            for input in 0..=255 {
                let old_result =
                    super::super::leaf_test_cpu::run(&old.bytes, 0x2000, entry(&old), input);
                let new_result =
                    super::super::leaf_test_cpu::run(&new.bytes, 0x2000, entry(&new), input);
                assert_eq!(old_result, new_result, "input={input}");
                let map = |b: u8| (b << 1) ^ if b & 0x80 != 0 { 0x1b } else { 0 };
                let input = if source.contains("$5A") {
                    input ^ 0x5a
                } else {
                    input
                };
                let expected = if calls_to(&before, RoutineId(0)) == 1 {
                    map(input)
                } else {
                    map(map(input))
                };
                assert_eq!(new_result.0, expected);
            }
        }
    }

    #[test]
    fn two_byte_actuals_are_captured_before_expansion() {
        let source = "BYTE input=$600,output=$601 BYTE FUNC Mix(BYTE left,right) RETURN((left LSH 1) XOR right) PROC Main() output=Mix(input,input+1) RETURN";
        let (before, old) = compile_image(source, false);
        let (after, new) = compile_image(source, true);
        assert_eq!(calls_to(&before, RoutineId(0)), 1);
        assert_eq!(calls_to(&after, RoutineId(0)), 0);
        for input in 0..=255 {
            let run = |image: &Image| {
                super::super::leaf_test_cpu::run(
                    &image.bytes,
                    0x2000,
                    image
                        .blocks
                        .iter()
                        .find(|(r, _, _)| *r == RoutineId(1))
                        .unwrap()
                        .2
                        .start,
                    input,
                )
            };
            assert_eq!(run(&old), run(&new));
            assert_eq!(run(&new).0, (input << 1) ^ input.wrapping_add(1));
        }
    }

    #[test]
    fn effects_addresses_and_mutable_parameters_are_rejected() {
        let bodies = [
            "value==+1 RETURN(value)",
            "RETURN(input)",
            "BYTE ARRAY data(2) RETURN(data(value))",
            "BYTE POINTER p RETURN(p^)",
            "DO value==+1 UNTIL value=0 OD RETURN(value)",
        ];
        for body in bodies {
            let source = format!(
                "BYTE input,output BYTE FUNC Map(BYTE value) {body} PROC Main() output=Map(input) RETURN"
            );
            let census = analyze(&lower(&source));
            assert!(census.leaves.is_empty(), "accepted {body}: {census:?}");
        }
        let source = SOURCE
            .replace("BYTE input,output", "BYTE input,output CARD address")
            .replace("output=Map(input)", "address=Map output=Map(input)");
        assert!(analyze(&lower(&source)).leaves.is_empty());
    }

    #[test]
    fn site_budget_and_default_configuration_retain_calls() {
        assert!(!Mir6502Config::default().enable_small_leaf_inlining);
        let source = SOURCE.replace(
            "output=Map(input)",
            &"output=Map(input) ".repeat(MAX_SITES_PER_GROUP + 1),
        );
        let (program, _) = compile_image(&source, true);
        assert_eq!(calls_to(&program, RoutineId(0)), MAX_SITES_PER_GROUP + 1);
        let mut config = Mir6502Config::optimized();
        config.enable_peepholes = false;
        let program = crate::mir6502::materialize_program(lower(SOURCE), &config).unwrap();
        assert_eq!(calls_to(&program, RoutineId(0)), 1);
    }

    #[test]
    fn machine_state_callers_are_not_expanded() {
        let mut program = lower(SOURCE);
        let caller = &mut program.routines[1];
        caller.blocks[0].ops.insert(
            0,
            MirOp::Move {
                dst: MirDef::VTemp(MirTempId(1000)),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
            },
        );
        assert!(!caller_supported(caller));
        caller.blocks[0].ops.remove(0);
        caller.abi = MirRoutineAbi::ActionObservable;
        assert!(!caller_supported(caller));
    }

    #[test]
    fn growth_budgets_are_cumulative_and_inclusive() {
        assert!(within_growth_budget(1, 32, 96, 224, 256));
        assert!(!within_growth_budget(1, 33, 0, 0, 33));
        assert!(!within_growth_budget(2, 33, 96, 96, 129));
        assert!(!within_growth_budget(2, 33, 0, 224, 257));
        assert!(!within_growth_budget(2, 33, 0, 224, 0));
        assert!(!within_growth_budget(1, 0, 0, 0, 257));
    }

    #[test]
    fn expansion_composes_with_pointer_input_and_static_table_consumer() {
        let source = SOURCE
            .replace(
                "BYTE input,output",
                "BYTE ARRAY table=$700 BYTE output=$601",
            )
            .replace(
                "output=Map(input)",
                "BYTE POINTER p p=$600 output=table(Map(p^ XOR $5A))",
            );
        let (before, old) = compile_image(&source, false);
        // Exercise expansion composition independently of profitability. This
        // larger consumer currently fails the conservative region cost proof.
        let (costed, _) = compile_image(&source, true);
        assert_eq!(calls_to(&costed, RoutineId(0)), 1);
        let mut expanded = lower(&source);
        let leaf = analyze(&expanded).leaves.into_values().next().unwrap();
        expand_group(&mut expanded.routines[1], &leaf).unwrap();
        let mut config = Mir6502Config::optimized();
        config.enable_small_leaf_inlining = false;
        let after = crate::mir6502::materialize_program(expanded, &config).unwrap();
        let new = Image::build(&after, 0x2000).unwrap();
        assert_eq!(calls_to(&before, RoutineId(0)), 1);
        assert_eq!(calls_to(&after, RoutineId(0)), 0);
        for input in 0..=255 {
            let run = |image: &Image| {
                super::super::leaf_test_cpu::run(
                    &image.bytes,
                    0x2000,
                    image
                        .blocks
                        .iter()
                        .find(|(r, _, _)| *r == RoutineId(1))
                        .unwrap()
                        .2
                        .start,
                    input,
                )
            };
            assert_eq!(run(&old), run(&new));
            let b = input ^ 0x5a;
            assert_eq!(
                run(&new).0,
                ((b << 1) ^ if b & 0x80 != 0 { 0x1b } else { 0 }) ^ 0xa5
            );
        }
    }

    #[test]
    fn argument_call_is_not_duplicated_and_tail_calls_are_not_overcredited() {
        let source = "BYTE count=$602,output=$601 BYTE FUNC Get() count==+1 RETURN(count) BYTE FUNC Map(BYTE value) RETURN(value LSH 1) PROC Main() output=Map(Get()) RETURN";
        let (before, old) = compile_image(source, false);
        let (after, new) = compile_image(source, true);
        assert_eq!(calls_to(&after, RoutineId(0)), 1);
        assert_eq!(calls_to(&after, RoutineId(1)), 0);
        let run = |image: &Image| {
            super::super::leaf_test_cpu::run(
                &image.bytes,
                0x2000,
                image
                    .blocks
                    .iter()
                    .find(|(r, _, _)| *r == RoutineId(2))
                    .unwrap()
                    .2
                    .start,
                0,
            )
        };
        assert_eq!(run(&old), run(&new));
        assert_eq!(run(&new).0, 2);
        assert_eq!(calls_to(&before, RoutineId(1)), 1);
        let (tail, _) = compile_image("PROC Ping() RETURN PROC Main() Ping() RETURN", true);
        assert_eq!(
            calls_to(&tail, RoutineId(0)),
            1,
            "removing a tail JMP saves only 3 cycles, below the threshold"
        );
    }

    #[test]
    fn discarded_results_keep_public_store_and_loop_callers_verify() {
        let source = SOURCE.replace("output=Map(input)", "Map(input)");
        let (program, image) = compile_image(&source, true);
        assert_eq!(calls_to(&program, RoutineId(0)), 0);
        assert!(
            program.routines[1]
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(
                    op,
                    MirOp::Store {
                        dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xa0))),
                        ..
                    }
                ))
        );
        assert!(!image.bytes.is_empty());
        let source = SOURCE.replace(
            "output=Map(input)",
            "FOR output=0 TO 3 DO input=Map(input) OD",
        );
        let mut program = lower(&source);
        let leaf = analyze(&program).leaves.into_values().next().unwrap();
        expand_group(&mut program.routines[1], &leaf).unwrap();
        crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
        crate::mir6502::materialize_program(program, &Mir6502Config::default()).unwrap();
    }
}
