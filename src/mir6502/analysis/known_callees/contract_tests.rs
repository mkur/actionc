use super::*;

fn load(reg: MirReg, value: u16) -> MirOp {
    MirOp::LoadImm { dst: MirDef::Reg(reg), value, width: MirWidth::Byte }
}
fn read(address: u16) -> MirOp {
    MirOp::Load { dst: MirDef::Reg(MirReg::A), src: MirAddr::Direct(MirMem::Absolute(address)), width: MirWidth::Byte }
}
fn store(address: u16) -> MirOp {
    MirOp::Store { dst: MirAddr::Direct(MirMem::Absolute(address)), src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte }
}
fn call(target: u32) -> MirOp {
    MirOp::Call { target: MirCallTarget::Routine(RoutineId(target)), args: vec![], result: None, additional_results: vec![],
        abi: MirCallAbi { library: None, params: vec![], result: None, additional_results: vec![], clobbers: Default::default(), preserves: Default::default() }, effects: Default::default() }
}
fn compatibility(before: &MirProgram, after: &MirProgram) -> Result<(), MirCalleeContractChange> {
    MirKnownCalleeSummaries::analyze_for_rewriting(before)
        .contracts_preserved_by(after, &MirKnownCalleeSummaries::analyze_for_rewriting(after))
}

#[test]
fn callee_contract_compatibility_accepts_narrower_effects_and_rejects_unknown_or_added_effects() {
    let before = program(vec![routine(0, vec![read(0x600), store(0x700)], MirTerminator::Return)], vec![]);
    let mut after = before.clone();
    after.routines[0].blocks[0].ops.clear();
    assert!(compatibility(&before, &after).is_ok());
    assert_eq!(compatibility(&after, &before).unwrap_err().fact, "expanded-reads");
    after.routines[0].blocks[0].ops = vec![read(0x600), store(0x701)];
    assert_eq!(compatibility(&before, &after).unwrap_err().fact, "expanded-writes");
    after.routines[0].blocks[0].ops = vec![MirOp::Barrier { effects: Default::default() }];
    assert_eq!(compatibility(&before, &after).unwrap_err().fact, "unknown-reads");
    assert!(compatibility(&after, &before).is_ok());
}

#[test]
fn callee_contract_compatibility_checks_descendant_exit_and_register_guarantees() {
    let before = program(vec![routine(0, vec![load(MirReg::A, 7)], MirTerminator::Return),
        routine(1, vec![call(0)], MirTerminator::Return), routine(2, vec![call(1)], MirTerminator::Return)], vec![]);
    let mut after = before.clone();
    after.routines[0].blocks[0].ops[0] = load(MirReg::A, 8);
    assert_eq!(compatibility(&before, &after).unwrap_err().fact, "accumulator");
    let old = MirKnownCalleeSummaries::analyze_for_rewriting(&before);
    let new = MirKnownCalleeSummaries::analyze_for_rewriting(&after);
    assert_ne!(old.get(RoutineId(2)).unwrap().accumulator(), new.get(RoutineId(2)).unwrap().accumulator());
    after = before.clone();
    after.routines[0].blocks[0].ops.push(load(MirReg::X, 1));
    assert_eq!(compatibility(&before, &after).unwrap_err().fact, "register-preservation");
}

#[test]
fn callee_contract_compatibility_keeps_publication_policy_and_machine_flag_provenance() {
    let before = program(vec![routine(0, vec![load(MirReg::A, 7)], MirTerminator::Return)], vec![]);
    let mutable = MirKnownCalleeSummaries::analyze(&before);
    let rewriting = MirKnownCalleeSummaries::analyze_for_rewriting(&before);
    assert_eq!(mutable.contracts_preserved_by(&before, &rewriting).unwrap_err().fact, "publication-policy");
    let mut after = before.clone();
    after.routines[0].blocks[0].ops[0] = load(MirReg::A, 8);
    assert_eq!(mutable.contracts_preserved_by(&after, &MirKnownCalleeSummaries::analyze(&after)).unwrap_err().fact, "exit-flags");
    // No caller observes incidental A here, and rewrite mode publishes no native Z/N.
    assert!(compatibility(&before, &after).is_ok());
}

#[test]
fn callee_contract_compatibility_rejects_backing_change_with_identical_instructions() {
    use crate::nir::SymbolId;
    let mut before = program(vec![routine(0, vec![MirOp::Store {
        dst: MirAddr::Direct(MirMem::Global { id: SymbolId(0), offset: 0 }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte }], MirTerminator::Return)], vec![]);
    before.globals.push(MirGlobal { id: SymbolId(0), name: "state".into(), kind: "byte".into(),
        width: Some(MirWidth::Byte), storage_size: 1, backing: MirGlobalBacking::Absolute(0x600), init: None });
    let mut after = before.clone();
    after.globals[0].backing = MirGlobalBacking::Absolute(0xe4);
    assert_eq!(before.routines, after.routines);
    assert_eq!(compatibility(&before, &after).unwrap_err().fact, "fixed-backing");
}

#[test]
fn callee_contract_compatibility_retains_published_entry_flags() {
    let mut before = program(vec![routine(0, vec![load(MirReg::A, 7)], MirTerminator::Return)], vec![]);
    before.routines[0].scalar_signature = Some(crate::mir6502::ir::MirScalarSignature { params: vec![], result: vec![] });
    let mut after = before.clone();
    after.routines[0].effects.reads.flags = true;
    assert_eq!(compatibility(&before, &after).unwrap_err().fact, "entry-flags");
    let old = MirKnownCalleeSummaries::analyze_for_rewriting(&before)
        .with_precise_entry_flags(&before, &BTreeSet::from([RoutineId(0)]));
    let new = MirKnownCalleeSummaries::analyze_for_rewriting(&after);
    assert!(old.contracts_preserved_by(&after, &new).is_err());
}

#[test]
fn terminal_native_writes_remain_visible_to_rewrites_and_returning_paths() {
    let source = "BYTE fault=[0] PROC Stop(BYTE code) fault=code DO OD RETURN PROC Main() Stop(1) RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    let sem = crate::semantic::ir::lower_program(&ast, &model);
    let mir = crate::mir6502::lower_program(&crate::nir::lower_program(&sem)).unwrap();
    let mut mir = crate::mir6502::materialize_program(mir, &Default::default()).unwrap();
    let id = mir.routines[0].id;
    let empty = |summaries: MirKnownCalleeSummaries| matches!(summaries.get(id).unwrap().writes(), MirKnownMemoryWrites::Exact(bytes) if bytes.is_empty());
    assert!(empty(MirKnownCalleeSummaries::analyze_for_residence(&mir)));
    assert!(!empty(MirKnownCalleeSummaries::analyze_for_rewriting(&mir)));
    // End the reachable entry block; source RETURN after DO OD is unreachable.
    mir.routines[0].blocks[0].terminator = MirTerminator::Return;
    assert!(!empty(MirKnownCalleeSummaries::analyze_for_residence(&mir)));
}
