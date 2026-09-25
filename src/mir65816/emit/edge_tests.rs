use super::*;

fn program() -> Mir65816Program {
    let mut p = super::word_tests::program();
    let r = &mut p.routines[0];
    let w = ByteSize::new(2);
    r.blocks.push(Mir65816Block {
        id: BlockId(99),
        params: vec![(TempId(0), w), (TempId(1), w)],
        ops: vec![],
        terminator: r.blocks.last().unwrap().terminator.clone(),
    });
    p
}
fn builder(r: &Mir65816Routine) -> Builder<'_> {
    // Use the ordinary allocated frame, then controlled physical homes so
    // tests exercise overlap, exact endpoints and malformed preflights.
    let mut b = Builder {
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
        routine: r,
        frame: AllocatedFrame::stack(&super::word_tests::program().routines[0]).unwrap(),
        code: TrackedEmitter65816::for_test(
            &AllocatedFrame::stack(&super::word_tests::program().routines[0]).unwrap(),
        ),
        blocks: BTreeMap::new(),
    };
    b.blocks.insert(BlockId(99), b.code.label());
    for (id, offset) in [(0, 2), (1, 4)] {
        b.frame
            .temps
            .insert(TempId(id), Location::Stack(Slot { offset, width: 2 }));
    }
    b.frame.edge_copies = vec![
        Slot {
            offset: 8,
            width: 4,
        },
        Slot {
            offset: 12,
            width: 4,
        },
    ];
    b
}
fn edge() -> Mir65816Edge {
    Mir65816Edge {
        target: BlockId(99),
        args: vec![
            Mir65816Value::Temp(TempId(1), ByteSize::new(2)),
            Mir65816Value::U16(0xa55a),
        ],
    }
}

#[test]
fn checked_acyclic_word_edges_emit_direct_copies_without_staging_in_every_mode() {
    let p = program();
    for mode in [None, Some(true), Some(false)] {
        let mut b = builder(&p.routines[0]);
        b.frame.edge_copies.clear();
        match mode {
            Some(true) => b.code.a8(),
            Some(false) => b.code.a16(),
            None => (),
        }
        let start = b.code.code().bytes.len();
        b.edge(&edge()).unwrap();
        let mut expected = vec![];
        if mode != Some(false) {
            expected.extend([0xc2, 0x20]);
        }
        expected.extend([0xa3, 4, 0x83, 2, 0xa9, 0x5a, 0xa5, 0x83, 4, 0x5c, 0, 0, 0]);
        assert_eq!(b.code.code().bytes[start..], expected);
        assert_eq!(b.code.code().fixups.len(), 1);
        let f = &b.code.code().fixups[0];
        assert_eq!(f.target, Target::Label(b.blocks[&BlockId(99)]));
        assert_eq!(
            (f.offset, f.addend, f.byte),
            (b.code.code().bytes.len() - 3, 0, None)
        );
        let end = b.code.code().bytes.len();
        b.code.a16();
        assert_eq!(b.code.code().bytes.len(), end);
    }
}

