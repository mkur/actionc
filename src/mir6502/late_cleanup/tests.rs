use super::*;
use crate::mir6502::trace::{Mir6502Trace, TraceOptions};

#[path = "ownership_tests.rs"]
mod ownership;

pub(super) fn home() -> MirMem {
    MirMem::Spill {
        id: MirSpillId(0),
        offset: 0,
    }
}
pub(super) fn imm(value: u16) -> MirOp {
    MirOp::LoadImm {
        dst: MirDef::Reg(MirReg::A),
        value,
        width: MirWidth::Byte,
    }
}
pub(super) fn store() -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(home()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }
}
pub(super) fn load(mem: MirMem) -> MirOp {
    MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(mem),
        width: MirWidth::Byte,
    }
}
pub(super) fn program(ops: Vec<MirOp>) -> MirProgram {
    MirProgram {
        globals: vec![],
        statics: vec![],
        machine_blocks: vec![],
        runtime_helpers: vec![],
        routines: vec![MirRoutine {
            id: RoutineId(0),
            name: "Main".into(),
            direct_result_abi: None,
            scalar_signature: None,
            inline: Default::default(),
            abi: MirRoutineAbi::ProgramEntry,
            frame: MirFrame {
                spills: vec![MirSpillId(0)],
                ..Default::default()
            },
            temps: vec![],
            effects: Default::default(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".into(),
                params: vec![],
                ops,
                terminator: MirTerminator::Return,
            }],
        }],
    }
}
fn run(
    p: &MirProgram,
    preservation: &Preservation,
) -> (
    MirProgram,
    serde_json::Value,
    Vec<crate::mir6502::trace::TraceEvent>,
) {
    run_config(p, preservation, false)
}
fn run_config(
    p: &MirProgram,
    preservation: &Preservation,
    producers: bool,
) -> (
    MirProgram,
    serde_json::Value,
    Vec<crate::mir6502::trace::TraceEvent>,
) {
    let session = Mir6502Trace::new(TraceOptions::default());
    let selected = select(
        p.clone(),
        preservation,
        None,
        &Mir6502Config {
            enable_late_dead_register_cleanup: producers,
            ..Mir6502Config::optimized()
        },
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &session.context(),
    )
    .unwrap();
    (selected, session.manifest(), session.events())
}
fn reason(events: &[crate::mir6502::trace::TraceEvent]) -> Option<&str> {
    events
        .iter()
        .find(|e| e.kind == "late-cleanup-result")
        .and_then(|e| e.data["reason"].as_str())
}

pub(super) fn assert_closed_attempts(events: &[crate::mir6502::trace::TraceEvent]) {
    for start in events
        .iter()
        .filter(|e| e.kind == "attempt-start" && e.data["scope"] == "attempt")
    {
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "attempt-end" && e.attempt == start.attempt)
                .count(),
            1,
            "{:?}",
            start.data
        );
    }
}

#[test]
fn removes_store_keeps_frame_and_producers_with_two_emissions() {
    let p = program(vec![imm(23), store(), imm(7)]);
    let (selected, manifest, events) = run(&p, &clean(&p));
    assert_closed_attempts(&events);
    assert_eq!(selected.routines[0].blocks[0].ops, [imm(23), imm(7)]);
    assert_eq!(selected.routines[0].frame, p.routines[0].frame);
    assert!(only_cleanup_deletions(&p, &selected, false, None));
    assert_eq!(manifest["work"]["emission_invocations"], 2);
    let result = &events
        .iter()
        .find(|e| e.kind == "late-cleanup-result")
        .unwrap()
        .data;
    assert_eq!(result["accepted"], true);
    assert_eq!(
        result["image_bytes_before"].as_u64().unwrap()
            - result["image_bytes_after"].as_u64().unwrap(),
        3
    );
    let (again, manifest, _) = run(&selected, &clean(&selected));
    assert_eq!(again, selected);
    assert!(manifest["work"]["emission_invocations"].is_null());
}

