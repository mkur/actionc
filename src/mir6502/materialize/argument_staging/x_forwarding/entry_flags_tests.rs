use super::*;
use crate::mir6502::{
    analysis::{
        argument_x_forwarding::tests::{memory, site},
        entry_flags::MirKnownCalleeEntryFlags,
        posthome::PostHomeAnalysisSnapshot,
        sites::MirRoutineGeneration,
    },
    rewrite::driver::MirRewriteError,
};
fn baseline() -> MirProgram {
    let source = include_str!("../../../../../fixtures/runtime/callee_entry_flags.act")
        .replace("\r\n", "\n");
    let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    crate::mir6502::materialize_program_with_origin(
        crate::mir6502::lower_program(&nir).unwrap(),
        &Mir6502Config {
            enable_precise_callee_entry_flags: false,
            ..Mir6502Config::optimized()
        },
        0x3000,
    )
    .unwrap()
}
fn config() -> Mir6502Config {
    Mir6502Config {
        enable_precise_callee_entry_flags: true,
        ..Mir6502Config::optimized()
    }
}
#[test]
fn callee_entry_flags_forward_only_the_nz_independent_call_with_final_costing() {
    let old = baseline();
    let m = memory(&old);
    let (sums, targets) = prepare_summaries(&old, true);
    assert_eq!(targets.len(), 2);
    let (new, stats, affected) = apply_with_summaries(&old, &m, None, &sums).unwrap();
    assert_eq!(affected.len(), 1);
    let final_p = select_costed(
        old.clone(),
        &config(),
        0x3000,
        crate::runtime::Runtime::ActionCart,
    )
    .unwrap();
    assert_eq!(new, final_p);
    assert_eq!(
        select_costed(
            final_p.clone(),
            &config(),
            0x3000,
            crate::runtime::Runtime::ActionCart
        )
        .unwrap(),
        final_p
    );
    assert_eq!(
        image_size(&old, 0x3000, crate::runtime::Runtime::ActionCart).unwrap()
            - image_size(&new, 0x3000, crate::runtime::Runtime::ActionCart).unwrap(),
        2
    );
    for (a, b) in old.routines.iter().zip(&new.routines) {
        assert_eq!(a.frame, b.frame);
        let stores = |r: &MirRoutine| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|o| matches!(o, MirOp::Store { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(stores(a), stores(b));
        if a.name == "Forward" {
            assert_ne!(a, b);
            assert_eq!(stats.count_for(a.id, "callee-entry-flags-selected"), 1);
        } else {
            assert_eq!(a, b);
        }
    }
}
#[test]
fn callee_entry_flags_driver_rejects_wrong_mask_target_and_stale_callee_body() {
    let p = baseline();
    let m = memory(&p);
    let (sums, _) = prepare_summaries(&p, true);
    let mut r = p
        .routines
        .iter()
        .find(|r| r.name == "Forward")
        .unwrap()
        .clone();
    let mut plan = None;
    MirPostHomeRewriteDriver::default()
        .run_fixed_point_with_physical_memory(&mut r, &sums, &m, |r, c| {
            let v = discover(
                r,
                c,
                &m,
                &r.blocks
                    .iter()
                    .map(|b| (b.id, (0..b.ops.len()).collect::<Vec<_>>()))
                    .collect(),
                &mut BTreeSet::new(),
            );
            if let Some(p) = v.first() {
                plan = Some(p.clone());
            }
            v
        })
        .unwrap();
    let original = p.routines.iter().find(|r| r.name == "Forward").unwrap();
    let plan = plan.unwrap();
    for change in 0..3 {
        let mut bad = plan.clone();
        let Some(MirPostHomeWitness::ArgumentXForwarding(ref mut proof)) = bad.witness else {
            panic!()
        };
        let w = proof.entry_flags.as_mut().unwrap();
        match change {
            0 => w.changed.c = true,
            1 => w.target = RoutineId(u32::MAX),
            _ => {
                let mut body = (*w.fact.source).clone();
                body.name.push('_');
                w.fact.source = std::sync::Arc::new(body);
            }
        }
        assert!(matches!(
            MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
                &mut original.clone(),
                &sums,
                &m,
                |_, _| vec![bad.clone()]
            ),
            Err(MirRewriteError::InvalidDeclaration { .. })
        ));
    }
    let mut changed = p.clone();
    let callee = changed
        .routines
        .iter_mut()
        .find(|r| r.name == "Observe")
        .unwrap();
    callee.blocks[0].ops.insert(
        0,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x680)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    );
    let callee_flags = MirKnownCalleeEntryFlags::analyze(callee);
    let (fresh, _) = prepare_summaries(&changed, true);
    assert!(matches!(
        MirPostHomeRewriteDriver::default().run_fixed_point_with_physical_memory(
            &mut original.clone(),
            &fresh,
            &m,
            |_, _| vec![plan.clone()]
        ),
        Err(MirRewriteError::InvalidDeclaration { .. })
    ));
    // A changed body retaining the same mask still requires a rebuilt witness.
    let nz = MirFlagSet {
        n: true,
        z: true,
        ..MirFlagSet::default()
    };
    assert!(callee_flags.unobserved(nz));
}
#[test]
fn callee_entry_flags_final_gate_rejects_weakened_input_contracts() {
    let p = baseline();
    let (sums, _) = prepare_summaries(&p, true);
    let mut changed = p.clone();
    let callee = changed
        .routines
        .iter_mut()
        .find(|r| r.name == "Observe")
        .unwrap();
    callee.blocks[0]
        .ops
        .retain(|op| matches!(op, MirOp::Call { .. }));
    let new = MirKnownCalleeSummaries::analyze_for_rewriting(&changed);
    assert!(!sums.entry_inputs_preserved_by(&changed, &new));
    let caller = p
        .routines
        .iter()
        .find(|r| r.name == "UnsafeForward")
        .unwrap();
    let snap = PostHomeAnalysisSnapshot::new_with_known_callees(
        caller,
        MirRoutineGeneration::initial(),
        &sums,
    )
    .unwrap();
    let c = PostHomeRewriteContext::new(&snap);
    assert!(
        matches!(c.argument_x_forwarding(c.point(site(caller)),&memory(&p)),Err(facts::Blocker::EntryFlagsLive(flags)) if flags.n&&flags.z)
    );
}
