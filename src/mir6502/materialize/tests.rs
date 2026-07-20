use super::*;
use crate::mir6502::ir::{
    MirBlock, MirCallResult, MirEdge, MirRegisterSet, MirStatic, MirStorageBacking, MirStorageInit,
    MirTemp,
};
use crate::mir6502::passes::MirPeepholeReportMode;
use crate::mir6502::{
    MirFrame, MirRoutine, MirRoutineAbi, MirStorageBase, MirStorageId, MirStorageSlot,
};
use crate::nir::{LocalId, ParamId, SymbolId};

#[test]
fn indirect_word_load_fold_keeps_spills_used_by_second_call_arg_home() {
    let low = MirSpillId(18);
    let high = MirSpillId(19);
    let spill_mem = |id| MirMem::Spill { id, offset: 0 };
    let load_indirect = |offset| MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset,
    };
    let store_a = |mem| MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    };
    let load_spill = |reg, id| MirOp::Load {
        dst: MirDef::Reg(reg),
        src: MirAddr::Direct(spill_mem(id)),
        width: MirWidth::Byte,
    };
    let ops = vec![
        load_indirect(0),
        store_a(spill_mem(low)),
        load_indirect(1),
        store_a(spill_mem(high)),
        load_spill(MirReg::A, low),
        store_a(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
        load_spill(MirReg::A, high),
        store_a(MirMem::FixedZeroPage(MirFixedZpSlot(0xA1))),
        load_spill(MirReg::A, low),
        load_spill(MirReg::X, high),
    ];

    let folded = fold_indirect_load_spill_consumers(ops, &MirTempLiveSet::default());

    assert!(folded.iter().any(|op| matches!(
        op,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill { id, offset: 0 }),
            ..
        } if *id == low
    )));
    assert!(folded.iter().any(|op| matches!(
        op,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill { id, offset: 0 }),
            ..
        } if *id == high
    )));
    assert_eq!(folded.len(), 8);
}

#[test]
fn indirect_word_load_fold_keeps_lane_live_in_successor() {
    let low = MirSpillId(18);
    let high = MirSpillId(19);
    let spill_mem = |id| MirMem::Spill { id, offset: 0 };
    let load_indirect = |offset| MirOp::LoadIndirect {
        consumer: DEFAULT_POINTER_PAIR,
        dst: MirDef::Reg(MirReg::A),
        offset,
    };
    let store_a = |mem| MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    };
    let load_spill = |id| MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(spill_mem(id)),
        width: MirWidth::Byte,
    };
    let ops = vec![
        load_indirect(0),
        store_a(spill_mem(low)),
        load_indirect(1),
        store_a(spill_mem(high)),
        load_spill(low),
        store_a(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
        load_spill(high),
        store_a(MirMem::FixedZeroPage(MirFixedZpSlot(0xA1))),
    ];
    let live_out = MirTempLiveSet::with_exact_lane(MirTempId(9), 0);

    let folded = fold_indirect_load_spill_consumers(ops, &live_out);

    assert!(folded.iter().any(|op| matches!(
        op,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill { id, offset: 0 }),
            ..
        } if *id == low
    )));
    assert!(!folded.iter().any(|op| matches!(
        op,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill { id, offset: 0 }),
            ..
        } if *id == high
    )));
}

#[test]
fn collapse_empty_jump_blocks_redirects_predecessors() {
    let mut routine = MirRoutine {
        id: RoutineId(0),
        name: "Main".to_string(),
        abi: MirRoutineAbi::Action,
        frame: MirFrame::default(),
        temps: Vec::new(),
        blocks: vec![
            MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Branch {
                    cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                    then_edge: MirEdge::plain(MirBlockId(1)),
                    else_edge: MirEdge::plain(MirBlockId(2)),
                },
            },
            MirBlock {
                id: MirBlockId(1),
                label: "jump_one".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            },
            MirBlock {
                id: MirBlockId(2),
                label: "jump_two".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            },
            MirBlock {
                id: MirBlockId(3),
                label: "done".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            },
        ],
        effects: MirEffects::default(),
    };

    collapse_empty_jump_blocks(&mut routine);

    assert_eq!(
        routine
            .blocks
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>(),
        vec![MirBlockId(0), MirBlockId(3)]
    );
    assert_eq!(
        routine.blocks[0].terminator,
        MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::ConstU8(1)),
            then_edge: MirEdge::plain(MirBlockId(3)),
            else_edge: MirEdge::plain(MirBlockId(3)),
        }
    );
}

#[test]
fn collapse_empty_jump_blocks_keeps_self_loop_targets() {
    let mut routine = MirRoutine {
        id: RoutineId(0),
        name: "Main".to_string(),
        abi: MirRoutineAbi::Action,
        frame: MirFrame::default(),
        temps: Vec::new(),
        blocks: vec![
            MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Branch {
                    cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                    then_edge: MirEdge::plain(MirBlockId(1)),
                    else_edge: MirEdge::plain(MirBlockId(2)),
                },
            },
            MirBlock {
                id: MirBlockId(1),
                label: "body".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            },
            MirBlock {
                id: MirBlockId(2),
                label: "to_forever".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            },
            MirBlock {
                id: MirBlockId(3),
                label: "forever".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            },
        ],
        effects: MirEffects::default(),
    };

    collapse_empty_jump_blocks(&mut routine);

    assert_eq!(
        routine
            .blocks
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>(),
        vec![MirBlockId(0), MirBlockId(1), MirBlockId(3)]
    );
    assert_eq!(
        routine.blocks[0].terminator,
        MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::ConstU8(1)),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(3)),
        }
    );
    assert_eq!(
        routine.blocks[2].terminator,
        MirTerminator::Jump(MirEdge::plain(MirBlockId(3)))
    );
}

#[test]
fn byte_le_branch_expands_to_any_flag_test() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut blocks = vec![
        MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Temp(MirTempId(0)),
                op: MirCompareOp::Le,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(MirMem::ZeroPage(MirZpSlot(0))),
                width: MirWidth::Byte,
                signed: false,
            }],
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(0)))),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
        },
        MirBlock {
            id: MirBlockId(1),
            label: "then".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
        MirBlock {
            id: MirBlockId(2),
            label: "else".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ];

    expand_compare_branch_consumers(&mut blocks, &layout, &Mir6502Config::default());

    assert_eq!(blocks.len(), 3);
    assert!(matches!(
        blocks[0].ops.as_slice(),
        [MirOp::Compare {
            dst: MirCondDest::Flags,
            op: MirCompareOp::Le,
            width: MirWidth::Byte,
            signed: false,
            ..
        }]
    ));
    assert_eq!(
        blocks[0].terminator,
        MirTerminator::Branch {
            cond: MirCond::AnyFlagTest([MirFlagTest::CClear, MirFlagTest::ZSet]),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        }
    );
}

#[test]
fn static_address_split_stays_symbolic_until_final_emit_layout() {
    let static_id = SymbolId(0);
    let mut program = empty_test_program();
    program.statics.push(MirStatic {
        id: static_id,
        name: "__test_static".to_string(),
        ty: "Byte*".to_string(),
        bytes: vec![0x41, 0x42],
        display: "AB".to_string(),
        alignment: 1,
        mutable: false,
        section: "static".to_string(),
    });
    let layout = MaterializeLayout::new(&program, 0x3000);

    let (lo, hi) = split_value(MirValue::StaticAddr(static_id), &layout);

    let mem = MirMem::Static {
        id: static_id,
        offset: 0,
    };
    assert_eq!(
        lo,
        MirValue::StorageAddrByte {
            mem: mem.clone(),
            byte: 0,
        }
    );
    assert_eq!(hi, MirValue::StorageAddrByte { mem, byte: 1 });
}

#[test]
fn signed_word_lt_branch_uses_compact_overflow_path_for_direct_values() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut blocks = vec![
        MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Temp(MirTempId(0)),
                op: MirCompareOp::Lt,
                left: MirValue::Word {
                    lo: Box::new(MirValue::ConstU8(0xFF)),
                    hi: Box::new(MirValue::ConstU8(0x7F)),
                },
                right: MirValue::Word {
                    lo: Box::new(MirValue::ConstU8(0x00)),
                    hi: Box::new(MirValue::ConstU8(0x80)),
                },
                width: MirWidth::Word,
                signed: true,
            }],
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(0)))),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
        },
        MirBlock {
            id: MirBlockId(1),
            label: "then".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
        MirBlock {
            id: MirBlockId(2),
            label: "else".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ];

    expand_compare_branch_consumers(&mut blocks, &layout, &Mir6502Config::default());

    assert!(
        blocks
            .iter()
            .any(|block| block.label.starts_with("cmp_i16_sub_")),
        "{blocks:#?}"
    );
    assert!(
        blocks
            .iter()
            .any(|block| block.label.starts_with("cmp_i16_v_set_")),
        "{blocks:#?}"
    );
    assert!(
        blocks
            .iter()
            .any(|block| block.label.starts_with("cmp_i16_v_clear_")),
        "{blocks:#?}"
    );
    assert!(
        !blocks
            .iter()
            .any(|block| block.label.starts_with("cmp_i16_left_sign_")),
        "{blocks:#?}"
    );
}

#[test]
fn compare_operand_prebranch_fold_enables_compact_signed_word_branch() {
    let mut program = empty_test_program();
    program.routines.push(MirRoutine {
        id: RoutineId(0),
        name: "Main".to_string(),
        abi: MirRoutineAbi::Action,
        frame: MirFrame::default(),
        temps: Vec::new(),
        blocks: Vec::new(),
        effects: MirEffects::default(),
    });
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut stats = MirPeepholeStats::default();
    let mut blocks = vec![
        MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirAddr::Direct(MirMem::Param {
                        id: ParamId(0),
                        offset: 0,
                    }),
                    width: MirWidth::Word,
                },
                MirOp::Compare {
                    dst: MirCondDest::Temp(MirTempId(0)),
                    op: MirCompareOp::Lt,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU16(0),
                    width: MirWidth::Word,
                    signed: true,
                },
            ],
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(0)))),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
        },
        MirBlock {
            id: MirBlockId(1),
            label: "then".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
        MirBlock {
            id: MirBlockId(2),
            label: "else".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ];

    fold_compare_operand_producers_before_branches(&mut blocks, RoutineId(0), &mut stats);
    assert!(matches!(
        blocks[0].ops.as_slice(),
        [MirOp::Compare {
            left: MirValue::Word { .. },
            right: MirValue::ConstU16(0),
            width: MirWidth::Word,
            signed: true,
            ..
        }]
    ));

    expand_compare_branch_consumers(&mut blocks, &layout, &Mir6502Config::default());

    assert!(
        blocks
            .iter()
            .any(|block| block.label.starts_with("cmp_i16_sub_")),
        "{blocks:#?}"
    );
    assert!(
        !blocks
            .iter()
            .any(|block| block.label.starts_with("cmp_i16_left_sign_")),
        "{blocks:#?}"
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("compare-operand-consumer-prebranch"),
        Some(&1)
    );
}

#[test]
fn byte_binary_compare_consumer_forwards_logic_result_to_a() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTemp(MirTempId(7));
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst: temp.clone(),
                left: MirValue::ConstU8(0xAA),
                right: MirValue::ConstU8(0x55),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Binary {
                dst: MirDef::Reg(MirReg::A),
                ..
            },
            MirOp::Compare {
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0),
                ..
            }
        ]
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("byte-binary-compare-consumer")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("byte-binary-compare-forwardable")
            .copied(),
        Some(1)
    );
}

#[test]
fn synthetic_high_byte_of_scalar_byte_storage_normalizes_to_zero() {
    let program = byte_scalar_storage_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);

    let ops = normalize_synthetic_byte_storage_high_ops(
        vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::X),
                src: MirAddr::Direct(MirMem::Local {
                    id: LocalId(0),
                    offset: 1,
                }),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Param {
                    id: ParamId(0),
                    offset: 1,
                }),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::PointerCell(MirMem::Local {
                    id: LocalId(0),
                    offset: 1,
                }),
                right: MirValue::PointerCell(MirMem::Param {
                    id: ParamId(0),
                    offset: 1,
                }),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        RoutineId(0),
        &layout,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::X),
                value: 0,
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                left: MirValue::ConstU8(0),
                right: MirValue::ConstU8(0),
                ..
            }
        ]
    ));
}

#[test]
fn word_load_store_of_scalar_byte_storage_uses_zero_high_byte() {
    let program = byte_scalar_storage_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(1)),
                src: MirAddr::Direct(MirMem::Local {
                    id: LocalId(0),
                    offset: 0,
                }),
                width: MirWidth::Word,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Param {
                    id: ParamId(0),
                    offset: 0,
                }),
                src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                width: MirWidth::Word,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );
    let ops = normalize_synthetic_byte_storage_high_ops(ops, RoutineId(0), &layout);

    assert!(
        matches!(
            ops.as_slice(),
            [
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirValue::PointerCell(MirMem::Local {
                        id: LocalId(0),
                        offset: 0
                    }),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Param {
                        id: ParamId(0),
                        offset: 0
                    }),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirValue::ConstU8(0),
                    width: MirWidth::Byte,
                }
            ]
        ),
        "{ops:#?}"
    );
}

#[test]
fn byte_binary_compare_consumer_forwards_carry_safe_add_result_to_a() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTemp(MirTempId(7));
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: temp.clone(),
                left: MirValue::PointerCell(MirMem::Local {
                    id: LocalId(0),
                    offset: 0,
                }),
                right: MirValue::ConstU8(3),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
                ..
            },
            MirOp::Compare {
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                ..
            }
        ]
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("byte-binary-compare-consumer")
            .copied(),
        Some(1)
    );
}

#[test]
fn byte_binary_compare_consumer_normalizes_unspecified_add_carry_before_forwarding() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTemp(MirTempId(7));
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: temp.clone(),
                left: MirValue::PointerCell(MirMem::Local {
                    id: LocalId(0),
                    offset: 0,
                }),
                right: MirValue::ConstU8(3),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
                ..
            },
            MirOp::Compare {
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                ..
            }
        ]
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("byte-binary-compare-consumer")
            .copied(),
        Some(1)
    );
}

#[test]
fn byte_binary_compare_consumer_forwards_lane_result_to_a() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTempByte {
        id: MirTempId(7),
        byte: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::And,
                dst: temp.clone(),
                left: MirValue::PointerCell(MirMem::Local {
                    id: LocalId(0),
                    offset: 0,
                }),
                right: MirValue::ConstU8(0x80),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Ne,
                left: MirValue::Def(temp),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Binary {
                dst: MirDef::Reg(MirReg::A),
                ..
            },
            MirOp::Compare {
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0),
                ..
            }
        ]
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("byte-binary-compare-consumer")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("byte-binary-compare-forwardable")
            .copied(),
        Some(1)
    );
}

#[test]
fn byte_binary_compare_consumer_keeps_result_used_after_compare() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTemp(MirTempId(7));
    let sink = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst: temp.clone(),
                left: MirValue::ConstU8(0xAA),
                right: MirValue::ConstU8(0x55),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp.clone()),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
            MirOp::Store {
                dst: MirAddr::Direct(sink),
                src: MirValue::Def(temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.first(),
        Some(MirOp::Binary {
            dst: MirDef::VTemp(MirTempId(7)),
            ..
        })
    ));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("byte-binary-compare-consumer")
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("byte-binary-compare-blocked-live-after")
            .copied(),
        Some(1)
    );
}

#[test]
fn compare_operand_consumer_folds_loadimm_move_and_direct_load() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let imm = MirDef::VTemp(MirTempId(71));
    let moved = MirDef::VTemp(MirTempId(72));
    let loaded = MirDef::VTemp(MirTempId(73));
    let source = MirMem::Local {
        id: LocalId(71),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::LoadImm {
                dst: imm.clone(),
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: moved.clone(),
                src: MirValue::Def(imm),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: loaded.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Lt,
                left: MirValue::Def(moved),
                right: MirValue::Def(loaded),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [MirOp::Compare {
            left: MirValue::ConstU8(7),
            right: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
            ..
        }] if mem == &source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("compare-operand-consumer")
            .copied(),
        Some(1)
    );
}

#[test]
fn compare_operand_consumer_folds_word_load_and_immediate() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let loaded = MirDef::VTemp(MirTempId(74));
    let imm = MirDef::VTemp(MirTempId(75));
    let source = MirMem::Local {
        id: LocalId(74),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: loaded.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Word,
            },
            MirOp::LoadImm {
                dst: imm.clone(),
                value: 0x1234,
                width: MirWidth::Word,
            },
            MirOp::Compare {
                dst: MirCondDest::Temp(MirTempId(76)),
                op: MirCompareOp::Eq,
                left: MirValue::Def(loaded),
                right: MirValue::Def(imm),
                width: MirWidth::Word,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [MirOp::Compare {
            left: MirValue::Word { lo, hi },
            right: MirValue::ConstU16(0x1234),
            width: MirWidth::Word,
            ..
        }] if matches!(lo.as_ref(), MirValue::PointerCell(mem) if mem == &source)
            && matches!(hi.as_ref(), MirValue::PointerCell(mem) if mem == &offset_mem(&source, 1))
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("compare-operand-consumer")
            .copied(),
        Some(1)
    );
}

#[test]
fn carry_observability_counts_unspecified_add_sub_carry() {
    let mut program = empty_test_program();
    program.routines[0].blocks = vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".to_string(),
        params: Vec::new(),
        ops: vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
        ],
        terminator: MirTerminator::Return,
    }];
    let mut stats = MirPeepholeStats::default();

    record_unspecified_add_sub_carry_observability(&program, &mut stats);

    let counts = stats.aggregate_counts();
    assert_eq!(counts.get("mir6502-carry-none-addsub").copied(), Some(2));
    assert_eq!(
        counts.get("mir6502-carry-none-addsub-byte").copied(),
        Some(1)
    );
    assert_eq!(
        counts.get("mir6502-carry-none-addsub-word").copied(),
        Some(1)
    );
    assert_eq!(counts.get("mir6502-carry-none-add").copied(), Some(1));
    assert_eq!(counts.get("mir6502-carry-none-sub").copied(), Some(1));
}

#[test]
fn binary_temp_consumer_observability_counts_store_consumer() {
    let temp = MirDef::VTemp(MirTempId(11));
    let ops = vec![
        MirOp::Binary {
            op: MirBinaryOp::Xor,
            dst: temp.clone(),
            left: MirValue::ConstU8(0xF0),
            right: MirValue::ConstU8(0x0F),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Local {
                id: LocalId(0),
                offset: 0,
            }),
            src: MirValue::Def(temp),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    record_binary_temp_consumer_observation(
        &ops,
        0,
        &MirTerminator::Return,
        RoutineId(0),
        &mut stats,
    );

    let counts = stats.aggregate_counts();
    assert_eq!(
        counts.get("binary-temp-consumer-candidates").copied(),
        Some(1)
    );
    assert_eq!(counts.get("binary-temp-consumer-store").copied(), Some(1));
    assert_eq!(counts.get("binary-temp-consumer-byte").copied(), Some(1));
    assert_eq!(counts.get("binary-temp-consumer-op-xor").copied(), Some(1));
    assert_eq!(
        counts.get("binary-temp-consumer-single-use").copied(),
        Some(1)
    );
}

#[test]
fn binary_temp_consumer_observability_counts_indirect_store_consumer() {
    let temp = MirDef::VTemp(MirTempId(12));
    let ops = vec![
        MirOp::Binary {
            op: MirBinaryOp::And,
            dst: temp.clone(),
            left: MirValue::ConstU8(0x7F),
            right: MirValue::ConstU8(0x55),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::StoreIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            src: MirValue::Def(temp),
            offset: 0,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    record_binary_temp_consumer_observation(
        &ops,
        0,
        &MirTerminator::Return,
        RoutineId(0),
        &mut stats,
    );

    let counts = stats.aggregate_counts();
    assert_eq!(
        counts.get("binary-temp-consumer-store-indirect").copied(),
        Some(1)
    );
    assert_eq!(counts.get("binary-temp-consumer-op-and").copied(), Some(1));
}

#[test]
fn binary_temp_consumer_observability_counts_call_arg_and_later_use() {
    let producer_operand = MirDef::VTemp(MirTempId(13));
    let temp = MirDef::VTemp(MirTempId(14));
    let ops = vec![
        MirOp::Binary {
            op: MirBinaryOp::Or,
            dst: temp.clone(),
            left: MirValue::Def(producer_operand),
            right: MirValue::ConstU8(0x80),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: vec![MirCallArg {
                value: MirValue::Def(temp.clone()),
                width: MirWidth::Byte,
                home: MirArgHome::Reg(MirReg::A),
            }],
            result: None,
            effects: MirEffects::default(),
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Local {
                id: LocalId(1),
                offset: 0,
            }),
            src: MirValue::Def(temp),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    record_binary_temp_consumer_observation(
        &ops,
        0,
        &MirTerminator::Return,
        RoutineId(0),
        &mut stats,
    );

    let counts = stats.aggregate_counts();
    assert_eq!(
        counts.get("binary-temp-consumer-call-arg").copied(),
        Some(1)
    );
    assert_eq!(
        counts.get("binary-temp-consumer-live-after").copied(),
        Some(1)
    );
    assert_eq!(
        counts.get("binary-temp-consumer-temp-operands").copied(),
        Some(1)
    );
}

#[test]
fn binary_temp_consumer_observability_counts_binary_chain() {
    let temp = MirDef::VTemp(MirTempId(15));
    let ops = vec![
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: temp.clone(),
            left: MirValue::ConstU8(1),
            right: MirValue::ConstU8(2),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::Clear),
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Binary {
            op: MirBinaryOp::Sub,
            dst: MirDef::VTemp(MirTempId(16)),
            left: MirValue::Def(temp),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::Set),
            carry_out: MirCarryOut::Ignore,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    record_binary_temp_consumer_observation(
        &ops,
        0,
        &MirTerminator::Return,
        RoutineId(0),
        &mut stats,
    );

    let counts = stats.aggregate_counts();
    assert_eq!(counts.get("binary-temp-consumer-binary").copied(), Some(1));
    assert_eq!(counts.get("binary-temp-consumer-op-add").copied(), Some(1));
}

#[test]
fn byte_binary_direct_store_consumer_forwards_temp_operand() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src_temp = MirDef::VTemp(MirTempId(17));
    let result_temp = MirDef::VTemp(MirTempId(18));
    let store_dst = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst: result_temp.clone(),
                left: MirValue::Def(src_temp.clone()),
                right: MirValue::ConstU8(0x55),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(def),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0x55),
                width: MirWidth::Byte,
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            }
        ] if def == &src_temp && dst == &store_dst
    ));
    assert_eq!(
        stats.aggregate_counts().get("byte-store-consumer").copied(),
        Some(1)
    );
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains("binary-store-forward: status=applied block=b0 op=#0"));
    assert!(report.contains("window=[#0=Binary"));
    assert!(report.contains("#1=Store"));
}

