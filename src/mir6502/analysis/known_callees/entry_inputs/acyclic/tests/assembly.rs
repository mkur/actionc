use super::*;

fn machine_program(bytes: &[u8]) -> MirProgram {
    let mut p = program();
    p.machine_blocks.push(MirMachineBlock {
        id: MirMachineBlockId(0),
        source_span: None,
        items: bytes.iter().copied().map(MirMachineItem::Byte).collect(),
    });
    p.routines[0].abi = MirRoutineAbi::ActionObservable;
    p.routines[0].blocks = vec![block(
        0,
        vec![MirOp::MachineBlock {
            id: MirMachineBlockId(0),
            effects: MirEffects::default(),
        }],
        MirTerminator::Unreachable,
    )];
    p
}

#[test]
fn certified_machine_leaf_composes_diamond_without_inventing_result_definitions() {
    // STA $AC; STX $AD; LDA #7; STA $A0; RTS.
    let p = machine_program(&[0x85, 0xac, 0x86, 0xad, 0xa9, 7, 0x85, 0xa0, 0x60]);
    let leaf =
        MirCalleeEntryFact::analyze(&p, RoutineId(0), MirCalleeEntryStage::ActualEntry).unwrap();
    assert!(leaf.inputs.registers.a && leaf.inputs.registers.x);
    assert!(leaf.inputs.fixed.is_empty() && !leaf.inputs.flags.any());
    assert!(leaf.defined_results.is_empty());
    let mut analysis = MirAcyclicEntryAnalysis::new(&p);
    assert_eq!(analysis.analyze(RoutineId(0)).as_deref(), Some(&leaf));
    let outer = analysis.analyze(RoutineId(3)).unwrap();
    assert_eq!(analysis.entries.len(), 4);
    assert!(Arc::ptr_eq(
        &outer,
        &analysis.analyze(RoutineId(3)).unwrap()
    ));
    // Reading the unproved assembly result still observes incoming $A0. A
    // declared byte result or finite write must not manufacture a definition.
    assert!(outer.inputs.fixed.contains(&0xa0));
    assert!(outer.is_current(&p, MirCalleeEntryStage::ActualEntry));
    assert!(!outer.is_current(&p, MirCalleeEntryStage::PlannedCapture));
    let mut reversed = p.clone();
    reversed.routines.reverse();
    assert_eq!(fact(&reversed).inputs, outer.inputs);

    for mutation in 0..4 {
        let mut changed = p.clone();
        match mutation {
            0 => changed.machine_blocks[0].items[5] = MirMachineItem::Byte(8),
            1 => changed.routines[0].effects.may_call_os = true,
            2 => changed.routines[0].direct_result_abi = Some(MirDirectResultAbi::Void),
            _ => changed.globals.push(MirGlobal {
                id: crate::nir::SymbolId(0),
                name: "state".into(),
                kind: "byte".into(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(0xe8),
                init: None,
            }),
        }
        assert!(
            !outer.is_current(&changed, MirCalleeEntryStage::ActualEntry),
            "{mutation}"
        );
    }
    let mut bad_site = p;
    let mut second = call(0);
    let MirOp::Call { args, .. } = &mut second else {
        unreachable!()
    };
    args.push(MirCallArg {
        value: MirValue::ConstU8(1),
        width: MirWidth::Byte,
        home: MirArgHome::Reg(MirReg::A),
    });
    bad_site.routines[1].blocks[0].ops.push(second);
    let mut analysis = MirAcyclicEntryAnalysis::new(&bad_site);
    assert!(analysis.analyze(RoutineId(0)).is_some());
    assert!(analysis.analyze(RoutineId(3)).is_none());
    assert_eq!(analysis.failure(RoutineId(3)).unwrap().0, RoutineId(1));
}

#[test]
fn machine_leaf_certification_retains_unsupported_and_mixed_body_boundaries() {
    for bytes in [
        vec![0x20, 0x00, 0x60, 0x60], // Hidden JSR.
        vec![0x4c, 0x00, 0x60],       // Open JMP.
        vec![0x02, 0x60],             // Unsupported opcode.
        vec![0xd0, 0x7f, 0x60],       // Branch outside the body.
        vec![0xea],                   // No return.
    ] {
        let p = machine_program(&bytes);
        let mut analysis = MirAcyclicEntryAnalysis::new(&p);
        assert!(analysis.analyze(RoutineId(3)).is_none(), "{bytes:x?}");
        assert!(analysis.analyze(RoutineId(3)).is_none());
        assert_eq!(
            analysis.failure(RoutineId(3)).unwrap().2,
            "uncertified machine leaf entry"
        );
    }
    for mutation in 0..4 {
        let mut p = machine_program(&[0x60]);
        match mutation {
            0 => p.routines[0].blocks[0].ops.insert(0, load(1)),
            1 => p.routines[0].blocks[0].ops.push(call(1)),
            2 => p.routines[0].effects.opaque = true,
            _ => p.routines[0].effects.may_call_os = true,
        }
        assert!(
            MirAcyclicEntryAnalysis::new(&p)
                .analyze(RoutineId(3))
                .is_none(),
            "{mutation}"
        );
    }
}

fn reviewed_leaf(bytes: &[u8], reads: Option<Vec<crate::callee6502::DataRange<crate::nir::SymbolId>>>,
    result: MirDirectResultAbi, mirror: bool) -> MirProgram {
    let mut p = machine_program(bytes);
    let routine = &mut p.routines[0];
    routine.direct_result_abi = Some(result);
    routine.frame.contains_source_asm = true;
    let mut review = crate::callee6502::Review {
        fingerprint: 0,
        facts: crate::callee6502::CallFacts {
            abi_inputs_only: true, returns_a_equals_a0: mirror, reads,
            writes: Some(vec![crate::callee6502::DataRange { global: None, start: 0xa0, end: 0xa0 }]),
            ..Default::default()
        },
    };
    review.fingerprint = crate::mir6502::analysis::known_callees::body_fingerprint(
        &p, &p.routines[0], review.version()).unwrap();
    p.routines[0].frame.callee_review = Some(review);
    p
}

#[test]
fn normal_return_value_composes_without_claiming_a_physical_write() {
    let byte = MirDirectResultAbi::Scalar(MirAbiIntegerWidth::Byte);
    let mut p = reviewed_leaf(&[0xa9, 7, 0x85, 0xa0, 0x60], Some(vec![]), byte, true);
    // Forward the slot through a wrapper. Its caller consumes the slot.
    p.routines[1].blocks[0].ops = vec![call(0)];
    p.routines[3].blocks[0].ops = vec![call(1), read(0xa0)];
    let mut analysis = MirAcyclicEntryAnalysis::new(&p);
    for id in [0, 1, 3] {
        let f = analysis.analyze(RoutineId(id)).unwrap();
        assert!(f.defined_results.is_empty(), "r{id}");
        assert!(f.home_results.slots.contains(&0xa0), "r{id}");
        assert!(!f.inputs.fixed.contains(&0xa0), "r{id}");
    }
    // Reading the mirror right after the leaf call requires no incoming A.
    p.routines[1].blocks[0].ops.push(store(0xe8));
    assert!(!MirAcyclicEntryAnalysis::new(&p).analyze(RoutineId(1)).unwrap().inputs.registers.a);
    // A path around the call still carries the incoming slot to the use.
    p.routines[1].blocks = vec![
        block(0, vec![], branch(1, 2)),
        block(1, vec![call(0)], MirTerminator::Return),
        block(2, vec![], MirTerminator::Return),
    ];
    assert!(fact(&p).inputs.fixed.contains(&0xa0));
}

#[test]
fn reviewed_result_preserves_possible_reads_and_only_supplies_certified_lanes() {
    let word = MirDirectResultAbi::Scalar(MirAbiIntegerWidth::Word);
    let old = crate::callee6502::DataRange { global: None, start: 0xa0, end: 0xa0 };
    for reads in [Some(vec![old]), None] {
        let mut p = reviewed_leaf(&[0xa5, 0xa0, 0x60], reads, word, true);
        p.routines[1].blocks[0].ops = vec![call(0), read(0xa1)];
        let mut analysis = MirAcyclicEntryAnalysis::new(&p);
        let entry = analysis.analyze(RoutineId(1)).unwrap();
        assert_eq!(entry.inputs.fixed, BTreeSet::from([0xa0, 0xa1]));
        assert!(entry.defined_results.is_empty());
    }
    let p = reviewed_leaf(&[0xa9, 7, 0x85, 0xa0, 0x60], Some(vec![]), MirDirectResultAbi::Void, false);
    assert!(fact(&p).inputs.fixed.contains(&0xa0));
}

#[test]
fn reviewed_zp_definition_composes_only_on_returning_paths_that_supply_it() {
    let mut p = reviewed_leaf(&[0xa9, 7, 0x85, 0xa0, 0x60], Some(vec![]), MirDirectResultAbi::Void, false);
    p.routines[0].frame.callee_review.as_mut().unwrap().facts.defines_on_return.insert(0xa0);
    p.routines[1].direct_result_abi = Some(MirDirectResultAbi::Void);
    p.routines[1].blocks[0].ops = vec![call(0)];
    p.routines[3].blocks[0].ops = vec![call(1), read(0xa0)];
    assert!(!fact(&p).inputs.fixed.contains(&0xa0));
    let wrap = MirAcyclicEntryAnalysis::new(&p).analyze(RoutineId(1)).unwrap();
    assert!(wrap.defined_results.is_empty());
    assert_eq!(wrap.home_results.defined_zp, BTreeSet::from([0xa0]));

    p.routines[1].blocks = vec![
        block(0, vec![], branch(1, 2)),
        block(1, vec![call(0)], MirTerminator::Return),
        block(2, vec![], MirTerminator::Return),
    ];
    assert!(fact(&p).inputs.fixed.contains(&0xa0));

    // An increment guarantees a store and also consumes the incoming byte.
    let mut inc = reviewed_leaf(&[0xe6, 0xa0, 0x60], None, MirDirectResultAbi::Void, false);
    inc.routines[0].frame.callee_review.as_mut().unwrap().facts.defines_on_return.insert(0xa0);
    inc.routines[1].blocks[0].ops = vec![call(0)];
    let wrap = MirAcyclicEntryAnalysis::new(&inc).analyze(RoutineId(1)).unwrap();
    assert!(wrap.inputs.fixed.contains(&0xa0));
    assert!(wrap.home_results.defined_zp.contains(&0xa0));
}
