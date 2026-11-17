use super::*;
use crate::mir6502::ir::*;
use crate::mir6502::trace::{Mir6502Trace, TraceOptions};

fn load(reg: MirReg, value: u16) -> MirOp {
    MirOp::LoadImm {
        dst: MirDef::Reg(reg),
        value,
        width: MirWidth::Byte,
    }
}
fn routine(id: u32, ops: Vec<MirOp>) -> MirRoutine {
    MirRoutine {
        id: RoutineId(id),
        name: format!("R{id}"),
        inline: Default::default(),
        scalar_signature: None,
        direct_result_abi: None,
        abi: MirRoutineAbi::Action,
        frame: Default::default(),
        temps: vec![],
        effects: Default::default(),
        blocks: vec![MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            params: vec![],
            ops,
            terminator: MirTerminator::Return,
        }],
    }
}
fn program() -> MirProgram {
    let call = MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(0)),
        args: vec![],
        result: None,
        additional_results: vec![],
        effects: Default::default(),
        abi: MirCallAbi {
            params: vec![],
            result: None,
            additional_results: vec![],
            clobbers: crate::mir6502::abi::action_call_clobbers(),
            preserves: Default::default(),
        },
    };
    let param = MirMem::Param {
        id: crate::nir::ParamId(0),
        offset: 0,
    };
    let mut caller = routine(
        1,
        vec![
            load(MirReg::A, 7),
            MirOp::Store {
                dst: MirAddr::Direct(param.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            call,
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(param),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x600)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ],
    );
    caller.frame.params.push(MirStorageSlot {
        purpose: Default::default(),
        id: MirStorageId(0),
        name: Some("keep".into()),
        storage: MirStorageClass::Scalar,
        storage_size: 1,
        scalar_width: Some(MirWidth::Byte),
        base: MirStorageBase::Param(crate::nir::ParamId(0)),
        offset: 0,
        mutable: true,
        init: None,
    });
    MirProgram {
        globals: vec![],
        statics: vec![],
        runtime_helpers: vec![],
        machine_blocks: vec![],
        routines: vec![routine(0, vec![load(MirReg::A, 0)]), caller],
    }
}

#[test]
fn core_callee_scope_reuses_unchanged_boundary_and_refreshes_compatible_body() {
    let mut p = program();
    let session = Mir6502Trace::new(TraceOptions::default());
    let mut scope = Scope::new();
    assert!(scope.refresh(&p, &session.context(), "initial").is_some());
    assert!(scope.refresh(&p, &session.context(), "unchanged").is_some());
    assert_eq!(session.manifest()["work"]["core_callee_summary_builds"], 1);
    p.routines[0].blocks[0].ops.push(MirOp::Move {
        dst: MirDef::Reg(MirReg::A),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    assert!(scope
        .refresh(&p, &session.context(), "compatible")
        .is_some());
    assert_eq!(session.manifest()["work"]["core_callee_summary_builds"], 2);
    assert_eq!(session.manifest()["work"]["core_callee_contract_checks"], 1);
}

#[test]
fn core_callee_scope_rechecks_metadata_calls_and_current_zp_allocation() {
    let baseline = program();
    for variant in 0..4 {
        let mut scope = Scope::new();
        scope
            .refresh(&baseline, &Trace::disabled(), "initial")
            .unwrap();
        let mut p = baseline.clone();
        match variant {
            0 => p.routines[0].direct_result_abi = Some(MirDirectResultAbi::Pointer),
            1 => p.routines[0].frame.local_homes = Some(vec![]),
            2 => {
                let MirOp::Call { target, .. } = &mut p.routines[1].blocks[0].ops[2] else {
                    panic!()
                };
                *target = MirCallTarget::Routine(RoutineId(1));
            }
            _ => p.globals.push(MirGlobal {
                id: crate::nir::SymbolId(0),
                name: "state".into(),
                kind: "byte".into(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(0xe4),
                init: None,
            }),
        }
        assert!(
            scope.refresh(&p, &Trace::disabled(), "changed").is_none(),
            "{variant}"
        );
    }
    let mut p = baseline;
    let slot = MirZpSlot(0);
    p.routines[0].frame.virtual_zero_page.push(slot);
    p.routines[0]
        .frame
        .zero_page_allocations
        .push(MirZpAllocation {
            slot,
            start: MirFixedZpSlot(0xe4),
            size: 1,
        });
    p.routines[0].blocks[0].ops.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::ZeroPage(slot)),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
    let mut scope = Scope::new();
    scope.refresh(&p, &Trace::disabled(), "initial").unwrap();
    p.routines[0].frame.zero_page_allocations[0].start = MirFixedZpSlot(0xe5);
    assert!(scope.refresh(&p, &Trace::disabled(), "moved").is_none());
}

#[test]
fn core_callee_scope_rolls_back_dependent_caller_rewrite_in_both_orders() {
    for reversed in [false, true] {
        let mut p = program();
        if reversed {
            p.routines.reverse();
        }
        let baseline = p.clone();
        let mut stats = MirPeepholeStats::default();
        stats.record(RoutineId(0), "before-scope");
        let mut homes = BTreeMap::from([(RoutineId(1), HomeFateTracker::default())]);
        let original_homes = homes.clone();
        let session = Mir6502Trace::new(TraceOptions::default());
        let mut attempts = 0;
        run_with(
            &mut p,
            &mut stats,
            &mut homes,
            &session.context(),
            |p, stats, homes, mut scope, trace| {
                attempts += 1;
                assert_eq!(*p, baseline);
                assert_eq!(*homes, original_homes);
                let known = match scope.as_deref_mut() {
                    Some(s) => s.refresh(p, trace, "initial").unwrap(),
                    None => MirKnownCalleeSummaries::default(),
                };
                let layout = MaterializeLayout::new(p, 0x3000);
                for r in &mut p.routines {
                    if r.id == RoutineId(0) && scope.is_some() {
                        r.blocks[0]
                            .ops
                            .extend([load(MirReg::X, 99), load(MirReg::Y, 99)]);
                    }
                    if r.id == RoutineId(1) {
                        super::super::run_analyzed_known_callee_preserved_index_param_carriers(
                            r,
                            &layout,
                            Some(&known),
                            stats,
                        )?;
                        let loads = r.blocks[0]
                            .ops
                            .iter()
                            .filter(|o| {
                                matches!(
                                    o,
                                    MirOp::Load {
                                        src: MirAddr::Direct(MirMem::Param { .. }),
                                        ..
                                    }
                                )
                            })
                            .count();
                        assert_eq!(loads, usize::from(scope.is_none()));
                    }
                }
                if let Some(scope) = scope {
                    stats.record(RoutineId(0), "rejected-change");
                    homes.clear();
                    return Ok(scope.refresh(p, trace, "changed-callee").is_some());
                }
                assert_eq!(stats.count_for(RoutineId(0), "rejected-change"), 0);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(p, baseline);
        assert_eq!(homes, original_homes);
        assert_eq!(stats.count_for(RoutineId(0), "before-scope"), 1);
        assert_eq!(stats.count_for(RoutineId(0), "rejected-change"), 0);
        assert_eq!(session.manifest()["work"]["core_callee_scope_fallbacks"], 1);
        let events = session.events();
        let starts: BTreeSet<_> = events
            .iter()
            .filter(|e| e.kind == "attempt-start")
            .map(|e| e.attempt)
            .collect();
        let ends: BTreeSet<_> = events
            .iter()
            .filter(|e| e.kind == "attempt-end")
            .map(|e| e.attempt)
            .collect();
        assert_eq!(starts, ends);
    }
}
