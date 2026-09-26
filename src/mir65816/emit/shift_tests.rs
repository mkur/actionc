use super::*;

#[test]
fn index_scaling_shifts_only_between_stride_bits() {
    let p = word_tests::program();
    for stride in [1u32, 2, 3, 4, 5, 257, 65535, 65536, 0x800000, 0xffffff] {
        let mut b = builder(&p.routines[0], 4);
        let address = Mir65816Address {
            base: Mir65816AddressBase::External(Mir65816ExternalAddress::Absolute(
                AddressValue::data(0x12ffff),
            )),
            displacement: ByteOffset::ZERO,
            index: Some(Mir65816Index {
                value: Mir65816Value::U24(0x12345),
                stride: ByteSize::new(stride),
            }),
            mode: Mir65816AddressMode::LongIndexed,
        };
        b.prepare_address(&address).unwrap();
        let shifts = [0x06, INDEX, 0x26, INDEX + 1, 0x26, INDEX + 2];
        assert_eq!(
            b.code
                .code()
                .bytes
                .windows(6)
                .filter(|window| *window == shifts)
                .count(),
            (31 - stride.leading_zeros()) as usize
        );
        assert!(!b.code.code().bytes.ends_with(&shifts));
    }
}

fn builder(routine: &Mir65816Routine, bytes: u8) -> Builder<'_> {
    let mut frame = AllocatedFrame::stack(routine).unwrap();
    for (id, offset) in [(100, 64), (101, 65)] {
        frame.temps.insert(
            TempId(id),
            Location::Stack(Slot {
                offset,
                width: bytes,
            }),
        );
    }
    let code = TrackedEmitter65816::for_test(&frame);
    Builder {
        routine,
        frame,
        code,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
    }
}

#[test]
fn constant_shifts_are_smaller_than_checked_loops_at_every_count() {
    let p = word_tests::program();
    for bytes in 1..=4 {
        for count in (0..=33).chain([255, 256, 0xffffff, u32::MAX]) {
            for left in [false, true] {
                let value = Mir65816Value::Temp(TempId(100), ByteSize::new(bytes.into()));
                let count = Mir65816Value::U32(count);
                let mut native = builder(&p.routines[0], bytes);
                assert!(
                    native
                        .constant_shift(
                            TempId(101),
                            bytes,
                            if left {
                                NirBinaryOp::Lsh
                            } else {
                                NirBinaryOp::Rsh
                            },
                            &value,
                            &count
                        )
                        .unwrap()
                );
                native.code.a16();
                let mut fallback = builder(&p.routines[0], bytes);
                fallback.code.a8();
                fallback
                    .shift(TempId(101), bytes, left, &value, &count)
                    .unwrap();
                fallback.code.a16();
                assert!(
                    native.code.code().bytes.len() < fallback.code.code().bytes.len(),
                    "{bytes}/{count:?}/{left}"
                );
            }
        }
    }
}

#[test]
fn word_shift_encoding_captures_before_overlapping_destination_stores() {
    let p = word_tests::program();
    let mut b = builder(&p.routines[0], 4);
    b.code.a16();
    let prefix = b.code.position();
    b.constant_shift(
        TempId(101),
        4,
        NirBinaryOp::Lsh,
        &Mir65816Value::Temp(TempId(100), ByteSize::new(4)),
        &Mir65816Value::U8(1),
    )
    .unwrap();
    assert_eq!(
        &b.code.code().bytes[prefix..],
        &[
            0xa3, 64, 0x85, 8, 0xa3, 66, 0x85, 10, // Capture both words.
            0x06, 8, 0x26, 10, // A16 ASL / ROL.
            0xa5, 8, 0x83, 65, 0xa5, 10, 0x83, 67,
        ]
    );

    // An overlapping-word three-byte capture already leaves A16. Initialize
    // the residual counter before selecting A8, without a SEP/REP round trip.
    let mut b = builder(&p.routines[0], 3);
    b.constant_shift(
        TempId(101),
        3,
        NirBinaryOp::Lsh,
        &Mir65816Value::Temp(TempId(100), ByteSize::new(3)),
        &Mir65816Value::U8(7),
    )
    .unwrap();
    assert!(
        !b.code
            .code()
            .bytes
            .windows(4)
            .any(|w| w == [0xe2, 0x20, 0xc2, 0x20])
    );
    assert!(
        b.code
            .code()
            .bytes
            .windows(6)
            .any(|w| w == [0xa9, 7, 0, 0xaa, 0xe2, 0x20])
    );
}

#[test]
fn constant_shift_preflight_and_variable_fallback_emit_nothing_on_rejection() {
    let p = word_tests::program();
    for case in 0..6 {
        let mut b = builder(&p.routines[0], 4);
        let mut value = Mir65816Value::Temp(TempId(100), ByteSize::new(4));
        match case {
            0 => value = Mir65816Value::Temp(TempId(100), ByteSize::new(3)),
            1 => {
                b.frame.temps.remove(&TempId(100));
            }
            2 => {
                b.frame.temps.insert(
                    TempId(100),
                    Location::Stack(Slot {
                        offset: 254,
                        width: 4,
                    }),
                );
            }
            3 => {
                b.frame.temps.insert(
                    TempId(101),
                    Location::Stack(Slot {
                        offset: 254,
                        width: 4,
                    }),
                );
            }
            4 => {
                b.frame.temps.insert(
                    TempId(101),
                    Location::Stack(Slot {
                        offset: 0,
                        width: 4,
                    }),
                );
            }
            _ => {
                b.frame.temps.insert(
                    TempId(101),
                    Location::Stack(Slot {
                        offset: 64,
                        width: 3,
                    }),
                );
            }
        }
        for count in [0, 1, 256] {
            assert!(
                b.constant_shift(
                    TempId(101),
                    4,
                    NirBinaryOp::Rsh,
                    &value,
                    &Mir65816Value::U32(count)
                )
                .is_err()
            );
            assert!(b.code.code().bytes.is_empty());
        }
    }
    let mut b = builder(&p.routines[0], 4);
    assert!(
        !b.constant_shift(
            TempId(101),
            4,
            NirBinaryOp::Rsh,
            &Mir65816Value::U32(1),
            &Mir65816Value::Temp(TempId(100), ByteSize::new(4))
        )
        .unwrap()
    );
    assert!(b.code.code().bytes.is_empty());
}

#[test]
fn short_word_shifts_load_once_before_overlapping_store() {
    let p = word_tests::program();
    for left in [false, true] {
        for count in 1..=7 {
            let mut b = builder(&p.routines[0], 2);
            b.code.a16();
            let at = b.code.position();
            assert!(
                b.constant_shift(
                    TempId(101),
                    2,
                    if left {
                        NirBinaryOp::Lsh
                    } else {
                        NirBinaryOp::Rsh
                    },
                    &Mir65816Value::Temp(TempId(100), ByteSize::new(2)),
                    &Mir65816Value::U8(count)
                )
                .unwrap()
            );
            let mut expected = vec![0xa3, 64];
            expected.extend(vec![if left { 0x0a } else { 0x4a }; count as usize]);
            expected.extend([0x83, 65]);
            assert_eq!(&b.code.code().bytes[at..], expected);
        }
    }
}
