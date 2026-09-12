use super::*;
use crate::mir6502::ir::MirEdgeArg;
use crate::mir6502::rewrite::driver::{MirPreHomeRewriteDriver, MirRewriteError};
use crate::mir6502::rewrite::pilots::discover_compare_branch_copies;
use crate::runtime::Runtime;

fn value(id: u32) -> MirValue {
    MirValue::Def(MirDef::VTemp(MirTempId(id)))
}

fn store(src: MirValue) -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0x620)),
        src,
        width: MirWidth::Byte,
    }
}

fn branch(condition: MirCond) -> MirTerminator {
    MirTerminator::Branch {
        cond: condition,
        then_edge: MirEdge::plain(MirBlockId(1)),
        else_edge: MirEdge::plain(MirBlockId(2)),
    }
}

fn copy_routine(copies: u32, width: MirWidth, signed: bool, op: MirCompareOp) -> MirRoutine {
    let mut program = empty_test_program();
    let routine = &mut program.routines[0];
    routine.temps = (0..copies + 3)
        .map(|id| MirTemp { id: MirTempId(id) })
        .collect();
    let mut ops = Vec::new();
    for (id, address) in [(copies + 1, 0x600), (copies + 2, 0x602)] {
        ops.push(MirOp::Load {
            dst: MirDef::VTemp(MirTempId(id)),
            src: MirAddr::Direct(MirMem::Absolute(address)),
            width,
        });
    }
    ops.push(MirOp::Compare {
        dst: MirCondDest::Temp(MirTempId(0)),
        op,
        left: value(copies + 1),
        right: value(copies + 2),
        width,
        signed,
    });
    for id in 1..=copies {
        ops.push(MirOp::Move {
            dst: MirDef::VTemp(MirTempId(id)),
            src: value(id - 1),
            width: MirWidth::Byte,
        });
    }
    routine.blocks = vec![
        MirBlock {
            id: MirBlockId(0),
            label: "entry".into(),
            params: vec![],
            ops,
            terminator: branch(MirCond::BoolValue(value(copies))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "yes".into(),
            params: vec![],
            ops: vec![store(MirValue::ConstU8(17))],
            terminator: MirTerminator::Return,
        },
        MirBlock {
            id: MirBlockId(2),
            label: "no".into(),
            params: vec![],
            ops: vec![store(MirValue::ConstU8(29))],
            terminator: MirTerminator::Return,
        },
    ];
    program.routines.remove(0)
}

fn simple() -> MirRoutine {
    copy_routine(1, MirWidth::Byte, false, MirCompareOp::Ge)
}

fn assert_blocked(mut routine: MirRoutine, reason: Option<&str>) {
    let before = routine.clone();
    let mut observations = BTreeSet::new();
    let result = MirPreHomeRewriteDriver::default()
        .run_fixed_point(&mut routine, |r, c| {
            discover_compare_branch_copies(r, c, &mut observations)
        })
        .unwrap();
    assert_eq!(result.applied, 0);
    assert_eq!(routine, before);
    if let Some(reason) = reason {
        assert!(
            observations.contains(&(MirBlockId(0), reason)),
            "{observations:?}"
        );
    }
}

#[test]
fn compare_branch_copy_folds_supported_predicates_and_chains_idempotently() {
    for (width, signed) in [
        (MirWidth::Byte, false),
        (MirWidth::Word, false),
        (MirWidth::Word, true),
    ] {
        for op in [
            MirCompareOp::Eq,
            MirCompareOp::Ne,
            MirCompareOp::Lt,
            MirCompareOp::Le,
            MirCompareOp::Gt,
            MirCompareOp::Ge,
        ] {
            for copies in [1, 3, 40] {
                let mut routine = copy_routine(copies, width, signed, op);
                let terminator = routine.blocks[0].terminator.clone();
                let mut expected = routine.blocks[0].ops[2].clone();
                if let MirOp::Compare { dst, .. } = &mut expected {
                    *dst = MirCondDest::Temp(MirTempId(copies));
                }
                let mut stats = MirPeepholeStats::default();
                run_analyzed_compare_branch_copies(&mut routine, &mut stats).unwrap();
                assert_eq!(routine.blocks[0].ops.len(), 3);
                assert_eq!(routine.blocks[0].ops[2], expected);
                assert_eq!(routine.blocks[0].terminator, terminator);
                assert_eq!(
                    stats.count_for(routine.id, "compare-branch-copy-elided"),
                    copies as usize
                );
                let folded = routine.clone();
                run_analyzed_compare_branch_copies(&mut routine, &mut stats).unwrap();
                assert_eq!(routine, folded);
                assert_eq!(
                    stats.count_for(routine.id, "compare-branch-copy-selected"),
                    1
                );
            }
        }
    }
}

#[test]
fn compare_branch_copy_rejects_other_uses_on_successors_and_backedges() {
    for temp in [0, 1] {
        for use_value in [
            value(temp),
            MirValue::Def(MirDef::VTempByte {
                id: MirTempId(temp),
                byte: 0,
            }),
            MirValue::Def(MirDef::VTempByte {
                id: MirTempId(temp),
                byte: 1,
            }),
        ] {
            for successor in [1, 2] {
                let mut routine = simple();
                routine.blocks[successor].ops.push(store(use_value.clone()));
                assert_blocked(routine.clone(), Some("compare-branch-copy-blocked-use"));
                routine.blocks[successor].terminator =
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(0)));
                assert_blocked(routine, Some("compare-branch-copy-blocked-use"));
            }
        }
    }
}

