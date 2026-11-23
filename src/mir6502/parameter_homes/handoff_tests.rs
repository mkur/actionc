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
fn global_state_does_not_replace_or_add_canonical_call_arguments() {
    for address in [0xe8, 0x600] {
        let p = source(&format!(
            "BYTE state=${address:04X}
             BYTE FUNC ReadState() RETURN(state)
             BYTE FUNC WithArg(BYTE arg) RETURN(arg+state)
             PROC Main() RETURN"
        ));
        for (name, outgoing) in [
            ("ReadState", vec![]),
            ("WithArg", vec![MirParamLane::Reg(MirReg::A)]),
        ] {
            let fact = MirCalleeEntryFact::analyze(&p, routine(&p, name).id,
                MirCalleeEntryStage::PlannedCapture).unwrap();
            assert_eq!(handoff::inputs_supplied(&fact, &outgoing), Ok(()));
            if name == "WithArg" {
                assert_eq!(handoff::inputs_supplied(&fact, &[]), Err("extra-entry-register"));
            }
        }
    }
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
    let copy = MirCalleeEntryFact::analyze(
        &program,
        routine(&program, "ReverseCopy").id,
        MirCalleeEntryStage::PlannedCapture,
    )
    .unwrap();
    assert_eq!(copy.inputs.fixed, BTreeSet::from([0xa3, 0xa4]));
    assert!(copy.inputs.registers.a && copy.inputs.registers.x && copy.inputs.registers.y);
    assert!(!copy.inputs.flags.any());
    let extra = MirCalleeEntryFact::analyze(
        &program,
        routine(&program, "ObserveExtra").id,
        MirCalleeEntryStage::PlannedCapture,
    );
    assert!(extra.is_none());
    for prefix in ["", "clc\n"] {
        let p = source(&format!(
            "PROC Sink=*()\nASM\n{prefix}lda #0\nbcs done\ndone:\nrts\nENDASM\nPROC Main() Sink() RETURN"
        ));
        let fact = MirCalleeEntryFact::analyze(
            &p,
            routine(&p, "Sink").id,
            MirCalleeEntryStage::PlannedCapture,
        );
        assert!(fact.is_none(), "unreviewed assembly {prefix:?}");
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
    assert!(
        handoff::verify(
            &program,
            &program.routines[index],
            MirCalleeEntryStage::PlannedCapture
        )
        .is_empty()
    );
    assert!(verify_lifetimes(&program.routines[index], None).is_empty());
    let mut without = program.routines[index].clone();
    without.frame.parameter_handoff = None;
    assert!(!verify_lifetimes(&without, None).is_empty());
    let mut clobber = program.routines[index].clone();
    clobber.blocks[0].ops.insert(
        0,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xa2))),
            src: MirValue::ConstU8(0),
            width: MirWidth::Byte,
        },
    );
    assert!(!verify_lifetimes(&clobber, None).is_empty());
    let mut after = program.clone();
    let read = after.routines[index].blocks[0].ops[0].clone();
    after.routines[index].blocks[0].ops.push(read);
    assert!(
        !handoff::verify(
            &after,
            &after.routines[index],
            MirCalleeEntryStage::PlannedCapture
        )
        .is_empty()
    );
    assert!(!verify_lifetimes(&after.routines[index], None).is_empty());
    let mut stale = program.clone();
    stale.routines[index]
        .frame
        .parameter_handoff
        .as_mut()
        .unwrap()
        .target = RoutineId(999);
    assert!(
        !handoff::verify(
            &stale,
            &stale.routines[index],
            MirCalleeEntryStage::PlannedCapture
        )
        .is_empty()
    );
}

