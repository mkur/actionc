use super::*;
use crate::mir6502::ir::*;

fn load(value: u16) -> MirOp {
    MirOp::LoadImm { dst: MirDef::Reg(MirReg::A), value, width: MirWidth::Byte }
}
fn read(slot: u8) -> MirOp {
    MirOp::Load { dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))), width: MirWidth::Byte }
}
fn store(slot: u8) -> MirOp {
    MirOp::Store { dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
        src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte }
}
fn call(target: u32) -> MirOp {
    MirOp::Call { target: MirCallTarget::Routine(RoutineId(target)), args: vec![], result: None,
        additional_results: vec![], effects: MirEffects::default(),
        abi: MirCallAbi { params: vec![], result: None, additional_results: vec![],
            clobbers: crate::mir6502::abi::action_call_clobbers(), preserves: MirRegisterSet::default() } }
}
fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
    MirBlock { id: MirBlockId(id), label: format!("b{id}"), params: vec![], ops, terminator }
}
fn branch(then_block: u32, else_block: u32) -> MirTerminator {
    MirTerminator::Branch { cond: MirCond::FlagTest(MirFlagTest::ZSet),
        then_edge: MirEdge::plain(MirBlockId(then_block)), else_edge: MirEdge::plain(MirBlockId(else_block)) }
}
fn program() -> MirProgram {
    let bodies = [vec![load(7), store(0xa0)], vec![call(0), read(0xa0)],
        vec![call(0), read(0xa0)], vec![call(1), call(2), read(0xa0)]];
    MirProgram { statics: vec![], globals: vec![], machine_blocks: vec![], runtime_helpers: vec![],
        routines: bodies.into_iter().enumerate().map(|(id, ops)| MirRoutine {
            id: RoutineId(id as u32), name: format!("R{id}"), abi: MirRoutineAbi::Action,
            inline: Default::default(), direct_result_abi: Some(MirDirectResultAbi::Scalar(MirWidth::Byte)),
            scalar_signature: Some(MirScalarSignature {
                params: vec![], result: vec![MirWidth::Byte] }),
            frame: MirFrame { direct_parameter_abi: Some(vec![]), ..Default::default() },
            temps: vec![], blocks: vec![block(0, ops, MirTerminator::Return)], effects: MirEffects::default(),
        }).collect() }
}
fn fact(p: &MirProgram) -> Arc<MirCalleeEntryFact> {
    MirAcyclicEntryAnalysis::new(p).analyze(RoutineId(3)).unwrap()
}

#[test]
fn acyclic_entries_compose_diamonds_results_and_transitive_freshness() {
    for abi in [MirRoutineAbi::Action, MirRoutineAbi::ActionObservable] {
        let mut p = program();
        p.routines[1].abi = abi;
        let mut context = MirAcyclicEntryAnalysis::new(&p);
        let f = context.analyze(RoutineId(3)).unwrap();
        assert_eq!(context.entries.len(), 4);
        assert!(Arc::ptr_eq(&f, &context.analyze(RoutineId(3)).unwrap()));
        assert!(f.inputs.fixed.is_empty(), "{:?}", f.inputs);
        assert!(!f.inputs.flags.any() && !f.inputs.registers.sp);
        assert_eq!(f.defined_results, BTreeSet::from([0xa0]));
        assert!(f.is_current(&p, MirCalleeEntryStage::ActualEntry));
        assert!(!f.is_current(&p, MirCalleeEntryStage::PlannedCapture));
        assert!(MirCalleeEntryFact::analyze(&p, RoutineId(1), MirCalleeEntryStage::PlannedCapture).is_none());
        for mutation in 0..4 {
            let mut changed = p.clone();
            let leaf = &mut changed.routines[0];
            match mutation {
                0 => leaf.blocks[0].ops.insert(0, read(0xe8)),
                1 => leaf.frame.fixed_zero_page.push(MirFixedZpSlot(0xe8)),
                2 => leaf.abi = MirRoutineAbi::ActionObservable,
                _ => leaf.frame.direct_parameter_abi = None,
            }
            assert!(!f.is_current(&changed, MirCalleeEntryStage::ActualEntry));
        }
        // Removing the typed guarantee or making the result store conditional
        // must not let an incidental/may-write erase the outer dependency.
        let mut untyped = p.clone();
        untyped.routines[0].scalar_signature = None;
        assert!(fact(&untyped).inputs.fixed.contains(&0xa0));
        let mut conditional = p.clone();
        conditional.routines[0].blocks = vec![
            block(0, vec![load(7)], branch(1, 2)),
            block(1, vec![store(0xa0)], MirTerminator::Return),
            block(2, vec![], MirTerminator::Return),
        ];
        assert!(fact(&conditional).inputs.fixed.contains(&0xa0));
    }
}

#[test]
fn acyclic_entry_requirements_follow_setup_on_every_path() {
    let mut p = program();
    let adc = |carry_in| MirOp::Binary { op: MirBinaryOp::Add, dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)), right: MirValue::ConstU8(0),
        width: MirWidth::Byte, carry_in: Some(carry_in), carry_out: MirCarryOut::Produce };
    p.routines[0].blocks[0].ops = vec![read(0xe8), adc(MirCarryIn::FromPrevious), store(0xa0)];
    let f = fact(&p);
    assert!(f.inputs.fixed.contains(&0xe8) && f.inputs.flags.c);
    // The same leaf is reached through both arms of the diamond. Initialize
    // the requirement in each middle routine, not just one cached call site.
    for index in [1, 2] {
        p.routines[index].blocks[0].ops.splice(0..0, [load(1), store(0xe8), adc(MirCarryIn::Clear)]);
    }
    assert!(!fact(&p).inputs.flags.c);
    assert!(!fact(&p).inputs.fixed.contains(&0xe8));
    p.routines[1].blocks = vec![
        block(0, vec![load(1)], branch(1, 2)),
        block(1, vec![store(0xe8), adc(MirCarryIn::Clear)], MirTerminator::Jump(MirEdge::plain(MirBlockId(2)))),
        block(2, vec![call(0), read(0xa0)], MirTerminator::Return),
    ];
    assert!(fact(&p).inputs.flags.c && fact(&p).inputs.fixed.contains(&0xe8));
    // ABI clobbers on a preceding call are may-writes, not definitions of C.
    p.routines[3].blocks[0].ops.insert(0, call(2));
    assert!(fact(&p).inputs.flags.c);
}

#[test]
fn acyclic_entry_rejects_cycles_unsupported_descendants_and_each_bad_site() {
    for mutation in 0..5 {
        let mut p = program();
        match mutation {
            0 => p.routines[0].blocks[0].ops.insert(0, call(3)),
            1 => p.routines[0].effects.may_call_os = true,
            2 => p.routines[0].effects.opaque = true,
            3 => p.routines[0].frame.direct_parameter_abi = None,
            _ => {
                let mut invalid = call(0);
                let MirOp::Call { args, .. } = &mut invalid else { unreachable!() };
                args.push(MirCallArg { value: MirValue::ConstU8(1), width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A) });
                p.routines[1].blocks[0].ops.push(invalid);
            }
        }
        let mut analysis = MirAcyclicEntryAnalysis::new(&p);
        assert!(analysis.analyze(RoutineId(3)).is_none(), "{mutation}");
        assert!(analysis.analyze(RoutineId(3)).is_none());
    }
}
