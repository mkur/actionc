use super::*;

fn copy_ops(index_width: MirWidth, element_size: u16) -> Vec<MirOp> {
    vec![
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(0)),
            src: MirAddr::Direct(MirMem::Param {
                id: ParamId(0),
                offset: 0,
            }),
            width: index_width,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(1)),
            src: MirAddr::ComputedIndex {
                base: MirValue::ConstU16(0x70F0),
                index: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                elem_size: element_size,
                offset: 3,
            },
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::ComputedIndex {
                base: pointer_value_from_mem(&MirMem::Param {
                    id: ParamId(1),
                    offset: 0,
                }),
                index: MirValue::ConstU8(255),
                elem_size: 1,
                offset: 0,
            },
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        },
    ]
}

#[test]
fn indexed_byte_copy_selects_static_source_without_a_second_pointer() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let ops = copy_ops(MirWidth::Byte, 1);
    let mut out = Vec::new();
    assert_eq!(
        try_fuse_indexed_byte_copy(
            &ops,
            1,
            &layout,
            &indexes::DelayedByteIndexPlan::empty(),
            &mut out
        ),
        2
    );
    assert!(out.iter().any(|op| matches!(
        op,
        MirOp::Load {
            src: MirAddr::AbsoluteIndexedY {
                base: MirMem::Absolute(0x70F3)
            },
            ..
        }
    )));
    assert!(
        !out.iter()
            .any(|op| matches!(op, MirOp::LoadIndirect { .. }))
    );
    assert!(matches!(
        out.last(),
        Some(MirOp::StoreIndirect {
            consumer: DEST_POINTER_PAIR,
            ..
        })
    ));
}

#[test]
fn indexed_byte_copy_keeps_word_indexes_and_scaled_elements_on_the_general_path() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    for (width, size) in [(MirWidth::Word, 1), (MirWidth::Byte, 2)] {
        let ops = copy_ops(width, size);
        let mut out = Vec::new();
        assert_eq!(
            try_fuse_indexed_byte_copy(
                &ops,
                1,
                &layout,
                &indexes::DelayedByteIndexPlan::empty(),
                &mut out
            ),
            2
        );
        assert!(!out.iter().any(|op| matches!(
            op,
            MirOp::Load {
                src: MirAddr::AbsoluteIndexedY { .. },
                ..
            }
        )));
        assert!(
            out.iter()
                .any(|op| matches!(op, MirOp::LoadIndirect { .. }))
        );
    }
}

#[test]
fn indexed_byte_copy_evaluates_delayed_byte_wrapping_before_the_table_address() {
    let program = empty_test_program();
    let layout = MaterializeLayout::new(&program, 0x3000);
    let mut ops = copy_ops(MirWidth::Byte, 1);
    ops.insert(
        1,
        MirOp::Binary {
            op: MirBinaryOp::Add,
            dst: MirDef::VTemp(MirTempId(2)),
            left: MirValue::Def(MirDef::VTemp(MirTempId(0))),
            right: MirValue::ConstU8(1),
            width: MirWidth::Byte,
            carry_in: None,
            carry_out: MirCarryOut::Ignore,
        },
    );
    let MirOp::Load {
        src: MirAddr::ComputedIndex { index, .. },
        ..
    } = &mut ops[2]
    else {
        panic!()
    };
    let delayed_index = MirValue::Def(MirDef::VTemp(MirTempId(2)));
    *index = delayed_index.clone();
    let delayed = indexes::collect_delayed_byte_index_plan(&ops);
    assert!(delayed.expr_for_value(&delayed_index).is_some());
    let mut out = Vec::new();
    assert_eq!(
        try_fuse_indexed_byte_copy(&ops, 2, &layout, &delayed, &mut out),
        2
    );
    assert!(out.iter().any(|op| matches!(
        op,
        MirOp::Binary {
            op: MirBinaryOp::Add,
            width: MirWidth::Byte,
            right: MirValue::ConstU8(1),
            ..
        }
    )));
    assert!(out.iter().any(|op| matches!(
        op,
        MirOp::Load {
            src: MirAddr::AbsoluteIndexedY {
                base: MirMem::Absolute(0x70F3)
            },
            ..
        }
    )));
}