#[test]
fn scalar_entry_projection_distinguishes_defined_and_incoming_fixed_bytes() {
    for (body, observes) in [
        ("cell=value out=cell", false),
        ("out=cell cell=value", true),
    ] {
        let mut p = source(&format!(
            "BYTE cell=$AC,out=$680\nPROC Sink(BYTE value) {body} RETURN\nPROC Main() Sink(1) RETURN"
        ));
        let fact = MirCalleeEntryFact::analyze(
            &p,
            routine(&p, "Sink").id,
            MirCalleeEntryStage::PlannedCapture,
        )
        .unwrap();
        assert!(!fact.inputs.fixed.contains(&0xac));
        // An independent physical read still has the original incoming-value
        // dependency; a typed store can discharge it on the same path.
        let cell = p.globals.iter().find(|g| g.name == "cell").unwrap().id;
        let sink = p.routines.iter_mut().find(|r| r.name == "Sink").unwrap();
        for op in sink.blocks.iter_mut().flat_map(|b| &mut b.ops) {
            if let MirOp::Load { src: MirAddr::Direct(mem), .. } = op
                && *mem == (MirMem::Global { id: cell, offset: 0 })
            {
                *mem = MirMem::FixedZeroPage(MirFixedZpSlot(0xac));
            }
        }
        let sink_id = sink.id;
        let raw = MirCalleeEntryFact::analyze(&p, sink_id,
            MirCalleeEntryStage::PlannedCapture).unwrap();
        assert_eq!(raw.inputs.fixed.contains(&0xac), observes);
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
    assert!(verify_storage(&proposal, forward, MirCalleeEntryStage::PlannedCapture).is_empty());
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
fn capture_boundary_supplies_working_homes_from_canonical_inputs() {
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
    let proposal = propose(&selected.input, &selected.executable.program, false, 0x3000).unwrap();
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
    let before = MirCalleeEntryFact::analyze(
        &selected.input,
        callee.id,
        MirCalleeEntryStage::PlannedCapture,
    )
    .unwrap();
    assert_eq!(before.inputs.fixed, BTreeSet::from([0xa3]));
    assert!(handoff::analyze(&selected.input, routine(&selected.input, "ForwardWords")).is_ok());
    assert!(!before.inputs.registers.sp);
    assert!(before.inputs.registers.a && before.inputs.registers.x && before.inputs.registers.y);
    let after = MirCalleeEntryFact::analyze(
        &selected.executable.program,
        callee.id,
        MirCalleeEntryStage::ActualEntry,
    )
    .unwrap();
    assert_eq!(after.inputs.fixed, BTreeSet::from([0xa3]));
    assert!(after.inputs.registers.a && after.inputs.registers.x && after.inputs.registers.y);
    assert!(!after.inputs.flags.any());
    let extra = MirCalleeEntryFact::analyze(
        &selected.executable.program,
        routine(&selected.input, "ExtraFixed").id,
        MirCalleeEntryStage::ActualEntry,
    )
    .unwrap();
    assert_eq!(extra.inputs.fixed, BTreeSet::from([0xa3]));
    assert!(
        MirCalleeEntryFact::analyze(
            &selected.input,
            routine(&selected.input, "ReadPointers").id,
            MirCalleeEntryStage::PlannedCapture
        )
        .is_some()
    );
    println!(
        "capture composed before={:?} after={:?} extra={:?}",
        before.inputs, after.inputs, extra.inputs
    );
}

#[test]
fn shared_capture_preserves_inline_bound_retained_absent_and_identity_lanes() {
    use crate::mir6502::abi::direct_param_capture;
    use MirParamLane::{Reg, ZeroPage};
    let p = source(
        "CARD out\nPROC Sink(CARD left,right) out=left out=right RETURN\nPROC Main() Sink(1,2) RETURN",
    );
    let original = routine(&p, "Sink");
    let ids: Vec<_> = original
        .frame
        .params
        .iter()
        .map(|p| match p.base {
            MirStorageBase::Param(id) => id,
            _ => panic!(),
        })
        .collect();
    let a = MirValue::Def(MirDef::Reg(MirReg::A));
    let x = MirValue::Def(MirDef::Reg(MirReg::X));
    let y = MirValue::Def(MirDef::Reg(MirReg::Y));
    let a3 = MirValue::PointerCell(MirMem::FixedZeroPage(MirFixedZpSlot(0xa3)));
    let stores = |r: &MirRoutine| {
        direct_param_capture(r)
            .into_iter()
            .map(|op| match op {
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Param { id, offset }),
                    src,
                    width: MirWidth::Byte,
                } => (id, offset, src),
                _ => panic!("unexpected capture {op:?}"),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        stores(original),
        vec![
            (ids[0], 0, a.clone()),
            (ids[0], 1, x.clone()),
            (ids[1], 0, y.clone()),
            (ids[1], 1, a3.clone())
        ]
    );
    let mut r = original.clone();
    r.frame.parameter_homes = Some(vec![
        MirParamBinding {
            param: ids[0],
            home: MirParamHome::ZeroPage(MirFixedZpSlot(0xa0)),
        },
        MirParamBinding {
            param: ids[1],
            home: MirParamHome::ZeroPage(MirFixedZpSlot(0xa2)),
        },
    ]);
    assert_eq!(
        stores(&r),
        vec![
            (ids[0], 0, a.clone()),
            (ids[0], 1, x.clone()),
            (ids[1], 0, y.clone())
        ]
    );
    r.frame.parameter_homes = Some(vec![
        MirParamBinding {
            param: ids[0],
            home: MirParamHome::Lanes(vec![ZeroPage(MirFixedZpSlot(0xa0)), Reg(MirReg::X)]),
        },
        MirParamBinding {
            param: ids[1],
            home: MirParamHome::Lanes(vec![Reg(MirReg::Y), ZeroPage(MirFixedZpSlot(0xa3))]),
        },
    ]);
    assert_eq!(stores(&r), vec![(ids[0], 0, a.clone())]);
    r.frame.params.truncate(1);
    r.frame.parameter_homes.as_mut().unwrap().truncate(1);
    r.frame.parameter_homes.as_mut().unwrap()[0].home =
        MirParamHome::Lanes(vec![Reg(MirReg::A), Reg(MirReg::X)]);
    assert!(stores(&r).is_empty());
    r.frame.parameter_homes = None;
    assert_eq!(stores(&r), vec![(ids[0], 1, x), (ids[0], 0, a)]);
    r = original.clone();
    r.frame.params[0].base = MirStorageBase::ParamAbiOnly(ids[0]);
    // Absent first word still occupies ABI lanes, and the legacy two-store
    // order captures the fixed byte before the remaining Y byte.
    assert_eq!(stores(&r), vec![(ids[1], 1, a3), (ids[1], 0, y)]);
}

#[test]
fn entry_facts_require_the_same_capture_stage_and_definition() {
    use MirCalleeEntryStage::{ActualEntry, PlannedCapture};
    let p = source("CARD out\nPROC Sink(CARD value) out=value RETURN\nPROC Main() Sink(1) RETURN");
    let id = routine(&p, "Sink").id;
    let before = MirCalleeEntryFact::analyze(&p, id, PlannedCapture).unwrap();
    assert!(before.is_current(&p, PlannedCapture));
    assert!(!before.is_current(&p, ActualEntry));
    let mut changed = p.clone();
    let r = changed.routines.iter_mut().find(|r| r.id == id).unwrap();
    let MirStorageBase::Param(param) = r.frame.params[0].base else {
        panic!()
    };
    r.frame.parameter_homes = Some(vec![MirParamBinding {
        param,
        home: MirParamHome::ZeroPage(MirFixedZpSlot(0xa0)),
    }]);
    assert!(!before.is_current(&changed, PlannedCapture));
    let mut expanded = p.clone();
    let r = expanded.routines.iter_mut().find(|r| r.id == id).unwrap();
    let mut ops = crate::mir6502::abi::direct_param_capture(r);
    ops.append(&mut r.blocks[0].ops);
    r.blocks[0].ops = ops;
    assert!(!before.is_current(&expanded, PlannedCapture));
    let actual = MirCalleeEntryFact::analyze(&expanded, id, ActualEntry).unwrap();
    assert!(actual.is_current(&expanded, ActualEntry));
    expanded
        .routines
        .iter_mut()
        .find(|r| r.id == id)
        .unwrap()
        .blocks[0]
        .ops
        .remove(0);
    assert!(!actual.is_current(&expanded, ActualEntry));
    let empty = source("PROC Sink() RETURN\nPROC Main() Sink() RETURN");
    let fact =
        MirCalleeEntryFact::analyze(&empty, routine(&empty, "Sink").id, PlannedCapture).unwrap();
    assert!(!fact.is_current(&empty, ActualEntry));
}

fn entry_probe() -> (MirProgram, usize, crate::nir::ParamId) {
    let mut p =
        source("CARD out\nPROC Sink(CARD value) out=value RETURN\nPROC Main() Sink(1) RETURN");
    let index = p.routines.iter().position(|r| r.name == "Sink").unwrap();
    let r = &mut p.routines[index];
    let MirStorageBase::Param(id) = r.frame.params[0].base else {
        panic!()
    };
    r.frame.parameter_homes = Some(vec![MirParamBinding {
        param: id,
        home: MirParamHome::ZeroPage(MirFixedZpSlot(0xa0)),
    }]);
    r.frame
        .fixed_zero_page
        .extend([MirFixedZpSlot(0xa0), MirFixedZpSlot(0xa1)]);
    (p, index, id)
}

#[test]
fn composed_capture_keeps_extra_registers_fixed_bytes_and_each_flag() {
    use MirCalleeEntryStage::PlannedCapture;
    let (base, index, _) = entry_probe();
    for flag in [
        MirFlagTest::CSet,
        MirFlagTest::ZSet,
        MirFlagTest::NSet,
        MirFlagTest::VSet,
    ] {
        let mut p = base.clone();
        let r = &mut p.routines[index];
        let mut exit = r.blocks[0].clone();
        exit.id = MirBlockId(991);
        exit.label = "exit".into();
        exit.ops.clear();
        exit.terminator = MirTerminator::Return;
        r.blocks[0].ops.clear();
        r.blocks[0].terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(flag.clone()),
            then_edge: MirEdge::plain(exit.id),
            else_edge: MirEdge::plain(exit.id),
        };
        r.blocks.push(exit);
        let inputs = MirCalleeEntryFact::analyze(&p, r_id(&p, index), PlannedCapture)
            .unwrap()
            .inputs;
        assert_eq!(
            (
                inputs.flags.c,
                inputs.flags.z,
                inputs.flags.n,
                inputs.flags.v
            ),
            (
                flag == MirFlagTest::CSet,
                flag == MirFlagTest::ZSet,
                flag == MirFlagTest::NSet,
                flag == MirFlagTest::VSet
            )
        );
    }
    let mut p = base;
    p.routines[index].blocks[0].ops = vec![
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x680)),
            src: MirValue::Def(MirDef::Reg(MirReg::Y)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xac))),
            width: MirWidth::Byte,
        },
    ];
    let fact = MirCalleeEntryFact::analyze(&p, r_id(&p, index), PlannedCapture).unwrap();
    assert!(fact.inputs.registers.a && fact.inputs.registers.x && fact.inputs.registers.y);
    assert_eq!(fact.inputs.fixed, BTreeSet::from([0xac]));
    p.routines[index].effects.reads.sp = true;
    assert!(
        MirCalleeEntryFact::analyze(&p, r_id(&p, index), PlannedCapture)
            .unwrap()
            .inputs
            .registers
            .sp
    );
}
fn r_id(p: &MirProgram, index: usize) -> RoutineId {
    p.routines[index].id
}

