use super::*;
use crate::mir6502::analysis::known_callees::{MirKnownMemoryReads, abi_inputs_review_fingerprint};

fn lower_with_home(source: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
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
    p
}

fn program(dead: bool) -> MirProgram {
    let source = ";@actionc reviewed-6502 97b7e58d569863fa\n;@actionc abi-inputs-only\n;@actionc reads-only none\n;@actionc writes-only $AC/$AD\nPROC Touch=*(BYTE value)\nASM\nsta $AC\nldx #$5A\nstx $AD\nrts\nENDASM\nCARD FUNC Hold(CARD keep) Touch(7) RETURN(keep)\nPROC Main() RETURN";
    let mut p =
        crate::mir6502::materialize_program(lower_with_home(source), &Default::default()).unwrap();
    if dead {
        // Keep a real resident object, then start a new logical value lifetime
        // after the call. Applying this after materialization avoids optimizing
        // away the entire constant-valued home and making the check vacuous.
        let caller = p.routines.iter_mut().find(|r| r.name == "Hold").unwrap();
        let MirStorageBase::Param(param) = caller.frame.params[0].base else {
            panic!()
        };
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
fn reviewed_abi_inputs_apply_only_to_the_current_direct_call_and_body() {
    let source = "BYTE ARRAY table(4)=[1 2 3 4]\nPROC Touch=*(BYTE ch)\nASM\nrol a\nand #0\ntax\nora table,x\nsta $A0\nrts\nENDASM\nCARD FUNC Hold(CARD keep) Touch(7) RETURN(keep)\nPROC Main() RETURN";
    let base = lower_with_home(source);
    let callee = &base.routines[0];
    let hash = abi_inputs_review_fingerprint(&base, callee).unwrap();
    assert!(crate::mir6502::materialize_program(base.clone(), &Default::default()).is_err());
    let annotated = source.replace(
        "PROC Touch",
        &format!(";@actionc abi-inputs-only {hash:016x}\nPROC Touch"),
    );
    for source in [annotated.clone(), annotated.replace('\n', "\r\n")] {
        let p = lower_with_home(&source);
        // The entry review certifies only the supplied lanes. The live home
        // still needs a separate guarantee about writes.
        assert!(crate::mir6502::materialize_program(p.clone(), &Default::default()).is_err());
        assert!(ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::ActualEntry).inputs(call(&p)).is_ok());
        assert_eq!(p.routines[0].frame.abi_inputs_only_review, Some(hash));
        assert!(
            MirCalleeEntryFact::analyze(&p, callee.id, MirCalleeEntryStage::ActualEntry).is_none()
        );
        assert!(
            ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry)
                .inputs(call(&p))
                .is_err()
        );
        let mut bad_call = call(&p).clone();
        let MirOp::Call { args, .. } = &mut bad_call else {
            unreachable!()
        };
        args.clear();
        assert!(
            ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::ActualEntry)
                .inputs(&bad_call)
                .is_err()
        );
    }
    // Each edit recompiles through the normal lowering path with the original review.
    for changed in [
        annotated.replace("and #0", "and #1"),
        annotated.replace("table", "replacement"),
        annotated.replace("table(4)", "table(5)"),
        annotated.replace("BYTE ch", "CARD ch"),
    ] {
        let p = lower_with_home(&changed);
        assert_ne!(
            abi_inputs_review_fingerprint(&p, &p.routines[0]),
            Some(hash)
        );
        assert!(crate::mir6502::materialize_program(p, &Default::default()).is_err());
    }
    let mut overlapping = lower_with_home(&annotated.replace("sta $A0", "sta $E5"));
    let current = abi_inputs_review_fingerprint(&overlapping, &overlapping.routines[0]).unwrap();
    overlapping.routines[0].frame.abi_inputs_only_review = Some(current);
    assert!(
        ResidenceCalls::with_acyclic(&overlapping, MirCalleeEntryStage::ActualEntry)
            .inputs(call(&overlapping))
            .is_ok()
    );
    assert!(
        crate::mir6502::materialize_program(overlapping, &Default::default()).is_err(),
        "live high byte overwritten"
    );
    let mut hidden_call = base;
    replace_machine(&mut hidden_call, &[0x20, 0, 6, 0x60]);
    assert!(abi_inputs_review_fingerprint(&hidden_call, &hidden_call.routines[0]).is_none());
}