#[test]
fn byte_memory_update_commuted_add_one_uses_inc() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let load_temp = MirDef::VTemp(MirTempId(181));
    let result_temp = MirDef::VTemp(MirTempId(182));
    let update_mem = MirMem::Local {
        id: LocalId(18),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: load_temp.clone(),
                src: MirAddr::Direct(update_mem.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::ConstU8(1),
                right: MirValue::Def(load_temp),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(update_mem.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [MirOp::UpdateMem {
            op: MirUpdateOp::Inc,
            mem,
            width: MirWidth::Byte,
        }] if mem == &update_mem
    ));
    assert_eq!(
        stats.aggregate_counts().get("byte-store-consumer").copied(),
        Some(1)
    );
}

#[test]
fn byte_memory_update_sub_const_uses_add_sub_store_materialization() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let load_temp = MirDef::VTemp(MirTempId(185));
    let result_temp = MirDef::VTemp(MirTempId(186));
    let update_mem = MirMem::Local {
        id: LocalId(21),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: load_temp.clone(),
                src: MirAddr::Direct(update_mem.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: result_temp.clone(),
                left: MirValue::Def(load_temp),
                right: MirValue::ConstU8(3),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Set),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(update_mem.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::PointerCell(mem),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(3),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Set),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ] if mem == &update_mem && dst == &update_mem
    ));
    assert_eq!(
        stats.aggregate_counts().get("byte-store-consumer").copied(),
        Some(1)
    );
}

#[test]
fn word_memory_update_commuted_add_one_uses_inc() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let result_temp = MirDef::VTemp(MirTempId(183));
    let update_mem = MirMem::Local {
        id: LocalId(19),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::ConstU16(1),
                right: pointer_value_from_mem(&update_mem),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(update_mem.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Word,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::optimized(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(
        matches!(
            ops.as_slice(),
            [MirOp::UpdateMem {
                op: MirUpdateOp::Inc,
                mem,
                width: MirWidth::Word,
            }] if mem == &update_mem
        ),
        "{ops:#?}"
    );
    assert_eq!(
        stats.aggregate_counts().get("word-store-consumer").copied(),
        Some(1)
    );
}

#[test]
fn loaded_word_memory_update_add_one_uses_inc_by_default() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let load_temp = MirDef::VTemp(MirTempId(284));
    let result_temp = MirDef::VTemp(MirTempId(285));
    let update_mem = MirMem::Local {
        id: LocalId(21),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: load_temp.clone(),
                src: MirAddr::Direct(update_mem.clone()),
                width: MirWidth::Word,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::Def(load_temp),
                right: MirValue::ConstU8(1),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(update_mem.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Word,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(
        matches!(
            ops.as_slice(),
            [MirOp::UpdateMem {
                op: MirUpdateOp::Inc,
                mem,
                width: MirWidth::Word,
            }] if mem == &update_mem
        ),
        "{ops:#?}"
    );
    assert_eq!(
        stats.aggregate_counts().get("word-store-consumer").copied(),
        Some(1)
    );
}

#[test]
fn loaded_word_memory_update_sub_one_uses_dec_by_default() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let load_temp = MirDef::VTemp(MirTempId(286));
    let result_temp = MirDef::VTemp(MirTempId(287));
    let update_mem = MirMem::Local {
        id: LocalId(22),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: load_temp.clone(),
                src: MirAddr::Direct(update_mem.clone()),
                width: MirWidth::Word,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: result_temp.clone(),
                left: MirValue::Def(load_temp),
                right: MirValue::ConstU8(1),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(update_mem.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Word,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(
        matches!(
            ops.as_slice(),
            [MirOp::UpdateMem {
                op: MirUpdateOp::Dec,
                mem,
                width: MirWidth::Word,
            }] if mem == &update_mem
        ),
        "{ops:#?}"
    );
    assert_eq!(
        stats.aggregate_counts().get("word-store-consumer").copied(),
        Some(1)
    );
}

#[test]
fn word_memory_update_commuted_add_const_uses_byte_to_word_update() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let result_temp = MirDef::VTemp(MirTempId(184));
    let update_mem = MirMem::Local {
        id: LocalId(20),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::ConstU16(2),
                right: pointer_value_from_mem(&update_mem),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(update_mem.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Word,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::optimized(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(
        matches!(
            ops.as_slice(),
            [MirOp::AddByteToWordMem {
                mem,
                value: MirValue::ConstU8(2),
            }] if mem == &update_mem
        ),
        "{ops:#?}"
    );
    assert_eq!(
        stats.aggregate_counts().get("word-store-consumer").copied(),
        Some(1)
    );
}

#[test]
fn byte_store_expr_consumer_materializes_loaded_add_chain_without_temps() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let left_temp = MirDef::VTemp(MirTempId(187));
    let right_temp = MirDef::VTemp(MirTempId(188));
    let first_temp = MirDef::VTemp(MirTempId(189));
    let result_temp = MirDef::VTemp(MirTempId(190));
    let left_mem = MirMem::Local {
        id: LocalId(22),
        offset: 0,
    };
    let right_mem = MirMem::Local {
        id: LocalId(23),
        offset: 0,
    };
    let store_dst = MirMem::Local {
        id: LocalId(24),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: left_temp.clone(),
                src: MirAddr::Direct(left_mem.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: right_temp.clone(),
                src: MirAddr::Direct(right_mem.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: first_temp.clone(),
                left: MirValue::Def(left_temp),
                right: MirValue::Def(right_temp),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::Def(first_temp),
                right: MirValue::ConstU8(5),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::PointerCell(left),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(right),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(5),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ] if left == &left_mem && right == &right_mem && dst == &store_dst
    ));
    assert_eq!(
        stats.aggregate_counts().get("store-expr-consumer").copied(),
        Some(1)
    );
}

#[test]
fn word_store_expr_consumer_materializes_loadimm_loaded_add_without_temps() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let imm_temp = MirDef::VTemp(MirTempId(191));
    let load_temp = MirDef::VTemp(MirTempId(192));
    let result_temp = MirDef::VTemp(MirTempId(193));
    let source = MirMem::Local {
        id: LocalId(25),
        offset: 0,
    };
    let store_dst = MirMem::Local {
        id: LocalId(26),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::LoadImm {
                dst: imm_temp.clone(),
                value: 2,
                width: MirWidth::Word,
            },
            MirOp::Load {
                dst: load_temp.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Word,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::Def(load_temp),
                right: MirValue::Def(imm_temp),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Word,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::PointerCell(lo_source),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(2),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Store {
                dst: MirAddr::Direct(lo_dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::PointerCell(hi_source),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(hi_dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ] if lo_source == &source
            && hi_source == &offset_mem(&source, 1)
            && lo_dst == &store_dst
            && hi_dst == &offset_mem(&store_dst, 1)
    ));
    assert_eq!(
        stats.aggregate_counts().get("store-expr-consumer").copied(),
        Some(1)
    );
}

#[test]
fn byte_shift_direct_store_consumer_forwards_immediate_shift() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src_temp = MirDef::VTemp(MirTempId(18));
    let result_temp = MirDef::VTemp(MirTempId(19));
    let store_dst = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Rsh,
                dst: result_temp.clone(),
                left: MirValue::Def(src_temp.clone()),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(def),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Rsh,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            }
        ] if def == &src_temp && dst == &store_dst
    ));
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains("binary-store-forward: status=applied block=b0 op=#0"));
}

#[test]
fn byte_binary_direct_store_consumer_keeps_live_result_temp() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let result_temp = MirDef::VTemp(MirTempId(19));
    let store_dst = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Or,
                dst: result_temp.clone(),
                left: MirValue::ConstU8(0x10),
                right: MirValue::ConstU8(0x01),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst),
                src: MirValue::Def(result_temp.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.first(),
        Some(MirOp::Binary {
            dst: MirDef::VTemp(MirTempId(19)),
            ..
        })
    ));
    assert!(!stats.aggregate_counts().contains_key("byte-store-consumer"));
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(
        report.contains(
            "binary-store-forward: status=blocked reason=result-live-after block=b0 op=#0"
        )
    );
    assert!(report.contains("window=[#0=Binary"));
    assert!(report.contains("#1=Store"));
    assert!(report.contains("#2=Move"));
}

#[test]
fn word_binary_byte_store_consumer_forwards_low_lane() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src_temp = MirTempId(20);
    let result_temp = MirDef::VTemp(MirTempId(21));
    let store_dst = MirMem::Local {
        id: LocalId(2),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::Def(MirDef::VTemp(src_temp)),
                right: MirValue::ConstU16(0x1234),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(MirDef::VTempByte { id, byte: 0 }),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0x34),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            }
        ] if *id == src_temp && dst == &store_dst
    ));
    assert_eq!(
        stats.aggregate_counts().get("byte-store-consumer").copied(),
        Some(1)
    );
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains("binary-store-forward: status=applied block=b0 op=#0"));
}

#[test]
fn word_binary_byte_store_consumer_keeps_full_word_when_flags_live() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let result_temp = MirDef::VTemp(MirTempId(22));
    let store_dst = MirMem::Local {
        id: LocalId(3),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: result_temp.clone(),
                left: MirValue::ConstU16(0x1200),
                right: MirValue::ConstU16(1),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst),
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        },
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.first(),
        Some(MirOp::Binary {
            width: MirWidth::Byte,
            dst: MirDef::VTempByte {
                id: MirTempId(22),
                byte: 0
            },
            ..
        })
    ));
    assert!(!stats.aggregate_counts().contains_key("byte-store-consumer"));
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains(
        "binary-store-forward: status=blocked reason=word-low-byte-flags-live block=b0 op=#0"
    ));
}

#[test]
fn word_rsh8_byte_store_consumer_forwards_high_lane_to_indexed_store() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src_temp = MirTempId(23);
    let result_temp = MirDef::VTemp(MirTempId(24));
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Rsh,
                dst: result_temp.clone(),
                left: MirValue::Def(MirDef::VTemp(src_temp)),
                right: MirValue::ConstU8(8),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: MirValue::ConstU16(0x4000),
                    index: MirValue::ConstU8(3),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(!ops.iter().any(|op| matches!(
        op,
        MirOp::Binary {
            op: MirBinaryOp::Rsh,
            ..
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::Def(MirDef::VTempByte { id, byte: 1 }),
            width: MirWidth::Byte,
        } if *id == src_temp
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY { .. },
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    )));
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains("binary-store-forward: status=applied block=b0 op=#0"));
}

#[test]
fn word_binary_direct_store_consumer_forwards_both_lanes() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let result_temp = MirDef::VTemp(MirTempId(24));
    let right_mem = MirMem::Local {
        id: LocalId(4),
        offset: 0,
    };
    let store_dst = MirMem::Local {
        id: LocalId(3),
        offset: 0,
    };
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::ConstU16(0x1201),
                right: MirValue::PointerCell(right_mem.clone()),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst.clone()),
                src: MirValue::Def(result_temp),
                width: MirWidth::Word,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(0x01),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(lo_src),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Store {
                dst: MirAddr::Direct(lo_dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(0x12),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(hi_src),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(hi_dst),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            }
        ] if lo_src == &right_mem
            && hi_src == &offset_mem(&right_mem, 1)
            && lo_dst == &store_dst
            && hi_dst == &offset_mem(&store_dst, 1)
    ));
    assert_eq!(
        stats.aggregate_counts().get("byte-store-consumer").copied(),
        Some(1)
    );
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains("binary-store-forward: status=applied block=b0 op=#0"));
}

#[test]
fn word_binary_direct_store_block_reports_temp_producers() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let left_temp = MirDef::VTemp(MirTempId(25));
    let right_temp = MirDef::VTemp(MirTempId(26));
    let result_temp = MirDef::VTemp(MirTempId(27));
    let left_src = MirMem::Param {
        id: ParamId(0),
        offset: 0,
    };
    let store_dst = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let _ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: left_temp.clone(),
                src: MirAddr::Direct(left_src),
                width: MirWidth::Word,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: right_temp.clone(),
                left: MirValue::ConstU8(4),
                right: MirValue::ConstU8(3),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::Def(left_temp),
                right: MirValue::Def(right_temp),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(store_dst),
                src: MirValue::Def(result_temp),
                width: MirWidth::Word,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    let counts = stats.aggregate_counts();
    assert_eq!(
        counts
            .get("binary-store-forward-word-temp-producer-load-direct")
            .copied(),
        Some(1)
    );
    assert_eq!(
        counts
            .get("binary-store-forward-word-temp-producer-binary")
            .copied(),
        Some(1)
    );
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains(
        "binary-store-forward: status=blocked reason=word-operand-temp temp-producers=[left:v25=load-direct-word@#0, right:v26=binary-byte@#1] block=b0 op=#2"
    ));
}

#[test]
fn staged_word_store_forward_stores_updated_word_directly_to_target() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source_lo = MirMem::ZeroPage(MirZpSlot(2));
    let source_hi = MirMem::ZeroPage(MirZpSlot(4));
    let target_lo = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
    let target_hi = offset_mem(&target_lo, 1);
    let mut stats = MirPeepholeStats::default();

    let ops = peepholes::fold_structural_peepholes(
        vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(source_lo.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(5),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Store {
                dst: MirAddr::Direct(source_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(source_hi.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(source_hi),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(source_lo),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(target_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(4))),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(target_hi.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        true,
        &mut stats,
    );

    assert_eq!(ops.len(), 6);
    assert!(matches!(
        &ops[2],
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            ..
        } if *mem == target_lo
    ));
    assert!(matches!(
        &ops[5],
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            ..
        } if *mem == target_hi
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("staged-word-store-forward")
            .copied(),
        Some(1)
    );
}

#[test]
fn direct_byte_word_update_does_not_fold_source_aliasing_target_word() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let target_lo = MirMem::Local {
        id: LocalId(31),
        offset: 0,
    };
    let target_hi = offset_mem(&target_lo, 1);
    let mut stats = MirPeepholeStats::default();

    let ops = peepholes::fold_structural_peepholes(
        vec![
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(target_lo.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(target_hi.clone()),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Store {
                dst: MirAddr::Direct(target_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(target_hi.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(target_hi.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        true,
        &mut stats,
    );

    assert!(
        !ops.iter()
            .any(|op| matches!(op, MirOp::AddByteToWordMem { mem, .. } if mem == &target_lo)),
        "{ops:#?}"
    );
    assert_eq!(ops.len(), 6, "{ops:#?}");
}

#[test]
fn key_style_updated_pointer_deref_keeps_pointer_in_scratch_pair() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let param_lo = MirMem::Param {
        id: ParamId(0),
        offset: 0,
    };
    let param_hi = offset_mem(&param_lo, 1);
    let value_slot = MirMem::ZeroPage(MirZpSlot(2));
    let return_slot = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
    let mut stats = MirPeepholeStats::default();

    let ops = peepholes::fold_structural_peepholes(
        vec![
            MirOp::Store {
                dst: MirAddr::Direct(param_hi.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(param_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(param_hi.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::LoadIndirect {
                consumer: fixed_pointer_consumer(POINTER_SCRATCH_LO),
                dst: MirDef::Reg(MirReg::A),
                offset: 0,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(3),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(value_slot.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::AddByteToWordMem {
                mem: param_lo.clone(),
                value: MirValue::PointerCell(value_slot.clone()),
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(param_lo.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(param_hi.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::LoadIndirect {
                consumer: fixed_pointer_consumer(POINTER_SCRATCH_LO),
                dst: MirDef::Reg(MirReg::A),
                offset: 0,
            },
            MirOp::Store {
                dst: MirAddr::Direct(return_slot),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        true,
        &mut stats,
    );

    assert_eq!(
        stats
            .aggregate_counts()
            .get("updated-pointer-deref-forward")
            .copied(),
        Some(1)
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, MirOp::AddByteToWordMem { mem, .. } if mem == &param_lo))
    );
    assert!(matches!(
        ops.get(3),
        Some(MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::X)),
            width: MirWidth::Byte,
        })
    ));
}

#[test]
fn next_style_binary_word_store_producer_initializes_return_word_directly() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let param_lo = MirMem::Param {
        id: ParamId(0),
        offset: 0,
    };
    let param_hi = offset_mem(&param_lo, 1);
    let staged_lo = MirMem::ZeroPage(MirZpSlot(2));
    let staged_hi = MirMem::ZeroPage(MirZpSlot(4));
    let value_slot = MirMem::ZeroPage(MirZpSlot(3));
    let return_lo = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
    let return_hi = offset_mem(&return_lo, 1);
    let mut stats = MirPeepholeStats::default();

    let ops = peepholes::fold_structural_peepholes(
        vec![
            MirOp::Store {
                dst: MirAddr::Direct(param_hi.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(param_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(staged_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(param_hi.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(staged_hi.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(param_lo.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(param_hi),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::LoadIndirect {
                consumer: fixed_pointer_consumer(POINTER_SCRATCH_LO),
                dst: MirDef::Reg(MirReg::A),
                offset: 0,
            },
            MirOp::Store {
                dst: MirAddr::Direct(value_slot.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(staged_lo.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(value_slot.clone()),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Store {
                dst: MirAddr::Direct(staged_lo),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(staged_hi.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(staged_hi.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(2))),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(5),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Store {
                dst: MirAddr::Direct(return_lo.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(staged_hi),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::FromPrevious),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(return_hi.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        true,
        &mut stats,
    );

    assert_eq!(
        stats
            .aggregate_counts()
            .get("binary-word-store-producer-forward")
            .copied(),
        Some(1)
    );
    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Store { .. },
            MirOp::Store { .. },
            MirOp::Store {
                dst: MirAddr::Direct(lo),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                ..
            },
            MirOp::Store { .. },
            MirOp::Store {
                dst: MirAddr::Direct(hi),
                src: MirValue::Def(MirDef::Reg(MirReg::X)),
                ..
            },
            MirOp::Store { .. },
            MirOp::LoadIndirect { .. },
            MirOp::Store { .. },
            MirOp::AddByteToWordMem { mem: add_mem, .. },
            MirOp::AddByteToWordMem {
                mem: const_mem,
                value: MirValue::ConstU8(5),
            },
        ] if lo == &return_lo && hi == &return_hi && add_mem == &return_lo && const_mem == &return_lo
    ));
}

#[test]
fn byte_binary_deref_store_consumer_forwards_after_address_materialization() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let result_temp = MirDef::VTemp(MirTempId(21));
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst: result_temp.clone(),
                left: MirValue::ConstU8(0x0f),
                right: MirValue::ConstU8(0xf0),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Deref {
                    ptr: MirValue::ConstU16(0x4200),
                    offset: 2,
                },
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::MaterializeAddress { .. },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(0x0f),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(0xf0),
                width: MirWidth::Byte,
                ..
            },
            MirOp::StoreIndirect {
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                offset: 2,
                ..
            }
        ]
    ));
    assert_eq!(
        stats.aggregate_counts().get("byte-store-consumer").copied(),
        Some(1)
    );
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains("binary-store-forward: status=applied block=b0 op=#0"));
}

#[test]
fn byte_binary_computed_index_store_consumer_forwards_after_address_materialization() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let result_temp = MirDef::VTemp(MirTempId(23));
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: result_temp.clone(),
                left: MirValue::ConstU8(0x10),
                right: MirValue::ConstU8(3),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: MirValue::ConstU16(0x4000),
                    index: MirValue::ConstU8(3),
                    elem_size: 1,
                    offset: 4,
                },
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::MaterializeIndexedAddress { .. },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(0x10),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(3),
                width: MirWidth::Byte,
                ..
            },
            MirOp::StoreIndirect {
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                offset: 4,
                ..
            }
        ]
    ));
    assert_eq!(
        stats.aggregate_counts().get("byte-store-consumer").copied(),
        Some(1)
    );
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains("binary-store-forward: status=applied block=b0 op=#0"));
}

#[test]
fn byte_binary_computed_byte_index_store_consumer_preserves_dynamic_y_index() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let base_temp = MirValue::Def(MirDef::VTemp(MirTempId(24)));
    let index_temp = MirValue::Def(MirDef::VTempByte {
        id: MirTempId(25),
        byte: 0,
    });
    let result_temp = MirDef::VTemp(MirTempId(26));
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: result_temp.clone(),
                left: MirValue::ConstU8(0x10),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Set),
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: base_temp,
                    index: index_temp,
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(result_temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::MaterializeAddress { .. },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::Y),
                ..
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(0x10),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: MirDef::Reg(MirReg::A),
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                ..
            },
            MirOp::Store {
                dst: MirAddr::FixedIndirectIndexedY { .. },
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            }
        ]
    ));
    assert_eq!(
        stats.aggregate_counts().get("byte-store-consumer").copied(),
        Some(1)
    );
    let report = stats::format_peephole_report(&program, &stats, MirPeepholeReportMode::Sites);
    assert!(report.contains("binary-store-forward: status=applied block=b0 op=#0"));
}

#[test]
fn constant_word_index_read_folds_into_indirect_offsets() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut out = Vec::new();

    materialize_computed_index_read(
        MirDef::VTemp(MirTempId(0)),
        MirValue::ConstU16(0x4000),
        MirValue::ConstU8(3),
        2,
        1,
        MirWidth::Word,
        &layout,
        &mut out,
    );

    assert!(matches!(
        out.as_slice(),
        [
            MirOp::MaterializeAddress { .. },
            MirOp::LoadIndirect {
                dst: MirDef::VTempByte {
                    id: MirTempId(0),
                    byte: 0
                },
                offset: 7,
                ..
            },
            MirOp::LoadIndirect {
                dst: MirDef::VTempByte {
                    id: MirTempId(0),
                    byte: 1
                },
                offset: 8,
                ..
            }
        ]
    ));
    assert!(
        !out.iter()
            .any(|op| matches!(op, MirOp::AdvanceAddress { .. }))
    );
}

#[test]
fn constant_word_index_write_folds_into_indirect_offsets() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut out = Vec::new();

    materialize_computed_index_write(
        MirValue::ConstU16(0x4000),
        MirValue::ConstU8(4),
        2,
        0,
        MirValue::ConstU16(0x1234),
        MirWidth::Word,
        &layout,
        &mut out,
    );

    assert!(matches!(
        out.as_slice(),
        [
            MirOp::MaterializeAddress { .. },
            MirOp::StoreIndirect {
                src: MirValue::ConstU8(0x34),
                offset: 8,
                ..
            },
            MirOp::StoreIndirect {
                src: MirValue::ConstU8(0x12),
                offset: 9,
                ..
            }
        ]
    ));
    assert!(
        !out.iter()
            .any(|op| matches!(op, MirOp::AdvanceAddress { .. }))
    );
}

#[test]
fn byte_read_with_word_index_materializes_full_address() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut out = Vec::new();

    materialize_computed_index_read(
        MirDef::Reg(MirReg::A),
        MirValue::ConstU16(0x4000),
        MirValue::Def(MirDef::VTemp(MirTempId(0))),
        1,
        0,
        MirWidth::Byte,
        &layout,
        &mut out,
    );

    assert!(matches!(
        out.as_slice(),
        [
            MirOp::MaterializeIndexedAddress {
                index: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                scale: 1,
                ..
            },
            MirOp::LoadIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                dst: MirDef::Reg(MirReg::A),
                offset: 0,
            }
        ]
    ));
    assert!(!out.iter().any(|op| matches!(
        op,
        MirOp::Load {
            src: MirAddr::FixedIndirectIndexedY { .. },
            ..
        }
    )));
}