#[test]
fn actual_entry_does_not_invent_missing_or_conditional_capture() {
    for abi in [MirRoutineAbi::Action, MirRoutineAbi::ActionObservable] {
        check_missing_or_conditional_capture(abi);
    }
}

fn check_missing_or_conditional_capture(abi: MirRoutineAbi) {
    use MirCalleeEntryStage::{ActualEntry, PlannedCapture};
    let (mut p, index, id) = entry_probe();
    p.routines[index].abi = abi.clone();
    let capture_low = MirOp::Store {
        dst: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    };
    let read = MirOp::Load {
        dst: MirDef::VTemp(MirTempId(0)),
        src: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
        width: MirWidth::Word,
    };
    let r = &mut p.routines[index];
    r.blocks[0].ops = vec![capture_low.clone(), read.clone()];
    assert_eq!(
        MirCalleeEntryFact::analyze(&p, r_id(&p, index), ActualEntry)
            .unwrap()
            .inputs
            .fixed,
        BTreeSet::from([0xa1])
    );
    let planned = MirCalleeEntryFact::analyze(&p, r_id(&p, index), PlannedCapture);
    if abi == MirRoutineAbi::Action {
        assert!(planned.unwrap().inputs.fixed.is_empty());
    } else {
        assert!(planned.is_none());
    }
    let r = &mut p.routines[index];
    let mut overwrite = r.blocks[0].clone();
    overwrite.id = MirBlockId(990);
    overwrite.label = "overwrite".into();
    overwrite.ops = vec![capture_low];
    let mut join = overwrite.clone();
    join.id = MirBlockId(991);
    join.label = "join".into();
    join.ops = vec![read];
    join.terminator = MirTerminator::Return;
    overwrite.terminator = MirTerminator::Jump(MirEdge::plain(join.id));
    r.blocks[0].ops.clear();
    r.blocks[0].terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(MirFlagTest::CSet),
        then_edge: MirEdge::plain(overwrite.id),
        else_edge: MirEdge::plain(join.id),
    };
    r.blocks.extend([overwrite, join]);
    let inputs = MirCalleeEntryFact::analyze(&p, r_id(&p, index), ActualEntry)
        .unwrap()
        .inputs;
    assert_eq!(inputs.fixed, BTreeSet::from([0xa0, 0xa1]));
    assert!(inputs.flags.c);
}

