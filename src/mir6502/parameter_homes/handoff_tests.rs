use super::*;
use crate::mir6502::analysis::known_callees::MirCalleeEntryFact;

fn source(text: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    super::super::lower_program(&nir).unwrap()
}
fn controls() -> MirProgram {
    source(
        &include_str!("../../../fixtures/runtime/parameter_argument_handoff.act")
            .replace("\r\n", "\n"),
    )
}
fn routine<'a>(program: &'a MirProgram, name: &str) -> &'a MirRoutine {
    program.routines.iter().find(|r| r.name == name).unwrap()
}

#[test]
fn wrapper_proof_admits_terminal_inputs_and_rejects_scope_boundaries() {
    let program = controls();
    for name in ["ForwardWords", "ForwardBytes", "CopyCounted"] {
        let fact = handoff::analyze(&program, routine(&program, name)).unwrap();
        assert!(fact.is_current(&program));
        assert_eq!(fact.outgoing.len(), 5);
    }
    for name in [
        "Mutated",
        "AfterCall",
        "Twice",
        "Exposed",
        "ExtraEntry",
        "Main",
    ] {
        assert!(
            handoff::analyze(&program, routine(&program, name)).is_err(),
            "{name}"
        );
    }
    let fact = handoff::analyze(&program, routine(&program, "ForwardWords")).unwrap();
    let mut changed = program.clone();
    changed
        .routines
        .iter_mut()
        .find(|r| r.name == "ForwardWords")
        .unwrap()
        .blocks[0]
        .ops
        .pop();
    assert!(!fact.is_current(&changed));
    let mut changed = program.clone();
    changed.machine_blocks[0]
        .items
        .push(MirMachineItem::Byte(0xea));
    assert!(!fact.is_current(&changed));
    let mut changed = program.clone();
    changed.globals[0].backing = MirGlobalBacking::Absolute(0xa0);
    assert!(!fact.is_current(&changed));
    let mut changed = program.clone();
    let r = changed
        .routines
        .iter_mut()
        .find(|r| r.name == "ForwardWords")
        .unwrap();
    let op = r.blocks[0].ops[0].clone();
    for _ in 0..65 {
        r.blocks[0].ops.insert(0, op.clone());
    }
    assert_eq!(
        handoff::analyze(&changed, routine(&changed, "ForwardWords")).unwrap_err(),
        "setup-budget"
    );
}

#[test]
fn shared_machine_entry_facts_distinguish_overwrites_flags_and_extra_reads() {
    let program = controls();
    let copy = MirCalleeEntryFact::analyze(&program, routine(&program, "ReverseCopy").id).unwrap();
    assert_eq!(copy.inputs.fixed, BTreeSet::from([0xa3, 0xa4]));
    assert!(copy.inputs.registers.a && copy.inputs.registers.x && copy.inputs.registers.y);
    assert!(!copy.inputs.flags.any());
    let extra =
        MirCalleeEntryFact::analyze(&program, routine(&program, "ObserveExtra").id).unwrap();
    assert_eq!(extra.inputs.fixed, BTreeSet::from([0xa3, 0xa4, 0xac]));
    for (prefix, incoming_carry) in [("", true), ("clc\n", false)] {
        let p = source(&format!(
            "PROC Sink=*()\nASM\n{prefix}lda #0\nbcs done\ndone:\nrts\nENDASM\nPROC Main() Sink() RETURN"
        ));
        let fact = MirCalleeEntryFact::analyze(&p, routine(&p, "Sink").id).unwrap();
        assert_eq!(fact.inputs.flags.c, incoming_carry);
        assert!(!fact.inputs.flags.n && !fact.inputs.flags.z);
    }
    // The callee's indirect writes are intentionally unbounded. Entry inputs
    // qualify without claiming an exact destination footprint.
    let exits = super::super::analysis::known_callees::MirKnownCalleeSummaries::analyze(&program);
    assert!(!matches!(
        exits.get(copy.source.id).map(|s| s.writes()),
        Some(super::super::analysis::known_callees::MirKnownMemoryWrites::Exact(_))
    ));
}

fn bound_control() -> (MirProgram, usize) {
    let mut program = controls();
    let index = program
        .routines
        .iter()
        .position(|r| r.name == "ForwardWords")
        .unwrap();
    let contract = handoff::analyze(&program, &program.routines[index])
        .unwrap()
        .contract();
    let r = &mut program.routines[index];
    r.frame.parameter_homes = Some(
        r.frame
            .params
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let MirStorageBase::Param(id) = s.base else {
                    panic!()
                };
                MirParamBinding {
                    param: id,
                    home: MirParamHome::ZeroPage(MirFixedZpSlot(0xa0 + 2 * i as u8)),
                }
            })
            .collect(),
    );
    r.frame
        .fixed_zero_page
        .extend((0xa0..=0xa3).map(MirFixedZpSlot));
    r.frame.parameter_handoff = Some(contract);
    (program, index)
}

