use super::*;

fn program() -> Mir65816Program {
    let ast = crate::parser::parse(
        &crate::lexer::tokenize("LONGCARD FUNC Work(LONGCARD a,b) a=a+b RETURN(a-b)").unwrap(),
    )
    .unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    crate::mir65816::lower_program(&nir).unwrap()
}

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::stack(routine).unwrap();
    let code = TrackedEmitter65816::for_test(&frame);
    Builder {
        routine,
        frame,
        code,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
    }
}

fn operands(r: &Mir65816Routine) -> (TempId, Mir65816Value, Mir65816Value) {
    r.blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| match op {
            Mir65816Op::Binary {
                dest, left, right, ..
            } => Some((*dest, left.clone(), right.clone())),
            _ => None,
        })
        .unwrap()
}

#[test]
fn long_arithmetic_preflights_last_bytes_delta_and_mutable_parameter_homes() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let Mir65816Value::Temp(input, _) = left else {
        panic!()
    };
    for id in [dest, input] {
        for (offset, delta, valid) in [
            (0, 0, false),
            (252, 0, true),
            (253, 0, false),
            (251, 1, true),
            (252, 1, false),
            (2, u32::MAX, false),
        ] {
            let mut b = builder(r);
            b.frame
                .temps
                .insert(id, Location::Stack(Slot { offset, width: 4 }));
            b.code.test_delta(delta);
            b.code.a8();
            let before = format!("{:?}", b.code);
            let result = b.long_binary(dest, 4, NirBinaryOp::Add, &left, &right);
            if valid {
                assert_eq!(result, Ok(true), "{id:?}/{offset}/{delta}");
            } else {
                assert!(result.is_err(), "{id:?}/{offset}/{delta}");
                assert_eq!(format!("{:?}", b.code), before);
            }
        }
    }
    let b = builder(r);
    assert!(r.frame.parameters[0].frame_object.is_some());
    for param in &r.frame.parameters {
        let (at, bytes) = b.parameter(param.param).unwrap();
        assert_eq!(bytes, 4);
        assert_eq!(
            b.long_arithmetic_operand(&Mir65816Value::Param(param.param))
                .unwrap(),
            Some(LongOperand::Stack {
                low: at as u8,
                high: (at + 2) as u8
            })
        );
    }
    for (value, expected) in [
        (Mir65816Value::U8(255), 255),
        (Mir65816Value::U16(0xffff), 0xffff),
        (Mir65816Value::U24(0xffffff), 0xffffff),
        (Mir65816Value::U32(0x80000000), 0x80000000),
        (Mir65816Value::U32(u32::MAX), u32::MAX),
    ] {
        assert_eq!(
            b.long_arithmetic_operand(&value).unwrap(),
            Some(LongOperand::Immediate(expected))
        );
    }
}

#[test]
fn long_arithmetic_fallbacks_and_bad_homes_leave_emitter_untouched() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let Mir65816Value::Temp(input, _) = left else {
        panic!()
    };
    for case in 0..12 {
        let mut b = builder(r);
        let mut a = left.clone();
        let mut rhs = right.clone();
        match case {
            0 => a = Mir65816Value::Null(ByteSize::new(4)),
            1 => a = Mir65816Value::RoutineAddress(0, ByteSize::new(4)),
            2 => {
                b.frame.temps.insert(
                    input,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 4,
                    }),
                );
            }
            3 => {
                b.frame.temps.insert(
                    dest,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 4,
                    }),
                );
            }
            4 => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 2,
                        width: 2,
                    }),
                );
                a = Mir65816Value::Temp(input, ByteSize::new(2));
            }
            5 => {
                b.frame.temps.remove(&dest);
            }
            6 => {
                b.frame.temps.insert(
                    dest,
                    Location::Stack(Slot {
                        offset: 2,
                        width: 3,
                    }),
                );
            }
            7 => {
                b.frame.temps.remove(&input);
            }
            8 => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 2,
                        width: 3,
                    }),
                );
            }
            9 => a = Mir65816Value::Param(ParamId(9999)),
            10 => {
                a = Mir65816Value::Null(ByteSize::new(4));
                rhs = Mir65816Value::Temp(TempId(9999), ByteSize::new(4));
            }
            11 => {
                b.frame.temps.insert(
                    dest,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 4,
                    }),
                );
                rhs = Mir65816Value::Temp(TempId(9999), ByteSize::new(4));
            }
            _ => unreachable!(),
        }
        b.code.a8();
        let before = format!("{:?}", b.code);
        let result = b.long_binary(dest, 4, NirBinaryOp::Sub, &a, &rhs);
        if case < 5 {
            assert_eq!(result, Ok(false), "{case}");
        } else {
            assert!(result.is_err(), "{case}");
        }
        assert_eq!(format!("{:?}", b.code), before, "{case}");
    }
    for (bytes, operation) in [
        (1, NirBinaryOp::Add),
        (2, NirBinaryOp::Sub),
        (3, NirBinaryOp::Add),
        (4, NirBinaryOp::Mul),
    ] {
        let mut b = builder(r);
        let before = format!("{:?}", b.code);
        assert_eq!(
            b.long_binary(dest, bytes, operation, &left, &right),
            Ok(false)
        );
        assert_eq!(format!("{:?}", b.code), before);
    }
}

