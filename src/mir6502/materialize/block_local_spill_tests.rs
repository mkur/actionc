//! Exercise placement after several values have been colored into one spill.
use super::*;

fn cell() -> MirMem {
    MirMem::Spill {
        id: MirSpillId(0),
        offset: 0,
    }
}

fn store() -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(cell()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }
}

fn load() -> MirOp {
    MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(cell()),
        width: MirWidth::Byte,
    }
}

fn call() -> MirOp {
    MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(1)),
        abi: MirCallAbi {
            params: vec![],
            result: None,
            additional_results: vec![],
            library: None,
            clobbers: MirRegisterSet::default(),
            preserves: MirRegisterSet::default(),
        },
        args: vec![],
        result: None,
        additional_results: vec![],
        effects: MirEffects::default(),
    }
}

fn placed(ops: Vec<MirOp>, free: usize) -> bool {
    let mut program = empty_test_program();
    let r = &mut program.routines[0];
    r.frame.spills = vec![MirSpillId(0)];
    r.frame.fixed_zero_page = (0xe0..=0xef).skip(free).map(MirFixedZpSlot).collect();
    r.blocks = vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".into(),
        params: vec![],
        ops,
        terminator: MirTerminator::Return,
    }];
    let homes = spills::lower_block_local_byte_spills_to_zero_page(&mut program);
    let selected = homes
        .get(&RoutineId(0))
        .is_some_and(|m| m.contains_key(&MirSpillId(0)));
    assert_eq!(program.routines[0].frame.spills.is_empty(), selected);
    selected
}

#[test]
fn block_local_spill_redefinitions_can_cross_dead_call_sites() {
    let helper = MirOp::RuntimeHelper {
        helper: MirRuntimeHelper::Mul,
        args: vec![],
        result: None,
        additional_results: vec![],
        effects: MirEffects::default(),
    };
    let capture = MirOp::SArgsCapture {
        first_param: ParamId(0),
        byte_count: 1,
        effects: MirEffects::default(),
    };
    for barrier in [call(), helper, capture] {
        let safe = vec![store(), load(), barrier.clone(), store(), load()];
        assert!(placed(safe.clone(), 1));
        assert!(!placed(safe, 0));
        assert!(!placed(
            vec![store(), load(), barrier.clone(), load(), store(), load()],
            1
        ));
        assert!(!placed(
            vec![
                store(),
                load(),
                barrier.clone(),
                store(),
                barrier.clone(),
                load()
            ],
            1
        ));
        let update = MirOp::UpdateMem {
            op: MirUpdateOp::Inc,
            mem: cell(),
            width: MirWidth::Byte,
        };
        assert!(!placed(vec![store(), load(), barrier, update, load()], 1));
    }
}

#[test]
fn block_local_spill_call_inputs_and_storage_extent_remain_protected() {
    let mut consumes = call();
    let MirOp::Call { args, .. } = &mut consumes else {
        unreachable!()
    };
    args.push(MirCallArg {
        value: MirValue::PointerCell(cell()),
        width: MirWidth::Byte,
        home: MirArgHome::Reg(MirReg::A),
    });
    assert!(placed(vec![store(), consumes.clone()], 1)); // Existing final-use argument.
    assert!(!placed(vec![store(), consumes, store(), load()], 1));

    let mut indirect = call();
    let MirOp::Call { target, .. } = &mut indirect else {
        unreachable!()
    };
    *target = MirCallTarget::Indirect {
        target: MirValue::Def(MirDef::VTemp(MirTempId(0))),
        width: MirWidth::Word,
    };
    assert!(!placed(vec![store(), indirect, store(), load()], 1));

    let mixed = MirOp::Store {
        dst: MirAddr::Direct(cell()),
        src: MirValue::Def(MirDef::VTemp(MirTempId(0))),
        width: MirWidth::Byte,
    };
    assert!(!placed(vec![store(), mixed, call(), store(), load()], 1));
    let indexed = MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::AbsoluteIndexedX { base: cell() },
        width: MirWidth::Byte,
    };
    assert!(!placed(vec![store(), indexed, call(), store(), load()], 1));

    let wide = MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(cell()),
        width: MirWidth::Word,
    };
    assert!(!placed(vec![store(), wide, call(), store(), load()], 1));
    let exposed = MirOp::Move {
        dst: MirDef::Reg(MirReg::X),
        src: MirValue::StorageAddrByte {
            mem: cell(),
            byte: 0,
        },
        width: MirWidth::Byte,
    };
    assert!(!placed(vec![store(), exposed, call(), store(), load()], 1));
}
