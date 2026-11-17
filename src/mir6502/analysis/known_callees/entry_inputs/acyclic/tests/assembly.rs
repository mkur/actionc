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