#[test]
fn certified_machine_residence_uses_current_writes_and_retains_scoped_admission() {
    let p = program(false);
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
            assert!(facts.inputs(call(&changed)).is_err());
            assert!(facts.writes(call(&changed)).is_err());
            assert!(!verify(&changed).is_empty(), "{byte:x} {absolute}");
        }
    }
}

#[test]
fn certified_inputs_do_not_turn_unknown_reads_or_writes_into_preservation() {
    for dead in [false, true] {
        let mut p = program(dead);
        p.routines[0].frame.callee_review.as_mut().unwrap().facts.writes = None;
        let mut facts = ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::ActualEntry);
        assert!(facts.inputs(call(&p)).is_ok());
        assert_eq!(
            facts.writes(call(&p)),
            Err("unknown residence callee writes")
        );
        assert_eq!(verify(&p).is_empty(), dead);
    }
    let mut p = program(false);
    p.routines[0].frame.callee_review.as_mut().unwrap().facts.reads = None;
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
    let p = program(true);
    for abi in [MirRoutineAbi::Action, MirRoutineAbi::ActionObservable] {
        // A changed byte invalidates the entire reviewed contract.
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
            MirKnownMemoryWrites::Unknown
        ));
        let mut facts = ResidenceCalls::with_acyclic(&changed, MirCalleeEntryStage::ActualEntry);
        assert_eq!(
            facts.inputs(call(&changed)),
            Err("missing reviewed assembly entry guarantee")
        );
        assert!(facts.writes(call(&changed)).is_err());
        assert!(!verify(&changed).is_empty());
    }
    for bytes in [
        vec![0xa5, 0xe8, 0x60],
        vec![0xb0, 0, 0x60],
        vec![0x8a, 0x60],
    ] {
        let mut changed = p.clone();
        replace_machine(&mut changed, &bytes);
        assert_eq!(
            ResidenceCalls::with_acyclic(&changed, MirCalleeEntryStage::ActualEntry)
                .inputs(call(&changed)),
            Err("missing reviewed assembly entry guarantee")
        );
    }
}

#[test]
fn nested_reviewed_write_rejects_an_overlapping_live_home_at_the_call() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/reviewed_conversion_home.act");
    let ast = crate::includes::load_program_with_includes(path).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let mut input = crate::mir6502::lower_program(&nir).unwrap();
    let hold = input
        .routines
        .iter_mut()
        .find(|r| r.name == "HoldConverted")
        .unwrap();
    let MirStorageBase::Param(param) = hold.frame.params[0].base else {
        panic!()
    };
    hold.frame.parameter_homes = Some(vec![MirParamBinding {
        param,
        home: MirParamHome::ZeroPage(MirFixedZpSlot(0xe4)),
    }]);
    hold.frame
        .fixed_zero_page
        .extend([MirFixedZpSlot(0xe4), MirFixedZpSlot(0xe5)]);
    let p = crate::mir6502::materialize_program(input, &Default::default()).unwrap();
    for start in [0xe4, 0xa0] {
        let mut candidate = p.clone();
        let hold = candidate
            .routines
            .iter_mut()
            .find(|r| r.name == "HoldConverted")
            .unwrap();
        hold.frame.parameter_homes.as_mut().unwrap()[0].home =
            MirParamHome::ZeroPage(MirFixedZpSlot(start));
        let call_site = hold
            .blocks
            .iter()
            .find_map(|b| {
                b.ops
                    .iter()
                    .position(|op| matches!(op, MirOp::Call { .. }))
                    .map(|op_index| crate::mir6502::analysis::sites::MirSite::Op {
                        block: b.id,
                        op_index,
                    })
            })
            .unwrap();
        let hold = candidate
            .routines
            .iter()
            .find(|r| r.name == "HoldConverted")
            .unwrap();
        let mut calls =
            ResidenceCalls::at_origin(&candidate, MirCalleeEntryStage::ActualEntry, 0x3000);
        let errors = verify_lifetimes(hold, Some(&mut calls));
        if start == 0xe4 {
            assert!(errors.is_empty(), "{errors:?}");
        } else {
            assert!(
                errors
                    .iter()
                    .any(|d| d.message.contains("clobbers live source-object home")
                        && d.message.contains(&format!("{call_site:?}"))),
                "{errors:?}"
            );
        }
    }
}

