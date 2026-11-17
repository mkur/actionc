use super::*;
use crate::mir6502::analysis::known_callees::MirKnownMemoryReads;

fn program(body: &str, dead: bool) -> MirProgram {
    let source = format!(";@actionc clobbers $AC/$AD\nPROC Touch=*(BYTE value)\nASM\n{body}\nENDASM\nCARD FUNC Hold(CARD keep) Touch(7) RETURN(keep)\nPROC Main() RETURN");
    let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let mut p = crate::mir6502::lower_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    let caller = p.routines.iter_mut().find(|r| r.name == "Hold").unwrap();
    let MirStorageBase::Param(param) = caller.frame.params[0].base else {
        panic!()
    };
    caller.frame.parameter_homes = Some(vec![MirParamBinding {
        param,
        home: MirParamHome::ZeroPage(MirFixedZpSlot(0xe4)),
    }]);
    caller
        .frame
        .fixed_zero_page
        .extend([MirFixedZpSlot(0xe4), MirFixedZpSlot(0xe5)]);
    let mut p = crate::mir6502::materialize_program(p, &Default::default()).unwrap();
    if dead {
        // Keep a real resident object, then start a new logical value lifetime
        // after the call. Applying this after materialization avoids optimizing
        // away the entire constant-valued home and making the check vacuous.
        let caller = p.routines.iter_mut().find(|r| r.name == "Hold").unwrap();
        let block = caller
            .blocks
            .iter_mut()
            .find(|b| b.ops.iter().any(|o| matches!(o, MirOp::Call { .. })))
            .unwrap();
        let site = block
            .ops
            .iter()
            .position(|o| matches!(o, MirOp::Call { .. }))
            .unwrap();
        block.ops.splice(
            site + 1..site + 1,
            (0..2).map(|offset| MirOp::Store {
                dst: MirAddr::Direct(MirMem::Param { id: param, offset }),
                src: MirValue::ConstU8(8),
                width: MirWidth::Byte,
            }),
        );
    }
    assert_eq!(self::caller(&p).frame.resident_bytes().len(), 2);
    p
}

fn caller(p: &MirProgram) -> &MirRoutine {
    p.routines.iter().find(|r| r.name == "Hold").unwrap()
}
fn call(p: &MirProgram) -> &MirOp {
    caller(p)
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find(|o| matches!(o, MirOp::Call { .. }))
        .unwrap()
}
fn verify(p: &MirProgram) -> Vec<MirDiagnostic> {
    verify_lifetimes(
        caller(p),
        Some(&mut ResidenceCalls::with_acyclic(
            p,
            MirCalleeEntryStage::ActualEntry,
        )),
    )
}
fn replace_machine(p: &mut MirProgram, bytes: &[u8]) {
    p.machine_blocks[0].items = bytes.iter().copied().map(MirMachineItem::Byte).collect();
}

#[test]
fn certified_machine_residence_uses_current_writes_and_retains_scoped_admission() {
    let p = program("sta $AC\nldx #$5A\nstx $AD\nrts", false);
    assert!(verify(&p).is_empty());
    assert_eq!(self::caller(&p).frame.resident_bytes().len(), 2);
    let mut facts = ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::ActualEntry);
    assert!(facts.inputs(call(&p)).is_ok());
    assert!(facts.writes(call(&p)).is_ok());
    let mut leaf_only = ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry);
    assert!(leaf_only.inputs(call(&p)).is_err());
    assert!(leaf_only.writes(call(&p)).is_err());
    assert!(
        ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::PlannedCapture)
            .inputs(call(&p))
            .is_err()
    );
    let mut bad_site = call(&p).clone();
    let MirOp::Call { args, .. } = &mut bad_site else {
        panic!()
    };
    args.clear();
    assert_eq!(
        facts.inputs(&bad_site),
        Err("mismatched residence arguments")
    );
    assert!(facts.writes(&bad_site).is_err());

    for byte in [0xe4, 0xe5] {
        for absolute in [false, true] {
            let mut changed = p.clone();
            let bytes = if absolute {
                vec![0x8d, byte, 0, 0x60]
            } else {
                vec![0x85, byte, 0x60]
            };
            replace_machine(&mut changed, &bytes);
            let mut facts =
                ResidenceCalls::with_acyclic(&changed, MirCalleeEntryStage::ActualEntry);
            assert!(facts.inputs(call(&changed)).is_ok());
            let fixed = facts.fixed_storage();
            assert!(!facts
                .writes(call(&changed))
                .unwrap()
                .preserves_fixed_byte(MirFixedZpSlot(byte), &fixed));
            assert!(!verify(&changed).is_empty(), "{byte:x} {absolute}");
        }
    }
}

