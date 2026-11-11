use super::*;
use crate::mir6502::{analysis::pointer_byte_plus_one::tests::baseline, ir::MirRoutine};

fn config() -> Mir6502Config {
    Mir6502Config {
        enable_pointer_byte_plus_one_fusion: true,
        ..Mir6502Config::optimized()
    }
}
fn selected(p: MirProgram) -> MirProgram {
    select_costed(p, &config(), 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
}
fn forward(p: &mut MirProgram) -> &mut MirRoutine {
    p.routines.iter_mut().find(|r| r.name == "Forward").unwrap()
}
fn start(r: &MirRoutine) -> usize {
    r.blocks[0]
        .ops
        .iter()
        .position(|o| matches!(o, MirOp::OffsetPointerByIndirectByte { .. }))
        .unwrap()
}

#[test]
fn pointer_byte_plus_one_selection_saves_five_bytes_and_is_stable_with_reporting() {
    let old = baseline();
    let new = selected(old.clone());
    assert_ne!(old, new);
    assert_eq!(
        image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        5
    );
    assert_eq!(selected(new.clone()), new);
    let reported = select_costed(
        old.clone(),
        &Mir6502Config {
            peephole_report: crate::mir6502::passes::MirPeepholeReportMode::PerRoutine,
            ..config()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(reported, new);
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        if a.name != "Forward" {
            assert_eq!(a, b);
        }
    }
}

#[test]
fn pointer_byte_plus_one_selection_reconstructs_ordered_read_certificates() {
    let mut p = baseline();
    let r = forward(&mut p);
    let i = start(r);
    let original = &r.blocks[0].ops[i..i + 2];
    let s = facts::shape(original).unwrap();
    assert!(facts::entry_read_homes(original, &[s.selected()]).is_some());
    let mut other = s;
    other.offset = 1;
    assert!(facts::entry_read_homes(original, &[other.selected()]).is_none());
    let mut other = s;
    other.dst = MirFixedZpSlot(0xAD);
    assert!(facts::entry_read_homes(original, &[other.selected()]).is_none());
    assert!(facts::entry_read_homes(&original[..1], &[s.selected()]).is_none());
    let removed = crate::mir6502::rewrite::posthome::removed_home_definitions(
        r.blocks[0].id,
        &(i..i + 2),
        &r.blocks[0].ops,
        &[s.selected()],
    );
    assert!(
        crate::mir6502::rewrite::posthome::replacement_home_dependencies(
            original,
            &removed,
            &[s.selected()]
        )
        .is_ok()
    );
    let wrong = facts::Shape { offset: 1, ..s }.selected();
    assert!(
        crate::mir6502::rewrite::posthome::replacement_home_dependencies(
            original,
            &removed,
            &[wrong]
        )
        .is_err()
    );
    let opaque = MirOp::Barrier {
        effects: MirEffects {
            opaque: true,
            ..MirEffects::default()
        },
    };
    assert!(
        crate::mir6502::rewrite::posthome::replacement_home_dependencies(
            original,
            &removed,
            &[opaque]
        )
        .is_err()
    );
}

#[test]
fn pointer_byte_plus_one_selection_preserves_published_x_and_accepts_an_independent_routine() {
    let mut p = baseline();
    let r = forward(&mut p);
    let i = start(r);
    r.blocks[0].ops.truncate(i + 2);
    r.blocks[0].ops.push(MirOp::LoadImm {
        dst: MirDef::Reg(MirReg::A),
        value: 0,
        width: MirWidth::Byte,
    });
    r.blocks[0].ops.push(MirOp::Binary {
        op: MirBinaryOp::Add,
        dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)),
        right: MirValue::ConstU8(0),
        width: MirWidth::Byte,
        carry_in: Some(MirCarryIn::Clear),
        carry_out: MirCarryOut::Ignore,
    });
    let id = r.id;
    let old_exits = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    assert!(old_exits.get(id).unwrap().preserves_register(MirReg::X));
    let r = p.routines.iter_mut().find(|r| r.name == "PlusTwo").unwrap();
    let i = start(r);
    if let MirOp::AddByteToWordMem { value, .. } = &mut r.blocks[0].ops[i + 1] {
        *value = MirValue::ConstU8(1);
    }
    let old = p.clone();
    let new = selected(p);
    assert_eq!(
        old.routines.iter().find(|r| r.id == id),
        new.routines.iter().find(|r| r.id == id)
    );
    assert_ne!(
        old.routines.iter().find(|r| r.name == "PlusTwo"),
        new.routines.iter().find(|r| r.name == "PlusTwo")
    );
    assert!(
        MirKnownCalleeSummaries::analyze_for_rewriting(&new)
            .get(id)
            .unwrap()
            .preserves_register(MirReg::X)
    );
}