#[test]
fn native_current_location_actual_entries_keep_abi_inputs_and_freshness() {
    use MirCalleeEntryStage::{ActualEntry, PlannedCapture};
    for (params, body, bytes) in [
        ("", "out=1", 0),
        ("BYTE a", "out=a", 1),
        ("CARD a", "out=a", 2),
        ("CARD POINTER p", "out=p^", 2),
        ("CARD a,b", "out=a+ b", 4),
    ] {
        let mut ordinary = None;
        for marker in ["", "=*"] {
            let p = source(&format!("CARD out\nPROC Sink{marker}({params}) {body} RETURN\nPROC Main() RETURN"));
            let p = super::super::materialize_program(p, &Default::default()).unwrap();
            let index = p.routines.iter().position(|r| r.name == "Sink").unwrap();
            let r = &p.routines[index];
            let fact = MirCalleeEntryFact::analyze(&p, r.id, ActualEntry).unwrap();
            assert!(fact.is_current(&p, ActualEntry));
            assert!(!fact.is_current(&p, PlannedCapture));
            assert_eq!(fact.inputs.fixed, if bytes == 4 { BTreeSet::from([0xa3]) } else { BTreeSet::new() });
            if marker.is_empty() {
                ordinary = Some(fact.inputs);
                continue;
            }
            assert_eq!(r.abi, MirRoutineAbi::ActionObservable);
            assert!(r.frame.parameter_homes.is_none());
            assert_eq!(Some(&fact.inputs), ordinary.as_ref());
            assert!(MirCalleeEntryFact::analyze(&p, r.id, PlannedCapture).is_none());
            let mut changed = p.clone();
            changed.routines[index].blocks[0].ops.insert(0, MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xac))),
                width: MirWidth::Byte,
            });
            assert!(!fact.is_current(&changed, ActualEntry));
            assert!(MirCalleeEntryFact::analyze(&changed, r.id, ActualEntry).unwrap().inputs.fixed.contains(&0xac));
            for abi in [MirRoutineAbi::ExternalInterface, MirRoutineAbi::ProgramEntryObservable] {
                let mut invalid = p.clone();
                invalid.routines[index].abi = abi;
                assert!(MirCalleeEntryFact::analyze(&invalid, r.id, ActualEntry).is_none());
            }
            let mut invalid = p.clone();
            invalid.routines[index].frame.direct_parameter_abi = None;
            assert!(MirCalleeEntryFact::analyze(&invalid, r.id, ActualEntry).is_none());
        }
    }
}