#[test]
fn acyclic_order_preserves_sources_and_rejects_cycles_and_partial_overlap() {
    use WordOperand::{Immediate as I, Stack as S};
    for (moves, expected) in [
        (vec![(S(6), 8, 2), (S(2), 12, 4)], Some(vec![1, 0])),
        (vec![(S(4), 8, 2), (I(0), 12, 4)], Some(vec![0, 1])),
        (vec![(S(2), 8, 2), (S(2), 12, 4)], Some(vec![1, 0])),
        (
            vec![(S(6), 12, 2), (S(6), 16, 4), (I(7), 20, 8)],
            Some(vec![0, 1, 2]),
        ),
        (vec![(S(4), 8, 2), (S(2), 12, 4)], None),
        (vec![(S(4), 12, 2), (S(6), 16, 4), (S(2), 20, 6)], None),
        (vec![(S(3), 8, 6), (I(0), 12, 4)], None),
        (vec![(S(8), 12, 2), (S(10), 16, 3)], None),
    ] {
        assert_eq!(acyclic_word_order(&moves), expected);
    }
    // Independent simultaneous-copy oracle, including self/repeated sources.
    for a in [2, 4, 6, 8] {
        for b in [2, 4, 6, 8] {
            for c in [2, 4, 6, 8] {
                let moves = [(S(a), 12, 2), (S(b), 16, 4), (S(c), 20, 6)];
                if let Some(order) = acyclic_word_order(&moves) {
                    let initial: Vec<u8> = (0..32).collect();
                    let mut expected = initial.clone();
                    for &(source, _, d) in &moves {
                        let S(s) = source else { unreachable!() };
                        expected[d as usize..d as usize + 2]
                            .copy_from_slice(&initial[s as usize..s as usize + 2]);
                    }
                    let mut actual = initial;
                    for i in order {
                        let (S(s), _, d) = moves[i] else {
                            unreachable!()
                        };
                        let value = [actual[s as usize], actual[s as usize + 1]];
                        actual[d as usize..d as usize + 2].copy_from_slice(&value);
                    }
                    assert_eq!(actual, expected);
                }
            }
        }
    }
}

#[test]
fn reordered_word_edges_restore_original_final_a_and_keep_frame() {
    let p = program();
    let mut b = builder(&p.routines[0]);
    b.frame.temps.insert(
        TempId(2),
        Location::Stack(Slot {
            offset: 6,
            width: 2,
        }),
    );
    let frame = format!("{:?}", b.frame);
    let e = Mir65816Edge {
        target: BlockId(99),
        args: vec![
            Mir65816Value::Temp(TempId(2), ByteSize::new(2)),
            Mir65816Value::Temp(TempId(0), ByteSize::new(2)),
        ],
    };
    b.edge(&e).unwrap();
    assert_eq!(
        b.code.code().bytes,
        [
            0xc2, 0x20, 0xa3, 2, 0x83, 4, 0xa3, 6, 0x83, 2, 0xa3, 4, 0x5c, 0, 0, 0
        ]
    );
    assert_eq!(format!("{:?}", b.frame), frame);
}

#[test]
fn cyclic_word_edges_capture_only_endangered_sources() {
    let p = program();
    let mut b = builder(&p.routines[0]);
    let mut e = edge();
    e.args[1] = Mir65816Value::Temp(TempId(0), ByteSize::new(2));
    b.edge(&e).unwrap();
    assert_eq!(
        b.code.code().bytes,
        [
            0xc2, 0x20, 0xa3, 2, 0x83, 8, 0xa3, 4, 0x83, 2, 0xa3, 8, 0x83, 4, 0x5c, 0, 0, 0
        ]
    );
}

#[test]
fn word_edge_preflight_checks_every_entry_without_mutation() {
    let p = program();
    for problem in 0..12 {
        let mut b = builder(&p.routines[0]);
        let mut e = edge();
        // Even an unsupported earlier value must not hide a later error.
        e.args[0] = Mir65816Value::Null(ByteSize::new(2));
        match problem {
            0 => e.args[1] = Mir65816Value::U8(1),
            1 => e.args[1] = Mir65816Value::Temp(TempId(999), ByteSize::new(2)),
            2 => {
                b.frame.temps.remove(&TempId(1));
            }
            3 => {
                b.frame.temps.insert(
                    TempId(1),
                    Location::Stack(Slot {
                        offset: 4,
                        width: 1,
                    }),
                );
            }
            4 => {
                b.frame.temps.insert(
                    TempId(1),
                    Location::Stack(Slot {
                        offset: 255,
                        width: 2,
                    }),
                );
            }
            5 => {
                b.frame.edge_copies.pop();
            }
            6 => b.frame.edge_copies[1].width = 1,
            7 => b.frame.edge_copies[1].offset = 255,
            8 => {
                b.blocks.clear();
            }
            9 => e.target = BlockId(999),
            10 => {
                e.args.pop();
            }
            11 => b.code.test_delta(u32::MAX),
            _ => unreachable!(),
        }
        b.code.a8();
        let before = format!("{:?}", b.code);
        assert!(b.word_edge(&e).is_err(), "{problem}");
        assert_eq!(format!("{:?}", b.code), before);
    }
}

