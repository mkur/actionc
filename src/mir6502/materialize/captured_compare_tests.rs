use super::*;
use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
use crate::mir6502::ir::{MirEdgeArg, MirTerminator};
use crate::mir6502::rewrite::driver::{MirPreHomeRewriteDriver, MirRewriteError};
use crate::mir6502::rewrite::pilots::{
    captured_byte_compare_rank, discover_captured_byte_compares,
};

fn apply(routine: &mut MirRoutine) -> (usize, BTreeSet<(MirBlockId, &'static str)>) {
    let mut observations = BTreeSet::new();
    let result = MirPreHomeRewriteDriver::default()
        .run_fixed_point_by_key(
            routine,
            |routine, context| discover_captured_byte_compares(routine, context, &mut observations),
            captured_byte_compare_rank,
        )
        .unwrap();
    (result.applied, observations)
}

fn assert_blocked(mut routine: MirRoutine, reason: Option<&str>) {
    let original = routine.clone();
    let (selected, observations) = apply(&mut routine);
    assert_eq!(selected, 0);
    assert_eq!(routine, original);
    if let Some(reason) = reason {
        assert!(
            observations.iter().any(|(_, observed)| *observed == reason),
            "{observations:?}"
        );
    }
}

#[test]
fn captured_compare_keeps_indexed_address_forms_and_branch_edges() {
    for op in [MirCompareOp::Le, MirCompareOp::Gt] {
        for source in [
            MirAddr::ComputedIndex {
                base: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                index: MirValue::ConstU8(2),
                elem_size: 1,
                offset: 1,
            },
            MirAddr::PointerIndex {
                ptr: MirMem::Param {
                    id: ParamId(0),
                    offset: 0,
                },
                index: MirValue::ConstU8(2),
                elem_size: 1,
                offset: 1,
            },
        ] {
            let mut routine = captured_byte_compare_routine(op);
            if let MirOp::Load { src, .. } = &mut routine.blocks[0].ops[2] {
                *src = source.clone();
            }
            let original = routine.clone();
            assert_eq!(apply(&mut routine).0, 1);
            assert_eq!(&routine.blocks[0].ops[..2], &original.blocks[0].ops[..2]);
            assert!(matches!(&routine.blocks[0].ops[2],
                MirOp::Load { dst: MirDef::Reg(MirReg::A), src, width: MirWidth::Byte } if src == &source));
            assert_eq!(routine.blocks[0].terminator, original.blocks[0].terminator);
        }
    }
}

#[test]
fn captured_compare_keeps_other_capture_uses_and_rejects_loaded_or_condition_uses() {
    for temp in [0, 1, 2] {
        let mut routine = captured_byte_compare_routine(MirCompareOp::Le);
        let extra = MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x680)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(temp))),
            width: MirWidth::Byte,
        };
        routine.blocks[1].ops.push(extra.clone());
        if temp == 1 {
            assert_eq!(apply(&mut routine).0, 1);
            assert_eq!(routine.blocks[1].ops, vec![extra]);
        } else {
            assert_blocked(
                routine,
                Some(if temp == 0 {
                    "captured-byte-compare-blocked-loaded-use"
                } else {
                    "captured-byte-compare-blocked-condition-use"
                }),
            );
        }
    }
}

