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

#[test]
fn removes_store_keeps_frame_and_producers_with_two_emissions() {
    let p = program(vec![imm(23), store(), imm(7)]);
    let (selected, manifest, events) = run(&p, &Preservation::new(true));
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
    let (again, manifest, _) = run(&selected, &Preservation::new(true));
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
        let (selected, manifest, events) = run(&p, &Preservation::new(true));
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
    let (selected, manifest, events) = run(&p, &Preservation::new(true));
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