#[test]
fn certified_inputs_do_not_turn_unknown_reads_or_writes_into_preservation() {
    for dead in [false, true] {
        let mut p = program("sta $AC\nldx #7\nstx $AD\nrts", dead);
        replace_machine(
            &mut p,
            &[0x85, 0xac, 0xa2, 7, 0x86, 0xad, 0xa0, 0, 0x91, 0xac, 0x60],
        );
        let mut facts = ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::ActualEntry);
        assert!(facts.inputs(call(&p)).is_ok());
        assert_eq!(
            facts.writes(call(&p)),
            Err("unknown residence callee writes")
        );
        assert_eq!(verify(&p).is_empty(), dead);
    }
    let mut p = program("sta $AC\nldx #7\nstx $AD\nrts", false);
    replace_machine(
        &mut p,
        &[0x85, 0xac, 0xa2, 7, 0x86, 0xad, 0xa0, 0, 0xb1, 0xac, 0x60],
    );
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let callee = p.routines.iter().find(|r| r.name == "Touch").unwrap();
    assert_eq!(
        known.get(callee.id).unwrap().reads(),
        &MirKnownMemoryReads::Unknown
    );
    assert!(matches!(
        known.get(callee.id).unwrap().writes(),
        MirKnownMemoryWrites::Exact(_)
    ));
    assert!(verify(&p).is_empty());
}

#[test]
fn uncertified_machine_entries_cannot_use_the_native_dead_call_fallback() {
    let p = program("sta $AC\nrts", true);
    for abi in [MirRoutineAbi::Action, MirRoutineAbi::ActionObservable] {
        // The effect summary supports ADC, but entry certification does not.
        let mut changed = p.clone();
        let callee = changed
            .routines
            .iter_mut()
            .find(|r| r.name == "Touch")
            .unwrap();
        callee.abi = abi;
        let id = callee.id;
        replace_machine(&mut changed, &[0x18, 0x69, 1, 0x85, 0xac, 0x60]);
        assert!(matches!(
            MirKnownCalleeSummaries::analyze_for_rewriting(&changed)
                .get(id)
                .unwrap()
                .writes(),
            MirKnownMemoryWrites::Exact(_)
        ));
        let mut facts = ResidenceCalls::with_acyclic(&changed, MirCalleeEntryStage::ActualEntry);
        assert_eq!(
            facts.inputs(call(&changed)),
            Err("unknown residence callee entry")
        );
        assert!(facts.writes(call(&changed)).is_err());
        assert!(!verify(&changed).is_empty());
    }
    for (bytes, reason) in [
        (vec![0xa5, 0xe8, 0x60], "extra-entry-fixed-byte"),
        (vec![0xb0, 0, 0x60], "extra-entry-flags-or-stack"),
        (vec![0x8a, 0x60], "extra-entry-register"),
    ] {
        let mut changed = p.clone();
        replace_machine(&mut changed, &bytes);
        assert_eq!(
            ResidenceCalls::with_acyclic(&changed, MirCalleeEntryStage::ActualEntry)
                .inputs(call(&changed)),
            Err(reason)
        );
    }
}