#[test]
fn supplied_home_checks_each_lane_of_native_wrapper_destinations() {
    let source = "PROC Draw(BYTE POINTER p BYTE n) p^=n RETURN
        PROC Forward(BYTE POINTER p BYTE n) Draw(p,n) RETURN
        CARD FUNC Hold(CARD keep) Forward($A2,BYTE(keep)) RETURN(keep+1)
        PROC Main() RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let input = crate::mir6502::lower_program(&nir).unwrap();
    let selected = crate::mir6502::materialize_program(input.clone(), &Default::default()).unwrap();
    for p in [&input, &selected] {
        let hold = p.routines.iter().find(|r| r.name == "Hold").unwrap();
        assert!(physical::has_pointee_writes(p, hold));
        assert!(
            physical::numeric_observations(p, None)
                .unwrap()
                .contains(&(0xa2, 1))
        );
    }
    for start in [0xa1, 0xa2, 0xa8] {
        let mut p = selected.clone();
        let hold = p.routines.iter_mut().find(|r| r.name == "Hold").unwrap();
        let MirStorageBase::Param(param) = hold.frame.params[0].base else {
            panic!()
        };
        hold.frame.parameter_homes = Some(vec![MirParamBinding {
            param,
            home: MirParamHome::ZeroPage(MirFixedZpSlot(start)),
        }]);
        let id = hold.id;
        let hold = p.routines.iter().find(|r| r.id == id).unwrap();
        let mut calls = ResidenceCalls::at_origin(&p, MirCalleeEntryStage::ActualEntry, 0x3000);
        let addresses = ObjectAddressControl::new(&p);
        let errors = super::super::lifetimes::verify_with_address_control(
            hold,
            Some(&mut calls),
            Some(&addresses),
        );
        let site = hold
            .blocks
            .iter()
            .find_map(|b| {
                b.ops
                    .iter()
                    .position(|op| matches!(op, MirOp::Call { .. }))
                    .map(|op_index| crate::mir6502::analysis::sites::MirSite::Op {
                        block: b.id,
                        op_index,
                    })
            })
            .unwrap();
        if start == 0xa8 {
            assert!(errors.is_empty(), "{errors:?}");
        } else {
            assert!(
                errors.iter().any(|d| d.message.contains("clobbers live")
                    && d.message.contains(&format!("{site:?}"))),
                "{errors:?}"
            );
        }
    }
}

#[test]
fn tn_print_live_index_rejects_internal_result_at_putchar() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/tn/modern/TN.ACT");
    let ast = crate::includes::load_program_with_includes(path).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let mut p = crate::mir6502::materialize_program(
        crate::mir6502::lower_program(&nir).unwrap(),
        &Default::default(),
    )
    .unwrap();
    let putchar = p.routines.iter().find(|r| r.name == "Putchar").unwrap().id;
    let print = p.routines.iter_mut().find(|r| r.name == "Print").unwrap();
    let MirStorageBase::Local(index) = print
        .frame
        .locals
        .iter()
        .find(|l| l.name.as_deref() == Some("i"))
        .unwrap()
        .base
    else {
        panic!()
    };
    print.frame.local_homes = Some(vec![MirLocalBinding {
        local: index,
        home: MirLocalHome::ZeroPage(MirFixedZpSlot(0xa0)),
    }]);
    let site = print
        .blocks
        .iter()
        .find_map(|b| {
            let op_index = b.ops.iter().position(|op| {
                matches!(op, MirOp::Call { target: MirCallTarget::Routine(id), .. } if *id == putchar)
            })?;
            Some(crate::mir6502::analysis::sites::MirSite::Op { block: b.id, op_index })
        })
        .unwrap();
    let print = p.routines.iter().find(|r| r.name == "Print").unwrap();
    let mut calls = ResidenceCalls::at_origin(&p, MirCalleeEntryStage::ActualEntry, 0x3000);
    let addresses = ObjectAddressControl::new(&p);
    let errors = super::super::lifetimes::verify_with_address_control(
        print,
        Some(&mut calls),
        Some(&addresses),
    );
    assert!(
        errors.iter().any(
            |d| d.message.contains("clobbers live") && d.message.contains(&format!("{site:?}"))
        ),
        "{errors:?}"
    );
}
