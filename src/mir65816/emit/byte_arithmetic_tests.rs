use super::*;

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let mut frame = AllocatedFrame::materialized_fixture(routine).unwrap();
    for (id, offset) in [(998, 255), (999, 254)] {
        frame
            .temps
            .insert(TempId(id), Location::Stack(Slot { offset, width: 1 }));
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
    }
}

const OPERATIONS: [(NirBinaryOp, u8, Option<u8>); 5] = [
    (NirBinaryOp::Add, 0x69, Some(0x18)),
    (NirBinaryOp::Sub, 0xe9, Some(0x38)),
    (NirBinaryOp::And, 0x29, None),
    (NirBinaryOp::Or, 0x09, None),
    (NirBinaryOp::Xor, 0x49, None),
];

#[test]
fn byte_immediates_use_exact_native_encodings_in_both_entry_modes() {
    let p = word_tests::program();
    for (operation, opcode, carry) in OPERATIONS {
        for right in [0, 1, 127, 128, 255] {
            for byte_mode in [false, true] {
                for same_home in [false, true] {
                    let mut b = builder(&p.routines[0]);
                    if same_home {
                        b.frame
                            .temps
                            .insert(TempId(999), b.frame.temps[&TempId(998)]);
                    }
                    if byte_mode {
                        b.code.a8();
                    } else {
                        b.code.a16();
                    }
                    let frame = b.frame.clone();
                    let at = b.code.position();
                    assert!(
                        b.direct_byte_binary(
                            TempId(999),
                            1,
                            operation,
                            &Mir65816Value::Temp(TempId(998), ByteSize::ONE),
                            &Mir65816Value::U8(right)
                        )
                        .unwrap()
                    );
                    let mut expected = if byte_mode { vec![] } else { vec![0xe2, 0x20] };
                    expected.extend([0xa3, 255]);
                    expected.extend(carry);
                    expected.extend([opcode, right, 0x83, if same_home { 255 } else { 254 }]);
                    assert_eq!(&b.code.code().bytes[at..], expected);
                    assert_eq!(b.frame.temps, frame.temps);
                    assert_eq!(b.frame.extent, frame.extent);
                }
            }
        }
    }
}

#[test]
fn byte_immediate_preflight_refuses_unsupported_inputs_without_changing_state() {
    let p = word_tests::program();
    for problem in 0..13 {
        let mut b = builder(&p.routines[0]);
        let mut left = Mir65816Value::Temp(TempId(998), ByteSize::ONE);
        let mut right = Mir65816Value::U8(17);
        let mut operation = NirBinaryOp::Add;
        let mut width = 1;
        match problem {
            0 => width = 2,
            1 => operation = NirBinaryOp::Lsh,
            2 => right = Mir65816Value::Null(ByteSize::ONE),
            3 => right = Mir65816Value::U16(17),
            4 => left = Mir65816Value::U16(17),
            5 => {
                b.frame.temps.insert(
                    TempId(998),
                    Location::DirectPage(Slot {
                        offset: RIGHT.into(),
                        width: 1,
                    }),
                );
            }
            6 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::DirectPage(Slot {
                        offset: RIGHT.into(),
                        width: 1,
                    }),
                );
            }
            7 => {
                b.frame.temps.insert(
                    TempId(998),
                    Location::Stack(Slot {
                        offset: 255,
                        width: 2,
                    }),
                );
            }
            8 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 254,
                        width: 2,
                    }),
                );
            }
            9 => {
                b.frame.temps.insert(
                    TempId(998),
                    Location::Stack(Slot {
                        offset: 256,
                        width: 1,
                    }),
                );
            }
            10 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 256,
                        width: 1,
                    }),
                );
            }
            11 => b.code.test_delta(1),
            12 => {
                b.frame.temps.insert(
                    TempId(998),
                    Location::Stack(Slot {
                        offset: 0,
                        width: 1,
                    }),
                );
                b.code.test_delta(1);
            }
            _ => unreachable!(),
        }
        let before = format!("{:?}", b.code);
        let result = b.direct_byte_binary(TempId(999), width, operation, &left, &right);
        if problem >= 7 {
            assert!(result.is_err(), "{problem}: {result:?}");
        } else {
            assert_eq!(result, Ok(false), "{problem}");
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
}

