use super::*;

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let mut frame = AllocatedFrame::materialized_fixture(routine).unwrap();
    for (id, offset) in [(998, 250), (999, 253)] {
        frame
            .temps
            .insert(TempId(id), Location::Stack(Slot { offset, width: 3 }));
    }
    Builder {
        stack_checks: false,
        routine,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
        scalar_borrowed: BTreeMap::new(),
        resident: BTreeMap::new(),
    }
}

fn address(value: Mir65816Value, stride: u32, displacement: u32) -> Mir65816Address {
    Mir65816Address {
        base: Mir65816AddressBase::Indirect(Mir65816Value::Temp(TempId(998), ByteSize::new(3))),
        index: Some(Mir65816Index {
            value,
            stride: ByteSize::new(stride),
        }),
        displacement: ByteOffset::new(displacement),
        mode: Mir65816AddressMode::LongIndexed,
    }
}

#[test]
fn constant_indexed_addresses_use_exact_native_copies_and_bank_carry() {
    let p = word_tests::program();
    for value in [
        Mir65816Value::U8(3),
        Mir65816Value::U16(3),
        Mir65816Value::U24(3),
        Mir65816Value::U32(3),
    ] {
        for (stride, displacement) in [(1, 0), (3, 17), (128, 255), (1, 65532)] {
            for byte_mode in [false, true] {
                let mut b = builder(&p.routines[0]);
                if byte_mode {
                    b.code.a8();
                } else {
                    b.code.a16();
                }
                let at = b.code.position();
                assert!(
                    b.indexed_pointer_address(
                        TempId(999),
                        &address(value.clone(), stride, displacement)
                    )
                    .unwrap()
                );
                let offset = (3 * stride + displacement) as u16;
                let mut expected = if byte_mode { vec![0xc2, 0x20] } else { vec![] };
                expected.extend([
                    0xa3,
                    250,
                    0x18,
                    0x69,
                    offset as u8,
                    (offset >> 8) as u8,
                    0x83,
                    253,
                    0xe2,
                    0x20,
                    0xa3,
                    252,
                    0x69,
                    0,
                    0x83,
                    255,
                ]);
                assert_eq!(&b.code.code().bytes[at..], expected);
            }
        }
    }
    let mut b = builder(&p.routines[0]);
    b.code.a16();
    let at = b.code.position();
    assert!(
        b.indexed_pointer_address(TempId(999), &address(Mir65816Value::U8(0), 257, 0))
            .unwrap()
    );
    assert_eq!(
        &b.code.code().bytes[at..],
        [0xa3, 250, 0x83, 253, 0xa3, 251, 0x83, 254]
    );
}

#[test]
fn indexed_address_refusals_and_bad_homes_leave_state_unchanged() {
    let p = word_tests::program();
    for problem in 0..10 {
        let mut b = builder(&p.routines[0]);
        let mut a = address(Mir65816Value::U32(3), 1, 0);
        match problem {
            0 => a.index.as_mut().unwrap().stride = ByteSize::ZERO,
            1 => a.index.as_mut().unwrap().stride = ByteSize::new(0x1000000),
            2 => a.index.as_mut().unwrap().value = Mir65816Value::U32(u32::MAX),
            3 => a.displacement = ByteOffset::new(65533),
            4 => {
                a.index.as_mut().unwrap().value = Mir65816Value::Temp(TempId(998), ByteSize::new(3))
            }
            5 => a.base = Mir65816AddressBase::Parameter(p.routines[0].frame.parameters[0].param),
            6 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 251,
                        width: 3,
                    }),
                );
            }
            7 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 250,
                        width: 3,
                    }),
                );
            }
            8 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 254,
                        width: 3,
                    }),
                );
            }
            _ => b.code.test_delta(1),
        }
        let before = format!("{:?}", b.code);
        let result = b.indexed_pointer_address(TempId(999), &a);
        if problem >= 8 {
            assert!(result.is_err());
        } else {
            assert_eq!(result, Ok(false), "{problem}");
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
}

fn byte_routine() -> Mir65816Routine {
    let mut r = word_tests::program().routines.remove(0);
    r.blocks[0].ops.insert(
        0,
        Mir65816Op::Cast {
            dest: TempId(997),
            from: ByteSize::ONE,
            from_signed: false,
            to: ByteSize::ONE,
            kind: NirCastKind::Integer,
            value: Mir65816Value::U8(0),
        },
    );
    r.temps.push((
        TempId(997),
        crate::nir::NirType {
            kind: crate::nir::NirTypeKind::Integer(crate::nir::NirIntegerType::U8),
            summary: "BYTE".into(),
            width: Some(ByteSize::ONE),
            pointer: false,
        },
    ));
    r
}

