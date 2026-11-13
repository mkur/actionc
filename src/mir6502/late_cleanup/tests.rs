use super::*;
use crate::mir6502::trace::{Mir6502Trace, TraceOptions};

fn home() -> MirMem {
    MirMem::Spill {
        id: MirSpillId(0),
        offset: 0,
    }
}
fn imm(value: u16) -> MirOp {
    MirOp::LoadImm {
        dst: MirDef::Reg(MirReg::A),
        value,
        width: MirWidth::Byte,
    }
}
fn store() -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(home()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }
}
fn load(mem: MirMem) -> MirOp {
    MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(mem),
        width: MirWidth::Byte,
    }
}
fn program(ops: Vec<MirOp>) -> MirProgram {
    MirProgram {
        globals: vec![],
        statics: vec![],
        machine_blocks: vec![],
        runtime_helpers: vec![],
        routines: vec![MirRoutine {
            id: RoutineId(0),
            name: "Main".into(),
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
    let session = Mir6502Trace::new(TraceOptions::default());
    let selected = select(
        p.clone(),
        preservation,
        None,
        &Mir6502Config::optimized(),
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

fn assert_closed_attempts(events: &[crate::mir6502::trace::TraceEvent]) {
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
    assert!(only_private_store_deletions(&p, &selected));
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
    assert!(!only_private_store_deletions(&p, &bad));
    let mut bad = p.clone();
    bad.routines[0].frame.spills.clear();
    assert!(!only_private_store_deletions(&p, &bad));
    let mut p = p;
    if let MirOp::Store { dst, .. } = &mut p.routines[0].blocks[0].ops[1] {
        *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xa3)));
    }
    let mut bad = p.clone();
    bad.routines[0].blocks[0].ops.remove(1);
    assert!(!only_private_store_deletions(&p, &bad));
}

fn clean(program: &MirProgram) -> Preservation {
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

#[test]
fn physical_zero_page_definition_survives_unknown_readers_until_definite_overwrite() {
    let private = MirMem::ZeroPage(MirZpSlot(0));
    let write = MirOp::Store {
        dst: MirAddr::Direct(private.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    };
    let read = MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::FixedIndirectIndexedY {
            zp: MirFixedZpSlot(0xa0),
        },
        width: MirWidth::Byte,
    };
    let mut p = program(vec![write.clone(), read.clone()]);
    let mut memory = MirPhysicalMemory::default();
    memory.insert(RoutineId(0), private, 0xe1, true);
    let known = MirKnownCalleeSummaries::default();
    assert!(
        !readers::Readers {
            program: &p,
            memory: &memory,
            known: &known,
            default_sargs: false,
            origin: 0x3000
        }
        .definition_unobserved(&p.routines[0], MirBlockId(0), 1, 0xe1)
    );
    p.routines[0].blocks[0].ops.insert(1, write.clone());
    assert!(
        readers::Readers {
            program: &p,
            memory: &memory,
            known: &known,
            default_sargs: false,
            origin: 0x3000
        }
        .definition_unobserved(&p.routines[0], MirBlockId(0), 1, 0xe1)
    );
    // A read on the next iteration observes the definition before its overwrite.
    p.routines[0].blocks[0].ops = vec![read, write];
    p.routines[0].blocks[0].terminator = MirTerminator::Jump(MirEdge {
        target: MirBlockId(0),
        args: vec![],
    });
    assert!(
        !readers::Readers {
            program: &p,
            memory: &memory,
            known: &known,
            default_sargs: false,
            origin: 0x3000
        }
        .definition_unobserved(&p.routines[0], MirBlockId(0), 2, 0xe1)
    );
}

#[test]
fn physical_zero_page_definition_is_observable_after_return_to_caller() {
    let private = MirMem::ZeroPage(MirZpSlot(0));
    let write = MirOp::Store {
        dst: MirAddr::Direct(private.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    };
    let mut p = program(vec![write]);
    let mut caller = p.routines[0].clone();
    caller.id = RoutineId(1);
    caller.blocks[0].ops = vec![
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(0)),
            abi: MirCallAbi {
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
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::FixedIndirectIndexedY {
                zp: MirFixedZpSlot(0xa0),
            },
            width: MirWidth::Byte,
        },
    ];
    p.routines.push(caller);
    let mut memory = MirPhysicalMemory::default();
    memory.insert(RoutineId(0), private, 0xe1, true);
    let known = MirKnownCalleeSummaries::default();
    let check = |p: &MirProgram| {
        readers::Readers {
            program: p,
            memory: &memory,
            known: &known,
            default_sargs: false,
            origin: 0x3000,
        }
        .definition_unobserved(&p.routines[0], MirBlockId(0), 1, 0xe1)
    };
    assert!(!check(&p));
    // A definite caller overwrite ends the lifetime, unlike a may-write call.
    p.routines[1].blocks[0].ops.insert(
        1,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xe1))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    );
    assert!(check(&p));
}