#[test]
fn unsupported_edges_fall_back_without_prefix_and_keep_byte_encodings() {
    let mut p = program();
    for form in 0..3 {
        let r = &mut p.routines[0];
        if form == 2 {
            r.blocks.last_mut().unwrap().params[1].1 = ByteSize::ONE;
        }
        let mut b = builder(r);
        let mut e = edge();
        match form {
            0 => e.args[0] = Mir65816Value::Null(ByteSize::new(2)),
            1 => {
                b.frame.temps.insert(
                    TempId(1),
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 2,
                    }),
                );
            }
            2 => {
                e.args[1] = Mir65816Value::U8(0x5a);
                e.args[0] = Mir65816Value::Temp(TempId(2), ByteSize::new(2));
                b.frame.temps.insert(
                    TempId(2),
                    Location::Stack(Slot {
                        offset: 4,
                        width: 2,
                    }),
                );
                b.frame.temps.insert(
                    TempId(1),
                    Location::Stack(Slot {
                        offset: 4,
                        width: 1,
                    }),
                );
            }
            _ => unreachable!(),
        }
        b.code.a16();
        let before = format!("{:?}", b.code);
        if form == 1 {
            assert!(b.word_edge(&e).is_err());
            assert_eq!(format!("{:?}", b.code), before);
            continue;
        }
        assert!(b.word_edge(&e).unwrap().is_none());
        assert_eq!(format!("{:?}", b.code), before);
        let start = b.code.code().bytes.len();
        b.edge(&e).unwrap();
        let source = if form == 0 {
            vec![0xa9, 0, 0x83, 8, 0xa9, 0, 0x83, 9]
        } else if form == 1 {
            vec![0xa5, 0, 0x83, 8, 0xa5, 1, 0x83, 9]
        } else {
            vec![0xa3, 4, 0x83, 8, 0xa3, 5, 0x83, 9]
        };
        let mut expected = vec![0xe2, 0x20];
        expected.extend(source);
        expected.extend([0xa9, 0x5a, 0x83, 12]);
        if form != 2 {
            expected.extend([0xa9, 0xa5, 0x83, 13]);
        }
        expected.extend([0xa3, 8, 0x83, 2, 0xa3, 9, 0x83, 3, 0xa3, 12]);
        expected.extend(if form == 1 { [0x85, 0] } else { [0x83, 4] });
        if form != 2 {
            expected.extend([0xa3, 13]);
            expected.extend(if form == 1 { [0x85, 1] } else { [0x83, 5] });
        }
        expected.extend([0xc2, 0x20, 0x5c, 0, 0, 0]);
        assert_eq!(b.code.code().bytes[start..], expected, "{form}");
    }
}

#[test]
fn only_accessed_word_extent_matters_after_transient_stack_movement() {
    let p = program();
    for field in 0..3 {
        for (offset, delta, ok) in [
            (254, 0, true),
            (255, 0, false),
            (253, 1, true),
            (254, 1, false),
            (0, 0, false),
            (2, u32::MAX, false),
        ] {
            let mut b = builder(&p.routines[0]);
            let mut e = edge();
            b.code.test_delta(delta);
            match field {
                0 => {
                    e.args[1] = Mir65816Value::Temp(TempId(2), ByteSize::new(2));
                    b.frame
                        .temps
                        .insert(TempId(2), Location::Stack(Slot { offset, width: 2 }));
                }
                1 => {
                    e.args[1] = Mir65816Value::Temp(TempId(0), ByteSize::new(2));
                    b.frame.edge_copies[0].offset = offset;
                }
                2 => {
                    b.frame
                        .temps
                        .insert(TempId(0), Location::Stack(Slot { offset, width: 2 }));
                }
                _ => unreachable!(),
            }
            assert_eq!(b.word_edge(&e).is_ok(), ok, "{field}/{offset}/{delta}");
        }
    }
}