#[test]
fn byte_write_with_word_index_materializes_full_address() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut out = Vec::new();

    materialize_computed_index_write(
        MirValue::ConstU16(0x4000),
        MirValue::Def(MirDef::VTemp(MirTempId(0))),
        1,
        0,
        MirValue::ConstU8(0x55),
        MirWidth::Byte,
        &layout,
        &mut out,
    );

    assert!(matches!(
        out.as_slice(),
        [
            MirOp::MaterializeIndexedAddress {
                index: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                scale: 1,
                ..
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(0x55),
                width: MirWidth::Byte,
            },
            MirOp::StoreIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                offset: 0,
            }
        ]
    ));
    assert!(!out.iter().any(|op| matches!(
        op,
        MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY { .. },
            ..
        }
    )));
}

#[test]
fn dynamic_byte_index_read_uses_absolute_y_for_known_base() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut out = Vec::new();
    let dst = MirAddr::Direct(MirMem::Local {
        id: LocalId(0),
        offset: 0,
    });

    materialize_dynamic_byte_index_read(
        MirValue::ConstU16(0x4000),
        MirValue::PointerCell(MirMem::FixedZeroPage(MirFixedZpSlot(0x5C))),
        3,
        &dst,
        &layout,
        &mut out,
    );

    assert!(matches!(
        out.as_slice(),
        [
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0x5C))),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::AbsoluteIndexedY {
                    base: MirMem::Absolute(0x4003),
                },
                width: MirWidth::Byte,
            },
            MirOp::Store { .. }
        ]
    ));
    assert!(!out.iter().any(|op| matches!(
        op,
        MirOp::MaterializeAddress { .. } | MirOp::AdvanceAddress { .. }
    )));
}

#[test]
fn dynamic_byte_index_write_uses_absolute_y_for_known_base() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut out = Vec::new();

    materialize_dynamic_byte_index_write(
        MirValue::ConstU16(0x4000),
        MirValue::PointerCell(MirMem::FixedZeroPage(MirFixedZpSlot(0x5C))),
        2,
        MirValue::ConstU8(0x7A),
        &layout,
        &mut out,
    );

    assert!(matches!(
        out.as_slice(),
        [
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0x5C))),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(0x7A),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::AbsoluteIndexedY {
                    base: MirMem::Absolute(0x4002),
                },
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            }
        ]
    ));
    assert!(!out.iter().any(|op| matches!(
        op,
        MirOp::MaterializeAddress { .. } | MirOp::AdvanceAddress { .. }
    )));
}

#[test]
fn delayed_byte_index_read_uses_absolute_y_for_storage_address_base() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut out = Vec::new();
    let base = MirMem::Global {
        id: crate::nir::SymbolId(0),
        offset: 0,
    };
    let index = MirMem::Global {
        id: crate::nir::SymbolId(1),
        offset: 0,
    };

    materialize_delayed_byte_indexed_read(
        MirDef::Reg(MirReg::A),
        storage_address_value(&base),
        &DelayedByteIndexExpr::Value(MirValue::PointerCell(index.clone())),
        0,
        &layout,
        &mut out,
    );

    assert!(matches!(
        out.as_slice(),
        [
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(load_index),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::AbsoluteIndexedY { base: load_base },
                width: MirWidth::Byte,
            },
        ] if load_index == &index && load_base == &base
    ));
    assert!(!out.iter().any(|op| matches!(
        op,
        MirOp::MaterializeAddress { .. }
            | MirOp::AdvanceAddress { .. }
            | MirOp::Load {
                src: MirAddr::FixedIndirectIndexedY { .. },
                ..
            }
    )));
}

#[test]
fn delayed_byte_index_write_uses_absolute_y_for_storage_address_base() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut out = Vec::new();
    let base = MirMem::Global {
        id: crate::nir::SymbolId(0),
        offset: 0,
    };
    let index = MirMem::Global {
        id: crate::nir::SymbolId(1),
        offset: 0,
    };

    materialize_delayed_byte_indexed_write(
        storage_address_value(&base),
        &DelayedByteIndexExpr::Value(MirValue::PointerCell(index.clone())),
        0,
        MirValue::ConstU8(0x7A),
        &layout,
        &mut out,
    );

    assert!(matches!(
        out.as_slice(),
        [
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(load_index),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(0x7A),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::AbsoluteIndexedY { base: store_base },
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        ] if load_index == &index && store_base == &base
    ));
    assert!(!out.iter().any(|op| matches!(
        op,
        MirOp::MaterializeAddress { .. }
            | MirOp::AdvanceAddress { .. }
            | MirOp::Store {
                dst: MirAddr::FixedIndirectIndexedY { .. },
                ..
            }
    )));
}

#[test]
fn indirect_const_store_skips_private_pointer_slots() {
    let lo = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let hi = MirMem::Local {
        id: LocalId(0),
        offset: 1,
    };
    let lo_slot = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let hi_slot = MirMem::Spill {
        id: MirSpillId(31),
        offset: 0,
    };
    let fixed_target = fixed_pointer_consumer(POINTER_SCRATCH_LO);
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(lo_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(hi_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo_slot),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi_slot),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::ConstU8(0x7C),
            width: MirWidth::Byte,
        },
        MirOp::StoreIndirect {
            consumer: fixed_target,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset: 0,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_indirect_byte_const_stores(ops, RoutineId(0), &mut stats);

    assert_eq!(rewritten.len(), 6);
    assert!(matches!(
        rewritten.as_slice(),
        [
            MirOp::Load {
                src: MirAddr::Direct(rewritten_lo),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                    POINTER_SCRATCH_LO
                ))),
                ..
            },
            MirOp::Load {
                src: MirAddr::Direct(rewritten_hi),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                    POINTER_SCRATCH_HI
                ))),
                ..
            },
            MirOp::Move {
                src: MirValue::ConstU8(0x7C),
                ..
            },
            MirOp::StoreIndirect {
                consumer,
                offset: 0,
                ..
            }
        ] if *rewritten_lo == lo
            && *rewritten_hi == hi
            && *consumer == fixed_pointer_consumer(POINTER_SCRATCH_LO)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("indirect-byte-const-store")
            .copied(),
        Some(1)
    );
}

#[test]
fn indirect_direct_compound_accepts_forwarded_value_source() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let hi = MirMem::Local {
        id: LocalId(0),
        offset: 1,
    };
    let value_source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let lo_slot = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let hi_slot = MirMem::Spill {
        id: MirSpillId(31),
        offset: 0,
    };
    let staged_target = fixed_pointer_consumer(POINTER_SCRATCH_LO);
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(lo_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(hi_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo_slot),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi_slot),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::LoadIndirect {
            consumer: staged_target,
            dst: MirDef::Reg(MirReg::A),
            offset: 0,
        },
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(value_source.clone()),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::Clear),
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::StoreIndirect {
            consumer: staged_target,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset: 0,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_indirect_byte_direct_compounds(ops, RoutineId(0), &layout, &mut stats);

    assert_eq!(rewritten.len(), 7);
    assert!(matches!(
        rewritten.as_slice(),
        [
            MirOp::Load {
                src: MirAddr::Direct(rewritten_lo),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                    POINTER_SCRATCH_LO
                ))),
                ..
            },
            MirOp::Load {
                src: MirAddr::Direct(rewritten_hi),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                    POINTER_SCRATCH_HI
                ))),
                ..
            },
            MirOp::LoadIndirect { consumer, .. },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                right: MirValue::PointerCell(rewritten_value),
                carry_in: Some(MirCarryIn::Clear),
                ..
            },
            MirOp::StoreIndirect {
                consumer: store_consumer,
                ..
            }
        ] if *rewritten_lo == lo
            && *rewritten_hi == hi
            && *rewritten_value == value_source
            && *consumer == fixed_pointer_consumer(POINTER_SCRATCH_LO)
            && *store_consumer == fixed_pointer_consumer(POINTER_SCRATCH_LO)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("indirect-byte-direct-compound")
            .copied(),
        Some(1)
    );
}

#[test]
fn indirect_const_compound_accepts_delayed_loaded_value() {
    let lo = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let hi = MirMem::Local {
        id: LocalId(0),
        offset: 1,
    };
    let lo_slot = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let hi_slot = MirMem::Spill {
        id: MirSpillId(31),
        offset: 0,
    };
    let value_slot = MirMem::Spill {
        id: MirSpillId(32),
        offset: 0,
    };
    let staged_target = fixed_pointer_consumer(POINTER_SCRATCH_LO);
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(lo_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(hi_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo_slot.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi_slot.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::LoadIndirect {
            consumer: staged_target,
            dst: MirDef::Reg(MirReg::A),
            offset: 0,
        },
        MirOp::Store {
            dst: MirAddr::Direct(value_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo_slot),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi_slot),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(value_slot),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: MirBinaryOp::Sub,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::Set),
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::StoreIndirect {
            consumer: staged_target,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset: 0,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_indirect_byte_const_compounds(ops, RoutineId(0), &mut stats);

    assert_eq!(rewritten.len(), 7);
    assert!(matches!(
        rewritten.as_slice(),
        [
            MirOp::Load {
                src: MirAddr::Direct(rewritten_lo),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                    POINTER_SCRATCH_LO
                ))),
                ..
            },
            MirOp::Load {
                src: MirAddr::Direct(rewritten_hi),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                    POINTER_SCRATCH_HI
                ))),
                ..
            },
            MirOp::LoadIndirect { consumer, .. },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                right: MirValue::ConstU8(1),
                carry_in: Some(MirCarryIn::Set),
                ..
            },
            MirOp::StoreIndirect {
                consumer: store_consumer,
                ..
            }
        ] if *rewritten_lo == lo
            && *rewritten_hi == hi
            && *consumer == fixed_pointer_consumer(POINTER_SCRATCH_LO)
            && *store_consumer == fixed_pointer_consumer(POINTER_SCRATCH_LO)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("indirect-byte-const-compound")
            .copied(),
        Some(1)
    );
}

#[test]
fn indirect_y_const_store_skips_private_pointer_and_index_slots() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let hi = MirMem::Local {
        id: LocalId(0),
        offset: 1,
    };
    let index_source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let lo_slot = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let hi_slot = MirMem::Spill {
        id: MirSpillId(31),
        offset: 0,
    };
    let index_slot = MirMem::Spill {
        id: MirSpillId(32),
        offset: 0,
    };
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(lo_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(hi_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(index_source.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(index_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo_slot),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi_slot),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_HI))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::Y),
            src: MirAddr::Direct(index_slot),
            width: MirWidth::Byte,
        },
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::ConstU8(0),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY {
                zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
            },
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_indirect_y_const_stores(ops, RoutineId(0), &layout, &mut stats);

    assert_eq!(rewritten.len(), 7);
    assert!(matches!(
        rewritten.as_slice(),
        [
            MirOp::Load {
                src: MirAddr::Direct(rewritten_lo),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                    POINTER_SCRATCH_LO
                ))),
                ..
            },
            MirOp::Load {
                src: MirAddr::Direct(rewritten_hi),
                ..
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                    POINTER_SCRATCH_HI
                ))),
                ..
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(rewritten_index),
                ..
            },
            MirOp::Move {
                src: MirValue::ConstU8(0),
                ..
            },
            MirOp::Store {
                dst: MirAddr::FixedIndirectIndexedY {
                    zp: MirFixedZpSlot(POINTER_SCRATCH_LO)
                },
                ..
            }
        ] if *rewritten_lo == lo
            && *rewritten_hi == hi
            && *rewritten_index == index_source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("indirect-y-const-store")
            .copied(),
        Some(1)
    );
}

#[test]
fn word_array_store_value_staging_delays_value_loads() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let value_lo = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let value_hi = MirMem::Local {
        id: LocalId(0),
        offset: 1,
    };
    let lo_slot = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let hi_slot = MirMem::Spill {
        id: MirSpillId(31),
        offset: 0,
    };
    let index_source = MirMem::Global {
        id: crate::nir::SymbolId(2),
        offset: 0,
    };
    let consumer = fixed_pointer_consumer(POINTER_SCRATCH_LO);
    let materialize = MirOp::MaterializeIndexedAddress {
        consumer,
        base: MirValue::Word {
            lo: Box::new(MirValue::PointerCell(MirMem::Global {
                id: crate::nir::SymbolId(1),
                offset: 0,
            })),
            hi: Box::new(MirValue::PointerCell(MirMem::Global {
                id: crate::nir::SymbolId(1),
                offset: 1,
            })),
        },
        index: MirValue::Def(MirDef::Reg(MirReg::A)),
        scale: 2,
    };
    let index_load = MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(index_source),
        width: MirWidth::Byte,
    };
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(value_lo.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(lo_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(value_hi.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(hi_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        index_load.clone(),
        materialize.clone(),
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(lo_slot),
            width: MirWidth::Byte,
        },
        MirOp::StoreIndirect {
            consumer,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset: 0,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(hi_slot),
            width: MirWidth::Byte,
        },
        MirOp::StoreIndirect {
            consumer,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            offset: 1,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_word_array_store_value_staging(ops, RoutineId(0), &layout, &mut stats);

    assert_eq!(rewritten.len(), 6);
    assert_eq!(rewritten[0], index_load);
    assert_eq!(rewritten[1], materialize);
    assert!(matches!(
        rewritten.as_slice(),
        [
            MirOp::Load { .. },
            MirOp::MaterializeIndexedAddress { .. },
            MirOp::Load {
                src: MirAddr::Direct(rewritten_lo),
                ..
            },
            MirOp::StoreIndirect {
                offset: 0,
                ..
            },
            MirOp::Load {
                src: MirAddr::Direct(rewritten_hi),
                ..
            },
            MirOp::StoreIndirect {
                offset: 1,
                ..
            }
        ] if *rewritten_lo == value_lo && *rewritten_hi == value_hi
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("word-array-store-value-staging")
            .copied(),
        Some(1)
    );
}

#[test]
fn ssa_lite_scanner_learns_register_and_memory_facts() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
    let tmp = MirMem::ZeroPage(MirZpSlot(0));
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(src.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(tmp.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ];

    let env = scan_ssa_lite_block_env(&ops, &layout);

    assert_eq!(
        env.reg_fact(MirReg::A),
        Some(&SsaLiteValueKey::DirectMem(src.clone()))
    );
    assert!(
        env.mem
            .iter()
            .any(|(mem, key)| *mem == tmp && *key == SsaLiteValueKey::DirectMem(src.clone()))
    );
    assert_eq!(env.stats.learned, 2);
    assert_eq!(env.stats.killed, 0);
}

#[test]
fn ssa_lite_scanner_kills_facts_on_calls_regardless_of_abi_clobbers() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
    let tmp = MirMem::ZeroPage(MirZpSlot(0));
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(src.clone()),
            width: MirWidth::Byte,
        },
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 0x2D,
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::Y),
            src: MirAddr::Direct(src.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(tmp),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects::default(),
        },
    ];

    let env = scan_ssa_lite_block_env(&ops, &layout);

    assert_eq!(env.reg_fact(MirReg::A), None);
    assert_eq!(env.reg_fact(MirReg::X), None);
    assert_eq!(env.reg_fact(MirReg::Y), None);
    assert!(env.mem.is_empty());
    assert_eq!(env.stats.learned, 4);
    assert_eq!(env.stats.killed, 4);
}

#[test]
fn calls_are_conservative_register_and_flag_barriers_for_peepholes() {
    let call = MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(1)),
        abi: MirCallAbi {
            params: Vec::new(),
            result: None,
            clobbers: MirRegisterSet::default(),
            preserves: MirRegisterSet::default(),
        },
        args: Vec::new(),
        result: None,
        effects: MirEffects::default(),
    };

    assert!(op_may_clobber_reg(&call, MirReg::A));
    assert!(op_may_clobber_reg(&call, MirReg::X));
    assert!(op_may_clobber_reg(&call, MirReg::Y));
    assert!(op_writes_flags(&call));
}