#[test]
fn pointer_byte_plus_one_unknown_cost_keeps_the_entire_baseline() {
    let mut old = baseline();
    let r = old
        .routines
        .iter_mut()
        .find(|r| r.name == "PlusZero")
        .unwrap();
    if let Some(MirOp::Call { target, .. }) = r.blocks[0]
        .ops
        .iter_mut()
        .find(|o| matches!(o, MirOp::Call { .. }))
    {
        *target = MirCallTarget::Routine(RoutineId(u32::MAX));
    }
    assert!(image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).is_none());
    assert_eq!(selected(old.clone()), old);
}

#[test]
fn pointer_byte_plus_one_selection_blocks_each_live_flag_across_successors() {
    use crate::mir6502::{
        analysis::{posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration},
        ir::{MirCond, MirEdge, MirFlagTest},
    };
    for flag in [
        MirFlagTest::CSet,
        MirFlagTest::ZSet,
        MirFlagTest::NSet,
        MirFlagTest::VSet,
    ] {
        let mut p = baseline();
        let r = forward(&mut p);
        let i = start(r);
        let tail = r.blocks[0].ops.split_off(i + 2);
        let block = r.blocks[0].clone();
        r.blocks[0].terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(flag),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        };
        for id in [1, 2] {
            r.blocks.push(crate::mir6502::ir::MirBlock {
                id: MirBlockId(id),
                ops: tail.clone(),
                terminator: MirTerminator::Return,
                ..block.clone()
            });
        }
        let snapshot = PostHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let context = PostHomeRewriteContext::new(&snapshot);
        assert_eq!(
            context.pointer_byte_plus_one(context.point(MirSite::Op {
                block: r.blocks[0].id,
                op_index: i
            })),
            Err(facts::Blocker::MachineState)
        );
    }
}

#[test]
fn pointer_byte_plus_one_selection_rebuilds_neighboring_plans_and_counts_each_once() {
    let mut p = baseline();
    let r = forward(&mut p);
    let id = r.id;
    let i = start(r);
    let pair = r.blocks[0].ops[i..i + 2].to_vec();
    r.blocks[0].ops.splice(i + 2..i + 2, pair);
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let (new, stats, affected) = apply(&p, &summaries, None).unwrap();
    assert!(affected.contains(&id));
    assert_eq!(stats.count_for(id, "pointer-byte-plus-one-candidate"), 2);
    assert_eq!(stats.count_for(id, "pointer-byte-plus-one-selected"), 2);
    assert_eq!(selected(p.clone()), new);
    assert_eq!(
        image_size(&p, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        10
    );
}

#[test]
fn pointer_byte_plus_one_driver_rejects_stale_and_invalidated_region_proofs() {
    use crate::mir6502::{
        analysis::{posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration},
        rewrite::driver::MirRewriteError,
    };
    let mut p = baseline();
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let mut r = forward(&mut p).clone();
    let mut saved: Option<MirPostHomeRewritePlan> = None;
    let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_known_callees(
        &mut r,
        &summaries,
        |r, c| {
            if let Some(plan) = &saved {
                return vec![plan.clone()];
            }
            let plans = discover(r, c, &mut BTreeSet::new());
            saved = plans.first().cloned();
            plans
        },
    );
    assert!(matches!(result, Err(MirRewriteError::StalePlan { .. })));
    let mut r = forward(&mut p).clone();
    let i = start(&r);
    let snapshot = PostHomeAnalysisSnapshot::new_with_known_callees(
        &r,
        MirRoutineGeneration::initial(),
        &summaries,
    )
    .unwrap();
    let plan = discover(
        &r,
        &PostHomeRewriteContext::new(&snapshot),
        &mut BTreeSet::new(),
    )
    .remove(0);
    r.blocks[0].ops.insert(
        i + 2,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x685)),
            src: MirValue::Def(MirDef::Reg(MirReg::X)),
            width: MirWidth::Byte,
        },
    );
    let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_known_callees(
        &mut r,
        &summaries,
        |_, _| vec![plan.clone()],
    );
    assert!(matches!(
        result,
        Err(MirRewriteError::InvalidDeclaration { .. })
    ));
}