#[test]
fn empty_edges_only_restore_a16_when_needed_and_keep_the_typed_target() {
    let mut p = program();
    p.routines[0].blocks.last_mut().unwrap().params.clear();
    for mode in [None, Some(true), Some(false)] {
        let mut b = builder(&p.routines[0]);
        match mode {
            Some(true) => b.code.a8(),
            Some(false) => b.code.a16(),
            None => (),
        }
        let mut e = edge();
        e.args.clear();
        let start = b.code.code().bytes.len();
        let frame = b.frame.clone();
        b.edge(&e).unwrap();
        let expected = if mode == Some(false) {
            vec![0x5c, 0, 0, 0]
        } else {
            vec![0xc2, 0x20, 0x5c, 0, 0, 0]
        };
        assert_eq!(b.code.code().bytes[start..], expected);
        assert_eq!(b.code.code().fixups.len(), 1);
        let f = &b.code.code().fixups[0];
        assert_eq!(
            (f.offset, f.target, f.addend, f.byte),
            (
                b.code.code().bytes.len() - 3,
                Target::Label(b.blocks[&e.target]),
                0,
                None
            )
        );
        assert_eq!(format!("{:?}", b.frame), format!("{:?}", frame));
        let end = b.code.code().bytes.clone();
        b.code.a16();
        assert_eq!(b.code.code().bytes, end);
    }
}

#[test]
fn malformed_empty_edges_fail_without_emission_or_mode_changes() {
    for problem in 0..4 {
        let mut p = program();
        if problem != 2 {
            p.routines[0].blocks.last_mut().unwrap().params.clear();
        }
        let mut b = builder(&p.routines[0]);
        let mut e = edge();
        e.args.clear();
        let message = match problem {
            0 => {
                e.target = BlockId(999);
                "unknown branch target"
            }
            1 => {
                b.blocks.clear();
                "missing branch target label"
            }
            2 => "edge argument count mismatch",
            3 => {
                e.args.push(Mir65816Value::U16(1));
                "edge argument count mismatch"
            }
            _ => unreachable!(),
        };
        b.code.a8();
        // Failed preflight records its barrier request, but code and execution state stay unchanged.
        let before = format!("{:?}", (b.code.code(), b.code.state_for_incoming_test()));
        assert_eq!(b.edge(&e), Err(message.into()));
        assert_eq!(
            format!("{:?}", (b.code.code(), b.code.state_for_incoming_test())),
            before
        );
    }
}

#[test]
fn single_word_edges_bypass_staging_and_keep_modes_fixups_and_frame() {
    let mut p = program();
    p.routines[0].blocks.last_mut().unwrap().params.truncate(1);
    let r = &p.routines[0];
    for mode in [None, Some(true), Some(false)] {
        for source in [
            Mir65816Value::U16(0xa55a),
            Mir65816Value::Temp(TempId(1), ByteSize::new(2)),
            Mir65816Value::Temp(TempId(0), ByteSize::new(2)),
            Mir65816Value::Param(r.frame.parameters[0].param),
        ] {
            let mut b = builder(r);
            b.frame.edge_copies.clear();
            match mode {
                Some(true) => b.code.a8(),
                Some(false) => b.code.a16(),
                None => (),
            }
            let start = b.code.code().bytes.len();
            let frame = format!("{:?}", b.frame);
            let mut expected = if mode == Some(false) {
                vec![]
            } else {
                vec![0xc2, 0x20]
            };
            match b.word_operand(&source).unwrap().unwrap() {
                WordOperand::Immediate(v) => expected.extend([0xa9, v as u8, (v >> 8) as u8]),
                WordOperand::Stack(offset) => expected.extend([0xa3, offset]),
                WordOperand::DirectPage(offset) => expected.extend([0xa5, offset]),
            }
            if b.word_operand(&source).unwrap() != Some(WordOperand::Stack(2)) {
                expected.extend([0x83, 2]);
            }
            expected.extend([0x5c, 0, 0, 0]);
            b.edge(&Mir65816Edge {
                target: BlockId(99),
                args: vec![source],
            })
            .unwrap();
            assert_eq!(b.code.code().bytes[start..], expected);
            assert_eq!(format!("{:?}", b.frame), frame);
            assert_eq!(b.code.code().fixups.len(), 1);
            let f = &b.code.code().fixups[0];
            assert_eq!(
                (f.offset, f.target, f.addend, f.byte),
                (
                    b.code.code().bytes.len() - 3,
                    Target::Label(b.blocks[&BlockId(99)]),
                    0,
                    None
                )
            );
            let bytes = b.code.code().bytes.clone();
            b.code.a16();
            assert_eq!(b.code.code().bytes, bytes);
        }
    }
}