#[test]
fn call_producer_fold_rewrites_indirect_targets() {
    let target_cell = MirMem::Global {
        id: crate::nir::SymbolId(1),
        offset: 0,
    };
    let ops = vec![
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(0)),
            src: MirAddr::Direct(target_cell.clone()),
            width: MirWidth::Word,
        },
        MirOp::Call {
            target: MirCallTarget::Indirect {
                target: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                width: MirWidth::Word,
            },
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects::default(),
        },
    ];

    let rewritten = fold_call_arg_producers(ops);

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        rewritten.first(),
        Some(MirOp::Call {
            target:
                MirCallTarget::Indirect {
                    target:
                        MirValue::Word {
                            lo,
                            hi,
                        },
                    width: MirWidth::Word,
                },
            ..
        }) if **lo == MirValue::PointerCell(target_cell.clone())
            && **hi == MirValue::PointerCell(offset_mem(&target_cell, 1))
    ));
}

#[test]
fn call_result_store_prepares_computed_target_address_before_call() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let result = MirDef::VTemp(MirTempId(11));
    let base = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let index = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Call {
                target: MirCallTarget::Builtin {
                    name: "KnownByte".to_string(),
                    address: Some(0x4000),
                },
                abi: MirCallAbi {
                    params: vec![MirArgHome::Reg(MirReg::A)],
                    result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: vec![MirCallArg {
                    value: MirValue::ConstU8(7),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                }],
                result: Some(MirCallResult {
                    dst: result.clone(),
                    width: MirWidth::Byte,
                    home: MirResultHome::ReturnSlot { offset: 0 },
                }),
                effects: MirEffects::default(),
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&base),
                    index: MirValue::PointerCell(index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(result),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::MaterializeIndexedAddress {
                consumer: DEST_POINTER_PAIR,
                base: prepared_base,
                index: MirValue::PointerCell(prepared_index),
                scale: 1,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::ConstU8(7),
                width: MirWidth::Byte,
            },
            MirOp::Call { result: None, .. },
            MirOp::StoreIndirect {
                consumer: DEST_POINTER_PAIR,
                src:
                    MirValue::PointerCell(MirMem::FixedZeroPage(MirFixedZpSlot(
                        0xA0
                    ))),
                offset: 0,
            },
        ] if *prepared_base == pointer_value_from_mem(&base) && *prepared_index == index
    ));
    let counts = stats.aggregate_counts();
    assert_eq!(counts.get("call-result-store-consumer").copied(), Some(1));
    assert_eq!(
        counts.get("call-result-ea-preserve-candidate").copied(),
        Some(1)
    );
    assert_eq!(counts.get("call-result-ea-preserve").copied(), Some(1));
}

#[test]
fn call_result_store_does_not_preserve_target_across_routine_call() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let result = MirDef::VTemp(MirTempId(12));

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Call {
                target: MirCallTarget::Routine(RoutineId(1)),
                abi: MirCallAbi {
                    params: Vec::new(),
                    result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: Vec::new(),
                result: Some(MirCallResult {
                    dst: result.clone(),
                    width: MirWidth::Byte,
                    home: MirResultHome::ReturnSlot { offset: 0 },
                }),
                effects: MirEffects::default(),
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: MirValue::ConstU16(0x4000),
                    index: MirValue::ConstU8(3),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(result),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.first(),
        Some(MirOp::Call { result: None, .. })
    ));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::MaterializeAddress {
            consumer: DEFAULT_POINTER_PAIR,
            ..
        } | MirOp::MaterializeIndexedAddress {
            consumer: DEFAULT_POINTER_PAIR,
            ..
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::StoreIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            ..
        }
    )));
    assert!(!ops.iter().any(|op| matches!(
        op,
        MirOp::StoreIndirect {
            consumer: DEST_POINTER_PAIR,
            ..
        }
    )));
    let counts = stats.aggregate_counts();
    assert_eq!(
        counts.get("call-result-ea-preserve-candidate").copied(),
        Some(1)
    );
    assert_eq!(
        counts
            .get("call-result-ea-preserve-blocked-clobber")
            .copied(),
        Some(1)
    );
}

#[test]
fn call_result_store_prepares_target_before_loaded_call_arg() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let arg = MirDef::VTemp(MirTempId(10));
    let result = MirDef::VTemp(MirTempId(11));
    let src_base = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let dst_base = MirMem::Local {
        id: LocalId(2),
        offset: 0,
    };
    let src_index = MirMem::Local {
        id: LocalId(4),
        offset: 0,
    };
    let dst_index = MirMem::Local {
        id: LocalId(5),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: arg.clone(),
                src: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&src_base),
                    index: MirValue::PointerCell(src_index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
            MirOp::Call {
                target: MirCallTarget::Builtin {
                    name: "KnownByte".to_string(),
                    address: Some(0x4000),
                },
                abi: MirCallAbi {
                    params: vec![MirArgHome::Reg(MirReg::A)],
                    result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: vec![MirCallArg {
                    value: MirValue::Def(arg),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                }],
                result: Some(MirCallResult {
                    dst: result.clone(),
                    width: MirWidth::Byte,
                    home: MirResultHome::ReturnSlot { offset: 0 },
                }),
                effects: MirEffects::default(),
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&dst_base),
                    index: MirValue::PointerCell(dst_index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(result),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    let target_prepare = ops
        .iter()
        .position(|op| {
            matches!(
                op,
                MirOp::MaterializeIndexedAddress {
                    consumer: DEST_POINTER_PAIR,
                    ..
                }
            )
        })
        .expect("prepared target address");
    let source_prepare = ops
        .iter()
        .position(|op| {
            matches!(
                op,
                MirOp::MaterializeAddress {
                    consumer: DEFAULT_POINTER_PAIR,
                    ..
                } | MirOp::MaterializeIndexedAddress {
                    consumer: DEFAULT_POINTER_PAIR,
                    ..
                }
            )
        })
        .expect("prepared source address");
    let call = ops
        .iter()
        .position(|op| matches!(op, MirOp::Call { result: None, .. }))
        .expect("call");
    let store = ops
        .iter()
        .position(|op| {
            matches!(
                op,
                MirOp::StoreIndirect {
                    consumer: DEST_POINTER_PAIR,
                    ..
                }
            )
        })
        .expect("store through prepared target");

    assert!(target_prepare < source_prepare);
    assert!(source_prepare < call);
    assert!(call < store);
    let counts = stats.aggregate_counts();
    assert_eq!(
        counts
            .get("call-result-ea-preserve-loaded-arg-candidate")
            .copied(),
        Some(1)
    );
    assert_eq!(
        counts.get("call-result-ea-preserve-loaded-arg").copied(),
        Some(1)
    );
    assert_eq!(
        counts.get("call-result-loaded-arg-store-consumer").copied(),
        Some(1)
    );
}

#[test]
fn call_result_store_rematerializes_delayed_byte_index_producers() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let dst_index_load = MirDef::VTemp(MirTempId(20));
    let dst_index = MirDef::VTemp(MirTempId(21));
    let src_index_load = MirDef::VTemp(MirTempId(22));
    let src_index = MirDef::VTemp(MirTempId(23));
    let arg = MirDef::VTemp(MirTempId(24));
    let result = MirDef::VTemp(MirTempId(25));
    let base = MirMem::Global {
        id: crate::nir::SymbolId(108),
        offset: 0,
    };
    let loop_index = MirMem::Global {
        id: crate::nir::SymbolId(5),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: dst_index_load.clone(),
                src: MirAddr::Direct(loop_index.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: dst_index.clone(),
                left: MirValue::ConstU8(13),
                right: MirValue::Def(dst_index_load.clone()),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Load {
                dst: src_index_load.clone(),
                src: MirAddr::Direct(loop_index.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: src_index.clone(),
                left: MirValue::ConstU8(12),
                right: MirValue::Def(src_index_load.clone()),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Load {
                dst: arg.clone(),
                src: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&base),
                    index: MirValue::Def(src_index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
            MirOp::Call {
                target: MirCallTarget::Builtin {
                    name: "KnownByte".to_string(),
                    address: Some(0x4000),
                },
                abi: MirCallAbi {
                    params: vec![MirArgHome::Reg(MirReg::A)],
                    result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: vec![MirCallArg {
                    value: MirValue::Def(arg),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                }],
                result: Some(MirCallResult {
                    dst: result.clone(),
                    width: MirWidth::Byte,
                    home: MirResultHome::ReturnSlot { offset: 0 },
                }),
                effects: MirEffects::default(),
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&base),
                    index: MirValue::Def(dst_index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(result),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    for temp in [MirTempId(20), MirTempId(21), MirTempId(22), MirTempId(23)] {
        assert!(
            !ops.iter()
                .any(|op| op_def(op).and_then(split_def_as_temp) == Some(temp))
        );
    }
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::AdvanceAddress {
            consumer: DEST_POINTER_PAIR,
            index: MirValue::Def(MirDef::Reg(MirReg::A)),
            scale: 1,
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::Binary {
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(mem),
            ..
        } if *mem == loop_index
    )));
    let counts = stats.aggregate_counts();
    assert_eq!(counts.get("delayed-byte-index-producer").copied(), Some(4));
    assert_eq!(
        counts.get("call-result-ea-preserve-loaded-arg").copied(),
        Some(1)
    );
}

#[test]
fn loaded_arg_call_result_store_rematerializes_simple_delayed_dest_index() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let dst_index = MirDef::VTemp(MirTempId(30));
    let src_index = MirDef::VTemp(MirTempId(31));
    let arg = MirDef::VTemp(MirTempId(32));
    let result = MirDef::VTemp(MirTempId(33));
    let base = MirMem::Global {
        id: crate::nir::SymbolId(108),
        offset: 0,
    };
    let loop_index = MirMem::Global {
        id: crate::nir::SymbolId(5),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: dst_index.clone(),
                src: MirAddr::Direct(loop_index.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: src_index.clone(),
                src: MirAddr::Direct(loop_index.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: arg.clone(),
                src: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&base),
                    index: MirValue::Def(src_index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
            MirOp::Call {
                target: MirCallTarget::Builtin {
                    name: "KnownByte".to_string(),
                    address: Some(0x4000),
                },
                abi: MirCallAbi {
                    params: vec![MirArgHome::Reg(MirReg::A)],
                    result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: vec![MirCallArg {
                    value: MirValue::Def(arg),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                }],
                result: Some(MirCallResult {
                    dst: result.clone(),
                    width: MirWidth::Byte,
                    home: MirResultHome::ReturnSlot { offset: 0 },
                }),
                effects: MirEffects::default(),
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&base),
                    index: MirValue::Def(dst_index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(result),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    for temp in [MirTempId(30), MirTempId(31)] {
        assert!(
            !ops.iter()
                .any(|op| op_def(op).and_then(split_def_as_temp) == Some(temp))
        );
    }
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::AdvanceAddress {
            consumer: DEST_POINTER_PAIR,
            index: MirValue::Def(MirDef::Reg(MirReg::A)),
            scale: 1,
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::StoreIndirect {
            consumer: DEST_POINTER_PAIR,
            offset: 0,
            ..
        }
    )));
}

#[test]
fn indexed_byte_copy_rematerializes_delayed_byte_index_producers() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let dst_index_load = MirDef::VTemp(MirTempId(30));
    let dst_index = MirDef::VTemp(MirTempId(31));
    let src_index = MirDef::VTemp(MirTempId(32));
    let value = MirDef::VTemp(MirTempId(33));
    let dst_base = MirMem::Global {
        id: crate::nir::SymbolId(108),
        offset: 0,
    };
    let src_base = MirMem::Global {
        id: crate::nir::SymbolId(85),
        offset: 0,
    };
    let loop_index = MirMem::Global {
        id: crate::nir::SymbolId(5),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: dst_index_load.clone(),
                src: MirAddr::Direct(loop_index.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Sub,
                dst: dst_index.clone(),
                left: MirValue::Def(dst_index_load.clone()),
                right: MirValue::ConstU8(2),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Load {
                dst: src_index.clone(),
                src: MirAddr::Direct(loop_index.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: value.clone(),
                src: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&src_base),
                    index: MirValue::Def(src_index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&dst_base),
                    index: MirValue::Def(dst_index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(value),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(!ops.iter().any(|op| {
        matches!(
            op,
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Spill { .. }),
                ..
            }
        )
    }));
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, MirOp::AdvanceAddress { .. }))
            .count(),
        2
    );
    let counts = stats.aggregate_counts();
    assert_eq!(counts.get("delayed-byte-index-producer").copied(), Some(3));
    assert_eq!(counts.get("indexed-byte-copy").copied(), Some(1));
}

#[test]
fn byte_rsh8_store_consumer_rematerializes_delayed_dest_index() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let index_load = MirDef::VTemp(MirTempId(30));
    let index = MirDef::VTemp(MirTempId(31));
    let word = MirDef::VTemp(MirTempId(32));
    let shifted = MirDef::VTemp(MirTempId(33));
    let dst_base = MirMem::Global {
        id: crate::nir::SymbolId(108),
        offset: 0,
    };
    let loop_index = MirMem::Global {
        id: crate::nir::SymbolId(5),
        offset: 0,
    };
    let word_source = MirMem::Global {
        id: crate::nir::SymbolId(109),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: index_load.clone(),
                src: MirAddr::Direct(loop_index.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: index.clone(),
                left: MirValue::Def(index_load.clone()),
                right: MirValue::ConstU8(24),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Load {
                dst: word.clone(),
                src: MirAddr::Direct(word_source),
                width: MirWidth::Word,
            },
            MirOp::Binary {
                op: MirBinaryOp::Rsh,
                dst: shifted.clone(),
                left: MirValue::Def(word),
                right: MirValue::ConstU8(8),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&dst_base),
                    index: MirValue::Def(index.clone()),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(shifted),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    for temp in [MirTempId(30), MirTempId(31)] {
        assert!(
            !ops.iter()
                .any(|op| op_def(op).and_then(split_def_as_temp) == Some(temp))
        );
    }
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(24),
            width: MirWidth::Byte,
            ..
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::Move {
            dst: MirDef::Reg(MirReg::Y),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY {
                zp: MirFixedZpSlot(POINTER_SCRATCH_LO),
            },
            ..
        }
    )));
    assert!(!ops.iter().any(|op| {
        matches!(
            op,
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Spill { .. }),
                ..
            }
        )
    }));
    let counts = stats.aggregate_counts();
    assert_eq!(counts.get("delayed-byte-index-producer").copied(), Some(2));
    assert_eq!(counts.get("byte-store-consumer").copied(), Some(1));
}

#[test]
fn delayed_byte_indexed_store_with_offset_advances_address() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let index = MirDef::VTemp(MirTempId(30));
    let base = MirMem::Global {
        id: crate::nir::SymbolId(108),
        offset: 0,
    };
    let loop_index = MirMem::Global {
        id: crate::nir::SymbolId(5),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: index.clone(),
                src: MirAddr::Direct(loop_index),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&base),
                    index: MirValue::Def(index),
                    elem_size: 1,
                    offset: 3,
                },
                src: MirValue::ConstU8(0x55),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(
        !ops.iter()
            .any(|op| op_def(op).and_then(split_def_as_temp) == Some(MirTempId(30)))
    );
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::AdvanceAddress {
            consumer: DEFAULT_POINTER_PAIR,
            index: MirValue::Def(MirDef::Reg(MirReg::A)),
            scale: 1,
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::StoreIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            offset: 3,
            ..
        }
    )));
    assert!(!ops.iter().any(|op| matches!(
        op,
        MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY { .. },
            ..
        }
    )));
}

#[test]
fn delayed_byte_indexed_read_with_offset_advances_address() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let index = MirDef::VTemp(MirTempId(30));
    let value = MirDef::VTemp(MirTempId(31));
    let base = MirMem::Global {
        id: crate::nir::SymbolId(108),
        offset: 0,
    };
    let loop_index = MirMem::Global {
        id: crate::nir::SymbolId(5),
        offset: 0,
    };
    let sink = MirMem::Global {
        id: crate::nir::SymbolId(109),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: index.clone(),
                src: MirAddr::Direct(loop_index),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: value.clone(),
                src: MirAddr::ComputedIndex {
                    base: pointer_value_from_mem(&base),
                    index: MirValue::Def(index),
                    elem_size: 1,
                    offset: 3,
                },
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(sink),
                src: MirValue::Def(value),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(
        !ops.iter()
            .any(|op| op_def(op).and_then(split_def_as_temp) == Some(MirTempId(30)))
    );
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::AdvanceAddress {
            consumer: DEFAULT_POINTER_PAIR,
            index: MirValue::Def(MirDef::Reg(MirReg::A)),
            scale: 1,
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::LoadIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            offset: 3,
            ..
        }
    )));
    assert!(!ops.iter().any(|op| matches!(
        op,
        MirOp::Load {
            src: MirAddr::FixedIndirectIndexedY { .. },
            ..
        }
    )));
}

#[test]
fn delayed_byte_indexed_read_helper_matches_computed_and_pointer_index() {
    let index_source = MirMem::Global {
        id: crate::nir::SymbolId(5),
        offset: 0,
    };
    let base_pointer = MirMem::Global {
        id: crate::nir::SymbolId(108),
        offset: 0,
    };

    let computed_ops = materialize_delayed_indexed_read_for_helper_boundary(
        index_source.clone(),
        MirAddr::ComputedIndex {
            base: pointer_value_from_mem(&base_pointer),
            index: MirValue::Def(MirDef::VTemp(MirTempId(30))),
            elem_size: 1,
            offset: 3,
        },
    );
    let pointer_ops = materialize_delayed_indexed_read_for_helper_boundary(
        index_source,
        MirAddr::PointerIndex {
            ptr: base_pointer,
            index: MirValue::Def(MirDef::VTemp(MirTempId(30))),
            elem_size: 1,
            offset: 3,
        },
    );

    assert_eq!(computed_ops, pointer_ops);
    assert!(computed_ops.iter().any(|op| matches!(
        op,
        MirOp::AdvanceAddress {
            consumer: DEFAULT_POINTER_PAIR,
            index: MirValue::Def(MirDef::Reg(MirReg::A)),
            scale: 1,
        }
    )));
    assert!(computed_ops.iter().any(|op| matches!(
        op,
        MirOp::LoadIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            offset: 3,
            ..
        }
    )));
}

#[test]
fn delayed_byte_indexed_store_helper_matches_computed_and_pointer_index() {
    let index_source = MirMem::Global {
        id: crate::nir::SymbolId(5),
        offset: 0,
    };
    let base_pointer = MirMem::Global {
        id: crate::nir::SymbolId(108),
        offset: 0,
    };

    let computed_ops = materialize_delayed_indexed_store_for_helper_boundary(
        index_source.clone(),
        MirAddr::ComputedIndex {
            base: pointer_value_from_mem(&base_pointer),
            index: MirValue::Def(MirDef::VTemp(MirTempId(30))),
            elem_size: 1,
            offset: 3,
        },
    );
    let pointer_ops = materialize_delayed_indexed_store_for_helper_boundary(
        index_source,
        MirAddr::PointerIndex {
            ptr: base_pointer,
            index: MirValue::Def(MirDef::VTemp(MirTempId(30))),
            elem_size: 1,
            offset: 3,
        },
    );

    assert_eq!(computed_ops, pointer_ops);
    assert!(computed_ops.iter().any(|op| matches!(
        op,
        MirOp::AdvanceAddress {
            consumer: DEFAULT_POINTER_PAIR,
            index: MirValue::Def(MirDef::Reg(MirReg::A)),
            scale: 1,
        }
    )));
    assert!(computed_ops.iter().any(|op| matches!(
        op,
        MirOp::StoreIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            offset: 3,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
        }
    )));
}

fn materialize_delayed_indexed_read_for_helper_boundary(
    index_source: MirMem,
    src: MirAddr,
) -> Vec<MirOp> {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let index = MirDef::VTemp(MirTempId(30));
    let value = MirDef::VTemp(MirTempId(31));
    let sink = MirMem::Global {
        id: crate::nir::SymbolId(109),
        offset: 0,
    };

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: index,
                src: MirAddr::Direct(index_source),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: value.clone(),
                src,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(sink),
                src: MirValue::Def(value),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    let counts = stats.aggregate_counts();
    assert_eq!(counts.get("delayed-byte-index-producer").copied(), Some(1));
    assert_eq!(counts.get("delayed-byte-index-consumer").copied(), Some(1));
    ops
}