#[test]
fn incomplete_aggregate_helper_and_uncertified_capture_contracts_stay_unknown() {
    use MirCalleeEntryStage::PlannedCapture;
    let (base, index, _) = entry_probe();
    for variant in 0..5 {
        let mut p = base.clone();
        let r = &mut p.routines[index];
        match variant {
            0 => r.frame.direct_parameter_abi = None,
            1 => r.frame.params[0].storage = MirStorageClass::Record,
            2 => r.frame.direct_parameter_abi = Some(vec![MirDirectParamAbi::Scalar(crate::mir6502::ir::MirAbiIntegerWidth::Byte)]),
            3 => r.abi = MirRoutineAbi::ActionObservable,
            _ => r.blocks[0].ops.push(MirOp::Barrier {
                effects: MirEffects::default(),
            }),
        }
        assert!(
            MirCalleeEntryFact::analyze(&p, r_id(&p, index), PlannedCapture).is_none(),
            "{variant}"
        );
    }
    let p =
        source("CARD out\nPROC Sink(CARD a,b) out=a out=b RETURN\nPROC Main() Sink(1,2) RETURN");
    // Inline fourth-byte capture is memory sourced. It must not be mistaken for
    // a flag-preserving store until its expansion has a shared proof.
    assert!(MirCalleeEntryFact::analyze(&p, routine(&p, "Sink").id, PlannedCapture).is_none());
    let p = source(
        "CARD out\nPROC Sink(CARD a,b,c) out=a out=b out=c RETURN\nPROC Main() Sink(1,2,3) RETURN",
    );
    assert!(MirCalleeEntryFact::analyze(&p, routine(&p, "Sink").id, PlannedCapture).is_none());
}