#[test]
fn single_word_edges_validate_operands_without_requiring_unused_staging() {
    let mut p = program();
    p.routines[0].blocks.last_mut().unwrap().params.truncate(1);
    for problem in (0..12).filter(|p| !(5..=7).contains(p)) {
        let mut b = builder(&p.routines[0]);
        let mut e = Mir65816Edge {
            target: BlockId(99),
            args: vec![Mir65816Value::Temp(TempId(1), ByteSize::new(2))],
        };
        match problem {
            0 => e.args[0] = Mir65816Value::U8(1),
            1 => e.args[0] = Mir65816Value::Temp(TempId(999), ByteSize::new(2)),
            2 => {
                b.frame.temps.remove(&TempId(0));
            }
            3 => {
                b.frame.temps.insert(
                    TempId(0),
                    Location::Stack(Slot {
                        offset: 2,
                        width: 1,
                    }),
                );
            }
            4 => {
                b.frame.temps.insert(
                    TempId(0),
                    Location::Stack(Slot {
                        offset: 255,
                        width: 2,
                    }),
                );
            }
            5 => b.frame.edge_copies.clear(),
            6 => b.frame.edge_copies[0].width = 2,
            7 => b.frame.edge_copies[0].offset = 255,
            8 => b.blocks.clear(),
            9 => e.target = BlockId(999),
            10 => e.args.clear(),
            11 => b.code.test_delta(u32::MAX),
            _ => unreachable!(),
        }
        b.code.a8();
        // Failed preflight records its barrier request, but code and execution state stay unchanged.
        let before = format!("{:?}", (b.code.code(), b.code.state_for_incoming_test()));
        assert!(b.edge(&e).is_err(), "{problem}");
        assert_eq!(
            format!("{:?}", (b.code.code(), b.code.state_for_incoming_test())),
            before
        );
    }
    for field in 0..2 {
        for (offset, delta, ok) in [
            (254, 0, true),
            (255, 0, false),
            (253, 1, true),
            (254, 1, false),
            (0, 0, false),
        ] {
            let mut b = builder(&p.routines[0]);
            b.code.test_delta(delta);
            match field {
                0 => {
                    b.frame
                        .temps
                        .insert(TempId(1), Location::Stack(Slot { offset, width: 2 }));
                }
                1 => {
                    b.frame
                        .temps
                        .insert(TempId(0), Location::Stack(Slot { offset, width: 2 }));
                }
                2 => b.frame.edge_copies[0].offset = offset,
                _ => unreachable!(),
            }
            let e = Mir65816Edge {
                target: BlockId(99),
                args: vec![Mir65816Value::Temp(TempId(1), ByteSize::new(2))],
            };
            assert_eq!(b.edge(&e).is_ok(), ok, "{field}/{offset}/{delta}");
        }
    }
}