#[test]
fn compare_branch_copy_rejects_edge_arguments_even_for_unrelated_values() {
    for temp in [0, 1, 2] {
        for then in [false, true] {
            let mut routine = simple();
            let MirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &mut routine.blocks[0].terminator
            else {
                unreachable!()
            };
            let edge = if then { then_edge } else { else_edge };
            edge.args.push(MirEdgeArg {
                value: value(temp),
                width: MirWidth::Byte,
            });
            routine.blocks[if then { 1 } else { 2 }].params.push(
                crate::mir6502::ir::MirBlockParam {
                    dest: MirTempId(4),
                    width: MirWidth::Byte,
                },
            );
            assert_blocked(routine, Some("compare-branch-copy-blocked-edge-args"));
        }
    }
}

#[test]
fn compare_branch_copy_rejects_reused_ids_and_operand_dependencies() {
    for temp in [0, 1] {
        let mut routine = simple();
        routine.blocks[0].ops.insert(
            0,
            MirOp::LoadImm {
                dst: MirDef::VTemp(MirTempId(temp)),
                value: 42,
                width: MirWidth::Byte,
            },
        );
        assert_blocked(routine.clone(), Some("compare-branch-copy-blocked-use"));
        if let MirOp::Compare { left, .. } = &mut routine.blocks[0].ops[3] {
            *left = value(temp);
        }
        assert_blocked(routine, Some("compare-branch-copy-blocked-use"));
    }
}

#[test]
fn compare_branch_copy_requires_a_contiguous_private_byte_suffix() {
    let effects = MirEffects::default();
    for intruder in [
        store(MirValue::ConstU8(7)),
        MirOp::Barrier {
            effects: effects.clone(),
        },
        MirOp::MachineBlock {
            id: MirMachineBlockId(0),
            effects: effects.clone(),
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                params: vec![],
                result: None,
                additional_results: vec![],
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: vec![],
            result: None,
            additional_results: vec![],
            effects,
        },
    ] {
        for offset in [3, 4] {
            let mut routine = simple();
            routine.blocks[0].ops.insert(offset, intruder.clone());
            assert_blocked(routine, None);
        }
    }
    for replacement in [
        MirOp::Move {
            dst: MirDef::VTemp(MirTempId(1)),
            src: value(0),
            width: MirWidth::Word,
        },
        MirOp::Move {
            dst: MirDef::VTemp(MirTempId(1)),
            src: value(1),
            width: MirWidth::Byte,
        },
        MirOp::Move {
            dst: MirDef::VTemp(MirTempId(1)),
            src: MirValue::Def(MirDef::VTempByte {
                id: MirTempId(0),
                byte: 0,
            }),
            width: MirWidth::Byte,
        },
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: value(0),
            width: MirWidth::Byte,
        },
    ] {
        let mut routine = simple();
        routine.blocks[0].ops[3] = replacement;
        assert_blocked(routine, None);
    }
    assert_blocked(
        copy_routine(0, MirWidth::Byte, false, MirCompareOp::Ge),
        None,
    );
    assert_blocked(
        copy_routine(1, MirWidth::Byte, true, MirCompareOp::Ge),
        None,
    );
}

