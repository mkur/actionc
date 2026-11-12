use super::*;
use crate::mir6502::analysis::word_load_sharing::tests::{baseline, start, window};
use crate::mir6502::rewrite::{driver::MirRewriteError, plan::MirPostHomeWitness};
use crate::mir6502::{MirPeepholeReportMode, MirRoutine};

fn config() -> Mir6502Config {
    Mir6502Config {
        enable_word_load_sharing: true,
        ..Mir6502Config::optimized()
    }
}
fn selected(p: MirProgram) -> MirProgram {
    select_costed(p, &config(), 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
}
fn ids(r: &MirRoutine) -> BTreeMap<MirBlockId, Vec<usize>> {
    r.blocks
        .iter()
        .map(|b| (b.id, (0..b.ops.len()).collect()))
        .collect()
}
fn size(p: &MirProgram) -> usize {
    image_size(p, 0x3000, crate::compiler::Runtime::ActionCart).unwrap()
}

#[test]
fn word_load_sharing_selects_both_copies_and_bounds_emission_work() {
    let old = baseline();
    let mut work = Work::default();
    let m = work.physical(&old, &old, 0x3000).unwrap();
    let (proposal, stats, affected) = apply(&old, &m, None).unwrap();
    assert_eq!(affected.len(), 2);
    assert!(
        affected
            .iter()
            .all(|&id| stats.count_for(id, "word-load-sharing-selected") == 1)
    );
    WORK.with(|c| c.borrow_mut().clear());
    let new = selected(old.clone());
    assert_eq!(new, proposal);
    assert_eq!(size(&old) - size(&new), 12);
    assert_eq!(WORK.with(|c| c.borrow().clone()), [7]);
    assert_eq!(selected(new.clone()), new);
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        if a.name == "Main" {
            assert_eq!(a, b);
        }
    }
    let report = select_costed(
        old,
        &Mir6502Config {
            peephole_report: MirPeepholeReportMode::Sites,
            ..config()
        },
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(report, new);
}

#[test]
fn word_load_sharing_driver_rejects_missing_forged_and_stale_witnesses() {
    let p = baseline();
    let m = Work::default().physical(&p, &p, 0x3000).unwrap();
    let r = p.routines.iter().find(|r| r.name == "Unlink").unwrap();
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
    for change in 0..4 {
        let mut wrong = plan.clone();
        match change {
            0 => wrong.witness = None,
            1 => wrong.replacement.swap(1, 2),
            2 => {
                if let Some(MirPostHomeWitness::WordLoadSharing(p)) = &mut wrong.witness {
                    p.addresses[0] += 1;
                }
            }
            _ => {
                if let Some(MirPostHomeWitness::WordLoadSharing(p)) = &mut wrong.witness {
                    p.routine = RoutineId(99);
                }
            }
        }
        let result = MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
            &mut r.clone(),
            &summaries,
            &m,
            |_, _| vec![wrong.clone()],
        );
        assert!(
            matches!(result, Err(MirRewriteError::InvalidDeclaration { .. })),
            "{result:?}"
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
fn word_load_sharing_keeps_blocked_routines_and_rebuilds_neighbors() {
    let mut p = baseline();
    let r = p.routines.iter_mut().find(|r| r.name == "Reset").unwrap();
    let MirSite::Op { block, op_index } = start(r) else {
        unreachable!()
    };
    let ops = &mut r.blocks.iter_mut().find(|b| b.id == block).unwrap().ops;
    for (i, address) in [(5, 0x680), (7, 0x681)] {
        if let MirOp::Store { dst, .. } = &mut ops[op_index + i] {
            *dst = MirAddr::Direct(MirMem::Absolute(address));
        }
    }
    let original = p
        .routines
        .iter()
        .find(|r| r.name == "Reset")
        .unwrap()
        .clone();
    let new = selected(p.clone());
    assert_eq!(size(&p) - size(&new), 6);
    assert_eq!(
        &original,
        new.routines.iter().find(|r| r.name == "Reset").unwrap()
    );

    let mut p = baseline();
    let r = p.routines.iter_mut().find(|r| r.name == "Unlink").unwrap();
    let original = window(r).to_vec();
    let MirSite::Op { block, op_index } = start(r) else {
        unreachable!()
    };
    r.blocks
        .iter_mut()
        .find(|b| b.id == block)
        .unwrap()
        .ops
        .splice(op_index..op_index, original);
    let mut work = Work::default();
    let m = work.physical(&p, &p, 0x3000).unwrap();
    let (new, stats, affected) = apply(&p, &m, None).unwrap();
    assert_eq!(affected.len(), 2);
    let id = new.routines.iter().find(|r| r.name == "Unlink").unwrap().id;
    assert_eq!(stats.count_for(id, "word-load-sharing-selected"), 2);
    assert_eq!(size(&p) - size(&new), 18);
}

#[test]
fn word_load_sharing_rejects_changed_maps_and_never_exceeds_the_budget() {
    let p = baseline();
    let mut work = Work::default();
    let m = work.physical(&p, &p, 0x3000).unwrap();
    let r = p.routines.iter().find(|r| r.name == "Unlink").unwrap();
    let proof = facts::prove(
        r,
        crate::mir6502::analysis::sites::MirRoutineGeneration::initial(),
        crate::mir6502::analysis::sites::MirProgramPoint {
            generation: crate::mir6502::analysis::sites::MirRoutineGeneration::initial(),
            site: start(r),
        },
        &m,
    )
    .unwrap();
    let mut changed = m.clone();
    if let MirMem::Global { id, .. } = proof.shape.source[0] {
        changed.global(id, proof.addresses[1], 2, true);
    }
    assert!(!facts::validate(
        &proof,
        r,
        proof.point.generation,
        &changed,
        window(r),
        &facts::replacement(window(r)).unwrap()
    ));
    let mut budget = Work::default();
    for _ in 0..7 {
        assert!(budget.charge().is_some());
    }
    assert!(budget.charge().is_none());
    assert_eq!(budget.emissions, 7);
    let mut no_candidate = p.clone();
    for r in &mut no_candidate.routines {
        for b in &mut r.blocks {
            b.ops.clear();
        }
    }
    WORK.with(|c| c.borrow_mut().clear());
    assert_eq!(selected(no_candidate.clone()), no_candidate);
    assert!(WORK.with(|c| c.borrow().is_empty()));
}