#[test]
fn explicit_and_successor_reads_keep_stores_without_cost_emission() {
    let mut p = program(vec![imm(23), store(), load(home())]);
    for successor in [false, true] {
        if successor {
            p.routines[0].blocks[0].ops.pop();
            p.routines[0].blocks[0].terminator = MirTerminator::Jump(MirEdge {
                target: MirBlockId(1),
                args: vec![],
            });
            p.routines[0].blocks.push(MirBlock {
                id: MirBlockId(1),
                label: "reader".into(),
                params: vec![],
                ops: vec![load(home())],
                terminator: MirTerminator::Return,
            });
        }
        let (selected, manifest, events) = run(&p, &clean(&p));
        assert_eq!(selected, p);
        assert_eq!(reason(&events), Some("NoDeadStores"));
        assert!(manifest["work"]["emission_invocations"].is_null());
    }
}

#[test]
fn separately_named_absolute_read_keeps_private_store() {
    let mut p = program(vec![imm(23), store(), imm(7)]);
    let old = measure(
        &p,
        &[&p],
        0x3000,
        crate::runtime::Runtime::ActionCart,
        &Trace::disabled(),
    )
    .unwrap();
    let address = old.memory.address(RoutineId(0), &home()).unwrap();
    p.routines[0].blocks[0]
        .ops
        .push(load(MirMem::Absolute(address)));
    let (selected, manifest, events) = run(&p, &clean(&p));
    assert_eq!(selected, p);
    assert_eq!(manifest["work"]["emission_invocations"], 1);
    assert_eq!(reason(&events), Some("ObservableAlias"));
}

#[test]
fn unsupported_earlier_proof_restores_exact_baseline() {
    let p = program(vec![imm(23), store(), imm(7)]);
    let mut log = Preservation::new(true);
    let mut before = p.clone();
    before.routines[0].blocks[0].ops.insert(0, imm(99));
    log.select(Family::CheckedRanges, before, |_| Ok(p.clone()))
        .unwrap();
    let (selected, manifest, events) = run(&p, &log);
    assert_closed_attempts(&events);
    assert_eq!(selected, p);
    assert_eq!(manifest["work"]["emission_invocations"], 2);
    assert_eq!(reason(&events), Some("EarlierDecision(CheckedRanges)"));
}

#[test]
fn change_guard_rejects_producer_frame_and_public_store_deletion() {
    let p = program(vec![imm(23), store(), imm(7)]);
    let mut bad = p.clone();
    bad.routines[0].blocks[0].ops.remove(0);
    assert!(!only_cleanup_deletions(&p, &bad, false, None));
    let mut bad = p.clone();
    bad.routines[0].frame.spills.clear();
    assert!(!only_cleanup_deletions(&p, &bad, false, None));
    let mut p = p;
    if let MirOp::Store { dst, .. } = &mut p.routines[0].blocks[0].ops[1] {
        *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xa3)));
    }
    let mut bad = p.clone();
    bad.routines[0].blocks[0].ops.remove(1);
    assert!(!only_cleanup_deletions(&p, &bad, false, None));
}

pub(super) fn clean(program: &MirProgram) -> Preservation {
    let mut log = Preservation::new(true);
    log.begin(program);
    log
}

#[test]
fn untracked_transformation_declines_before_costing() {
    let p = program(vec![imm(23), store(), imm(7)]);
    let log = clean(&p);
    let mut changed = p.clone();
    changed.routines[0].blocks[0].ops[0] = imm(24);
    let (selected, manifest, events) = run(&changed, &log);
    assert_eq!(selected, changed);
    assert_eq!(reason(&events), Some("UntrackedDecision"));
    assert!(manifest["work"]["emission_invocations"].is_null());
}

#[test]
fn published_callee_accumulator_cannot_change() {
    let mut before = program(vec![imm(23)]);
    let mut caller = before.routines[0].clone();
    caller.id = RoutineId(1);
    caller.name = "Caller".into();
    caller.blocks[0].ops = vec![MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(0)),
        abi: MirCallAbi {
            library: None,
            params: vec![],
            result: None,
            additional_results: vec![],
            clobbers: Default::default(),
            preserves: Default::default(),
        },
        args: vec![],
        result: None,
        additional_results: vec![],
        effects: Default::default(),
    }];
    before.routines.push(caller);
    let mut after = before.clone();
    after.routines[0].blocks[0].ops[0] = imm(24);
    assert!(!callee_contracts_preserved(&before, &after));
    assert!(callee_contracts_preserved(&before, &before));
}