#[test]
fn call_consumption_precedes_clobbers_but_cannot_hide_setup_or_postcall_uses() {
    let (program, index) = bound_control();
    assert!(handoff::verify(&program, &program.routines[index]).is_empty());
    assert!(verify_lifetimes(&program.routines[index]).is_empty());
    let mut without = program.routines[index].clone();
    without.frame.parameter_handoff = None;
    assert!(!verify_lifetimes(&without).is_empty());
    let mut clobber = program.routines[index].clone();
    clobber.blocks[0].ops.insert(
        0,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xa2))),
            src: MirValue::ConstU8(0),
            width: MirWidth::Byte,
        },
    );
    assert!(!verify_lifetimes(&clobber).is_empty());
    let mut after = program.clone();
    let read = after.routines[index].blocks[0].ops[0].clone();
    after.routines[index].blocks[0].ops.push(read);
    assert!(!handoff::verify(&after, &after.routines[index]).is_empty());
    assert!(!verify_lifetimes(&after.routines[index]).is_empty());
    let mut stale = program.clone();
    stale.routines[index]
        .frame
        .parameter_handoff
        .as_mut()
        .unwrap()
        .target = RoutineId(999);
    assert!(!handoff::verify(&stale, &stale.routines[index]).is_empty());
}

#[test]
fn scalar_entry_projection_distinguishes_defined_and_incoming_fixed_bytes() {
    for (body, observes) in [
        ("cell=value out=cell", false),
        ("out=cell cell=value", true),
    ] {
        let p = source(&format!(
            "BYTE cell=$AC,out=$680\nPROC Sink(BYTE value) {body} RETURN\nPROC Main() Sink(1) RETURN"
        ));
        let fact = MirCalleeEntryFact::analyze(&p, routine(&p, "Sink").id).unwrap();
        assert_eq!(fact.inputs.fixed.contains(&0xac), observes);
    }
}

#[test]
fn wrapper_definition_witness_rejects_a_missing_producer() {
    let mut p = controls();
    let r = p
        .routines
        .iter_mut()
        .find(|r| r.name == "ForwardWords")
        .unwrap();
    r.blocks[0].ops.remove(0);
    assert!(matches!(
        handoff::analyze(&p, routine(&p, "ForwardWords")),
        Err("missing-argument-definition")
    ));
}

#[test]
fn direct_selection_retains_registers_and_declines_fixed_slot_cycles() {
    let program = controls();
    let proposal = handoff::propose(&program, &program).unwrap();
    let forward = routine(&proposal, "ForwardWords");
    let lanes: Vec<_> = forward
        .frame
        .params
        .iter()
        .flat_map(|p| {
            let MirStorageBase::Param(id) = p.base else {
                panic!()
            };
            forward.frame.parameter_lanes(id).unwrap()
        })
        .collect();
    assert_eq!(
        lanes,
        vec![
            MirParamLane::ZeroPage(MirFixedZpSlot(0xa0)),
            MirParamLane::Reg(MirReg::X),
            MirParamLane::Reg(MirReg::Y),
            MirParamLane::ZeroPage(MirFixedZpSlot(0xa3))
        ]
    );
    assert!(
        routine(&proposal, "CopyCounted")
            .frame
            .parameter_handoff
            .is_some()
    );
    assert!(verify_storage(&proposal, forward).is_empty());
    let extra = source(
        &include_str!("../../../fixtures/runtime/parameter_argument_handoff_direct.act")
            .replace("\r\n", "\n"),
    );
    let proposed = handoff::propose(&extra, &extra).unwrap();
    assert_eq!(routine(&extra, "Swapped"), routine(&proposed, "Swapped"));
    for name in ["Repeated", "Computed"] {
        assert!(
            routine(&proposed, name).frame.parameter_handoff.is_some(),
            "{name}"
        );
    }
    let owned = source(&format!(
        "BYTE owned=$A0\n{}",
        include_str!("../../../fixtures/runtime/parameter_argument_handoff_direct.act")
            .replace("\r\n", "\n")
    ));
    assert!(handoff::propose(&owned, &owned).is_none());
}

