use super::*;

fn leaf(p: &MirProgram) -> MirCalleeEntryFact {
    MirCalleeEntryFact::analyze(p, RoutineId(0), MirCalleeEntryStage::ActualEntry).unwrap()
}

#[test]
fn result_contract_limits_outputs_and_leaf_queries_agree() {
    for (contract, expected) in [
        (None, vec![]),
        (Some(MirDirectResultAbi::Void), vec![]),
        (Some(MirDirectResultAbi::Scalar(MirWidth::Byte)), vec![0xa0]),
        (
            Some(MirDirectResultAbi::Scalar(MirWidth::Word)),
            vec![0xa0, 0xa1],
        ),
        (Some(MirDirectResultAbi::Pointer), vec![0xa0, 0xa1]),
    ] {
        let mut p = program();
        p.routines[0].direct_result_abi = contract;
        // The old signature must neither enable nor override the new contract.
        p.routines[0].blocks[0].ops = vec![load(7), store(0xa0), store(0xa1), store(0xe4)];
        let direct = leaf(&p);
        let composed = MirAcyclicEntryAnalysis::new(&p)
            .analyze(RoutineId(0))
            .unwrap();
        assert_eq!(direct.defined_results, expected.into_iter().collect());
        assert_eq!(direct.defined_results, composed.defined_results);
        assert_eq!(direct.inputs, composed.inputs);
        assert!(
            MirCalleeEntryFact::analyze(&p, RoutineId(0), MirCalleeEntryStage::PlannedCapture)
                .unwrap()
                .defined_results
                .is_empty()
        );
        p.routines[0].scalar_signature = None;
        assert_eq!(leaf(&p).defined_results, direct.defined_results);
    }
}

#[test]
fn result_bytes_require_definitions_on_all_reachable_return_paths() {
    let mut p = program();
    p.routines[0].direct_result_abi = Some(MirDirectResultAbi::Scalar(MirWidth::Word));
    p.routines[0].blocks = vec![
        block(0, vec![load(7), store(0xa0)], branch(1, 2)),
        block(1, vec![store(0xa1)], MirTerminator::Return),
        block(2, vec![], MirTerminator::Return),
        block(9, vec![], MirTerminator::Return), // unreachable
    ];
    p.routines[3].blocks[0].ops = vec![call(0), read(0xa0), read(0xa1)];
    assert_eq!(leaf(&p).defined_results, BTreeSet::from([0xa0]));
    assert_eq!(fact(&p).inputs.fixed, BTreeSet::from([0xa1]));
    p.routines[0].blocks[2].ops.push(store(0xa1));
    assert_eq!(leaf(&p).defined_results, BTreeSet::from([0xa0, 0xa1]));
    assert!(fact(&p).inputs.fixed.is_empty());

    for end in [
        MirTerminator::Exit,
        MirTerminator::Unreachable,
        MirTerminator::Jump(MirEdge::plain(MirBlockId(0))),
    ] {
        p.routines[0].blocks = vec![
            block(0, vec![load(7), store(0xa0), store(0xa1)], end),
            block(9, vec![], MirTerminator::Return),
        ];
        assert!(
            leaf(&p).defined_results.is_empty(),
            "no reachable normal return"
        );
    }
}

#[test]
fn possible_call_writes_do_not_supply_a_missing_result_byte() {
    for writes in [
        MirMemoryEffect::Unknown,
        MirMemoryEffect::Regions(vec![MirMemoryRegion {
            kind: MirMemoryRegionKind::ZeroPage,
            offset: 0xa1,
            size: 1,
        }]),
    ] {
        let mut p = program();
        p.routines[0].direct_result_abi = Some(MirDirectResultAbi::Scalar(MirWidth::Word));
        p.routines[1].direct_result_abi = Some(MirDirectResultAbi::Void);
        p.routines[1].blocks[0].ops = vec![];
        let mut writer = call(1);
        let MirOp::Call { effects, .. } = &mut writer else {
            unreachable!()
        };
        effects.memory_writes = writes;
        p.routines[0].blocks[0].ops = vec![load(7), store(0xa0), writer];
        p.routines[3].blocks[0].ops = vec![call(0), read(0xa0), read(0xa1)];
        let mut analysis = MirAcyclicEntryAnalysis::new(&p);
        assert_eq!(
            analysis.analyze(RoutineId(0)).unwrap().defined_results,
            BTreeSet::from([0xa0])
        );
        assert_eq!(
            analysis.analyze(RoutineId(3)).unwrap().inputs.fixed,
            BTreeSet::from([0xa1])
        );
    }
}