#[test]
fn selective_plans_match_simultaneous_byte_copies_exhaustively() {
    use WordOperand::{Immediate as I, Stack as S};
    use copies::{WordCopies, WordStrategy};
    let sources = [S(2), S(4), S(6), S(8), S(10), I(0), I(0xffff)];
    for n in 1..=4u32 {
        for code in 0..7u32.pow(n) {
            let mut code = code;
            let moves: Vec<_> = (0..n)
                .map(|i| {
                    let s = sources[(code % 7) as usize];
                    code /= 7;
                    (s, 2 + 2 * i as u8)
                })
                .collect();
            let plan = WordCopies::plan(moves.clone());
            let captures = plan.captures().unwrap();
            for pattern in [0u8, 0x55, 0xff] {
                let initial: Vec<u8> = (0..32u8).map(|b| pattern ^ b.wrapping_mul(17)).collect();
                let load = |mem: &[u8], s| match s {
                    I(v) => v.to_le_bytes(),
                    S(s) => [mem[s as usize], mem[s as usize + 1]],
                    WordOperand::DirectPage(_) => unreachable!(),
                };
                let mut expected = initial.clone();
                for &(s, d) in &moves {
                    expected[d as usize..d as usize + 2].copy_from_slice(&load(&initial, s));
                }
                let saved: Vec<_> = captures
                    .iter()
                    .map(|&i| load(&initial, moves[i].0))
                    .collect();
                let order = match &plan.strategy {
                    WordStrategy::Direct(order) => order.clone(),
                    WordStrategy::Selective(_) => (0..moves.len()).collect(),
                    WordStrategy::Complete => panic!("disjoint words must not need full staging"),
                };
                let mut actual = initial.clone();
                let mut a = [0; 2];
                for i in order {
                    let (s, d) = moves[i];
                    a = captures
                        .iter()
                        .position(|&j| j == i)
                        .map_or_else(|| load(&actual, s), |k| saved[k]);
                    actual[d as usize..d as usize + 2].copy_from_slice(&a);
                }
                if let WordStrategy::Direct(order) = &plan.strategy {
                    if order.last().copied() != Some(moves.len() - 1) {
                        a = load(&actual, S(moves.last().unwrap().1));
                    }
                }
                assert_eq!(actual, expected);
                assert_eq!(a, load(&initial, moves.last().unwrap().0));
            }
        }
    }
    for moves in [vec![(S(3), 6), (I(0), 4)], vec![(S(8), 2), (S(10), 3)]] {
        assert_eq!(WordCopies::plan(moves).strategy, WordStrategy::Complete);
    }
}

#[test]
fn selective_capture_mapping_is_checked_before_emission_in_every_mode() {
    use copies::{WordCopies, WordStrategy};
    let p = program();
    let mut e = edge();
    e.args[1] = Mir65816Value::Temp(TempId(0), ByteSize::new(2));
    for mode in [None, Some(true), Some(false)] {
        for problem in 0..6 {
            let mut b = builder(&p.routines[0]);
            match mode {
                Some(true) => b.code.a8(),
                Some(false) => b.code.a16(),
                None => (),
            };
            match problem {
                0 => b.frame.edge_copies.clear(),
                1 => b.frame.edge_copies[0].width = 1,
                2 => b.frame.edge_copies[0].offset = 2,
                3 => b.frame.edge_copies[0].offset = 3,
                4 => b.frame.edge_copies[0].offset = 255,
                5 => b.code.test_delta(u32::MAX),
                _ => unreachable!(),
            }
            let before = format!("{:?}", b.code);
            assert!(b.word_edge(&e).is_err());
            assert_eq!(format!("{:?}", b.code), before);
        }
    }
    let b = builder(&p.routines[0]);
    for captures in [vec![], vec![0], vec![1, 1], vec![2], vec![1, 0]] {
        let mut plan =
            WordCopies::plan(vec![(WordOperand::Stack(4), 2), (WordOperand::Stack(2), 4)]);
        plan.strategy = WordStrategy::Selective(captures);
        assert!(b.frame.word_staging(&plan, 0).is_err());
    }
}