#[test]
fn compare_branch_copy_rejects_live_machine_state_on_either_edge() {
    for successor in [1, 2] {
        for reg in [MirReg::A, MirReg::X, MirReg::Y] {
            let mut routine = simple();
            routine.blocks[successor]
                .ops
                .insert(0, store(MirValue::Def(MirDef::Reg(reg))));
            assert_blocked(routine, Some("compare-branch-copy-blocked-machine-state"));
        }
        for flag in [
            MirFlagTest::CSet,
            MirFlagTest::ZSet,
            MirFlagTest::NSet,
            MirFlagTest::VSet,
        ] {
            let mut routine = simple();
            routine.blocks[successor].ops.clear();
            routine.blocks[successor].terminator = branch(MirCond::FlagTest(flag));
            assert_blocked(routine, Some("compare-branch-copy-blocked-machine-state"));
        }
    }
}

#[test]
fn compare_branch_copy_plans_are_generation_scoped() {
    let mut routine = simple();
    let snapshot = crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot::new(
        &routine,
        MirRoutineGeneration::initial(),
    )
    .unwrap();
    let plans = discover_compare_branch_copies(
        &routine,
        &PreHomeRewriteContext::new(&snapshot),
        &mut BTreeSet::new(),
    );
    assert_eq!(plans.len(), 1);
    drop(snapshot);
    let mut driver = MirPreHomeRewriteDriver::default();
    driver.apply_batch(&mut routine, plans.clone()).unwrap();
    assert!(matches!(
        driver.apply_batch(&mut routine, plans),
        Err(MirRewriteError::StalePlan { .. })
    ));
}

#[test]
fn compare_branch_copy_runs_before_value_expansion_and_respects_configuration() {
    for config in [
        Mir6502Config::default(),
        Mir6502Config::optimized(),
        Mir6502Config {
            enable_peepholes: false,
            ..Mir6502Config::default()
        },
    ] {
        let mut routine = simple();
        let layout = MaterializeLayout::new(&empty_test_program(), 0x3000);
        let mut stats = MirPeepholeStats::default();
        run_prehome_canonicalization_group(&mut routine, &config, &layout, &mut stats).unwrap();
        let has_boolean_join = routine
            .blocks
            .iter()
            .any(|b| b.label.starts_with("cmp_value_join_"));
        assert_eq!(has_boolean_join, !config.enable_peepholes);
        assert_eq!(
            stats.count_for(routine.id, "compare-branch-copy-elided"),
            usize::from(config.enable_peepholes)
        );
    }
}

#[test]
fn compare_branch_copy_source_probe_emits_a_direct_predicate_branch() {
    let tokens = crate::lexer::tokenize(include_str!(
        "../../../fixtures/mir6502/compare_branch_copy.act"
    ))
    .unwrap();
    let ast = crate::parser::parse(&tokens).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
    let raw = crate::mir6502::lower_program(&nir).unwrap();
    assert!(
        raw.routines
            .iter()
            .flat_map(|r| &r.blocks)
            .any(|b| compare_branch_copy_candidate(b).is_some())
    );
    for runtime in [Runtime::Standalone, Runtime::ActionCart] {
        for config in [Mir6502Config::default(), Mir6502Config::optimized()] {
            let program = crate::mir6502::materialize_program_with_origin_and_runtime(
                raw.clone(),
                &config,
                0x3000,
                runtime,
            )
            .unwrap();
            crate::mir6502::verify_program(&program, crate::mir6502::MirPhase::PreEmission)
                .unwrap();
            let main = program.routines.iter().find(|r| r.name == "Main").unwrap();
            assert!(
                !main
                    .blocks
                    .iter()
                    .any(|b| b.label.starts_with("cmp_value_")),
                "{}",
                crate::mir6502::format_program(&program)
            );
            assert!(main.blocks.iter().any(|b| {
                b.ops.iter().any(|op| {
                    matches!(
                        op,
                        MirOp::Compare {
                            dst: MirCondDest::Flags,
                            op: MirCompareOp::Ge,
                            right: MirValue::ConstU8(4),
                            ..
                        }
                    )
                }) && matches!(
                    b.terminator,
                    MirTerminator::Branch {
                        cond: MirCond::FlagTest(MirFlagTest::CSet)
                            | MirCond::FusedCompare {
                                flag_test: MirFlagTest::CSet,
                                ..
                            },
                        ..
                    }
                )
            }));
        }
    }
}