#[test]
fn long_arithmetic_admits_identity_and_disjoint_homes_but_not_partial_overlap() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let Mir65816Value::Temp(a, _) = left else {
        panic!()
    };
    let Mir65816Value::Temp(b, _) = right else {
        panic!()
    };
    for operation in [
        NirBinaryOp::Add,
        NirBinaryOp::Sub,
        NirBinaryOp::And,
        NirBinaryOp::Or,
        NirBinaryOp::Xor,
    ] {
        for (first, second, valid) in [
            (32, 40, true),
            (40, 32, true),
            (32, 32, true),
            (28, 36, true),
            (29, 40, false),
            (31, 40, false),
            (33, 40, false),
            (40, 30, false),
            (40, 34, false),
            (40, 35, false),
        ] {
            let mut builder = builder(r);
            for (id, offset) in [(dest, 32), (a, first), (b, second)] {
                builder
                    .frame
                    .temps
                    .insert(id, Location::Stack(Slot { offset, width: 4 }));
            }
            builder.code.a8();
            let before = format!("{:?}", builder.code);
            assert_eq!(
                builder.long_binary(dest, 4, operation, &left, &right),
                Ok(valid)
            );
            if !valid {
                assert_eq!(format!("{:?}", builder.code), before);
            }
        }
    }
}

#[test]
fn native_long_bitwise_reads_two_checked_words_without_carry_or_scratch() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let Mir65816Value::Temp(a, _) = left else {
        panic!()
    };
    let Mir65816Value::Temp(b, _) = right else {
        panic!()
    };
    for (op, stack, imm) in [
        (NirBinaryOp::And, 0x23, 0x29),
        (NirBinaryOp::Or, 0x03, 0x09),
        (NirBinaryOp::Xor, 0x43, 0x49),
    ] {
        for literal in [false, true] {
            let mut s = builder(r);
            for (id, offset) in [(a, 32), (b, 40), (dest, 48)] {
                s.frame
                    .temps
                    .insert(id, Location::Stack(Slot { offset, width: 4 }));
            }
            let rhs = if literal {
                Mir65816Value::U32(0x800100ff)
            } else {
                right.clone()
            };
            assert!(s.long_binary(dest, 4, op, &left, &rhs).unwrap());
            let mut expected = vec![0xc2, 0x20];
            for half in [0, 2] {
                expected.extend([0xa3, 32 + half]);
                if literal {
                    expected.extend([
                        imm,
                        if half == 0 { 0xff } else { 1 },
                        if half == 0 { 0 } else { 0x80 },
                    ]);
                } else {
                    expected.extend([stack, 40 + half]);
                }
                expected.extend([0x83, 48 + half]);
            }
            assert_eq!(s.code.code().bytes, expected);
        }
        let mut s = builder(r);
        s.frame.temps.insert(
            b,
            Location::Stack(Slot {
                offset: 253,
                width: 4,
            }),
        );
        let before = format!("{:?}", s.code);
        assert!(s.long_binary(dest, 4, op, &left, &right).is_err());
        assert_eq!(before, format!("{:?}", s.code));
    }
}