fn materialize_delayed_indexed_store_for_helper_boundary(
    index_source: MirMem,
    dst: MirAddr,
) -> Vec<MirOp> {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let index = MirDef::VTemp(MirTempId(30));

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Load {
                dst: index,
                src: MirAddr::Direct(index_source),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst,
                src: MirValue::ConstU8(0x55),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    let counts = stats.aggregate_counts();
    assert_eq!(counts.get("delayed-byte-index-producer").copied(), Some(1));
    assert_eq!(counts.get("delayed-byte-index-consumer").copied(), Some(1));
    ops
}

#[test]
fn call_result_store_keeps_existing_path_when_call_may_clobber_prepared_address() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let result = MirDef::VTemp(MirTempId(12));

    let ops = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        vec![
            MirOp::Call {
                target: MirCallTarget::Routine(RoutineId(1)),
                abi: MirCallAbi {
                    params: Vec::new(),
                    result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: Vec::new(),
                result: Some(MirCallResult {
                    dst: result.clone(),
                    width: MirWidth::Byte,
                    home: MirResultHome::ReturnSlot { offset: 0 },
                }),
                effects: MirEffects {
                    memory_writes: MirMemoryEffect::Unknown,
                    ..MirEffects::default()
                },
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: MirValue::ConstU16(0x4000),
                    index: MirValue::ConstU8(3),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(result),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(matches!(
        ops.first(),
        Some(MirOp::Call { result: None, .. })
    ));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::MaterializeAddress {
            consumer: DEFAULT_POINTER_PAIR,
            ..
        } | MirOp::MaterializeIndexedAddress {
            consumer: DEFAULT_POINTER_PAIR,
            ..
        }
    )));
    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::StoreIndirect {
            consumer: DEFAULT_POINTER_PAIR,
            ..
        }
    )));
    assert!(!ops.iter().any(|op| matches!(
        op,
        MirOp::MaterializeAddress {
            consumer: DEST_POINTER_PAIR,
            ..
        } | MirOp::MaterializeIndexedAddress {
            consumer: DEST_POINTER_PAIR,
            ..
        } | MirOp::StoreIndirect {
            consumer: DEST_POINTER_PAIR,
            ..
        }
    )));
    let counts = stats.aggregate_counts();
    assert_eq!(
        counts.get("call-result-ea-preserve-candidate").copied(),
        Some(1)
    );
    assert_eq!(
        counts
            .get("call-result-ea-preserve-blocked-clobber")
            .copied(),
        Some(1)
    );
}

#[test]
fn param_forwarding_rewrites_indirect_call_targets() {
    let param_lo = MirMem::Param {
        id: ParamId(0),
        offset: 0,
    };
    let param_hi = MirMem::Param {
        id: ParamId(0),
        offset: 1,
    };
    let ops = vec![
        MirOp::Store {
            dst: MirAddr::Direct(param_hi.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::X)),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(param_lo.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Call {
            target: MirCallTarget::Indirect {
                target: MirValue::Word {
                    lo: Box::new(MirValue::PointerCell(param_lo)),
                    hi: Box::new(MirValue::PointerCell(param_hi)),
                },
                width: MirWidth::Word,
            },
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects::default(),
        },
    ];

    let rewritten = forward_param_register_homes(ops);

    assert!(matches!(
        rewritten.get(2),
        Some(MirOp::Call {
            target:
                MirCallTarget::Indirect {
                    target:
                        MirValue::Word {
                            lo,
                            hi,
                        },
                    ..
                },
            ..
        }) if **lo == MirValue::Def(MirDef::Reg(MirReg::A))
            && **hi == MirValue::Def(MirDef::Reg(MirReg::X))
    ));
}

#[test]
fn ssa_lite_scanner_kills_memory_facts_on_non_direct_store() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src = MirMem::FixedZeroPage(MirFixedZpSlot(0x5C));
    let tmp = MirMem::ZeroPage(MirZpSlot(0));
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(src),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(tmp.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY {
                zp: MirFixedZpSlot(0xAC),
            },
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ];

    let env = scan_ssa_lite_block_env(&ops, &layout);

    assert_eq!(env.reg_fact(MirReg::A), None);
    assert!(env.mem_fact(&tmp).is_none());
    assert_eq!(env.stats.learned, 2);
    assert_eq!(env.stats.killed, 2);
}

#[test]
fn dead_private_scratch_store_keeps_values_live_into_successors() {
    let spill = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let store = MirOp::Store {
        dst: MirAddr::Direct(spill.clone()),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    };

    assert_eq!(
        dead_private_scratch_store_at(
            std::slice::from_ref(&store),
            0,
            &MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        ),
        None
    );

    assert_eq!(
        dead_private_scratch_store_at(
            &[
                store.clone(),
                MirOp::Store {
                    dst: MirAddr::Direct(spill),
                    src: MirValue::ConstU8(0),
                    width: MirWidth::Byte,
                },
            ],
            0,
            &MirTerminator::Jump(MirEdge::plain(MirBlockId(1)))
        ),
        Some(1)
    );
}

#[test]
fn staged_rhs_keeps_private_scratch_values_live_into_successors() {
    let rhs_source = MirMem::Global {
        id: crate::nir::SymbolId(1),
        offset: 0,
    };
    let rhs_slot = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let left_source = MirMem::Global {
        id: crate::nir::SymbolId(2),
        offset: 0,
    };
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(rhs_source),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(rhs_slot.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(left_source),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            dst: MirCondDest::Flags,
            op: MirCompareOp::Eq,
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(rhs_slot),
            width: MirWidth::Byte,
            signed: false,
        },
    ];

    assert!(staged_compare_rhs_at(&ops, 0, &MirTerminator::Return).is_some());
    assert_eq!(
        staged_compare_rhs_at(&ops, 0, &MirTerminator::Jump(MirEdge::plain(MirBlockId(1)))),
        None
    );
}

#[test]
fn spill_store_reload_pair_keeps_values_live_into_successors() {
    let spill = MirSpillId(30);
    let ops = vec![
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill {
                id: spill,
                offset: 0,
            }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Spill {
                id: spill,
                offset: 0,
            }),
            width: MirWidth::Byte,
        },
    ];

    assert!(can_remove_spill_store_reload_pair_at(
        &ops,
        0,
        &MirTerminator::Return
    ));
    assert!(!can_remove_spill_store_reload_pair_at(
        &ops,
        0,
        &MirTerminator::Jump(MirEdge::plain(MirBlockId(1)))
    ));
}

#[test]
fn ssa_lite_forwards_memory_facts_over_single_forward_predecessor() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let mut routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(source.clone()),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(scratch.clone()),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "next".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Load {
                dst: MirDef::Reg(MirReg::X),
                src: MirAddr::Direct(scratch),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Return,
        },
    ]);
    let mut stats = MirPeepholeStats::default();

    fold_ssa_lite_single_predecessor_loads(&mut routine, &layout, &mut stats);

    assert!(matches!(
        routine.blocks[1].ops.first(),
        Some(MirOp::Load {
            dst: MirDef::Reg(MirReg::X),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }) if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-cross-block-forwards")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-cross-block-seeds")
            .copied(),
        Some(1)
    );
}

#[test]
fn ssa_lite_does_not_forward_memory_facts_into_joins() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let mut routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "left".to_string(),
            params: Vec::new(),
            ops: vec![
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(source),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(scratch.clone()),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(2))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "right".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(2))),
        },
        MirBlock {
            id: MirBlockId(2),
            label: "join".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Load {
                dst: MirDef::Reg(MirReg::X),
                src: MirAddr::Direct(scratch.clone()),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Return,
        },
    ]);
    let mut stats = MirPeepholeStats::default();

    fold_ssa_lite_single_predecessor_loads(&mut routine, &layout, &mut stats);

    assert!(matches!(
        routine.blocks[2].ops.first(),
        Some(MirOp::Load {
            dst: MirDef::Reg(MirReg::X),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }) if *mem == scratch
    ));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("ssa-lite-cross-block-forwards")
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-cross-block-join-skipped")
            .copied(),
        Some(1)
    );
}

#[test]
fn ssa_lite_carries_memory_facts_through_linear_chains() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let mut routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(source.clone()),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(scratch.clone()),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "middle".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(2))),
        },
        MirBlock {
            id: MirBlockId(2),
            label: "exit".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Load {
                dst: MirDef::Reg(MirReg::X),
                src: MirAddr::Direct(scratch),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Return,
        },
    ]);
    let mut stats = MirPeepholeStats::default();

    fold_ssa_lite_single_predecessor_loads(&mut routine, &layout, &mut stats);

    assert!(matches!(
        routine.blocks[2].ops.first(),
        Some(MirOp::Load {
            dst: MirDef::Reg(MirReg::X),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }) if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-cross-block-forwards")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-cross-block-seeds")
            .copied(),
        Some(2)
    );
}

#[test]
fn temp_liveness_tracks_lane_use_in_successor() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define".to_string(),
            params: Vec::new(),
            ops: vec![
                MirOp::Load {
                    dst: MirDef::VTempByte { id, byte: 0 },
                    src: MirAddr::Direct(MirMem::Local {
                        id: LocalId(1),
                        offset: 0,
                    }),
                    width: MirWidth::Byte,
                },
                MirOp::LoadImm {
                    dst: MirDef::VTempByte { id, byte: 1 },
                    value: 0,
                    width: MirWidth::Byte,
                },
            ],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "compare".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::VTempByte { id, byte: 0 }),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            }],
            terminator: MirTerminator::Return,
        },
    ]);

    let liveness = analyze_temp_liveness(&routine);

    assert!(
        liveness
            .block(0)
            .expect("entry block")
            .live_out
            .exact_lane_live(id, 0)
    );
    assert!(
        !liveness
            .block(0)
            .expect("entry block")
            .live_out
            .exact_lane_live(id, 1)
    );
    assert!(
        liveness
            .block(1)
            .expect("successor block")
            .live_in
            .exact_lane_live(id, 0)
    );
}

#[test]
fn temp_liveness_tracks_full_temp_use_in_successor() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Load {
                dst: MirDef::VTempByte { id, byte: 0 },
                src: MirAddr::Direct(MirMem::Local {
                    id: LocalId(1),
                    offset: 0,
                }),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "branch".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
                then_edge: MirEdge::plain(MirBlockId(2)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
        },
        MirBlock {
            id: MirBlockId(2),
            label: "exit".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ]);

    let liveness = analyze_temp_liveness(&routine);

    assert!(
        liveness
            .block(0)
            .expect("entry block")
            .live_out
            .full_temp_live(id)
    );
    assert!(
        liveness
            .block(1)
            .expect("successor block")
            .live_in
            .full_temp_live(id)
    );
    let entry_live_in = &liveness.block(0).expect("entry block").live_in;
    assert!(!entry_live_in.full_temp_live(id));
    assert!(!entry_live_in.exact_lane_live(id, 0));
    assert!(entry_live_in.exact_lane_live(id, 1));
}

#[test]
fn temp_liveness_kills_full_temp_use_after_both_lanes_are_defined() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define".to_string(),
            params: Vec::new(),
            ops: vec![
                MirOp::LoadImm {
                    dst: MirDef::VTempByte { id, byte: 0 },
                    value: 1,
                    width: MirWidth::Byte,
                },
                MirOp::LoadImm {
                    dst: MirDef::VTempByte { id, byte: 1 },
                    value: 2,
                    width: MirWidth::Byte,
                },
            ],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "branch".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
                then_edge: MirEdge::plain(MirBlockId(2)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
        },
        MirBlock {
            id: MirBlockId(2),
            label: "exit".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ]);

    let liveness = analyze_temp_liveness(&routine);

    assert!(
        liveness
            .block(0)
            .expect("entry block")
            .live_out
            .full_temp_live(id)
    );
    let entry_live_in = &liveness.block(0).expect("entry block").live_in;
    assert!(!entry_live_in.full_temp_live(id));
    assert!(!entry_live_in.exact_lane_live(id, 0));
    assert!(!entry_live_in.exact_lane_live(id, 1));
}

#[test]
fn temp_liveness_narrows_full_use_to_low_lane_after_high_lane_def() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define_high".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::LoadImm {
                dst: MirDef::VTempByte { id, byte: 1 },
                value: 2,
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "branch".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
                then_edge: MirEdge::plain(MirBlockId(2)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
        },
        MirBlock {
            id: MirBlockId(2),
            label: "exit".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ]);

    let liveness = analyze_temp_liveness(&routine);

    assert!(
        liveness
            .block(0)
            .expect("entry block")
            .live_out
            .full_temp_live(id)
    );
    let entry_live_in = &liveness.block(0).expect("entry block").live_in;
    assert!(!entry_live_in.full_temp_live(id));
    assert!(entry_live_in.exact_lane_live(id, 0));
    assert!(!entry_live_in.exact_lane_live(id, 1));
}

#[test]
fn temp_liveness_join_preserves_full_requirement_from_one_successor() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
        },
        MirBlock {
            id: MirBlockId(1),
            label: "use_low".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::VTempByte { id, byte: 0 }),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
        },
        MirBlock {
            id: MirBlockId(2),
            label: "use_full".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
                then_edge: MirEdge::plain(MirBlockId(3)),
                else_edge: MirEdge::plain(MirBlockId(3)),
            },
        },
        MirBlock {
            id: MirBlockId(3),
            label: "exit".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ]);

    let liveness = analyze_temp_liveness(&routine);

    assert!(
        liveness
            .block(0)
            .expect("entry block")
            .live_out
            .full_temp_live(id)
    );
    let low_use_live_in = &liveness.block(1).expect("low-use block").live_in;
    assert!(!low_use_live_in.full_temp_live(id));
    assert!(low_use_live_in.exact_lane_live(id, 0));
    assert!(
        liveness
            .block(2)
            .expect("full-use block")
            .live_in
            .full_temp_live(id)
    );
}

#[test]
fn temp_liveness_loop_converges_with_lane_definition_in_preheader() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define_high".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::LoadImm {
                dst: MirDef::VTempByte { id, byte: 1 },
                value: 2,
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "loop".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(2)),
            },
        },
        MirBlock {
            id: MirBlockId(2),
            label: "exit".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ]);

    let liveness = analyze_temp_liveness(&routine);

    let entry_live_in = &liveness.block(0).expect("entry block").live_in;
    assert!(!entry_live_in.full_temp_live(id));
    assert!(entry_live_in.exact_lane_live(id, 0));
    assert!(!entry_live_in.exact_lane_live(id, 1));
    assert!(
        liveness
            .block(1)
            .expect("loop block")
            .live_in
            .full_temp_live(id)
    );
}

#[test]
fn temp_liveness_word_op_after_low_def_requires_only_high_lane() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![MirBlock {
        id: MirBlockId(0),
        label: "compare".to_string(),
        params: Vec::new(),
        ops: vec![
            MirOp::LoadImm {
                dst: MirDef::VTempByte { id, byte: 0 },
                value: 1,
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::VTemp(id)),
                right: MirValue::ConstU16(0),
                width: MirWidth::Word,
                signed: false,
            },
        ],
        terminator: MirTerminator::Return,
    }]);

    let liveness = analyze_temp_liveness(&routine);

    let live_in = &liveness.block(0).expect("entry block").live_in;
    assert!(!live_in.full_temp_live(id));
    assert!(!live_in.exact_lane_live(id, 0));
    assert!(live_in.exact_lane_live(id, 1));
}

#[test]
fn temp_liveness_word_op_after_high_def_requires_only_low_lane() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![MirBlock {
        id: MirBlockId(0),
        label: "compare".to_string(),
        params: Vec::new(),
        ops: vec![
            MirOp::LoadImm {
                dst: MirDef::VTempByte { id, byte: 1 },
                value: 2,
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::VTemp(id)),
                right: MirValue::ConstU16(0),
                width: MirWidth::Word,
                signed: false,
            },
        ],
        terminator: MirTerminator::Return,
    }]);

    let liveness = analyze_temp_liveness(&routine);

    let live_in = &liveness.block(0).expect("entry block").live_in;
    assert!(!live_in.full_temp_live(id));
    assert!(live_in.exact_lane_live(id, 0));
    assert!(!live_in.exact_lane_live(id, 1));
}

#[test]
fn temp_liveness_word_terminator_after_lane_def_requires_only_missing_lane() {
    let id = MirTempId(9);
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "branch".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::LoadImm {
                dst: MirDef::VTempByte { id, byte: 0 },
                value: 1,
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Branch {
                cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
                then_edge: MirEdge::plain(MirBlockId(1)),
                else_edge: MirEdge::plain(MirBlockId(1)),
            },
        },
        MirBlock {
            id: MirBlockId(1),
            label: "exit".to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator: MirTerminator::Return,
        },
    ]);

    let liveness = analyze_temp_liveness(&routine);

    let live_in = &liveness.block(0).expect("entry block").live_in;
    assert!(!live_in.full_temp_live(id));
    assert!(!live_in.exact_lane_live(id, 0));
    assert!(live_in.exact_lane_live(id, 1));
}

#[test]
fn ssa_lite_forwards_accumulator_loads_through_scratch() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(source.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(scratch.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(scratch),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert_eq!(rewritten.len(), 2);
    assert_eq!(
        rewritten
            .iter()
            .filter(|op| matches!(
                op,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(mem),
                    width: MirWidth::Byte,
                } if *mem == source
            ))
            .count(),
        1
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-redundant-reloads")
            .copied(),
        Some(1)
    );
}

#[test]
fn ssa_lite_keeps_x_reload_for_call_argument_staging() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let ops = vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 0x2D,
            width: MirWidth::Byte,
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects::default(),
        },
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 0x2D,
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert_eq!(
        rewritten
            .iter()
            .filter(|op| matches!(
                op,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::X),
                    value: 0x2D,
                    width: MirWidth::Byte,
                }
            ))
            .count(),
        2
    );
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("ssa-lite-redundant-reloads")
    );
}

#[test]
fn ssa_lite_removes_x_reload_inside_no_call_window() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let ops = vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 0x2D,
            width: MirWidth::Byte,
        },
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 0x2D,
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert_eq!(
        rewritten
            .iter()
            .filter(|op| matches!(
                op,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::X),
                    value: 0x2D,
                    width: MirWidth::Byte,
                }
            ))
            .count(),
        1
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-redundant-reloads")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-redundant-reloads-call-free")
            .copied(),
        Some(1)
    );
}

#[test]
fn ssa_lite_keeps_accumulator_reload_after_calls() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let ops = vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0x42,
            width: MirWidth::Byte,
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects::default(),
        },
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0x42,
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert_eq!(
        rewritten
            .iter()
            .filter(|op| matches!(
                op,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 0x42,
                    width: MirWidth::Byte,
                }
            ))
            .count(),
        2
    );
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("ssa-lite-redundant-reloads")
    );
}

#[test]
fn ssa_lite_reports_reload_retained_at_call_boundary() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let ops = vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0x42,
            width: MirWidth::Byte,
        },
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0x42,
            width: MirWidth::Byte,
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects::default(),
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert_eq!(
        rewritten
            .iter()
            .filter(|op| matches!(
                op,
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 0x42,
                    width: MirWidth::Byte,
                }
            ))
            .count(),
        2
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-reload-retained-call-barrier")
            .copied(),
        Some(1)
    );
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("ssa-lite-redundant-reloads")
    );
}

#[test]
fn ssa_lite_keeps_redundant_reload_when_flags_feed_branch() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(source.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(scratch.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(scratch),
            width: MirWidth::Byte,
        },
    ];
    let terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(MirFlagTest::ZSet),
        then_edge: MirEdge::plain(MirBlockId(1)),
        else_edge: MirEdge::plain(MirBlockId(2)),
    };
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(ops, RoutineId(0), &layout, &terminator, &mut stats);

    assert!(matches!(
        rewritten.last(),
        Some(MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }) if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-reload-retained-flags")
            .copied(),
        Some(1)
    );
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("ssa-lite-redundant-reloads")
    );
}

#[test]
fn dead_private_scratch_store_removes_non_accumulator_stores() {
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let ops = vec![MirOp::Store {
        dst: MirAddr::Direct(scratch),
        src: MirValue::Def(MirDef::Reg(MirReg::X)),
        width: MirWidth::Byte,
    }];
    let mut stats = MirPeepholeStats::default();

    let rewritten =
        fold_dead_private_scratch_stores(ops, RoutineId(0), &MirTerminator::Return, &mut stats);

    assert!(rewritten.is_empty());
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-dead-scratch-stores")
            .copied(),
        Some(1)
    );
}

#[test]
fn dead_private_scratch_store_reports_live_out_retention() {
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let ops = vec![MirOp::Store {
        dst: MirAddr::Direct(scratch),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_dead_private_scratch_stores(
        ops,
        RoutineId(0),
        &MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        &mut stats,
    );

    assert_eq!(rewritten.len(), 1);
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-store-retained-live-out")
            .copied(),
        Some(1)
    );
}

#[test]
fn dead_reg_write_before_overwrite_removes_unused_loads() {
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(0),
                offset: 0,
            }),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(1),
                offset: 0,
            }),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_dead_reg_writes_before_overwrite(
        ops,
        RoutineId(0),
        &MirTerminator::Return,
        &mut stats,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        rewritten.as_slice(),
        [MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(1),
                offset: 0
            }),
            width: MirWidth::Byte,
        }]
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-dead-reg-writes")
            .copied(),
        Some(1)
    );
}