#[test]
fn an_extra_expanded_entry_requirement_restores_the_complete_selected_artifact() {
    let text = "CARD first=$680,second=$682,seed=$684
        PROC Sum(CARD left,right) first=left+right second=right+257 RETURN
        PROC Forward(CARD left,right) Sum(left,right) RETURN
        PROC Main() Forward(seed,seed+17) DO OD RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
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
    let leaf = propose(&selected.input, &selected.executable.program, false, 0x3000).unwrap();
    let selected = try_proposal(
        &nir,
        selected,
        &resume,
        leaf,
        "parameter-homes",
        &config,
        0x3000,
        runtime,
        &trace,
    );
    let callee = routine(&selected.input, "Sum").id;
    let planned =
        MirCalleeEntryFact::analyze(&selected.input, callee, MirCalleeEntryStage::PlannedCapture)
            .unwrap();
    assert!(!planned.inputs.flags.any());
    let proposal =
        Arc::new(handoff::propose(&selected.input, &selected.executable.program).unwrap());
    let stage = "parameter-argument-handoff";
    let child = trace.child(stage, "batch-trial");
    let before = session.manifest()["work"]["materializations"]
        .as_u64()
        .unwrap();
    let mut expanded = super::super::preparation::resume_output(
        &nir,
        &resume,
        (*proposal).clone(),
        &config,
        0x3000,
        runtime,
        &child,
        None,
    )
    .unwrap()
    .program;

    // The ordinary handoff passed expanded verification. At Sum's entry,
    // ROR seed rotates the caller's carry into bit 7 of seed's low byte.
    let sum = expanded
        .routines
        .iter_mut()
        .find(|r| r.id == callee)
        .unwrap();
    sum.blocks[0].ops.insert(
        0,
        MirOp::UpdateMem {
            op: MirUpdateOp::Ror,
            mem: MirMem::Absolute(0x0684),
            width: MirWidth::Byte,
        },
    );
    let actual =
        MirCalleeEntryFact::analyze(&expanded, callee, MirCalleeEntryStage::ActualEntry).unwrap();
    assert!(actual.inputs.flags.c);
    let trial = super::super::finalize_output(&nir, expanded, 0x3000, runtime, &child);
    let diagnostics = trial
        .as_ref()
        .err()
        .expect("handoff accepted an incoming carry");
    assert_eq!(
        diagnostics.as_slice(),
        [MirDiagnostic::routine(
            "Forward",
            "invalid parameter handoff: extra-entry-flags-or-stack",
        )]
    );

    let expected_input = Arc::clone(&selected.input);
    let expected = selected.executable.clone();
    let selected = select_trial_result(selected, proposal, trial, stage, &child);
    assert!(Arc::ptr_eq(&selected.input, &expected_input));
    assert_eq!(selected.executable.output, expected.output);
    assert_eq!(selected.executable.program, expected.program);
    assert_eq!(selected.executable.memory, expected.memory);
    assert_eq!(selected.executable.costs, expected.costs);
    assert_eq!(selected.executable.trace_snapshot, expected.trace_snapshot);
    assert_eq!(session.manifest()["work"]["materializations"], before + 1);
    assert!(
        session
            .events()
            .iter()
            .any(|e| e.kind == "parameter-argument-handoff-result"
                && e.data["accepted"] == false
                && e.data["reason"] == "invalid-trial")
    );
}