fn sink() -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0x7000)),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }
}

#[test]
fn producer_cleanup_removes_newly_dead_store_inputs_and_preserves_frames() {
    let p = program(vec![imm(23), store(), load(home()), imm(7), sink()]);
    let (old, _, _) = run(&p, &clean(&p));
    assert_eq!(old, p, "store-only cleanup sees the explicit load");
    let (new, work, events) = run_config(&p, &clean(&p), true);
    assert_eq!(
        new.routines[0].blocks[0].ops,
        vec![imm(7), sink()],
        "{events:?}"
    );
    assert_eq!(new.routines[0].frame, p.routines[0].frame);
    assert_eq!(work["work"]["emission_invocations"], 2);
    let outcome = &events
        .iter()
        .find(|e| e.kind == "late-cleanup-result")
        .unwrap()
        .data;
    assert_eq!(outcome["removed_stores"], 1);
    assert_eq!(outcome["removed_producers"], 2);
    assert_closed_attempts(&events);
}

#[test]
fn producer_cleanup_works_without_a_store_and_preserves_observable_reads() {
    let indirect = MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::FixedIndirectIndexedY {
            zp: MirFixedZpSlot(0xac),
        },
        width: MirWidth::Byte,
    };
    for (source, removed) in [
        (
            load(MirMem::Global {
                id: crate::nir::SymbolId(0),
                offset: 0,
            }),
            true,
        ),
        (load(MirMem::Absolute(0xd20a)), false),
        (load(MirMem::Absolute(0x7000)), false),
        (load(MirMem::FixedZeroPage(MirFixedZpSlot(0x14))), false),
        (indirect, false),
    ] {
        let mut p = program(vec![source.clone(), imm(7), sink()]);
        p.globals.push(MirGlobal {
            id: crate::nir::SymbolId(0),
            name: "input".into(),
            kind: "byte".into(),
            width: Some(MirWidth::Byte),
            storage_size: 1,
            backing: MirGlobalBacking::Ordinary { offset: 0 },
            init: None,
        });
        let (new, _, events) = run_config(&p, &clean(&p), true);
        assert_eq!(new != p, removed, "{source:?}: {events:?}");
        if removed {
            assert_eq!(new.routines[0].blocks[0].ops, vec![imm(7), sink()]);
        }
    }
}

#[test]
fn producer_liveness_keeps_successor_register_and_flag_consumers() {
    for flag in [None, Some(MirFlagTest::ZSet), Some(MirFlagTest::NSet)] {
        let mut p = program(vec![load(home())]);
        p.routines[0].blocks[0].terminator = MirTerminator::Jump(MirEdge::plain(MirBlockId(1)));
        p.routines[0].blocks.push(MirBlock {
            id: MirBlockId(1),
            label: "consumer".into(),
            params: vec![],
            ops: if flag.is_none() { vec![sink()] } else { vec![] },
            terminator: flag
                .clone()
                .map_or(MirTerminator::Return, |flag| MirTerminator::Branch {
                    cond: MirCond::FlagTest(flag),
                    then_edge: MirEdge::plain(MirBlockId(2)),
                    else_edge: MirEdge::plain(MirBlockId(2)),
                }),
        });
        if flag.is_some() {
            p.routines[0].blocks.push(MirBlock {
                id: MirBlockId(2),
                label: "exit".into(),
                params: vec![],
                ops: vec![imm(7), sink()],
                terminator: MirTerminator::Return,
            });
        }
        let (new, work, _) = run_config(&p, &clean(&p), true);
        assert_eq!(new, p);
        assert!(work["work"]["emission_invocations"].is_null());
    }
    // The next iteration consumes A before the defining load is reached again.
    let mut p = program(vec![sink(), load(home())]);
    p.routines[0].blocks[0].terminator = MirTerminator::Jump(MirEdge::plain(MirBlockId(0)));
    assert_eq!(run_config(&p, &clean(&p), true).0, p);
}