fn byte_builder(r: &Mir65816Routine) -> Builder<'_> {
    let mut b = builder(r);
    b.frame.temps.insert(
        TempId(997),
        Location::Stack(Slot {
            offset: 249,
            width: 1,
        }),
    );
    b
}

#[test]
fn byte_indexed_addresses_encode_bounded_scales_in_both_entry_modes() {
    let r = byte_routine();
    // Explicit independent scale templates: 1, 3, 128, 257.
    for (stride, scale) in [
        (1, vec![]),
        (3, vec![0x85, INDEX, 0x0a, 0x18, 0x65, INDEX]),
        (128, vec![0x0a; 7]),
        (
            257,
            [vec![0x85, INDEX], vec![0x0a; 8], vec![0x18, 0x65, INDEX]].concat(),
        ),
    ] {
        for displacement in [0, 65535 - 255 * stride] {
            for byte in [false, true] {
                let mut b = byte_builder(&r);
                if byte {
                    b.code.a8();
                } else {
                    b.code.a16();
                }
                let at = b.code.position();
                let frame = b.frame.clone();
                assert!(
                    b.indexed_pointer_address(
                        TempId(999),
                        &address(
                            Mir65816Value::Temp(TempId(997), ByteSize::ONE),
                            stride,
                            displacement
                        )
                    )
                    .unwrap()
                );
                let mut expected = if byte { vec![] } else { vec![0xe2, 0x20] };
                expected.extend([0xa3, 249, 0xc2, 0x20, 0x29, 255, 0]);
                expected.extend(&scale);
                if displacement != 0 {
                    expected.extend([0x18, 0x69, displacement as u8, (displacement >> 8) as u8]);
                }
                expected.extend([
                    0x18, 0x63, 250, 0x83, 253, 0xe2, 0x20, 0xa3, 252, 0x69, 0, 0x83, 255,
                ]);
                assert_eq!(&b.code.code().bytes[at..], expected);
                assert_eq!(b.frame.temps, frame.temps);
                assert_eq!(b.frame.extent, frame.extent);
            }
        }
    }
}

#[test]
fn byte_indexed_address_preflight_rejects_bad_types_extents_and_overlap_atomically() {
    for problem in 0..15 {
        let mut r = byte_routine();
        let ty = &mut r.temps.last_mut().unwrap().1;
        match problem {
            0 => ty.kind = crate::nir::NirTypeKind::Integer(crate::nir::NirIntegerType::I8),
            1 => ty.kind = crate::nir::NirTypeKind::Bool,
            2 => ty.width = Some(ByteSize::new(2)),
            _ => {}
        }
        let mut b = byte_builder(&r);
        let mut a = address(Mir65816Value::Temp(TempId(997), ByteSize::ONE), 257, 0);
        match problem {
            3 => a.index.as_mut().unwrap().stride = ByteSize::new(258),
            4 => a.displacement = ByteOffset::new(1),
            5 => {
                a.index.as_mut().unwrap().value = Mir65816Value::Temp(TempId(997), ByteSize::new(2))
            }
            6 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 250,
                        width: 3,
                    }),
                );
            }
            7 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 251,
                        width: 3,
                    }),
                );
            }
            8 => {
                b.frame.temps.insert(
                    TempId(997),
                    Location::DirectPage(Slot {
                        offset: INDEX.into(),
                        width: 1,
                    }),
                );
            }
            9 => {
                b.frame.temps.insert(
                    TempId(997),
                    Location::Stack(Slot {
                        offset: 256,
                        width: 1,
                    }),
                );
            }
            10 => {
                b.frame.temps.insert(
                    TempId(997),
                    Location::Stack(Slot {
                        offset: 0,
                        width: 1,
                    }),
                );
            }
            11 => {
                b.frame.temps.insert(
                    TempId(997),
                    Location::Stack(Slot {
                        offset: 249,
                        width: 2,
                    }),
                );
            }
            12 => {
                b.frame.temps.insert(
                    TempId(998),
                    Location::Stack(Slot {
                        offset: 254,
                        width: 3,
                    }),
                );
            }
            13 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 254,
                        width: 3,
                    }),
                );
            }
            14 => b.code.test_delta(1),
            _ => {}
        }
        let before = format!("{:?}", b.code);
        let result = b.indexed_pointer_address(TempId(999), &a);
        if problem >= 9 {
            assert!(result.is_err(), "{problem}: {result:?}");
        } else {
            assert_eq!(result, Ok(false), "{problem}");
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
}
