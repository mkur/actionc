use super::*;
#[test]
fn top_bit_selection_requires_exact_adjacent_sole_use_private_values() {
    let p = word_tests::program();
    let r = &p.routines[0];
    for bytes in [1u8, 2] {
        for case in 0..10 {
            let frame = AllocatedFrame::stack(r).unwrap();
            let mut b = Builder {
                code: TrackedEmitter65816::for_test(&frame),
                frame,
                routine: r,
                blocks: BTreeMap::new(),
                next_block: None,
                loop_x: None,
                borrowed: BTreeMap::new(),
            };
            for (id, offset, width) in [(100, 32, bytes), (101, 40, bytes), (102, 44, 1)] {
                b.frame
                    .temps
                    .insert(TempId(id), Location::Stack(Slot { offset, width }));
            }
            let w = ByteSize::new(bytes.into());
            let constant = if bytes == 1 {
                Mir65816Value::U8(0x80)
            } else {
                Mir65816Value::U16(0x8000)
            };
            let edge = Mir65816Edge {
                target: BlockId(1),
                args: vec![],
            };
            let mut block = Mir65816Block {
                id: BlockId(0),
                params: vec![],
                ops: vec![
                    Mir65816Op::Binary {
                        dest: TempId(101),
                        width: w,
                        signed: false,
                        operation: NirBinaryOp::And,
                        left: Mir65816Value::Temp(TempId(100), w),
                        right: constant,
                    },
                    Mir65816Op::Compare {
                        dest: TempId(102),
                        width: w,
                        signed: false,
                        operation: NirCompareOp::Ne,
                        left: Mir65816Value::Temp(TempId(101), w),
                        right: Mir65816Value::U8(0),
                    },
                ],
                terminator: Mir65816Terminator::Branch {
                    condition: Mir65816Value::Temp(TempId(102), ByteSize::ONE),
                    then_edge: edge.clone(),
                    else_edge: edge,
                },
            };
            let mut counts = BTreeMap::from([(TempId(101), 1), (TempId(102), 1)]);
            match case {
                1 => {
                    counts.insert(TempId(101), 2);
                }
                2 => {
                    counts.insert(TempId(102), 2);
                }
                3 => {
                    let Mir65816Op::Binary { right, .. } = &mut block.ops[0] else {
                        panic!()
                    };
                    *right = Mir65816Value::U8(0x40);
                }
                4 => {
                    let Mir65816Op::Compare { operation, .. } = &mut block.ops[1] else {
                        panic!()
                    };
                    *operation = NirCompareOp::Ge;
                }
                5 => {
                    block.ops.insert(1, block.ops[1].clone());
                }
                6 => block.terminator = Mir65816Terminator::Exit,
                7 => {
                    b.frame.temps.insert(
                        TempId(100),
                        Location::Stack(Slot {
                            offset: 32,
                            width: bytes + 1,
                        }),
                    );
                }
                8 => {
                    b.frame.temps.insert(
                        TempId(101),
                        Location::Stack(Slot {
                            offset: 256,
                            width: bytes,
                        }),
                    );
                }
                9 => {
                    b.frame.temps.insert(
                        TempId(100),
                        Location::Stack(Slot {
                            offset: 256,
                            width: bytes,
                        }),
                    );
                }
                _ => {}
            }
            let before = format!("{:?}", b.code);
            let selected = plan(&b, &block, &counts);
            if case >= 7 {
                assert!(selected.is_err());
            } else {
                assert_eq!(selected.unwrap().is_some(), case == 0);
            }
            assert_eq!(format!("{:?}", b.code), before);
        }
    }
}