#[test]
fn dead_reg_write_before_overwrite_keeps_used_accumulator() {
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(0),
                offset: 0,
            }),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Local {
                id: LocalId(1),
                offset: 0,
            }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(2),
                offset: 0,
            }),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_dead_reg_writes_before_overwrite(
        ops,
        RoutineId(0),
        &MirTerminator::Return,
        &mut stats,
    );

    assert_eq!(rewritten.len(), 3);
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("ssa-lite-dead-reg-writes")
    );
}

#[test]
fn dead_private_scratch_store_keeps_memory_source_stores() {
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let source = MirMem::Absolute(0x02FC);
    let ops = vec![MirOp::Store {
        dst: MirAddr::Direct(scratch),
        src: MirValue::PointerCell(source),
        width: MirWidth::Byte,
    }];
    let mut stats = MirPeepholeStats::default();

    let rewritten =
        fold_dead_private_scratch_stores(ops, RoutineId(0), &MirTerminator::Return, &mut stats);

    assert_eq!(rewritten.len(), 1);
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("ssa-lite-dead-scratch-stores")
    );
}

#[test]
fn ssa_lite_forwards_scratch_sources_into_byte_consumers() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let sink = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(source.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(scratch.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            dst: MirCondDest::Flags,
            op: MirCompareOp::Eq,
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(scratch.clone()),
            width: MirWidth::Byte,
            signed: false,
        },
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(scratch.clone()),
            width: MirWidth::Byte,
            carry_in: Some(MirCarryIn::Clear),
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Store {
            dst: MirAddr::Direct(sink),
            src: MirValue::PointerCell(scratch),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert!(matches!(
        rewritten.get(2),
        Some(MirOp::Compare {
            right: MirValue::PointerCell(mem),
            ..
        }) if *mem == source
    ));
    assert!(matches!(
        rewritten.get(3),
        Some(MirOp::Binary {
            right: MirValue::PointerCell(mem),
            ..
        }) if *mem == source
    ));
    assert!(matches!(
        rewritten.get(4),
        Some(MirOp::Store {
            src: MirValue::PointerCell(mem),
            ..
        }) if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-consumer-forwards")
            .copied(),
        Some(3)
    );
}

#[test]
fn ssa_lite_forwards_constant_scratch_sources_into_byte_consumers() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let ops = vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 7,
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(scratch.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Compare {
            dst: MirCondDest::Flags,
            op: MirCompareOp::Eq,
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::PointerCell(scratch),
            width: MirWidth::Byte,
            signed: false,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert!(matches!(
        rewritten.get(2),
        Some(MirOp::Compare {
            right: MirValue::ConstU8(7),
            ..
        })
    ));
}

#[test]
fn ssa_lite_forwards_scratch_sources_into_register_moves() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let ops = vec![
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(source.clone()),
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(scratch.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Move {
            dst: MirDef::Reg(MirReg::X),
            src: MirValue::PointerCell(scratch),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert!(matches!(
        rewritten.get(2),
        Some(MirOp::Load {
            dst: MirDef::Reg(MirReg::X),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }) if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-consumer-forwards")
            .copied(),
        Some(1)
    );
}

#[test]
fn ssa_lite_forwards_constant_scratch_sources_into_register_moves() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let ops = vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 19,
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(scratch.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
        MirOp::Move {
            dst: MirDef::Reg(MirReg::Y),
            src: MirValue::PointerCell(scratch),
            width: MirWidth::Byte,
        },
    ];
    let mut stats = MirPeepholeStats::default();

    let rewritten = fold_ssa_lite_byte_loads(
        ops,
        RoutineId(0),
        &layout,
        &MirTerminator::Return,
        &mut stats,
    );

    assert!(matches!(
        rewritten.get(2),
        Some(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::Y),
            value: 19,
            width: MirWidth::Byte,
        })
    ));
}

#[test]
fn ssa_lite_forwards_byte_consumers_over_linear_edges() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let scratch = MirMem::Spill {
        id: MirSpillId(30),
        offset: 0,
    };
    let mut routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "entry".to_string(),
            params: Vec::new(),
            ops: vec![
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(source.clone()),
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(scratch.clone()),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "compare".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::Reg(MirReg::A)),
                right: MirValue::PointerCell(scratch),
                width: MirWidth::Byte,
                signed: false,
            }],
            terminator: MirTerminator::Return,
        },
    ]);
    let mut stats = MirPeepholeStats::default();

    fold_ssa_lite_single_predecessor_loads(&mut routine, &layout, &mut stats);

    assert!(matches!(
        routine.blocks[1].ops.first(),
        Some(MirOp::Compare {
            right: MirValue::PointerCell(mem),
            ..
        }) if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("ssa-lite-cross-block-forwards")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_classifier_accepts_forwardable_byte_values() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let local = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let temp = MirDef::VTemp(MirTempId(3));

    assert_eq!(
        classify_mir_copy_prop_byte_value(&MirValue::ConstU8(7), &layout),
        Some(MirCopyPropByteValue::ConstU8(7))
    );
    assert_eq!(
        classify_mir_copy_prop_byte_value(&MirValue::ConstU16(255), &layout),
        Some(MirCopyPropByteValue::ConstU8(255))
    );
    assert_eq!(
        classify_mir_copy_prop_byte_value(&MirValue::Def(temp.clone()), &layout),
        Some(MirCopyPropByteValue::Temp(temp))
    );
    assert_eq!(
        classify_mir_copy_prop_byte_value(&MirValue::PointerCell(local.clone()), &layout),
        Some(MirCopyPropByteValue::DirectMem(local))
    );
}

#[test]
fn mir_copy_prop_classifier_rejects_unsafe_or_non_byte_values() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);

    assert_eq!(
        classify_mir_copy_prop_byte_value(&MirValue::ConstU16(256), &layout),
        None
    );
    assert_eq!(
        classify_mir_copy_prop_byte_value(&MirValue::Def(MirDef::Reg(MirReg::A)), &layout),
        None
    );
    assert_eq!(
        classify_mir_copy_prop_byte_value(
            &MirValue::PointerCell(MirMem::Absolute(0xD01F)),
            &layout
        ),
        None
    );
    assert_eq!(
        classify_mir_copy_prop_byte_value(
            &MirValue::Word {
                lo: Box::new(MirValue::ConstU8(1)),
                hi: Box::new(MirValue::ConstU8(0)),
            },
            &layout
        ),
        None
    );
}

#[test]
fn ssa_lite_v2_observes_temp_alias_copy_prop_potential() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let temp = MirDef::VTemp(MirTempId(7));
    let stats = scan_ssa_lite_v2_observability(
        &[
            MirOp::Load {
                dst: temp.clone(),
                src: MirAddr::Direct(source),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(0))),
                src: MirValue::Def(temp),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
    );

    assert_eq!(stats.temp_aliases_learned, 1);
    assert_eq!(stats.replaceable_temp_uses, 1);
    assert_eq!(stats.copy_prop_candidates, 1);
}

#[test]
fn ssa_lite_v2_observes_memory_forward_potential_and_store_kills() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let scratch = MirMem::ZeroPage(MirZpSlot(0));
    let stats = scan_ssa_lite_v2_observability(
        &[
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(scratch.clone()),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::X),
                src: MirAddr::Direct(scratch.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(source),
                src: MirValue::ConstU8(3),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
    );

    assert_eq!(stats.mem_facts_learned, 2);
    assert_eq!(stats.replaceable_loads, 1);
    assert_eq!(stats.memory_forward_candidates, 1);
    assert!(stats.facts_killed_by_store > 0);
}

#[test]
fn ssa_lite_v2_observes_repeated_address_setup() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let consumer = fixed_pointer_consumer(POINTER_SCRATCH_LO);
    let value = MirValue::PointerCell(MirMem::Local {
        id: LocalId(1),
        offset: 0,
    });
    let op = MirOp::MaterializeAddress { consumer, value };
    let stats = scan_ssa_lite_v2_observability(&[op.clone(), op], RoutineId(0), &layout);

    assert_eq!(stats.address_facts_learned, 1);
    assert_eq!(stats.address_reuse_candidates, 1);
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_byte_compare() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTemp(MirTempId(7));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: temp.clone(),
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Compare {
            left: MirValue::ConstU8(7),
            right: MirValue::ConstU8(0),
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_direct_mem_temp_into_byte_compare() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let temp = MirDef::VTemp(MirTempId(8));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::Load {
                dst: temp.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Compare {
            left: MirValue::PointerCell(mem),
            right: MirValue::ConstU8(0),
            ..
        } if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_direct_mem_temp_into_direct_byte_store() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let destination = MirMem::Local {
        id: LocalId(2),
        offset: 0,
    };
    let temp = MirDef::VTemp(MirTempId(79));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: temp.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(destination.clone()),
                src: MirValue::Def(temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        MirOp::Store {
            dst: MirAddr::Direct(dst),
            src: MirValue::PointerCell(src),
            width: MirWidth::Byte,
        } if *dst == destination && *src == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_block_local_temp_alias_and_removes_transient_def() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirDef::VTemp(MirTempId(80));
    let alias = MirDef::VTemp(MirTempId(81));
    let destination = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: source.clone(),
                src: MirAddr::Direct(MirMem::Absolute(0xD01F)),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: alias.clone(),
                src: MirValue::Def(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(destination.clone()),
                src: MirValue::Def(alias),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 2);
    assert!(matches!(
        &ops[1],
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::Def(def),
            width: MirWidth::Byte,
        } if *mem == destination && *def == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-temp-alias-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_invalidates_alias_when_source_temp_is_redefined() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirDef::VTemp(MirTempId(82));
    let alias = MirDef::VTemp(MirTempId(83));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::Load {
                dst: source.clone(),
                src: MirAddr::Direct(MirMem::Absolute(0xD01F)),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: alias.clone(),
                src: MirValue::Def(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: source,
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Local {
                    id: LocalId(1),
                    offset: 0,
                }),
                src: MirValue::Def(alias.clone()),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[3],
        MirOp::Store {
            src: MirValue::Def(def),
            ..
        } if *def == alias
    ));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-temp-alias-uses")
    );
}

#[test]
fn mir_copy_prop_removes_dead_forwarded_direct_mem_temp_def() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let temp = MirDef::VTemp(MirTempId(8));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: temp.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        MirOp::Compare {
            left: MirValue::PointerCell(mem),
            right: MirValue::ConstU8(0),
            ..
        } if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_keeps_temp_def_used_by_terminator() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTemp(MirTempId(7));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![MirOp::LoadImm {
            dst: temp.clone(),
            value: 1,
            width: MirWidth::Byte,
        }],
        &MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::Def(temp)),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        },
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(ops[0], MirOp::LoadImm { .. }));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-dead-temp-defs")
    );
}

#[test]
fn mir_copy_prop_keeps_full_temp_def_live_into_successor() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let id = MirTempId(7);
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::LoadImm {
                dst: MirDef::VTemp(id),
                value: 1,
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "compare".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::VTemp(id)),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            }],
            terminator: MirTerminator::Return,
        },
    ]);
    let liveness = analyze_temp_liveness(&routine);
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator_and_live_out(
        routine.blocks[0].ops.clone(),
        &routine.blocks[0].terminator,
        liveness.live_out(0).expect("entry live-out"),
        routine.blocks[0].id,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(ops[0], MirOp::LoadImm { .. }));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-dead-temp-defs")
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-def-blocked-successor-live")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_observes_dead_forwarded_temp_byte_def_candidate() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp_byte = MirDef::VTempByte {
        id: MirTempId(9),
        byte: 0,
    };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::LoadImm {
                dst: temp_byte.clone(),
                value: 3,
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp_byte),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        MirOp::Compare {
            left: MirValue::ConstU8(3),
            right: MirValue::ConstU8(0),
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-candidates")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-load-imm-candidates")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-lane-safe-candidates")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-lane-safe-byte0-candidates")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-lane-safe-load-imm-candidates")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-load-imm-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_blocks_temp_byte_def_used_by_full_temp_terminator() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let id = MirTempId(9);
    let temp_byte = MirDef::VTempByte { id, byte: 0 };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![MirOp::LoadImm {
            dst: temp_byte,
            value: 3,
            width: MirWidth::Byte,
        }],
        &MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        },
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(ops[0], MirOp::LoadImm { .. }));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-dead-temp-byte-def-candidates")
    );
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-dead-temp-byte-def-load-imm-candidates")
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-blocked-full-temp-live")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-blocked-byte0")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_blocks_temp_byte_def_with_exact_lane_use() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp_byte = MirDef::VTempByte {
        id: MirTempId(9),
        byte: 1,
    };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::LoadImm {
                dst: temp_byte.clone(),
                value: 3,
                width: MirWidth::Byte,
            },
            MirOp::Barrier {
                effects: MirEffects::default(),
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp_byte),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 3);
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-blocked-exact-lane-live")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-blocked-byte1")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_blocks_temp_byte_def_with_sibling_lane_use() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let id = MirTempId(9);
    let lo = MirDef::VTempByte { id, byte: 0 };
    let hi = MirDef::VTempByte { id, byte: 1 };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::LoadImm {
                dst: lo,
                value: 3,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(0))),
                src: MirValue::Def(hi),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 2);
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-blocked-sibling-lane-live")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-blocked-byte0")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_removes_lane_safe_temp_byte_direct_load_def() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let temp_byte = MirDef::VTempByte {
        id: MirTempId(9),
        byte: 0,
    };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: temp_byte.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp_byte),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        MirOp::Compare {
            left: MirValue::PointerCell(mem),
            right: MirValue::ConstU8(0),
            ..
        } if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-load-direct-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_keeps_temp_byte_direct_load_def_live_into_successor() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let id = MirTempId(9);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Load {
                dst: MirDef::VTempByte { id, byte: 0 },
                src: MirAddr::Direct(source),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "compare".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::VTempByte { id, byte: 0 }),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            }],
            terminator: MirTerminator::Return,
        },
    ]);
    let liveness = analyze_temp_liveness(&routine);
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator_and_live_out(
        routine.blocks[0].ops.clone(),
        &routine.blocks[0].terminator,
        liveness.live_out(0).expect("entry live-out"),
        routine.blocks[0].id,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], MirOp::Load { .. }));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-dead-temp-byte-load-direct-defs")
    );
}

#[test]
fn mir_copy_prop_removes_lane_safe_temp_byte_move_def() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirDef::VTempByte {
        id: MirTempId(8),
        byte: 0,
    };
    let dst = MirDef::VTempByte {
        id: MirTempId(9),
        byte: 0,
    };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Move {
                dst,
                src: MirValue::Def(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(source),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], MirOp::Compare { .. }));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-move-candidates")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_removes_lane_safe_temp_byte_binary_def_without_carry_effects() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp_byte = MirDef::VTempByte {
        id: MirTempId(9),
        byte: 0,
    };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst: temp_byte,
                left: MirValue::ConstU8(0xAA),
                right: MirValue::ConstU8(0x55),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::ConstU8(0),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], MirOp::Compare { .. }));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-binary-candidates")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-binary-lane-safe")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_keeps_lane_safe_temp_byte_binary_def_that_produces_carry() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp_byte = MirDef::VTempByte {
        id: MirTempId(9),
        byte: 0,
    };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: temp_byte,
                left: MirValue::ConstU8(1),
                right: MirValue::ConstU8(2),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Produce,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::ConstU8(0),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 2);
    assert!(matches!(&ops[0], MirOp::Binary { .. }));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-binary-candidates")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-lane-safe-binary-candidates")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-binary-blocked-carry-out")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_binary_candidate_reason_prefers_successor_live() {
    let id = MirTempId(9);
    let op = MirOp::Binary {
        op: MirBinaryOp::Add,
        dst: MirDef::VTempByte { id, byte: 0 },
        left: MirValue::ConstU8(1),
        right: MirValue::ConstU8(2),
        width: MirWidth::Byte,
        carry_in: Some(MirCarryIn::Clear),
        carry_out: MirCarryOut::Produce,
    };

    assert_eq!(
        temp_byte_binary_candidate_reason_for_test(&op, id, 0, true, true, false, false),
        "blocked-successor-live"
    );
}

#[test]
fn mir_copy_prop_binary_candidate_reason_reports_carry_blockers() {
    let id = MirTempId(9);
    let carry_out_op = MirOp::Binary {
        op: MirBinaryOp::Add,
        dst: MirDef::VTempByte { id, byte: 0 },
        left: MirValue::ConstU8(1),
        right: MirValue::ConstU8(2),
        width: MirWidth::Byte,
        carry_in: Some(MirCarryIn::Clear),
        carry_out: MirCarryOut::Produce,
    };
    let carry_from_previous_op = MirOp::Binary {
        op: MirBinaryOp::Add,
        dst: MirDef::VTempByte { id, byte: 1 },
        left: MirValue::ConstU8(1),
        right: MirValue::ConstU8(0),
        width: MirWidth::Byte,
        carry_in: Some(MirCarryIn::FromPrevious),
        carry_out: MirCarryOut::Ignore,
    };

    assert_eq!(
        temp_byte_binary_candidate_reason_for_test(
            &carry_out_op,
            id,
            0,
            false,
            false,
            false,
            false
        ),
        "blocked-carry-out"
    );
    assert_eq!(
        temp_byte_binary_candidate_reason_for_test(
            &carry_from_previous_op,
            id,
            1,
            false,
            false,
            false,
            false
        ),
        "blocked-carry-from-previous"
    );
}

#[test]
fn mir_copy_prop_keeps_temp_byte_move_def_live_into_successor() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let id = MirTempId(9);
    let source = MirDef::VTempByte {
        id: MirTempId(8),
        byte: 0,
    };
    let routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Move {
                dst: MirDef::VTempByte { id, byte: 0 },
                src: MirValue::Def(source),
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "compare".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::VTempByte { id, byte: 0 }),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            }],
            terminator: MirTerminator::Return,
        },
    ]);
    let liveness = analyze_temp_liveness(&routine);
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator_and_live_out(
        routine.blocks[0].ops.clone(),
        &routine.blocks[0].terminator,
        liveness.live_out(0).expect("entry live-out"),
        routine.blocks[0].id,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], MirOp::Move { .. }));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-byte-def-blocked-successor-live")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_keeps_temp_byte_direct_load_def_with_exact_lane_use() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let temp_byte = MirDef::VTempByte {
        id: MirTempId(9),
        byte: 0,
    };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: temp_byte.clone(),
                src: MirAddr::Direct(source),
                width: MirWidth::Byte,
            },
            MirOp::Barrier {
                effects: MirEffects::default(),
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp_byte),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 3);
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-dead-temp-byte-load-direct-defs")
    );
}

#[test]
fn mir_copy_prop_keeps_temp_byte_direct_load_def_with_full_temp_terminator_use() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let id = MirTempId(9);
    let temp_byte = MirDef::VTempByte { id, byte: 0 };
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![MirOp::Load {
            dst: temp_byte,
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(1),
                offset: 0,
            }),
            width: MirWidth::Byte,
        }],
        &MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))),
            then_edge: MirEdge::plain(MirBlockId(1)),
            else_edge: MirEdge::plain(MirBlockId(2)),
        },
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-dead-temp-byte-load-direct-defs")
    );
}