#[test]
fn returned_byte_does_not_hide_an_incoming_read_and_metadata_invalidates_ancestors() {
    let mut p = program();
    p.routines[0].blocks[0].ops.insert(0, read(0xa0));
    let f = fact(&p);
    assert!(f.inputs.fixed.contains(&0xa0));
    assert_eq!(leaf(&p).defined_results, BTreeSet::from([0xa0]));
    for contract in [
        None,
        Some(MirDirectResultAbi::Void),
        Some(MirDirectResultAbi::Pointer),
    ] {
        let mut changed = p.clone();
        changed.routines[0].direct_result_abi = contract;
        assert!(!f.is_current(&changed, MirCalleeEntryStage::ActualEntry));
    }
}

fn consuming_call(width: MirWidth, home: MirResultHome) -> MirOp {
    let mut op = call(0);
    let MirOp::Call { result, abi, .. } = &mut op else {
        unreachable!()
    };
    abi.result = Some(home.clone());
    *result = Some(MirCallResult {
        dst: MirDef::Reg(MirReg::A),
        width,
        home,
    });
    op
}

#[test]
fn consumed_result_contracts_are_checked_at_every_call_site() {
    use crate::mir6502::parameter_homes::ResidenceCalls;
    let mut p = program();
    for home in [
        MirResultHome::ReturnSlot { offset: 0 },
        MirResultHome::FixedZeroPage(MirFixedZpSlot(0xa0)),
        MirResultHome::Absolute(0xa0),
    ] {
        let good = consuming_call(MirWidth::Byte, home);
        let mut calls = ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry);
        assert!(calls.inputs(&good).is_ok());
        for change in 0..6 {
            let mut bad = good.clone();
            let MirOp::Call {
                result,
                abi,
                additional_results,
                ..
            } = &mut bad
            else {
                unreachable!()
            };
            match change {
                0 => result.as_mut().unwrap().width = MirWidth::Word,
                1 => result.as_mut().unwrap().home = MirResultHome::Reg(MirReg::A),
                2 => abi.result = Some(MirResultHome::ReturnSlot { offset: 1 }),
                3 => abi.result = None,
                4 => additional_results.push(result.as_ref().unwrap().clone()),
                _ => abi.additional_results.push(MirHelperResult {
                    home: MirResultHome::ReturnSlot { offset: 1 },
                    width: MirWidth::Byte,
                }),
            }
            assert!(calls.inputs(&bad).is_err(), "{change}");
            let mut changed = p.clone();
            changed.routines[3].blocks[0].ops = vec![good.clone(), bad];
            assert!(
                MirAcyclicEntryAnalysis::new(&changed)
                    .analyze(RoutineId(3))
                    .is_none()
            );
        }
    }
    // Ignoring a result is valid; claiming a void result is not.
    assert!(
        ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry)
            .inputs(&call(0))
            .is_ok()
    );
    p.routines[0].direct_result_abi = Some(MirDirectResultAbi::Void);
    assert!(
        ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry)
            .inputs(&consuming_call(
                MirWidth::Byte,
                MirResultHome::ReturnSlot { offset: 0 }
            ))
            .is_err()
    );
}

#[test]
fn integer_and_pointer_parameter_results_compose_identically() {
    for parameter in ["CARD", "BYTE POINTER"] {
        let source = format!(
            "BYTE FUNC Fetch({parameter} p) RETURN(7)
            BYTE FUNC Middle({parameter} p) RETURN(Fetch(p))
            PROC Main() RETURN"
        );
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let n = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let p = crate::mir6502::materialize_program(
            crate::mir6502::lower_program(&n).unwrap(),
            &crate::mir6502::Mir6502Config::default(),
        )
        .unwrap();
        let mut analysis = MirAcyclicEntryAnalysis::new(&p);
        for name in ["Fetch", "Middle"] {
            let r = p.routines.iter().find(|r| r.name == name).unwrap();
            let f = analysis.analyze(r.id).unwrap();
            assert_eq!(f.defined_results, BTreeSet::from([0xa0]));
            assert!(!f.inputs.fixed.contains(&0xa0), "{parameter} {name}");
        }
    }
}