#[test]
fn captured_compare_checks_captures_backedges_and_edge_arguments() {
    let mut redefined = captured_byte_compare_routine(MirCompareOp::Le);
    redefined.blocks[0].ops.insert(
        1,
        MirOp::LoadImm {
            dst: MirDef::VTemp(MirTempId(1)),
            value: 7,
            width: MirWidth::Byte,
        },
    );
    assert_blocked(redefined, Some("captured-byte-compare-blocked-capture"));

    let mut backedge = captured_byte_compare_routine(MirCompareOp::Le);
    backedge.blocks[1].ops.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0x680)),
        src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
        width: MirWidth::Byte,
    });
    backedge.blocks[1].terminator = MirTerminator::Jump(MirEdge::plain(MirBlockId(0)));
    assert_blocked(backedge, Some("captured-byte-compare-blocked-loaded-use"));

    let mut edge = captured_byte_compare_routine(MirCompareOp::Le);
    edge.temps.push(MirTemp { id: MirTempId(4) });
    edge.blocks[1]
        .params
        .push(crate::mir6502::ir::MirBlockParam {
            dest: MirTempId(4),
            width: MirWidth::Byte,
        });
    if let MirTerminator::Branch { then_edge, .. } = &mut edge.blocks[0].terminator {
        then_edge.args.push(MirEdgeArg {
            value: MirValue::Def(MirDef::VTemp(MirTempId(0))),
            width: MirWidth::Byte,
        });
    }
    assert_blocked(edge, Some("captured-byte-compare-blocked-loaded-use"));

    let mut joined = captured_byte_compare_routine(MirCompareOp::Le);
    let mut join = joined.blocks[0].clone();
    join.id = MirBlockId(3);
    join.ops.remove(0);
    if let MirTerminator::Branch {
        then_edge,
        else_edge,
        ..
    } = &mut join.terminator
    {
        then_edge.target = MirBlockId(4);
        else_edge.target = MirBlockId(5);
    }
    joined.temps.push(MirTemp { id: MirTempId(4) });
    joined.blocks[0].ops = vec![MirOp::LoadImm {
        dst: MirDef::VTemp(MirTempId(4)),
        value: 1,
        width: MirWidth::Byte,
    }];
    if let MirTerminator::Branch { cond, .. } = &mut joined.blocks[0].terminator {
        *cond = MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(4))));
    }
    for index in [1, 2] {
        joined.blocks[index].ops = vec![MirOp::LoadImm {
            dst: MirDef::VTemp(MirTempId(1)),
            value: index as u16,
            width: MirWidth::Byte,
        }];
        joined.blocks[index].terminator = MirTerminator::Jump(MirEdge::plain(MirBlockId(3)));
    }
    joined.blocks.push(join);
    for id in [4, 5] {
        joined.blocks.push(MirBlock {
            id: MirBlockId(id),
            label: format!("exit{id}"),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        });
    }
    assert_blocked(joined, Some("captured-byte-compare-blocked-capture"));
}

#[test]
fn captured_compare_rejects_live_machine_outputs_and_effect_gaps() {
    for reg in [MirReg::A, MirReg::X, MirReg::Y] {
        let mut routine = captured_byte_compare_routine(MirCompareOp::Le);
        routine.blocks[1].ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x680)),
            src: MirValue::Def(MirDef::Reg(reg)),
            width: MirWidth::Byte,
        });
        assert_blocked(routine, Some("captured-byte-compare-blocked-machine-state"));
    }
    for flag in [
        MirFlagTest::CSet,
        MirFlagTest::ZSet,
        MirFlagTest::NSet,
        MirFlagTest::VSet,
    ] {
        let mut routine = captured_byte_compare_routine(MirCompareOp::Le);
        routine.blocks[1].terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(flag),
            then_edge: MirEdge::plain(MirBlockId(2)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        };
        assert_blocked(routine, Some("captured-byte-compare-blocked-machine-state"));
    }
    let effects = MirEffects::default();
    for intruder in [
        MirOp::Barrier {
            effects: effects.clone(),
        },
        MirOp::MachineBlock {
            id: MirMachineBlockId(0),
            effects: effects.clone(),
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x680)),
            src: MirValue::ConstU8(7),
            width: MirWidth::Byte,
        },
        MirOp::Call {
            target: crate::mir6502::ir::MirCallTarget::Routine(RoutineId(1)),
            abi: crate::mir6502::ir::MirCallAbi {
                params: Vec::new(),
                result: None,
                additional_results: Vec::new(),
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            additional_results: Vec::new(),
            effects,
        },
    ] {
        let mut gap = captured_byte_compare_routine(MirCompareOp::Le);
        gap.blocks[0].ops.insert(3, intruder);
        assert_blocked(gap, None);
    }
}