#[test]
fn resident_pointer_selection_requires_a_parameter_definition() {
    let mut program = controls();
    let index = program
        .routines
        .iter()
        .position(|r| r.name == "CopyCounted")
        .unwrap();
    let r = &mut program.routines[index];
    let MirOp::Load { dst, .. } = r.blocks[0].ops[1].clone() else {
        panic!()
    };
    r.blocks[0].ops[1] = MirOp::Binary {
        op: MirBinaryOp::Add,
        dst,
        left: MirValue::ConstU16(0x7000),
        right: MirValue::ConstU16(1),
        width: MirWidth::Word,
        carry_in: Some(MirCarryIn::Clear),
        carry_out: MirCarryOut::Ignore,
    };
    assert!(handoff::analyze(&program, &program.routines[index]).is_ok());
    let proposal = handoff::propose(&program, &program).unwrap();
    assert!(proposal.routines[index].frame.parameter_handoff.is_none());
}

#[test]
fn full_word_argument_cannot_be_proved_from_a_partial_definition() {
    let mut p = controls();
    let index = p
        .routines
        .iter()
        .position(|r| r.name == "ForwardWords")
        .unwrap();
    let MirOp::Load { dst, width, .. } = &mut p.routines[index].blocks[0].ops[0] else {
        panic!()
    };
    let MirDef::VTemp(id) = *dst else { panic!() };
    *dst = MirDef::VTempByte { id, byte: 0 };
    *width = MirWidth::Byte;
    assert_eq!(
        handoff::analyze(&p, &p.routines[index]).unwrap_err(),
        "incomplete-argument-definition"
    );
    let mut complete = controls();
    let MirOp::Call { args, .. } = complete.routines[index].blocks[0].ops.last_mut().unwrap()
    else {
        panic!()
    };
    args[2].value = MirValue::Def(MirDef::VTempByte { id, byte: 0 });
    assert!(handoff::analyze(&complete, &complete.routines[index]).is_ok());
}

#[test]
fn capture_boundary_baseline_exposes_false_working_home_inputs() {
    let text =
        include_str!("../../../fixtures/runtime/callee_entry_capture.act").replace("\r\n", "\n");
    let ast = crate::parser::parse(&crate::lexer::tokenize(&text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    let config = super::super::Mir6502Config {
        enable_small_leaf_inlining: false,
        ..super::super::Mir6502Config::optimized()
    };
    let runtime = crate::runtime::Runtime::ActionCart;
    let session = super::super::trace::Mir6502Trace::default();
    let trace = session.context();
    let (program, resume) =
        super::super::prepare_program_for_emission(&nir, 0x3000, &config, runtime, &trace).unwrap();
    let resume = resume.unwrap();
    let baseline = super::super::finalize_output(&nir, program, 0x3000, runtime, &trace).unwrap();
    let selected = SelectedPlacement {
        input: Arc::new(resume.core.checkpoint.input.program.clone()),
        executable: baseline,
    };
    let proposal = propose(&selected.input).unwrap();
    let selected = try_proposal(
        &nir,
        selected,
        &resume,
        proposal,
        "parameter-homes",
        &config,
        0x3000,
        runtime,
        &trace,
    );
    let callee = routine(&selected.input, "CaptureWords");
    assert!(callee.frame.parameter_homes.is_some());
    let before = MirCalleeEntryFact::analyze(&selected.input, callee.id).unwrap();
    assert_eq!(
        before.inputs.fixed,
        BTreeSet::from([0xa0, 0xa1, 0xa2, 0xa3])
    );
    assert_eq!(
        handoff::analyze(&selected.input, routine(&selected.input, "ForwardWords")).unwrap_err(),
        "extra-entry-flags-or-stack"
    );
    assert!(before.inputs.registers.sp); // Normal Return, not a non-ABI stack input.
    let after = MirCalleeEntryFact::analyze(&selected.executable.program, callee.id).unwrap();
    assert_eq!(after.inputs.fixed, BTreeSet::from([0xa3]));
    assert!(after.inputs.registers.a && after.inputs.registers.x && after.inputs.registers.y);
    assert!(!after.inputs.flags.any());
    let extra = MirCalleeEntryFact::analyze(
        &selected.executable.program,
        routine(&selected.input, "ExtraFixed").id,
    )
    .unwrap();
    assert_eq!(extra.inputs.fixed, BTreeSet::from([0xa3, 0xac]));
    assert!(
        MirCalleeEntryFact::analyze(&selected.input, routine(&selected.input, "ReadPointers").id)
            .is_none()
    );
    println!(
        "capture baseline before={:?} after={:?} extra={:?}",
        before.inputs, after.inputs, extra.inputs
    );
}