#[test]
fn producer_purity_rechecks_missing_hardware_and_executable_placement() {
    let op = load(home());
    for (address, ordinary, executable, accepted) in [
        (0x4000, true, false, true),
        (0xd20a, true, false, false),
        (0x4000, false, false, false),
        (0x4000, true, true, false),
    ] {
        let mut memory = MirPhysicalMemory::default();
        memory.insert(RoutineId(0), home(), address, ordinary);
        memory.protect_execution_ranges(if executable {
            vec![(0x4000, 0x4100)]
        } else {
            vec![(0x3000, 0x3100)]
        });
        assert_eq!(
            producers::pure_register(&op, RoutineId(0), Some(&memory)).is_some(),
            accepted
        );
    }
    assert_eq!(
        producers::pure_register(&op, RoutineId(0), Some(&MirPhysicalMemory::default())),
        None
    );
}

#[test]
fn producer_cleanup_rolls_back_when_a_callee_exit_fact_would_change() {
    // Local liveness permits dropping the final constant, but its caller
    // observes the accumulator exit fact published by the earlier summary.
    let mut p = program(vec![imm(23)]);
    p.routines[0].abi = MirRoutineAbi::Action;
    let mut caller = program(vec![sink()]).routines.remove(0);
    caller.id = RoutineId(1);
    caller.name = "Caller".into();
    caller.blocks[0].ops.insert(
        0,
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(0)),
            abi: MirCallAbi {
                library: None,
                params: vec![],
                result: None,
                additional_results: vec![],
                clobbers: Default::default(),
                preserves: Default::default(),
            },
            args: vec![],
            result: None,
            additional_results: vec![],
            effects: Default::default(),
        },
    );
    p.routines.push(caller);
    let (new, work, events) = run_config(&p, &clean(&p), true);
    assert_eq!(new, p);
    assert_eq!(reason(&events), Some("CalleeContract"));
    assert_eq!(work["work"]["emission_invocations"], 1);
}

#[test]
fn producer_cleanup_cost_rollback_keeps_a_zero_byte_redundant_load() {
    let ldy = MirOp::LoadImm {
        dst: MirDef::Reg(MirReg::Y),
        value: 7,
        width: MirWidth::Byte,
    };
    let sty = MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0x7001)),
        src: MirValue::Def(MirDef::Reg(MirReg::Y)),
        width: MirWidth::Byte,
    };
    let p = program(vec![ldy.clone(), ldy, sty, imm(7), sink()]);
    let (new, work, events) = run_config(&p, &clean(&p), true);
    assert_eq!(new, p);
    assert_eq!(reason(&events), Some("Cost"), "{events:?}");
    assert_eq!(work["work"]["emission_invocations"], 2);
}

#[test]
fn producer_cleanup_handles_native_transfers_and_keeps_live_x_y_values() {
    for source in [MirReg::X, MirReg::Y] {
        let transfer = MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::Def(MirDef::Reg(source)),
            width: MirWidth::Byte,
        };
        let p = program(vec![transfer, imm(7), sink()]);
        let (new, _, events) = run_config(&p, &clean(&p), true);
        assert_eq!(
            new.routines[0].blocks[0].ops,
            vec![imm(7), sink()],
            "{events:?}"
        );
        let transfer = MirOp::Move {
            dst: MirDef::Reg(source),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        };
        let mut p = program(vec![
            imm(23),
            transfer,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Absolute(0x7001)),
                src: MirValue::Def(MirDef::Reg(source)),
                width: MirWidth::Byte,
            },
            imm(7),
            sink(),
        ]);
        assert_eq!(run_config(&p, &clean(&p), true).0, p);
        // Once the consumer disappears, both the transfer and its A input die.
        p.routines[0].blocks[0].ops.remove(2);
        let (new, _, _) = run_config(&p, &clean(&p), true);
        assert_eq!(new.routines[0].blocks[0].ops, vec![imm(7), sink()]);
    }
}