#[test]
fn mir_copy_prop_const_compare_stops_at_barrier() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTemp(MirTempId(7));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: temp.clone(),
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::Barrier {
                effects: MirEffects::default(),
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(temp.clone()),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::Compare {
            left: MirValue::Def(def),
            right: MirValue::ConstU8(0),
            ..
        } if *def == temp
    ));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-const-uses")
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_byte_store_src() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirDef::VTemp(MirTempId(8));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: temp.clone(),
                value: 9,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(0))),
                src: MirValue::Def(temp),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Store {
            src: MirValue::ConstU8(9),
            width: MirWidth::Byte,
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_through_byte_move() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let first = MirDef::VTemp(MirTempId(9));
    let second = MirDef::VTemp(MirTempId(10));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: first.clone(),
                value: 5,
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: second.clone(),
                src: MirValue::Def(first),
                width: MirWidth::Byte,
            },
            MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(second),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Move {
            src: MirValue::ConstU8(5),
            width: MirWidth::Byte,
            ..
        }
    ));
    assert!(matches!(
        &ops[2],
        MirOp::Compare {
            left: MirValue::ConstU8(5),
            right: MirValue::ConstU8(0),
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(2)
    );
}

#[test]
fn mir_copy_prop_forwards_direct_mem_temp_into_register_move_and_removes_home() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let temp = MirDef::VTemp(MirTempId(86));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: temp.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(temp),
                width: MirWidth::Byte,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
        } if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_keeps_captured_register_move_across_call_barrier() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let temp = MirDef::VTemp(MirTempId(87));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::Load {
                dst: temp.clone(),
                src: MirAddr::Direct(source),
                width: MirWidth::Byte,
            },
            MirOp::Call {
                target: MirCallTarget::Routine(RoutineId(1)),
                abi: MirCallAbi {
                    params: Vec::new(),
                    result: None,
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: Vec::new(),
                result: None,
                effects: MirEffects::default(),
            },
            MirOp::Move {
                dst: MirDef::Reg(MirReg::A),
                src: MirValue::Def(temp.clone()),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::Def(def),
            width: MirWidth::Byte,
        } if *def == temp
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        None
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_extend_src() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src = MirDef::VTemp(MirTempId(11));
    let dst = MirDef::VTemp(MirTempId(12));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: src.clone(),
                value: 0x7F,
                width: MirWidth::Byte,
            },
            MirOp::Extend {
                dst,
                src: MirValue::Def(src),
                from_width: MirWidth::Byte,
                to_width: MirWidth::Word,
                signed: false,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Extend {
            src: MirValue::ConstU8(0x7F),
            from_width: MirWidth::Byte,
            to_width: MirWidth::Word,
            signed: false,
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_bytes_into_truncate_src() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirDef::VTempByte {
        id: MirTempId(13),
        byte: 0,
    };
    let hi = MirDef::VTempByte {
        id: MirTempId(13),
        byte: 1,
    };
    let dst = MirDef::VTemp(MirTempId(14));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: lo.clone(),
                value: 0x34,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: hi.clone(),
                value: 0x12,
                width: MirWidth::Byte,
            },
            MirOp::Truncate {
                dst,
                src: MirValue::Word {
                    lo: Box::new(MirValue::Def(lo)),
                    hi: Box::new(MirValue::Def(hi)),
                },
                from_width: MirWidth::Word,
                to_width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::Truncate {
            src: MirValue::Word { lo, hi },
            from_width: MirWidth::Word,
            to_width: MirWidth::Byte,
            ..
        } if **lo == MirValue::ConstU8(0x34) && **hi == MirValue::ConstU8(0x12)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(2)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temps_into_byte_binary() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let left = MirDef::VTemp(MirTempId(11));
    let right = MirDef::VTemp(MirTempId(12));
    let dst = MirDef::VTemp(MirTempId(13));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: left.clone(),
                value: 2,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: right.clone(),
                value: 3,
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst,
                left: MirValue::Def(left),
                right: MirValue::Def(right),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::Binary {
            left: MirValue::ConstU8(2),
            right: MirValue::ConstU8(3),
            width: MirWidth::Byte,
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(2)
    );
}

#[test]
fn mir_copy_prop_forwards_direct_mem_left_temp_into_byte_binary() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let left_mem = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let left = MirDef::VTemp(MirTempId(11));
    let dst = MirDef::VTemp(MirTempId(13));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::Load {
                dst: left.clone(),
                src: MirAddr::Direct(left_mem.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Xor,
                dst,
                left: MirValue::Def(left),
                right: MirValue::ConstU8(0x0F),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Binary {
            left: MirValue::PointerCell(left),
            right: MirValue::ConstU8(0x0F),
            width: MirWidth::Byte,
            ..
        } if *left == left_mem
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_keeps_direct_mem_binary_temp_after_pointer_scratch_setup() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::FixedZeroPage(MirFixedZpSlot(POINTER_SCRATCH_LO));
    let temp = MirDef::VTemp(MirTempId(11));
    let dst = MirDef::VTemp(MirTempId(12));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::Load {
                dst: temp.clone(),
                src: MirAddr::Direct(source),
                width: MirWidth::Byte,
            },
            MirOp::MaterializeAddress {
                consumer: fixed_pointer_consumer(POINTER_SCRATCH_LO),
                value: MirValue::ConstU16(0x4000),
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst,
                left: MirValue::ConstU8(1),
                right: MirValue::Def(temp.clone()),
                width: MirWidth::Byte,
                carry_in: Some(MirCarryIn::Clear),
                carry_out: MirCarryOut::Ignore,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::Binary {
            right: MirValue::Def(def),
            width: MirWidth::Byte,
            ..
        } if *def == temp
    ));
    assert!(
        !stats
            .aggregate_counts()
            .contains_key("mir-copy-prop-const-uses")
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_byte_unary() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src = MirDef::VTemp(MirTempId(14));
    let dst = MirDef::VTemp(MirTempId(15));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: src.clone(),
                value: 0x0F,
                width: MirWidth::Byte,
            },
            MirOp::Unary {
                op: MirUnaryOp::BitNot,
                dst,
                src: MirValue::Def(src),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Unary {
            src: MirValue::ConstU8(0x0F),
            width: MirWidth::Byte,
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_direct_mem_temp_into_byte_unary() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let src = MirDef::VTemp(MirTempId(14));
    let dst = MirDef::VTemp(MirTempId(15));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::Load {
                dst: src.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Unary {
                op: MirUnaryOp::BitNot,
                dst,
                src: MirValue::Def(src),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Unary {
            src: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
            ..
        } if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_byte_to_word_mem_update() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let value = MirDef::VTemp(MirTempId(16));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: value.clone(),
                value: 4,
                width: MirWidth::Byte,
            },
            MirOp::AddByteToWordMem {
                mem: MirMem::ZeroPage(MirZpSlot(1)),
                value: MirValue::Def(value),
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::AddByteToWordMem {
            value: MirValue::ConstU8(4),
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_indirect_store_src() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let src = MirDef::VTemp(MirTempId(17));
    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(MirZpSlot(2)));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: src.clone(),
                value: 6,
                width: MirWidth::Byte,
            },
            MirOp::StoreIndirect {
                consumer,
                src: MirValue::Def(src),
                offset: 0,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::StoreIndirect {
            src: MirValue::ConstU8(6),
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_address_advance_index() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let index = MirDef::VTemp(MirTempId(18));
    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(MirZpSlot(3)));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: index.clone(),
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::AdvanceAddress {
                consumer,
                index: MirValue::Def(index),
                scale: 1,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::AdvanceAddress {
            index: MirValue::ConstU8(7),
            scale: 1,
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_direct_mem_temp_into_address_advance_and_removes_home() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Param {
        id: ParamId(0),
        offset: 0,
    };
    let index = MirDef::VTemp(MirTempId(90));
    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(MirZpSlot(3)));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: index.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::AdvanceAddress {
                consumer,
                index: MirValue::Def(index),
                scale: 3,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        MirOp::AdvanceAddress {
            index: MirValue::PointerCell(mem),
            scale: 3,
            ..
        } if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_indexed_address_index() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let index = MirDef::VTemp(MirTempId(19));
    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(MirZpSlot(4)));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: index.clone(),
                value: 8,
                width: MirWidth::Byte,
            },
            MirOp::MaterializeIndexedAddress {
                consumer,
                base: MirValue::ConstU16(0x4000),
                index: MirValue::Def(index),
                scale: 1,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::MaterializeIndexedAddress {
            index: MirValue::ConstU8(8),
            scale: 1,
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_direct_mem_temp_into_indexed_address_and_removes_home() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let index = MirDef::VTemp(MirTempId(88));
    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(MirZpSlot(4)));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: index.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::MaterializeIndexedAddress {
                consumer,
                base: MirValue::ConstU16(0x4000),
                index: MirValue::Def(index),
                scale: 2,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        MirOp::MaterializeIndexedAddress {
            index: MirValue::PointerCell(mem),
            scale: 2,
            ..
        } if *mem == source
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_stages_index_before_overwriting_same_fixed_zp_pointer() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::FixedZeroPage(MirFixedZpSlot(0x80));
    let index = MirDef::VTemp(MirTempId(89));
    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
        lo: MirFixedZpSlot(0x80),
    });
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: index.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::MaterializeIndexedAddress {
                consumer,
                base: MirValue::ConstU16(0x4000),
                index: MirValue::Def(index),
                scale: 1,
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );
    let mut spills = Vec::new();
    let ops = materialize_temp_ops(ops, &mut spills);

    assert!(spills.is_empty());
    assert!(matches!(
        ops.as_slice(),
        [
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            },
            MirOp::MaterializeIndexedAddress {
                consumer: staged_consumer,
                index: MirValue::Def(MirDef::Reg(MirReg::A)),
                scale: 1,
                ..
            }
        ] if *mem == source && *staged_consumer == consumer
    ));
}

#[test]
fn mir_copy_prop_forwards_const_temp_bytes_into_indexed_address_base() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirDef::VTempByte {
        id: MirTempId(20),
        byte: 0,
    };
    let hi = MirDef::VTempByte {
        id: MirTempId(20),
        byte: 1,
    };
    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(MirZpSlot(5)));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: lo.clone(),
                value: 0x78,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: hi.clone(),
                value: 0x56,
                width: MirWidth::Byte,
            },
            MirOp::MaterializeIndexedAddress {
                consumer,
                base: MirValue::Word {
                    lo: Box::new(MirValue::Def(lo)),
                    hi: Box::new(MirValue::Def(hi)),
                },
                index: MirValue::ConstU8(2),
                scale: 1,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::MaterializeIndexedAddress {
            base: MirValue::Word { lo, hi },
            index: MirValue::ConstU8(2),
            ..
        } if **lo == MirValue::ConstU8(0x78) && **hi == MirValue::ConstU8(0x56)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(2)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_pointer_index_load() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let index = MirDef::VTemp(MirTempId(21));
    let dst = MirDef::VTemp(MirTempId(22));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: index.clone(),
                value: 3,
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst,
                src: MirAddr::PointerIndex {
                    ptr: MirMem::ZeroPage(MirZpSlot(6)),
                    index: MirValue::Def(index),
                    elem_size: 1,
                    offset: 0,
                },
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Load {
            src: MirAddr::PointerIndex {
                index: MirValue::ConstU8(3),
                elem_size: 1,
                offset: 0,
                ..
            },
            width: MirWidth::Byte,
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_computed_index_load() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let index = MirDef::VTemp(MirTempId(23));
    let dst = MirDef::VTemp(MirTempId(24));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: index.clone(),
                value: 5,
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst,
                src: MirAddr::ComputedIndex {
                    base: MirValue::ConstU16(0x4000),
                    index: MirValue::Def(index),
                    elem_size: 2,
                    offset: 1,
                },
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Load {
            src: MirAddr::ComputedIndex {
                base: MirValue::ConstU16(0x4000),
                index: MirValue::ConstU8(5),
                elem_size: 2,
                offset: 1,
            },
            width: MirWidth::Byte,
            ..
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_bytes_into_computed_index_load_base() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirDef::VTempByte {
        id: MirTempId(25),
        byte: 0,
    };
    let hi = MirDef::VTempByte {
        id: MirTempId(25),
        byte: 1,
    };
    let index = MirDef::VTemp(MirTempId(26));
    let dst = MirDef::VTemp(MirTempId(27));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: lo.clone(),
                value: 0x00,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: hi.clone(),
                value: 0x42,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: index.clone(),
                value: 5,
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst,
                src: MirAddr::ComputedIndex {
                    base: MirValue::Word {
                        lo: Box::new(MirValue::Def(lo)),
                        hi: Box::new(MirValue::Def(hi)),
                    },
                    index: MirValue::Def(index),
                    elem_size: 2,
                    offset: 1,
                },
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[3],
        MirOp::Load {
            src: MirAddr::ComputedIndex {
                base: MirValue::Word { lo, hi },
                index: MirValue::ConstU8(5),
                elem_size: 2,
                offset: 1,
            },
            width: MirWidth::Byte,
            ..
        } if **lo == MirValue::ConstU8(0x00) && **hi == MirValue::ConstU8(0x42)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(3)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_bytes_into_deref_load_pointer() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirDef::VTempByte {
        id: MirTempId(28),
        byte: 0,
    };
    let hi = MirDef::VTempByte {
        id: MirTempId(28),
        byte: 1,
    };
    let dst = MirDef::VTemp(MirTempId(29));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: lo.clone(),
                value: 0x34,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: hi.clone(),
                value: 0x12,
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst,
                src: MirAddr::Deref {
                    ptr: MirValue::Word {
                        lo: Box::new(MirValue::Def(lo)),
                        hi: Box::new(MirValue::Def(hi)),
                    },
                    offset: 2,
                },
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::Load {
            src: MirAddr::Deref {
                ptr: MirValue::Word { lo, hi },
                offset: 2,
            },
            width: MirWidth::Byte,
            ..
        } if **lo == MirValue::ConstU8(0x34) && **hi == MirValue::ConstU8(0x12)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(2)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temps_into_deref_store() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirDef::VTempByte {
        id: MirTempId(30),
        byte: 0,
    };
    let hi = MirDef::VTempByte {
        id: MirTempId(30),
        byte: 1,
    };
    let src = MirDef::VTemp(MirTempId(31));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: lo.clone(),
                value: 0x78,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: hi.clone(),
                value: 0x56,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: src.clone(),
                value: 0x9A,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Deref {
                    ptr: MirValue::Word {
                        lo: Box::new(MirValue::Def(lo)),
                        hi: Box::new(MirValue::Def(hi)),
                    },
                    offset: 3,
                },
                src: MirValue::Def(src),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[3],
        MirOp::Store {
            dst: MirAddr::Deref {
                ptr: MirValue::Word { lo, hi },
                offset: 3,
            },
            src: MirValue::ConstU8(0x9A),
            width: MirWidth::Byte,
        } if **lo == MirValue::ConstU8(0x78) && **hi == MirValue::ConstU8(0x56)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(3)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temps_into_computed_index_store() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirDef::VTempByte {
        id: MirTempId(25),
        byte: 0,
    };
    let hi = MirDef::VTempByte {
        id: MirTempId(25),
        byte: 1,
    };
    let index = MirDef::VTemp(MirTempId(26));
    let src = MirDef::VTemp(MirTempId(27));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: lo.clone(),
                value: 0x00,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: hi.clone(),
                value: 0x41,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: index.clone(),
                value: 6,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: src.clone(),
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: MirValue::Word {
                        lo: Box::new(MirValue::Def(lo)),
                        hi: Box::new(MirValue::Def(hi)),
                    },
                    index: MirValue::Def(index),
                    elem_size: 2,
                    offset: 1,
                },
                src: MirValue::Def(src.clone()),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[4],
        MirOp::Store {
            dst: MirAddr::ComputedIndex {
                base: MirValue::Word { lo, hi },
                index: MirValue::ConstU8(6),
                elem_size: 2,
                offset: 1,
            },
            src: MirValue::ConstU8(7),
            width: MirWidth::Byte,
        } if **lo == MirValue::ConstU8(0x00) && **hi == MirValue::ConstU8(0x41)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(4)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_pointer_index_store() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let index = MirDef::VTemp(MirTempId(23));
    let src = MirDef::VTemp(MirTempId(24));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: index.clone(),
                value: 4,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: src.clone(),
                value: 5,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::PointerIndex {
                    ptr: MirMem::ZeroPage(MirZpSlot(7)),
                    index: MirValue::Def(index),
                    elem_size: 1,
                    offset: 0,
                },
                src: MirValue::Def(src.clone()),
                width: MirWidth::Byte,
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::Store {
            dst: MirAddr::PointerIndex {
                index: MirValue::ConstU8(4),
                elem_size: 1,
                offset: 0,
                ..
            },
            src: MirValue::ConstU8(5),
            width: MirWidth::Byte,
        }
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(2)
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_into_byte_call_arg() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let arg_temp = MirDef::VTemp(MirTempId(20));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: arg_temp.clone(),
                value: 0x2A,
                width: MirWidth::Byte,
            },
            MirOp::Call {
                target: MirCallTarget::Routine(RoutineId(1)),
                abi: MirCallAbi {
                    params: vec![MirArgHome::Reg(MirReg::A)],
                    result: None,
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: vec![MirCallArg {
                    value: MirValue::Def(arg_temp),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                }],
                result: None,
                effects: MirEffects::default(),
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[1],
        MirOp::Call {
            args,
            ..
        } if matches!(args.as_slice(), [MirCallArg {
            value: MirValue::ConstU8(0x2A),
            width: MirWidth::Byte,
            home: MirArgHome::Reg(MirReg::A),
        }])
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_forwards_direct_mem_temp_into_byte_call_arg_and_removes_home() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let arg_temp = MirDef::VTemp(MirTempId(84));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses_with_terminator(
        vec![
            MirOp::Load {
                dst: arg_temp.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Call {
                target: MirCallTarget::Routine(RoutineId(1)),
                abi: MirCallAbi {
                    params: vec![MirArgHome::Reg(MirReg::A)],
                    result: None,
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: vec![MirCallArg {
                    value: MirValue::Def(arg_temp),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                }],
                result: None,
                effects: MirEffects::default(),
            },
        ],
        &MirTerminator::Return,
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        MirOp::Call {
            args,
            ..
        } if matches!(args.as_slice(), [MirCallArg {
            value: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
            home: MirArgHome::Reg(MirReg::A),
        }] if *mem == source)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        Some(1)
    );
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-dead-temp-defs")
            .copied(),
        Some(1)
    );
}

#[test]
fn mir_copy_prop_keeps_captured_call_arg_after_source_memory_changes() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(1),
        offset: 0,
    };
    let arg_temp = MirDef::VTemp(MirTempId(85));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::Load {
                dst: arg_temp.clone(),
                src: MirAddr::Direct(source.clone()),
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(source),
                src: MirValue::ConstU8(7),
                width: MirWidth::Byte,
            },
            MirOp::Call {
                target: MirCallTarget::Routine(RoutineId(1)),
                abi: MirCallAbi {
                    params: vec![MirArgHome::Reg(MirReg::A)],
                    result: None,
                    clobbers: MirRegisterSet::default(),
                    preserves: MirRegisterSet::default(),
                },
                args: vec![MirCallArg {
                    value: MirValue::Def(arg_temp.clone()),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                }],
                result: None,
                effects: MirEffects::default(),
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::Call {
            args,
            ..
        } if matches!(args.as_slice(), [MirCallArg {
            value: MirValue::Def(value),
            width: MirWidth::Byte,
            home: MirArgHome::Reg(MirReg::A),
        }] if *value == arg_temp)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-direct-mem-uses")
            .copied(),
        None
    );
}

#[test]
fn mir_copy_prop_forwards_const_temp_bytes_into_materialize_address_word() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let lo = MirDef::VTempByte {
        id: MirTempId(21),
        byte: 0,
    };
    let hi = MirDef::VTempByte {
        id: MirTempId(21),
        byte: 1,
    };
    let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Virtual(MirZpSlot(5)));
    let mut stats = MirPeepholeStats::default();

    let ops = fold_mir_copy_prop_const_uses(
        vec![
            MirOp::LoadImm {
                dst: lo.clone(),
                value: 0x34,
                width: MirWidth::Byte,
            },
            MirOp::LoadImm {
                dst: hi.clone(),
                value: 0x12,
                width: MirWidth::Byte,
            },
            MirOp::MaterializeAddress {
                consumer,
                value: MirValue::Word {
                    lo: Box::new(MirValue::Def(lo)),
                    hi: Box::new(MirValue::Def(hi)),
                },
            },
        ],
        RoutineId(0),
        &layout,
        &mut stats,
    );

    assert!(matches!(
        &ops[2],
        MirOp::MaterializeAddress {
            value: MirValue::Word { lo, hi },
            ..
        } if **lo == MirValue::ConstU8(0x34) && **hi == MirValue::ConstU8(0x12)
    ));
    assert_eq!(
        stats
            .aggregate_counts()
            .get("mir-copy-prop-const-uses")
            .copied(),
        Some(2)
    );
}

#[test]
fn pre_home_fixed_point_removes_newly_dead_producer_on_second_change_round() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let source = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let source_temp = MirTempId(0);
    let dead_temp = MirTempId(1);
    let mut routine = ssa_lite_edge_test_routine(vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".to_string(),
        params: Vec::new(),
        ops: vec![
            MirOp::Load {
                dst: MirDef::VTemp(source_temp),
                src: MirAddr::Direct(source),
                width: MirWidth::Byte,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::VTempByte {
                    id: dead_temp,
                    byte: 0,
                },
                left: MirValue::Def(MirDef::VTemp(source_temp)),
                right: MirValue::ConstU8(1),
                width: MirWidth::Byte,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
        ],
        terminator: MirTerminator::Return,
    }]);
    routine.temps = vec![MirTemp { id: source_temp }, MirTemp { id: dead_temp }];
    let mut stats = MirPeepholeStats::default();

    let result = run_pre_home_cleanup_fixed_point(&mut routine, &layout, &mut stats);

    assert!(routine.blocks[0].ops.is_empty());
    assert_eq!(result.change_rounds, 2);
    assert_eq!(result.rounds, 3);
    assert_eq!(result.removed_ops, 2);
    assert!(result.converged);
    assert_eq!(
        routine.temps,
        vec![MirTemp { id: source_temp }, MirTemp { id: dead_temp }]
    );
}