#[test]
fn callee_facts_compare_entry_supplies_the_ordinary_call_contract() {
    let config = super::super::Mir6502Config {
        enable_small_leaf_inlining: false, ..super::super::Mir6502Config::optimized()
    };
    let p = super::super::materialize_program(
        source(include_str!("../../../fixtures/runtime/callee_facts_compare.act")), &config).unwrap();
    let r = routine(&p, "CompareHead");
    let entry = MirCalleeEntryFact::analyze(&p, r.id, MirCalleeEntryStage::ActualEntry).unwrap();
    assert!(!entry.inputs.flags.any(), "{:?}", entry.inputs);
    assert!(!entry.inputs.registers.sp);
    let mut calls = ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry);
    let call = routine(&p, "KeepCompare").blocks.iter().flat_map(|b| &b.ops)
        .find(|op| matches!(op, MirOp::Call { target: MirCallTarget::Routine(id), .. } if *id == r.id)).unwrap();
    assert!(calls.writes(call).is_ok());
}

#[test]
fn callee_facts_array_pointer_entry_keeps_storage_and_validates_both_stages() {
    let raw = source(include_str!("../../../fixtures/runtime/callee_facts_array_entry.act"));
    let config = super::super::Mir6502Config {
        enable_small_leaf_inlining: false, ..super::super::Mir6502Config::optimized()
    };
    let expanded = super::super::materialize_program(raw.clone(), &config).unwrap();
    for (p, stage) in [(&raw, MirCalleeEntryStage::PlannedCapture),
        (&expanded, MirCalleeEntryStage::ActualEntry)] {
        let r = routine(p, "Locate");
        assert_eq!(r.frame.params[0].storage, MirStorageClass::Array);
        assert_eq!(r.frame.direct_parameter_abi, Some(vec![MirDirectParamAbi::Pointer,
            MirDirectParamAbi::Scalar(crate::mir6502::ir::MirAbiIntegerWidth::Byte)]));
        let fact = MirCalleeEntryFact::analyze(p, r.id, stage).unwrap();
        assert!(fact.inputs.registers.a && fact.inputs.registers.x && fact.inputs.registers.y);
        assert!(!fact.inputs.registers.sp);
        if stage == MirCalleeEntryStage::ActualEntry {
            assert!(!fact.inputs.flags.any(), "{:?}", fact.inputs);
        }
        for invalid in 0..7 {
            let mut changed = p.clone();
            let r = changed.routines.iter_mut().find(|q| q.id == r.id).unwrap();
            match invalid {
                0 => r.frame.direct_parameter_abi.as_mut().unwrap()[0] = MirDirectParamAbi::Scalar(crate::mir6502::ir::MirAbiIntegerWidth::Word),
                1 => r.frame.params[0].storage = MirStorageClass::Record,
                2 => r.frame.params[0].storage_size = 1,
                3 => r.frame.params[0].scalar_width = Some(MirWidth::Byte),
                4 => { r.frame.direct_parameter_abi.as_mut().unwrap().pop(); },
                5 => r.frame.params[0].offset = 1,
                _ => r.frame.params[0].base = MirStorageBase::Local(crate::nir::LocalId(0)),
            }
            let id = r.id;
            assert!(!fact.is_current(&changed, stage));
            assert!(MirCalleeEntryFact::analyze(&changed, id, stage).is_none(), "{invalid}");
        }
    }
    let mut calls = ResidenceCalls::new(&expanded, MirCalleeEntryStage::ActualEntry);
    let call = routine(&expanded, "KeepLocate").blocks.iter().flat_map(|b| &b.ops)
        .find(|op| matches!(op, MirOp::Call { target: MirCallTarget::Routine(id), .. }
            if *id == routine(&expanded, "Locate").id)).unwrap();
    assert!(calls.writes(call).is_ok());
}
