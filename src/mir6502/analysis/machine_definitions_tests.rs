//! Shared-definition regressions: compare every flag consumer on the same body.
use super::*;
use crate::mir6502::{analysis::{
    argument_x_forwarding::tests::baseline, cfg::MirCfg,
    entry_flags::MirKnownCalleeEntryFlags, known_callees::MirKnownCalleeSummaries,
    machine_liveness::MirMachineLiveness,
}, ir::*};

fn compare(dst: MirCondDest, op: MirCompareOp, reg: MirReg, right: u8) -> MirOp {
    MirOp::Compare { dst, op, left: MirValue::Def(MirDef::Reg(reg)),
        right: MirValue::ConstU8(right), width: MirWidth::Byte, signed: false }
}
fn flags(bits: u8) -> MirFlagSet {
    MirFlagSet { c: bits & 1 != 0, z: bits & 2 != 0, n: bits & 4 != 0, v: bits & 8 != 0 }
}
fn routine(ops: Vec<MirOp>) -> MirRoutine {
    let mut r = baseline().routines.into_iter().find(|r| r.name == "ObserveBytes").unwrap();
    r.blocks.truncate(1);
    r.blocks[0].ops = ops;
    r.blocks[0].terminator = MirTerminator::Return;
    r
}

#[test]
fn machine_definitions_agree_across_precise_legacy_and_ordinary_entry_queries() {
    for reg in [MirReg::A, MirReg::X, MirReg::Y] {
        for op in [MirCompareOp::Eq, MirCompareOp::Ne, MirCompareOp::Lt, MirCompareOp::Ge] {
            for rhs in [0, 40, 255] {
                let compare = compare(MirCondDest::Flags, op, reg, rhs);
                let definitions = if reg == MirReg::A && rhs == 0
                    && matches!(op, MirCompareOp::Eq | MirCompareOp::Ne) { 6 } else { 7 };
                assert_eq!(classify_op(&compare).machine.flag_writes, flags(definitions));
                // Identical explicit observation policy for all three queries.
                let r = routine(vec![compare, MirOp::Barrier { effects: MirEffects {
                    reads: MirRegisterSet { flags: true, ..Default::default() }, ..Default::default()
                }}]);
                let cfg = MirCfg::from_routine(&r).unwrap();
                let expected = flags(15 ^ definitions);
                assert_eq!(MirMachineLiveness::analyze(&r, &cfg).live_in(cfg.entry().unwrap()).unwrap().flags, expected);
                let entry = MirKnownCalleeEntryFlags::analyze(&r);
                assert_eq!(entry, MirKnownCalleeEntryFlags::Known { may_observe: expected });
                let id = r.id;
                let mut p = baseline();
                *p.routines.iter_mut().find(|other| other.id == id).unwrap() = r;
                assert_eq!(MirKnownCalleeSummaries::analyze_for_rewriting(&p).scalar_entry_ignores_flags(id),
                    entry.unobserved(MirFlagSet::all()));
            }
        }
    }
}

#[test]
fn machine_definitions_keep_folded_and_unselected_comparisons_conservative() {
    let folded = compare(MirCondDest::Temp(MirTempId(0)), MirCompareOp::Le, MirReg::A, 255);
    let effects = classify_op(&folded);
    assert_eq!(effects.machine.flag_writes, flags(6));
    assert!(effects.machine.register_writes.a);
    assert!(effects.machine.possible_flag_writes().c);
    for width in [MirWidth::Word] {
        let mut wide = compare(MirCondDest::Flags, MirCompareOp::Ge, MirReg::A, 40);
        if let MirOp::Compare { width: w, .. } = &mut wide { *w = width; }
        let effects = classify_op(&wide);
        assert!(!effects.machine.flag_writes.any());
        assert!(effects.machine.possible_flag_writes().c);
    }
    // A selected word comparison's CMP low / SBC high chain reads internal C,
    // so its entry C is dead, while removing CMP exposes that dependency.
    let low = compare(MirCondDest::Flags, MirCompareOp::Ge, MirReg::A, 40);
    let high = MirOp::Binary { op: MirBinaryOp::Sub, dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)), right: MirValue::ConstU8(1),
        width: MirWidth::Byte, carry_in: Some(MirCarryIn::FromPrevious), carry_out: MirCarryOut::Ignore };
    for (ops, incoming_c) in [(vec![low, high.clone()], false), (vec![high], true)] {
        let r = routine(ops);
        let cfg = MirCfg::from_routine(&r).unwrap();
        assert_eq!(MirMachineLiveness::entry_flag_inputs(&r, &cfg).unwrap().c, incoming_c);
    }
}

#[test]
fn machine_definitions_do_not_hide_zero_work_or_finite_clobbers() {
    let a = MirDef::Reg(MirReg::A);
    let identity = MirOp::Move { dst: a.clone(), src: MirValue::Def(a.clone()), width: MirWidth::Byte };
    let zero_shift = MirOp::Binary { op: MirBinaryOp::Lsh, dst: a.clone(),
        left: MirValue::Def(a), right: MirValue::ConstU8(0), width: MirWidth::Byte,
        carry_in: Some(MirCarryIn::FromPrevious), carry_out: MirCarryOut::Ignore };
    let clobber = MirOp::MachineBlock { id: MirMachineBlockId(0), effects: MirEffects {
        clobbers: MirRegisterSet { a: true, flags: true, ..Default::default() }, ..Default::default()
    }};
    for op in [identity, zero_shift, clobber] {
        let effects = classify_op(&op);
        assert!(!effects.machine.register_writes.a && !effects.machine.flag_writes.any());
        let r = routine(vec![op, MirOp::Barrier { effects: MirEffects {
            reads: MirRegisterSet { a: true, flags: true, ..Default::default() }, ..Default::default()
        }}]);
        let cfg = MirCfg::from_routine(&r).unwrap();
        let incoming = MirMachineLiveness::analyze(&r, &cfg).live_in(cfg.entry().unwrap()).unwrap();
        assert!(incoming.registers.a);
        assert_eq!(incoming.flags, MirFlagSet::all());
    }
    // With nonzero count the selected rotate both reads and writes C.
    let rotate = MirOp::Binary { op: MirBinaryOp::Lsh, dst: MirDef::Reg(MirReg::A),
        left: MirValue::Def(MirDef::Reg(MirReg::A)), right: MirValue::ConstU8(1),
        width: MirWidth::Byte, carry_in: Some(MirCarryIn::FromPrevious), carry_out: MirCarryOut::Ignore };
    let effects = classify_op(&rotate);
    assert!(effects.machine.flag_reads.c && effects.machine.flag_writes.c);
}

#[test]
fn local_flag_queries_use_individual_definitions_and_reads_before_writes() {
    let term = MirTerminator::Branch { cond: MirCond::FlagTest(MirFlagTest::CSet),
        then_edge: MirEdge::plain(MirBlockId(0)), else_edge: MirEdge::plain(MirBlockId(0)) };
    let load = MirOp::LoadImm { dst: MirDef::Reg(MirReg::A), value: 0, width: MirWidth::Byte };
    assert!(MirMachineLiveness::flags_observed_before_tail(&[load.clone()], &term).c);
    for (op, needs_c) in [(MirCompareOp::Eq, true), (MirCompareOp::Ge, false)] {
        let cmp = compare(MirCondDest::Flags, op, MirReg::A, 0);
        assert_eq!(MirMachineLiveness::flags_observed_before_tail(&[load.clone(), cmp], &term).c, needs_c);
    }
}
