use super::*;
use crate::mir6502::ir::MirZpSlot;

fn layout() -> MaterializeLayout {
    layout_at(0x3000)
}
fn layout_at(origin: u16) -> MaterializeLayout {
    MaterializeLayout::new(
        &MirProgram {
            statics: vec![],
            globals: vec![],
            routines: vec![],
            machine_blocks: vec![],
            runtime_helpers: vec![],
        },
        origin,
    )
}
fn ops(op: MirBinaryOp, moves: u32) -> Vec<MirOp> {
    let mut ops = vec![
        MirOp::LoadImm {
            dst: MirDef::VTemp(MirTempId(0)),
            value: 0x01FF,
            width: MirWidth::Word,
        },
        MirOp::LoadImm {
            dst: MirDef::VTemp(MirTempId(1)),
            value: 2,
            width: MirWidth::Word,
        },
        MirOp::Binary {
            op,
            dst: MirDef::VTemp(MirTempId(2)),
            left: MirValue::Def(MirDef::VTemp(MirTempId(0))),
            right: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Word,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
    ];
    for i in 0..moves {
        ops.push(MirOp::Move {
            dst: MirDef::VTemp(MirTempId(3 + i)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(2 + i))),
            width: MirWidth::Word,
        });
    }
    ops.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0xD8)),
        src: MirValue::Def(MirDef::VTemp(MirTempId(2 + moves))),
        width: MirWidth::Word,
    });
    ops
}
#[test]
fn word_move_store_preserves_arithmetic_through_one_or_more_moves() {
    for arithmetic in [MirBinaryOp::Add, MirBinaryOp::Sub] {
        for moves in [1, 2, 4] {
            let ops = ops(arithmetic, moves);
            let plan =
                collect_store_expr_plan(&ops, 0, &MirTerminator::Return, &layout(), None, true)
                    .unwrap();
            assert_eq!(plan.consumed, ops.len());
            assert!(
                matches!(plan.expr,StoreExpr::Binary{op,width:MirWidth::Word,..} if op==arithmetic)
            );
            let mut out = vec![];
            materialize_store_expr_plan(plan, &Mir6502Config::optimized(), &layout(), &mut out);
            assert_eq!(
                out.iter()
                    .filter(|op| matches!(op,MirOp::Binary{op,..} if *op==arithmetic))
                    .count(),
                2
            );
            assert!(out.iter().all(|op| {
                crate::mir6502::analysis::effects::classify_op(op)
                    .logical
                    .temp_defs
                    .is_empty()
            }));
        }
    }
}
#[test]
fn word_move_store_rejects_width_changes_duplicate_uses_and_ordering_barriers() {
    let original = ops(MirBinaryOp::Add, 1);
    let mut narrowed = original.clone();
    if let MirOp::Move { width, .. } = &mut narrowed[3] {
        *width = MirWidth::Byte;
    }
    let mut duplicated = original.clone();
    duplicated.insert(
        3,
        MirOp::Move {
            dst: MirDef::VTemp(MirTempId(9)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
            width: MirWidth::Word,
        },
    );
    let mut barrier = original.clone();
    barrier.insert(
        3,
        MirOp::Barrier {
            effects: Default::default(),
        },
    );
    for rejected in [narrowed, duplicated, barrier] {
        assert!(
            collect_store_expr_plan(&rejected, 0, &MirTerminator::Return, &layout(), None, true)
                .is_none()
        );
    }
    let mut live = original;
    live.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0xDA)),
        src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
        width: MirWidth::Word,
    });
    assert!(
        collect_store_expr_plan(&live, 0, &MirTerminator::Return, &layout(), None, true).is_none()
    );
}