#[test]
fn pre_materialization_forwards_whole_storage_address_into_indexed_store() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let address_temp = MirTempId(0);
    let target = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let mut routine = ssa_lite_edge_test_routine(vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".to_string(),
        params: Vec::new(),
        ops: vec![
            MirOp::LeaAddr {
                dst: MirDef::VTemp(address_temp),
                target: target.clone(),
                width: MirWidth::Word,
            },
            MirOp::Store {
                dst: MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(address_temp)),
                    index: MirValue::ConstU8(3),
                    elem_size: 2,
                    offset: 0,
                },
                src: MirValue::ConstU16(0x1234),
                width: MirWidth::Word,
            },
        ],
        terminator: MirTerminator::Return,
    }]);
    routine.temps = vec![MirTemp { id: address_temp }];

    cleanup_pre_materialization_temp_artifacts(&mut routine, &layout);

    assert_eq!(routine.blocks[0].ops.len(), 1);
    assert!(matches!(
        &routine.blocks[0].ops[0],
        MirOp::Store {
            dst: MirAddr::ComputedIndex { base, .. },
            ..
        } if *base == storage_address_value(&target)
    ));
}

#[test]
fn pre_materialization_does_not_forward_storage_address_into_arithmetic() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let address_temp = MirTempId(0);
    let result_temp = MirTempId(1);
    let mut routine = ssa_lite_edge_test_routine(vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".to_string(),
        params: Vec::new(),
        ops: vec![
            MirOp::LeaAddr {
                dst: MirDef::VTemp(address_temp),
                target: MirMem::Local {
                    id: LocalId(0),
                    offset: 0,
                },
                width: MirWidth::Word,
            },
            MirOp::Binary {
                op: MirBinaryOp::Add,
                dst: MirDef::VTemp(result_temp),
                left: MirValue::Def(MirDef::VTemp(address_temp)),
                right: MirValue::ConstU16(2),
                width: MirWidth::Word,
                carry_in: None,
                carry_out: MirCarryOut::Ignore,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Local {
                    id: LocalId(1),
                    offset: 0,
                }),
                src: MirValue::Def(MirDef::VTemp(result_temp)),
                width: MirWidth::Word,
            },
        ],
        terminator: MirTerminator::Return,
    }]);
    routine.temps = vec![MirTemp { id: address_temp }, MirTemp { id: result_temp }];

    cleanup_pre_materialization_temp_artifacts(&mut routine, &layout);

    assert!(routine.blocks[0].ops.iter().any(|op| matches!(
        op,
        MirOp::LeaAddr {
            dst: MirDef::VTemp(id),
            ..
        } if *id == address_temp
    )));
    assert!(routine.blocks[0].ops.iter().any(|op| matches!(
        op,
        MirOp::Binary {
            left: MirValue::Def(MirDef::VTemp(id)),
            ..
        } if *id == address_temp
    )));
}

#[test]
fn return_slot_word_result_forwards_to_repeated_next_call_args() {
    let temp = MirTempId(0);
    let value = MirValue::Def(MirDef::VTemp(temp));
    let first = MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(1)),
        abi: MirCallAbi {
            params: Vec::new(),
            result: Some(MirResultHome::ReturnSlot { offset: 0 }),
            clobbers: MirRegisterSet::default(),
            preserves: MirRegisterSet::default(),
        },
        args: Vec::new(),
        result: Some(MirCallResult {
            dst: MirDef::VTemp(temp),
            width: MirWidth::Word,
            home: MirResultHome::ReturnSlot { offset: 0 },
        }),
        effects: MirEffects::default(),
    };
    let second = MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(2)),
        abi: MirCallAbi {
            params: vec![
                MirArgHome::BytePair {
                    lo: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA0))),
                    hi: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA1))),
                },
                MirArgHome::RegisterPair {
                    lo: MirReg::A,
                    hi: MirReg::X,
                },
            ],
            result: None,
            clobbers: MirRegisterSet::default(),
            preserves: MirRegisterSet::default(),
        },
        args: vec![
            MirCallArg {
                value: value.clone(),
                width: MirWidth::Word,
                home: MirArgHome::BytePair {
                    lo: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA0))),
                    hi: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA1))),
                },
            },
            MirCallArg {
                value,
                width: MirWidth::Word,
                home: MirArgHome::RegisterPair {
                    lo: MirReg::A,
                    hi: MirReg::X,
                },
            },
        ],
        result: None,
        effects: MirEffects::default(),
    };

    let (ops, stats) =
        forward_return_slot_call_result_args(vec![first, second], &MirTerminator::Return);

    assert_eq!(stats.candidates, 1);
    assert_eq!(stats.forwarded, 1);
    assert_eq!(stats.blocked_home_overlap, 0);
    assert!(matches!(&ops[0], MirOp::Call { result: None, .. }));
    let expected = pointer_value_from_mem(&return_slot_mem(0));
    assert!(matches!(
        &ops[1],
        MirOp::Call { args, .. } if args.len() == 2
            && args[0].value == expected
            && args[1].value == expected
    ));
}

#[test]
fn return_slot_result_forward_blocks_overlapping_different_arg_home() {
    let temp = MirTempId(0);
    let first = MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(1)),
        abi: MirCallAbi {
            params: Vec::new(),
            result: Some(MirResultHome::ReturnSlot { offset: 0 }),
            clobbers: MirRegisterSet::default(),
            preserves: MirRegisterSet::default(),
        },
        args: Vec::new(),
        result: Some(MirCallResult {
            dst: MirDef::VTemp(temp),
            width: MirWidth::Word,
            home: MirResultHome::ReturnSlot { offset: 0 },
        }),
        effects: MirEffects::default(),
    };
    let second = MirOp::Call {
        target: MirCallTarget::Routine(RoutineId(2)),
        abi: MirCallAbi {
            params: vec![
                MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA0)),
                MirArgHome::RegisterPair {
                    lo: MirReg::A,
                    hi: MirReg::X,
                },
            ],
            result: None,
            clobbers: MirRegisterSet::default(),
            preserves: MirRegisterSet::default(),
        },
        args: vec![
            MirCallArg {
                value: MirValue::ConstU16(0x1234),
                width: MirWidth::Word,
                home: MirArgHome::FixedZeroPage(MirFixedZpSlot(0xA0)),
            },
            MirCallArg {
                value: MirValue::Def(MirDef::VTemp(temp)),
                width: MirWidth::Word,
                home: MirArgHome::RegisterPair {
                    lo: MirReg::A,
                    hi: MirReg::X,
                },
            },
        ],
        result: None,
        effects: MirEffects::default(),
    };
    let original = vec![first, second];

    let (ops, stats) =
        forward_return_slot_call_result_args(original.clone(), &MirTerminator::Return);

    assert_eq!(stats.candidates, 1);
    assert_eq!(stats.forwarded, 0);
    assert_eq!(stats.blocked_home_overlap, 1);
    assert_eq!(ops, original);
}

#[test]
fn pre_home_fixed_point_removes_dead_word_lane_and_keeps_live_sibling() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let word_temp = MirTempId(0);
    let destination = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let high_def = MirDef::VTempByte {
        id: word_temp,
        byte: 1,
    };
    let mut routine = ssa_lite_edge_test_routine(vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".to_string(),
        params: Vec::new(),
        ops: vec![
            MirOp::LoadImm {
                dst: MirDef::VTempByte {
                    id: word_temp,
                    byte: 0,
                },
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::LoadIndirect {
                consumer: DEFAULT_POINTER_PAIR,
                dst: high_def.clone(),
                offset: 1,
            },
            MirOp::Store {
                dst: MirAddr::Direct(destination),
                src: MirValue::Def(high_def.clone()),
                width: MirWidth::Byte,
            },
        ],
        terminator: MirTerminator::Return,
    }]);
    routine.temps = vec![MirTemp { id: word_temp }];
    let mut stats = MirPeepholeStats::default();

    let result = run_pre_home_cleanup_fixed_point(&mut routine, &layout, &mut stats);

    assert!(result.converged);
    assert!(!routine.blocks[0].ops.iter().any(|op| matches!(
        op,
        MirOp::LoadImm {
            dst: MirDef::VTempByte { id, byte: 0 },
            ..
        } if *id == word_temp
    )));
    assert!(routine.blocks[0].ops.iter().any(|op| matches!(
        op,
        MirOp::LoadIndirect { dst, .. } if *dst == high_def
    )));
    assert!(routine.blocks[0].ops.iter().any(|op| matches!(
        op,
        MirOp::Store {
            src: MirValue::Def(src),
            ..
        } if *src == high_def
    )));
}

#[test]
fn pre_home_fixed_point_preserves_successor_live_temp() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirTempId(0);
    let mut routine = ssa_lite_edge_test_routine(vec![
        MirBlock {
            id: MirBlockId(0),
            label: "define".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::LoadImm {
                dst: MirDef::VTemp(temp),
                value: 1,
                width: MirWidth::Byte,
            }],
            terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
        },
        MirBlock {
            id: MirBlockId(1),
            label: "use".to_string(),
            params: Vec::new(),
            ops: vec![MirOp::Compare {
                dst: MirCondDest::Flags,
                op: MirCompareOp::Eq,
                left: MirValue::Def(MirDef::VTemp(temp)),
                right: MirValue::ConstU8(0),
                width: MirWidth::Byte,
                signed: false,
            }],
            terminator: MirTerminator::Return,
        },
    ]);
    routine.temps = vec![MirTemp { id: temp }];
    let before = routine.blocks.clone();
    let mut stats = MirPeepholeStats::default();

    let result = run_pre_home_cleanup_fixed_point(&mut routine, &layout, &mut stats);

    assert!(result.converged);
    assert_eq!(result.rounds, 1);
    assert_eq!(routine.blocks, before);
}

#[test]
fn pre_home_fixed_point_is_idempotent_after_convergence() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let temp = MirTempId(0);
    let mut routine = ssa_lite_edge_test_routine(vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".to_string(),
        params: Vec::new(),
        ops: vec![MirOp::LoadImm {
            dst: MirDef::VTemp(temp),
            value: 1,
            width: MirWidth::Byte,
        }],
        terminator: MirTerminator::Return,
    }]);
    routine.temps = vec![MirTemp { id: temp }];
    let mut stats = MirPeepholeStats::default();

    let first = run_pre_home_cleanup_fixed_point(&mut routine, &layout, &mut stats);
    let converged_blocks = routine.blocks.clone();
    let second = run_pre_home_cleanup_fixed_point(&mut routine, &layout, &mut stats);

    assert!(first.converged);
    assert!(second.converged);
    assert_eq!(second.rounds, 1);
    assert_eq!(second.change_rounds, 0);
    assert_eq!(routine.blocks, converged_blocks);
}

fn ssa_lite_edge_test_routine(blocks: Vec<MirBlock>) -> MirRoutine {
    MirRoutine {
        id: RoutineId(0),
        name: "Main".to_string(),
        abi: MirRoutineAbi::Action,
        frame: MirFrame::default(),
        temps: Vec::new(),
        blocks,
        effects: MirEffects::default(),
    }
}

fn empty_test_program() -> MirProgram {
    MirProgram {
        statics: Vec::new(),
        globals: Vec::new(),
        routines: vec![MirRoutine {
            id: RoutineId(0),
            name: "Main".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: Vec::new(),
            effects: MirEffects::default(),
        }],
        machine_blocks: Vec::new(),
        runtime_helpers: Vec::new(),
    }
}

#[test]
fn descriptor_lea_materializes_pointer_bytes_for_index_base() {
    let descriptor = MirMem::Local {
        id: LocalId(0),
        offset: 0,
    };
    let mut program = empty_test_program();
    program.routines[0].frame.locals.push(MirStorageSlot {
        id: MirStorageId(0),
        name: Some("items".to_string()),
        width: MirWidth::Word,
        base: MirStorageBase::Local(LocalId(0)),
        offset: 0,
        mutable: true,
        init: Some(MirStorageInit::Descriptor {
            backing: MirStorageBacking {
                bytes: vec![0x34, 0x12, 0x78, 0x56],
                zero_fill: 0,
                section: "local.backing".to_string(),
            },
            descriptor_size: 2,
            size_word: None,
            mutable: true,
            section: "local".to_string(),
        }),
    });
    program.routines[0].temps = vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }];
    program.routines[0].blocks = vec![MirBlock {
        id: MirBlockId(0),
        label: "entry".to_string(),
        params: Vec::new(),
        ops: vec![
            MirOp::LeaAddr {
                dst: MirDef::VTemp(MirTempId(0)),
                target: descriptor.clone(),
                width: MirWidth::Word,
            },
            MirOp::Load {
                dst: MirDef::VTemp(MirTempId(1)),
                src: MirAddr::ComputedIndex {
                    base: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                    index: MirValue::ConstU8(1),
                    elem_size: 2,
                    offset: 0,
                },
                width: MirWidth::Word,
            },
        ],
        terminator: MirTerminator::Return,
    }];

    let materialized = materialize_program(program, &Mir6502Config::default(), 0x3000)
        .expect("descriptor lea materializes");
    let ops = &materialized.routines[0].blocks[0].ops;

    assert!(ops.iter().any(|op| matches!(
        op,
        MirOp::Load {
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
            ..
        } if mem == &descriptor
    )));
    assert!(ops.iter().all(|op| !matches!(
        op,
        MirOp::Move {
            src: MirValue::StorageAddrByte { mem, .. },
            ..
        } if mem == &descriptor
    )));
}

fn byte_scalar_storage_test_program() -> MirProgram {
    let mut program = empty_test_program();
    program.routines[0].frame.params.push(MirStorageSlot {
        id: MirStorageId(0),
        name: Some("p".to_string()),
        width: MirWidth::Byte,
        base: MirStorageBase::Param(ParamId(0)),
        offset: 0,
        mutable: true,
        init: None,
    });
    program.routines[0].frame.locals.push(MirStorageSlot {
        id: MirStorageId(1),
        name: Some("l".to_string()),
        width: MirWidth::Byte,
        base: MirStorageBase::Local(LocalId(0)),
        offset: 0,
        mutable: true,
        init: None,
    });
    program
}

#[test]
fn call_arg_expr_materializes_low_byte_word_add_args() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let ops = vec![
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(13)),
            src: MirAddr::Direct(MirMem::Param {
                id: ParamId(0),
                offset: 0,
            }),
            width: MirWidth::Word,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(14)),
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(4),
                offset: 0,
            }),
            width: MirWidth::Word,
        },
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::VTemp(MirTempId(15)),
            left: MirValue::Def(MirDef::VTemp(MirTempId(13))),
            right: MirValue::Def(MirDef::VTemp(MirTempId(14))),
            width: MirWidth::Word,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(16)),
            src: MirAddr::Direct(MirMem::Param {
                id: ParamId(1),
                offset: 0,
            }),
            width: MirWidth::Word,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(17)),
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(5),
                offset: 0,
            }),
            width: MirWidth::Word,
        },
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::VTemp(MirTempId(18)),
            left: MirValue::Def(MirDef::VTemp(MirTempId(16))),
            right: MirValue::Def(MirDef::VTemp(MirTempId(17))),
            width: MirWidth::Word,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(7)),
            abi: MirCallAbi {
                params: vec![MirArgHome::Reg(MirReg::A), MirArgHome::Reg(MirReg::X)],
                result: Some(MirResultHome::ReturnSlot { offset: 0 }),
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: vec![
                MirCallArg {
                    value: MirValue::Def(MirDef::VTempByte {
                        id: MirTempId(15),
                        byte: 0,
                    }),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                },
                MirCallArg {
                    value: MirValue::Def(MirDef::VTempByte {
                        id: MirTempId(18),
                        byte: 0,
                    }),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::X),
                },
            ],
            result: None,
            effects: MirEffects::default(),
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(20)),
            src: MirAddr::Direct(MirMem::Param {
                id: ParamId(0),
                offset: 0,
            }),
            width: MirWidth::Word,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(21)),
            src: MirAddr::Direct(MirMem::Param {
                id: ParamId(1),
                offset: 0,
            }),
            width: MirWidth::Word,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(22)),
            src: MirAddr::Direct(MirMem::Local {
                id: LocalId(2),
                offset: 0,
            }),
            width: MirWidth::Byte,
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(6)),
            abi: MirCallAbi {
                params: vec![
                    MirArgHome::Reg(MirReg::A),
                    MirArgHome::Reg(MirReg::X),
                    MirArgHome::Reg(MirReg::Y),
                ],
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: vec![
                MirCallArg {
                    value: MirValue::Def(MirDef::VTempByte {
                        id: MirTempId(20),
                        byte: 0,
                    }),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::A),
                },
                MirCallArg {
                    value: MirValue::Def(MirDef::VTempByte {
                        id: MirTempId(21),
                        byte: 0,
                    }),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::X),
                },
                MirCallArg {
                    value: MirValue::Def(MirDef::VTemp(MirTempId(22))),
                    width: MirWidth::Byte,
                    home: MirArgHome::Reg(MirReg::Y),
                },
            ],
            result: None,
            effects: MirEffects::default(),
        },
    ];
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let out = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        ops,
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert!(
        out.iter().all(|op| !matches!(
            op,
            MirOp::Binary {
                width: MirWidth::Word,
                ..
            }
        )),
        "{out:#?}"
    );
    assert!(out.iter().all(|op| !op_uses_temp(op, MirTempId(15))));
    assert!(out.iter().all(|op| !op_uses_temp(op, MirTempId(18))));
    assert!(out.iter().all(|op| !op_uses_temp(op, MirTempId(20))));
    assert!(out.iter().all(|op| !op_uses_temp(op, MirTempId(21))));
    assert!(out.iter().all(|op| !op_uses_temp(op, MirTempId(22))));
}

#[test]
fn call_arg_expr_preserves_byte_mul_high_byte_for_word_arg() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let ops = vec![
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(0)),
            src: MirAddr::Direct(MirMem::Global {
                id: crate::nir::SymbolId(1),
                offset: 0,
            }),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::VTemp(MirTempId(1)),
            left: MirValue::Def(MirDef::VTemp(MirTempId(0))),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(2)),
            src: MirAddr::Direct(MirMem::Global {
                id: crate::nir::SymbolId(2),
                offset: 0,
            }),
            width: MirWidth::Byte,
        },
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::VTemp(MirTempId(3)),
            left: MirValue::Def(MirDef::VTemp(MirTempId(2))),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Binary {
            op: MirBinaryOp::Mul,
            dst: MirDef::VTemp(MirTempId(4)),
            left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            right: MirValue::Def(MirDef::VTemp(MirTempId(3))),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(0)),
            abi: MirCallAbi {
                params: vec![MirArgHome::RegisterPair {
                    lo: MirReg::A,
                    hi: MirReg::X,
                }],
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: vec![MirCallArg {
                value: MirValue::Word {
                    lo: Box::new(MirValue::Def(MirDef::VTemp(MirTempId(4)))),
                    hi: Box::new(MirValue::ConstU8(0)),
                },
                width: MirWidth::Word,
                home: MirArgHome::RegisterPair {
                    lo: MirReg::A,
                    hi: MirReg::X,
                },
            }],
            result: None,
            effects: MirEffects::default(),
        },
    ];
    let mut helpers = Vec::new();
    let mut stats = MirPeepholeStats::default();
    let out = materialize_ops(
        RoutineId(0),
        MirBlockId(0),
        ops,
        &MirTerminator::Return,
        &Mir6502Config::default(),
        &layout,
        &mut helpers,
        &mut stats,
    );

    assert_eq!(helpers, vec![MirRuntimeHelper::Mul]);
    let helper_index = out
        .iter()
        .position(|op| {
            matches!(
                op,
                MirOp::RuntimeHelper {
                    helper: MirRuntimeHelper::Mul,
                    ..
                }
            )
        })
        .expect("expected mul helper");
    let call_index = out
        .iter()
        .position(|op| matches!(op, MirOp::Call { .. }))
        .expect("expected call");
    assert!(helper_index < call_index, "{out:#?}");
    assert!(
        !out[helper_index + 1..call_index].iter().any(|op| matches!(
            op,
            MirOp::Move {
                dst: MirDef::Reg(MirReg::X),
                src: MirValue::ConstU8(0),
                ..
            }
        )),
        "mul high byte in X must flow into the call argument:\n{out:#?}"
    );
    assert!(matches!(
        &out[call_index],
        MirOp::Call {
            args,
            abi,
            ..
        } if args == &vec![
            MirCallArg {
                value: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
                home: MirArgHome::Reg(MirReg::A),
            },
            MirCallArg {
                value: MirValue::Def(MirDef::Reg(MirReg::X)),
                width: MirWidth::Byte,
                home: MirArgHome::Reg(MirReg::X),
            },
        ] && abi.params == vec![MirArgHome::Reg(MirReg::A), MirArgHome::Reg(MirReg::X)]
    ));
}
