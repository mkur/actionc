use super::*;
use crate::mir6502::analysis::pointer_load_sharing::tests::{baseline, forward, memory, start};
use crate::mir6502::rewrite::driver::MirRewriteError;

fn selected(p: MirProgram) -> MirProgram {
    select_costed(
        p,
        &Mir6502Config {
            enable_pointer_load_sharing: true,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap()
}
fn ids(r: &crate::mir6502::ir::MirRoutine) -> BTreeMap<MirBlockId, Vec<usize>> {
    r.blocks
        .iter()
        .map(|b| (b.id, (0..b.ops.len()).collect()))
        .collect()
}

#[test]
fn pointer_load_sharing_selection_saves_six_bytes_and_is_idempotent() {
    let old = baseline();
    let m = memory(&old);
    let (proposed, stats, affected) = apply(&old, &m, None).unwrap();
    assert_eq!(affected.len(), 1);
    let id = old
        .routines
        .iter()
        .find(|r| r.name == "Forward")
        .unwrap()
        .id;
    assert_eq!(stats.count_for(id, "pointer-load-sharing-candidate"), 1);
    assert_eq!(stats.count_for(id, "pointer-load-sharing-selected"), 1);
    let new = selected(old.clone());
    assert_eq!(new, proposed);
    assert_eq!(selected(new.clone()), new);
    assert_eq!(
        image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        6
    );
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        if a.name != "Forward" {
            assert_eq!(a, b);
        }
    }
    let reported = select_costed(
        old,
        &Mir6502Config {
            enable_pointer_load_sharing: true,
            peephole_report: crate::mir6502::MirPeepholeReportMode::Sites,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(reported, new);
}

#[test]
fn pointer_load_sharing_driver_rejects_missing_changed_and_stale_proofs() {
    let mut p = baseline();
    let m = memory(&p);
    let r = forward(&mut p).clone();
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
        let mut wrong = plan.clone();
        match change {
            0 => wrong.pointer_load_sharing = None,
            1 => wrong.replacement.swap(1, 3),
            _ => wrong.pointer_load_sharing.as_mut().unwrap().source_address += 1,
        }
        let error = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
            &mut r.clone(),
            &summaries,
            &m,
            |_, _| vec![wrong.clone()],
        );
        assert!(
            matches!(error, Err(MirRewriteError::InvalidDeclaration { .. })),
            "{error:?}"
        );
    }
    let no_map = MirPostHomeRewriteDriver::default()
        .run_fixed_point(&mut r.clone(), |_, _| vec![plan.clone()]);
    assert!(matches!(
        no_map,
        Err(MirRewriteError::InvalidDeclaration { .. })
    ));
    let stale = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
        &mut r.clone(),
        &summaries,
        &m,
        |_, _| vec![plan.clone()],
    );
    assert!(matches!(stale, Err(MirRewriteError::StalePlan { .. })));
}

#[test]
fn pointer_load_sharing_selection_rebuilds_neighbors_and_keeps_blocked_routines() {
    let mut old = baseline();
    let r = forward(&mut old);
    let i = start(r);
    let window = r.blocks[0].ops[i..i + facts::WINDOW].to_vec();
    r.blocks[0]
        .ops
        .splice(i + facts::WINDOW..i + facts::WINDOW, window);
    let good = r.clone();
    let blocked = old
        .routines
        .iter_mut()
        .find(|r| r.name == "PlusZero")
        .unwrap();
    blocked.blocks = good.blocks.clone();
    let i = start(blocked);
    for (n, slot) in [(2, 0xAC), (5, 0xAD)] {
        if let MirOp::Store { dst, .. } = &mut blocked.blocks[0].ops[i + n] {
            *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot)));
        }
    }
    // Both exact windows in this routine must remain blocked.
    let j = i + facts::WINDOW;
    for (n, slot) in [(2, 0xAC), (5, 0xAD)] {
        if let MirOp::Store { dst, .. } = &mut blocked.blocks[0].ops[j + n] {
            *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot)));
        }
    }
    let blocked = blocked.clone();
    let m = memory(&old);
    let (proposed, stats, affected) = apply(&old, &m, None).unwrap();
    assert_eq!(affected, BTreeSet::from([good.id]));
    assert_eq!(
        stats.count_for(good.id, "pointer-load-sharing-candidate"),
        2
    );
    assert_eq!(stats.count_for(good.id, "pointer-load-sharing-selected"), 2);
    assert_eq!(
        proposed
            .routines
            .iter()
            .find(|r| r.id == blocked.id)
            .unwrap(),
        &blocked
    );
    assert_eq!(selected(old.clone()), proposed);
    assert_eq!(
        image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
            - image_size(&proposed, 0x3000, crate::compiler::Runtime::ActionCart).unwrap(),
        12
    );
}

#[test]
fn pointer_load_sharing_unknown_emission_cost_restores_the_baseline() {
    let mut old = baseline();
    let call = old
        .routines
        .iter_mut()
        .find(|r| r.name == "PlusTwo")
        .unwrap()
        .blocks[0]
        .ops
        .iter_mut()
        .find(|op| matches!(op, MirOp::Call { .. }))
        .unwrap();
    if let MirOp::Call { target, .. } = call {
        *target = MirCallTarget::Routine(RoutineId(u32::MAX));
    }
    assert!(image_size(&old, 0x3000, crate::compiler::Runtime::ActionCart).is_none());
    assert_eq!(selected(old.clone()), old);
}
