use super::*;

fn program() -> Mir65816Program {
    let mut p = word_tests::program();
    p.routines[0].blocks.push(Mir65816Block {
        id: BlockId(99),
        params: vec![(TempId(90), ByteSize::new(4)), (TempId(91), ByteSize::ONE)],
        ops: vec![],
        terminator: Mir65816Terminator::Exit,
    });
    p
}
fn builder(r: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::stack(&word_tests::program().routines[0]).unwrap();
    let mut b = Builder {
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        routine: r,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
    };
    for (id, offset, width) in [(90, 16, 4), (91, 20, 1), (92, 24, 4)] {
        b.frame
            .temps
            .insert(TempId(id), Location::Stack(Slot { offset, width }));
    }
    b.frame.edge_copies = vec![
        Slot {
            offset: 40,
            width: 4,
        },
        Slot {
            offset: 44,
            width: 1,
        },
    ];
    b
}
fn edge() -> Mir65816Edge {
    Mir65816Edge {
        target: BlockId(99),
        args: vec![
            Mir65816Value::Temp(TempId(90), ByteSize::new(4)),
            Mir65816Value::U8(7),
        ],
    }
}

#[test]
fn identities_do_not_stage_and_last_identity_repairs_byte_and_flags() {
    let p = program();
    let mut b = builder(&p.routines[0]);
    b.code.a16();
    let start = b.code.position();
    b.emit_mixed_edge(&edge()).unwrap();
    assert_eq!(
        &b.code.code().bytes[start..],
        &[0xe2, 0x20, 0xa9, 7, 0x83, 44, 0xa3, 44, 0x83, 20]
    );
    let mut b = builder(&p.routines[0]);
    let mut e = edge();
    e.args[1] = Mir65816Value::Temp(TempId(91), ByteSize::ONE);
    b.code.a8();
    let start = b.code.position();
    b.emit_mixed_edge(&e).unwrap();
    assert_eq!(&b.code.code().bytes[start..], &[0xa3, 20]);
}

#[test]
fn identity_decisions_match_simultaneous_bytes_including_overlapping_destinations() {
    let p = program();
    for source in 14..28 {
        for destination in 14..28 {
            let mut b = builder(&p.routines[0]);
            b.frame.temps.insert(
                TempId(92),
                Location::Stack(Slot {
                    offset: source,
                    width: 4,
                }),
            );
            b.frame.temps.insert(
                TempId(91),
                Location::Stack(Slot {
                    offset: destination,
                    width: 1,
                }),
            );
            let mut e = edge();
            e.args[0] = Mir65816Value::Temp(TempId(92), ByteSize::new(4));
            let plan = b.mixed_edge_plan(&e).unwrap();
            let initial: Vec<u8> = (0..64).map(|v| v * 3).collect();
            let mut expected = initial.clone();
            expected[16..20].copy_from_slice(&initial[source as usize..source as usize + 4]);
            expected[destination as usize] = 7;
            let mut actual = initial.clone();
            if !plan[0].identity {
                actual[16..20].copy_from_slice(&initial[source as usize..source as usize + 4]);
            }
            actual[destination as usize] = 7;
            assert_eq!(actual, expected);
            assert_eq!(
                plan[0].identity,
                source == 16 && !(16..20).contains(&destination)
            );
        }
    }
}

#[test]
fn all_homes_and_staging_are_validated_before_emission() {
    let p = program();
    for invalid in 0..5 {
        let mut b = builder(&p.routines[0]);
        match invalid {
            0 => b.frame.edge_copies.clear(),
            1 => b.frame.edge_copies[0].offset = 254,
            2 => b.frame.edge_copies[0].offset = 16,
            3 => b.frame.edge_copies[1].offset = 41,
            _ => {
                b.frame.temps.insert(
                    TempId(90),
                    Location::Stack(Slot {
                        offset: 16,
                        width: 3,
                    }),
                );
            }
        }
        let before = format!("{:?}", b.code);
        assert!(b.emit_mixed_edge(&edge()).is_err());
        assert_eq!(format!("{:?}", b.code), before);
    }
}

#[test]
fn native_copy_cost_includes_hidden_b_repair_and_interleaved_modes() {
    let p = program();
    let mut b = builder(&p.routines[0]);
    let mut e = edge();
    e.args[0] = Mir65816Value::Temp(TempId(92), ByteSize::new(4));
    e.args[1] = Mir65816Value::Temp(TempId(91), ByteSize::ONE);
    let copies = b.mixed_edge_plan(&e).unwrap();
    assert!(b.native_mixed_profitable(&copies));
    b.code.a16();
    let at = b.code.position();
    b.emit_mixed_edge(&e).unwrap();
    assert_eq!(
        &b.code.code().bytes[at..],
        &[
            0x85, 8, 0xa3, 24, 0x83, 40, 0xa3, 26, 0x83, 42, 0xa3, 40, 0x83, 16, 0xa3, 42, 0x83,
            18, 0xa5, 8, 0xe2, 0x20, 0xa3, 20,
        ]
    );
    e.args[1] = Mir65816Value::U8(7);
    // One LONG interleaved with a real BYTE move loses to mode overhead.
    assert!(!b.native_mixed_profitable(&b.mixed_edge_plan(&e).unwrap()));
}