#[test]
fn partial_word_overlap_keeps_complete_word_staging() {
    let p = program();
    let mut b = builder(&p.routines[0]);
    b.frame.temps.insert(
        TempId(2),
        Location::Stack(Slot {
            offset: 3,
            width: 2,
        }),
    );
    let mut e = edge();
    e.args[0] = Mir65816Value::Temp(TempId(2), ByteSize::new(2));
    b.edge(&e).unwrap();
    assert_eq!(
        b.code.code().bytes,
        [
            0xc2, 0x20, 0xa3, 3, 0x83, 8, 0xa9, 0x5a, 0xa5, 0x83, 12, 0xa3, 8, 0x83, 2, 0xa3, 12,
            0x83, 4, 0x5c, 0, 0, 0
        ]
    );
}

#[test]
fn direct_identities_preserve_last_word_flags_without_writing_self_homes() {
    use WordOperand::{Immediate as I, Stack as S};
    for (moves, expected, repair, cost) in [
        (vec![(S(2), 2)], vec![], true, (2, 5)),
        (vec![(S(2), 2), (S(4), 4)], vec![], true, (2, 5)),
        (vec![(S(2), 2), (I(0), 4)], vec![1], false, (5, 8)),
        (vec![(I(0), 2), (S(4), 4)], vec![0], true, (7, 13)),
        (vec![(S(2), 2), (S(2), 4)], vec![1], false, (4, 10)),
    ] {
        let plan = copies::WordCopies::plan(moves);
        assert_eq!(plan.direct_emission(), Some((expected, repair)));
        assert_eq!(plan.cost(), cost);
    }
    let p = program();
    for mode in [None, Some(true), Some(false)] {
        let mut b = builder(&p.routines[0]);
        if mode == Some(true) {
            b.code.a8();
        } else if mode == Some(false) {
            b.code.a16();
        }
        let at = b.code.position();
        b.edge(&Mir65816Edge {
            target: BlockId(99),
            args: vec![
                Mir65816Value::Temp(TempId(0), ByteSize::new(2)),
                Mir65816Value::Temp(TempId(1), ByteSize::new(2)),
            ],
        })
        .unwrap();
        let mut expected = if mode == Some(false) {
            vec![]
        } else {
            vec![0xc2, 0x20]
        };
        expected.extend([0xa3, 4, 0x5c, 0, 0, 0]);
        assert_eq!(b.code.code().bytes[at..], expected);
    }
}

#[test]
fn direct_identity_omission_obeys_a_parallel_byte_oracle() {
    use WordOperand::Stack as S;
    for a in [2, 4, 6, 8] {
        for b in [2, 4, 6, 8] {
            for c in [2, 4, 6, 8] {
                let moves = vec![(S(a), 2), (S(b), 4), (S(c), 6)];
                let plan = copies::WordCopies::plan(moves.clone());
                let Some((order, repair)) = plan.direct_emission() else {
                    continue;
                };
                let initial: Vec<u8> = (0..32).collect();
                let mut expected = initial.clone();
                let mut actual = initial.clone();
                for &(source, d) in &moves {
                    let S(s) = source else { panic!() };
                    expected[d as usize..d as usize + 2]
                        .copy_from_slice(&initial[s as usize..s as usize + 2]);
                }
                let mut accumulator = None;
                for i in order {
                    let (S(s), d) = moves[i] else { panic!() };
                    let word = [actual[s as usize], actual[s as usize + 1]];
                    actual[d as usize..d as usize + 2].copy_from_slice(&word);
                    accumulator = Some(word);
                }
                if repair {
                    accumulator = Some([actual[6], actual[7]]);
                }
                assert_eq!(actual, expected);
                assert_eq!(accumulator, Some([expected[6], expected[7]]));
            }
        }
    }
}