#[test]
fn captured_compare_keeps_signed_word_and_other_predicate_fallbacks() {
    for op in [
        MirCompareOp::Eq,
        MirCompareOp::Ne,
        MirCompareOp::Lt,
        MirCompareOp::Ge,
    ] {
        assert_blocked(captured_byte_compare_routine(op), None);
    }
    let mut signed = captured_byte_compare_routine(MirCompareOp::Le);
    if let MirOp::Compare { signed, .. } = &mut signed.blocks[0].ops[3] {
        *signed = true;
    }
    assert_blocked(signed, None);
    let mut word = captured_byte_compare_routine(MirCompareOp::Le);
    if let MirOp::Compare { width, .. } = &mut word.blocks[0].ops[3] {
        *width = MirWidth::Word;
    }
    assert_blocked(word, None);
    let mut wide_capture = captured_byte_compare_routine(MirCompareOp::Le);
    if let MirOp::Load { width, .. } = &mut wide_capture.blocks[0].ops[0] {
        *width = MirWidth::Word;
    }
    assert_blocked(wide_capture, Some("captured-byte-compare-blocked-capture"));
}

#[test]
fn captured_compare_rejects_stale_transactions() {
    let mut routine = captured_byte_compare_routine(MirCompareOp::Le);
    let mut driver = MirPreHomeRewriteDriver::default();
    let snapshot = PreHomeAnalysisSnapshot::new(&routine, driver.generation()).unwrap();
    let context = PreHomeRewriteContext::new(&snapshot);
    let plans = discover_captured_byte_compares(&routine, &context, &mut BTreeSet::new());
    drop(snapshot);
    driver.apply_batch(&mut routine, plans.clone()).unwrap();
    assert!(matches!(
        driver.apply_batch(&mut routine, plans),
        Err(MirRewriteError::StalePlan { .. })
    ));
}

#[test]
fn captured_compare_emitted_predicates_match_all_unsigned_byte_pairs() {
    let source =
        include_str!("../../../fixtures/runtime/direct_comparisons.act").replace("\r\n", "\n");
    let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    for enabled in [false, true] {
        let config = Mir6502Config {
            enable_captured_byte_comparisons: enabled,
            ..Mir6502Config::optimized()
        };
        let output = crate::mir6502::generate_output_with_config_and_runtime(
            &nir,
            0x3000,
            &config,
            crate::runtime::Runtime::Standalone,
        )
        .unwrap();
        let mut memory = [0u8; 65536];
        memory[0x3000..0x3000 + output.bytes.len()].copy_from_slice(&output.bytes);
        for name in ["AtMost", "Greater"] {
            let entry = output
                .routine_addresses
                .iter()
                .find(|r| r.name.eq_ignore_ascii_case(name))
                .unwrap()
                .address;
            memory[0x2000..0x2008].copy_from_slice(&[
                0xA9,
                0x00,
                0xA2,
                0x70,
                0x20,
                entry as u8,
                (entry >> 8) as u8,
                0x60,
            ]);
            for left in 0..=255u8 {
                for right in 0..=255u8 {
                    memory[0xE0] = left;
                    memory[0x7000] = right;
                    super::super::leaf_test_cpu::run_memory(&mut memory, 0x2000);
                    let truth = if name == "Greater" {
                        left > right
                    } else {
                        left <= right
                    };
                    assert_eq!(
                        memory[0x680],
                        if truth { 17 } else { 29 },
                        "{name}/{enabled}/{left}/{right}"
                    );
                    assert_eq!(memory[0xE0], left);
                    assert_eq!(memory[0x7000], right);
                }
            }
        }
    }
}