#[test]
fn private_byte_operands_preserve_roles_and_allow_identical_homes() {
    let p = word_tests::program();
    for (operation, immediate, carry) in OPERATIONS {
        let stack_opcode = match immediate {
            0x69 => 0x63,
            0xe9 => 0xe3,
            0x29 => 0x23,
            0x09 => 0x03,
            0x49 => 0x43,
            _ => unreachable!(),
        };
        for (left, right, destination, delta) in [
            (253, 254, 255, 0),
            (255, 254, 255, 0),
            (254, 255, 255, 0),
            (255, 255, 255, 0),
            (252, 253, 254, 1),
        ] {
            for byte in [false, true] {
                let mut b = builder(&p.routines[0]);
                for (id, offset) in [(997, left), (998, right), (999, destination)] {
                    b.frame
                        .temps
                        .insert(TempId(id), Location::Stack(Slot { offset, width: 1 }));
                }
                b.code.test_delta(delta);
                if byte {
                    b.code.a8();
                } else {
                    b.code.a16();
                }
                let at = b.code.position();
                assert!(
                    b.direct_byte_binary(
                        TempId(999),
                        1,
                        operation,
                        &Mir65816Value::Temp(TempId(997), ByteSize::ONE),
                        &Mir65816Value::Temp(TempId(998), ByteSize::ONE)
                    )
                    .unwrap()
                );
                let mut expected = if byte { vec![] } else { vec![0xe2, 0x20] };
                expected.extend([0xa3, (u32::from(left) + delta) as u8]);
                expected.extend(carry);
                expected.extend([
                    stack_opcode,
                    (u32::from(right) + delta) as u8,
                    0x83,
                    (u32::from(destination) + delta) as u8,
                ]);
                assert_eq!(&b.code.code().bytes[at..], expected);
            }
        }
    }
}

#[test]
fn private_right_operand_preflight_is_atomic() {
    let p = word_tests::program();
    for (offset, bytes, delta) in [(0, 1, 0), (0, 1, 1), (255, 2, 0), (256, 1, 0), (255, 1, 1)] {
        let mut b = builder(&p.routines[0]);
        b.frame.temps.insert(
            TempId(998),
            Location::Stack(Slot {
                offset,
                width: bytes,
            }),
        );
        b.code.test_delta(delta);
        let before = format!("{:?}", b.code);
        assert!(
            b.direct_byte_binary(
                TempId(999),
                1,
                NirBinaryOp::Sub,
                &Mir65816Value::U8(17),
                &Mir65816Value::Temp(TempId(998), ByteSize::ONE)
            )
            .is_err()
        );
        assert_eq!(format!("{:?}", b.code), before);
    }
}

#[test]
fn byte_parameters_use_the_current_immutable_or_mutable_home() {
    let ast=crate::parser::parse(&crate::lexer::tokenize("BYTE FUNC Read(BYTE p) RETURN(p) BYTE FUNC Mutate(BYTE p) p==+1 RETURN(p) PROC Main() RETURN").unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    let p = crate::mir65816::lower_program(&nir).unwrap();
    for (r, mutable) in p.routines[..2].iter().zip([false, true]) {
        assert_eq!(r.frame.parameters[0].frame_object.is_some(), mutable);
        let mut b = builder(r);
        let parameter = r.frame.parameters[0].param;
        let (offset, width) = b.parameter(parameter).unwrap();
        assert_eq!(width, 1);
        b.code.a8();
        let at = b.code.position();
        assert!(
            b.direct_byte_binary(
                TempId(999),
                1,
                NirBinaryOp::Sub,
                &Mir65816Value::U8(17),
                &Mir65816Value::Param(parameter)
            )
            .unwrap()
        );
        assert_eq!(
            &b.code.code().bytes[at..],
            [0xa9, 17, 0x38, 0xe3, offset as u8, 0x83, 254]
        );
    }
}