fn frame_source() -> MirRoutine {
    use crate::mir6502::ir::*;
    MirRoutine {
        id: RoutineId(0),
        name: "WordMove".into(),
        abi: MirRoutineAbi::Action,
        inline: Default::default(),
        scalar_signature: None,
        effects: Default::default(),
        temps: (0..4).map(|id| MirTemp { id: MirTempId(id) }).collect(),
        frame: MirFrame {
            locals: vec![MirStorageSlot {
                id: MirStorageId(0),
                name: None,
                purpose: MirStoragePurpose::Storage,
                storage: MirStorageClass::Scalar,
                storage_size: 2,
                scalar_width: Some(MirWidth::Word),
                base: MirStorageBase::Local(crate::nir::LocalId(0)),
                offset: 0,
                mutable: true,
                init: None,
            }],
            ..Default::default()
        },
        blocks: vec![MirBlock {
            id: MirBlockId(0),
            label: "entry".into(),
            params: vec![],
            ops: ops(MirBinaryOp::Add, 1),
            terminator: MirTerminator::Return,
        }],
    }
}
#[test]
fn word_move_store_uses_an_origin_bound_for_declared_scalar_roots() {
    let mut r = frame_source();
    r.blocks[0].ops[0] = MirOp::Load {
        dst: MirDef::VTemp(MirTempId(0)),
        src: MirAddr::Direct(MirMem::Local {
            id: crate::nir::LocalId(0),
            offset: 0,
        }),
        width: MirWidth::Word,
    };
    let plan = |r: &MirRoutine, origin| {
        let p = MirProgram {
            routines: vec![r.clone()],
            statics: vec![],
            globals: vec![],
            machine_blocks: vec![],
            runtime_helpers: vec![],
        };
        collect_store_expr_plan(
            &r.blocks[0].ops,
            0,
            &MirTerminator::Return,
            &MaterializeLayout::new(&p, origin),
            Some(r),
            true,
        )
    };
    assert!(plan(&r, 0x3000).is_some());
    assert!(plan(&r, 0x0080).is_none());
    r.frame.locals[0].init = Some(crate::mir6502::ir::MirStorageInit::ZeroFill {
        bytes: 2,
        mutable: true,
        section: "data".into(),
    });
    assert!(plan(&r, 0x3000).is_none());
    r.frame.locals[0].init = None;
    r.frame.locals[0].base = MirStorageBase::LocalAlias {
        id: crate::nir::LocalId(0),
        target: crate::nir::LocalId(1),
    };
    assert!(plan(&r, 0x3000).is_none());
}
#[test]
fn word_move_store_checks_fixed_ranges_and_declines_unknown_placements() {
    let mut input = ops(MirBinaryOp::Add, 1);
    for (i, address) in [(0, 0xD0), (1, 0xD2)] {
        input[i] = MirOp::Load {
            dst: MirDef::VTemp(MirTempId(i as u32)),
            src: MirAddr::Direct(MirMem::Absolute(address)),
            width: MirWidth::Word,
        };
    }
    assert!(
        collect_store_expr_plan(&input, 0, &MirTerminator::Return, &layout(), None, true).is_some()
    );
    // Even fixed RAM ranges must not become early writes into generated code.
    assert!(
        collect_store_expr_plan(
            &input,
            0,
            &MirTerminator::Return,
            &layout_at(0x80),
            None,
            true,
        )
        .is_none()
    );
    for destination in [
        MirMem::Absolute(0xD1),
        MirMem::Absolute(0xD3),
        MirMem::Absolute(0xD0),
        MirMem::Absolute(0xFF),
        MirMem::ZeroPage(MirZpSlot(0)),
        MirMem::Local {
            id: crate::nir::LocalId(0),
            offset: 0,
        },
    ] {
        if let MirOp::Store { dst, .. } = input.last_mut().unwrap() {
            *dst = MirAddr::Direct(destination);
        }
        assert!(
            collect_store_expr_plan(&input, 0, &MirTerminator::Return, &layout(), None, true)
                .is_none()
        );
    }
    if let MirOp::Store { dst, .. } = input.last_mut().unwrap() {
        *dst = MirAddr::Direct(MirMem::Absolute(0xD8));
    }
    if let MirOp::Load { src, .. } = &mut input[0] {
        *src = MirAddr::Direct(MirMem::Absolute(0xD20A));
    }
    assert!(
        collect_store_expr_plan(&input, 0, &MirTerminator::Return, &layout(), None, true).is_none()
    );
}
#[test]
fn word_move_store_driver_preserves_escaped_results_and_reaches_a_fixed_point() {
    use crate::mir6502::rewrite::driver::MirPreHomeRewriteDriver;
    let mut r = frame_source();
    let run = |r: &mut MirRoutine| {
        MirPreHomeRewriteDriver::default()
            .run_fixed_point_by_key(
                r,
                |r, ctx| {
                    crate::mir6502::rewrite::pilots::discover_store_consumers(
                        r,
                        ctx,
                        &Mir6502Config::optimized(),
                        &layout(),
                    )
                },
                crate::mir6502::rewrite::pilots::store_consumer_rank,
            )
            .unwrap()
    };
    assert!(run(&mut r).applied > 0);
    let stable = r.clone();
    assert_eq!(run(&mut r).applied, 0);
    assert_eq!(r, stable);
    let mut r = frame_source();
    r.blocks[0].ops.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::Absolute(0xDA)),
        src: MirValue::Def(MirDef::VTemp(MirTempId(2))),
        width: MirWidth::Word,
    });
    let before = r.clone();
    assert_eq!(run(&mut r).applied, 0);
    assert_eq!(r, before);
}
