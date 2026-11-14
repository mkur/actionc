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
