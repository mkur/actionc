use super::*;
use crate::mir6502::analysis::argument_x_forwarding::tests::{baseline, memory, site};
use crate::mir6502::rewrite::driver::MirRewriteError;
fn selected(p: MirProgram) -> MirProgram {
    select_costed(
        p,
        &Mir6502Config {
            enable_argument_x_forwarding: true,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap()
}
fn ids(r: &MirRoutine) -> BTreeMap<MirBlockId, Vec<usize>> {
    r.blocks
        .iter()
        .map(|b| (b.id, (0..b.ops.len()).collect()))
        .collect()
}
#[test]
fn argument_x_forwarding_selection_saves_four_bytes_without_removing_stores_or_ops() {
    let old = baseline();
    let m = memory(&old);
    let (new, stats, affected) = apply(&old, &m, None).unwrap();
    assert_eq!(affected.len(), 3);
    assert_eq!(selected(old.clone()), new);
    assert_eq!(selected(new.clone()), new);
    assert_eq!(
        image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        4
    );
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        assert_eq!(
            a.blocks.iter().map(|b| b.ops.len()).sum::<usize>(),
            b.blocks.iter().map(|b| b.ops.len()).sum::<usize>()
        );
        let stores = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|o| matches!(o, MirOp::Store { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(stores(a), stores(b));
        if affected.contains(&a.id) {
            assert_eq!(stats.count_for(a.id, "argument-x-forwarding-selected"), 1);
        } else {
            assert_eq!(a, b);
        }
    }
    let report = select_costed(
        old,
        &Mir6502Config {
            enable_argument_x_forwarding: true,
            peephole_report: crate::mir6502::MirPeepholeReportMode::Sites,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(report, new);
}
#[test]
fn argument_x_forwarding_driver_rejects_missing_altered_and_stale_witnesses() {
    let p = baseline();
    let m = memory(&p);
    let r = p
        .routines
        .iter()
        .find(|r| r.name == "Forward")
        .unwrap()
        .clone();
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let mut saved = None;
    MirPostHomeRewriteDriver::default()
        .run_fixed_point_with_physical_memory(&mut r.clone(), &summaries, &m, |r, c| {
            if saved.is_some() {
                return vec![];
            }
            let plans = discover(r, c, &m, &ids(r), &mut BTreeSet::new());
            saved = plans.first().cloned();
            plans
        })
        .unwrap();
    let plan = saved.unwrap();
    for change in 0..3 {
        let mut bad = plan.clone();
        match change {
            0 => bad.witness = None,
            1 => bad.replacement.swap(0, 1),
            _ => {
                if let Some(MirPostHomeWitness::ArgumentXForwarding(p)) = bad.witness.as_mut() {
                    p.address += 1;
                }
            }
        }
        let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
            &mut r.clone(),
            &summaries,
            &m,
            |_, _| vec![bad.clone()],
        );
        assert!(
            matches!(result, Err(MirRewriteError::InvalidDeclaration { .. })),
            "{result:?}"
        );
    }
    let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
        &mut r.clone(),
        &summaries,
        &m,
        |_, _| vec![plan.clone()],
    );
    assert!(matches!(result, Err(MirRewriteError::StalePlan { .. })));
    let result = MirPostHomeRewriteDriver::default()
        .run_fixed_point(&mut r.clone(), |_, _| vec![plan.clone()]);
    assert!(matches!(
        result,
        Err(MirRewriteError::InvalidDeclaration { .. })
    ));
}
#[test]
fn argument_x_forwarding_keeps_unqualified_flags_and_selects_independent_routines() {
    let mut p = baseline();
    let r = p
        .routines
        .iter_mut()
        .find(|r| r.name == "ForwardByte")
        .unwrap();
    let s = facts::shape(r, site(r)).unwrap();
    let MirSite::Op { op_index, .. } = s.call else {
        unreachable!()
    };
    if let MirOp::Call { effects, .. } = &mut r.blocks[0].ops[op_index] {
        effects.reads.flags = true;
    }
    let unchanged = r.clone();
    let new = selected(p.clone());
    assert_eq!(
        new.routines.iter().find(|r| r.id == unchanged.id).unwrap(),
        &unchanged
    );
    assert_eq!(
        image_size(&p, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        2
    );
}
#[test]
fn argument_x_forwarding_unknown_cost_keeps_the_entire_baseline() {
    let mut p = baseline();
    let r = p.routines.iter_mut().find(|r| r.name == "Plain").unwrap();
    let call = r.blocks[0]
        .ops
        .iter_mut()
        .find(|o| matches!(o, MirOp::Call { .. }))
        .unwrap();
    if let MirOp::Call { target, .. } = call {
        *target = MirCallTarget::Routine(RoutineId(u32::MAX));
    }
    assert_eq!(selected(p.clone()), p);
}
