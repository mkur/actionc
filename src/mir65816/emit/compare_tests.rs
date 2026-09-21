use super::*;

fn program() -> Mir65816Program {
    let ast = crate::parser::parse(
        &crate::lexer::tokenize("BYTE FUNC Work(CARD a,b) a==+1 RETURN(a<b) PROC Main() RETURN")
            .unwrap(),
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
    super::super::super::lower_program(&nir).unwrap()
}
fn builder(r: &Mir65816Routine) -> Builder<'_> {
    Builder {
        next_block: None,
        routine: r,
        frame: AllocatedFrame::new(r).unwrap(),
        code: TrackedEmitter65816::for_test(&AllocatedFrame::new(r).unwrap()),
        blocks: BTreeMap::new(),
    }
}
fn operands(r: &Mir65816Routine) -> (TempId, Mir65816Value, Mir65816Value) {
    r.blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| match op {
            Mir65816Op::Compare {
                dest, left, right, ..
            } => Some((*dest, left.clone(), right.clone())),
            _ => None,
        })
        .unwrap()
}
fn encode(operand: WordOperand, load: bool) -> Vec<u8> {
    match operand {
        WordOperand::Immediate(n) => vec![if load { 0xa9 } else { 0xc9 }, n as u8, (n >> 8) as u8],
        WordOperand::Stack(d) => vec![if load { 0xa3 } else { 0xc3 }, d],
    }
}

#[test]
fn word_comparison_predicates_use_checked_operands_and_byte_results() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    assert!(r.frame.parameters[0].frame_object.is_some());
    let pairs = [
        (left.clone(), right.clone()),
        (left.clone(), Mir65816Value::U16(0x8000)),
        (Mir65816Value::U8(255), right.clone()),
        (
            Mir65816Value::Param(r.frame.parameters[0].param),
            Mir65816Value::Param(r.frame.parameters[1].param),
        ),
        (Mir65816Value::U16(0xffff), Mir65816Value::U16(0)),
        (left.clone(), left.clone()),
    ];
    for (op, branch, swap) in [
        (NirCompareOp::Eq, 0xf0, false),
        (NirCompareOp::Ne, 0xd0, false),
        (NirCompareOp::Lt, 0x90, false),
        (NirCompareOp::Ge, 0xb0, false),
        (NirCompareOp::Gt, 0x90, true),
        (NirCompareOp::Le, 0xb0, true),
    ] {
        for signed in [false, true] {
            if signed && !matches!(op, NirCompareOp::Eq | NirCompareOp::Ne) {
                continue;
            }
            for (left, right) in &pairs {
                let mut b = builder(r);
                let dest_offset = b.temp(dest).unwrap().slot().offset;
                b.code.a16();
                let prefix = b.code.code().bytes.len();
                let a = b.word_operand(left).unwrap().unwrap();
                let c = b.word_operand(right).unwrap().unwrap();
                let (a, c) = if swap { (c, a) } else { (a, c) };
                let mut expected = encode(a, true);
                expected.extend(encode(c, false));
                expected.extend([
                    branch ^ 0x20,
                    4,
                    0x5c,
                    0,
                    0,
                    0,
                    0xe2,
                    0x20,
                    0xa9,
                    0,
                    0x5c,
                    0,
                    0,
                    0,
                    0xe2,
                    0x20,
                    0xa9,
                    1,
                    0xe2,
                    0x20,
                    0x83,
                    dest_offset as u8,
                ]);
                let frame = b.frame.clone();
                b.operation(&Mir65816Op::Compare {
                    dest,
                    width: ByteSize::new(2),
                    signed,
                    operation: op,
                    left: left.clone(),
                    right: right.clone(),
                })
                .unwrap();
                assert_eq!(&b.code.code().bytes[prefix..], expected);
                assert_eq!(b.code.code().fixups.len(), 2);
                assert_eq!(b.code.code().labels.len(), 2);
                assert_eq!(b.code.code().fixups[0].target, Target::Label(Label(0)));
                assert_eq!(b.code.code().fixups[1].target, Target::Label(Label(1)));
                let end = b.code.code().bytes.clone();
                b.code.a8();
                assert_eq!(b.code.code().bytes, end);
                assert_eq!(b.frame.temps, frame.temps);
                assert_eq!(b.frame.extent, frame.extent);
                assert_eq!(b.frame.edge_copies, frame.edge_copies);
            }
        }
    }
}

#[test]
fn comparison_fallback_is_nonmutating_for_signed_order_and_unsupported_homes() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let Mir65816Value::Temp(input, _) = left else {
        panic!()
    };
    for problem in 0..14 {
        let mut b = builder(r);
        let (mut bytes, mut signed, mut op) = (2, false, NirCompareOp::Eq);
        let mut left = left.clone();
        match problem {
            0..=3 => {
                signed = true;
                op = [
                    NirCompareOp::Lt,
                    NirCompareOp::Le,
                    NirCompareOp::Gt,
                    NirCompareOp::Ge,
                ][problem];
            }
            4..=6 => bytes = [1, 3, 4][problem - 4],
            7 => {
                b.frame.temps.insert(
                    dest,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 1,
                    }),
                );
            }
            8 => {
                b.frame.temps.insert(
                    input,
                    Location::DirectPage(Slot {
                        offset: 2,
                        width: 2,
                    }),
                );
            }
            9 => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 2,
                        width: 1,
                    }),
                );
                left = Mir65816Value::Temp(input, ByteSize::ONE);
            }
            10 => left = Mir65816Value::U24(1),
            11 => left = Mir65816Value::U32(1),
            12 => left = Mir65816Value::Null(ByteSize::new(2)),
            13 => left = Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
            _ => unreachable!(),
        }
        b.code.a8();
        let before = format!("{:?}", b.code);
        assert!(
            !b.word_compare(dest, bytes, signed, op, &left, &right)
                .unwrap()
        );
        assert_eq!(format!("{:?}", b.code), before, "problem {problem}");
    }
}

#[test]
fn comparison_preflight_reports_malformed_later_operands_without_emission() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let Mir65816Value::Temp(input, _) = right else {
        panic!()
    };
    for problem in 0..9 {
        let mut routine = r.clone();
        let mut b = builder(r);
        let mut right = right.clone();
        match problem {
            0 => {
                b.frame.temps.remove(&dest);
            }
            1 => {
                b.frame.temps.insert(
                    dest,
                    Location::Stack(Slot {
                        offset: 1,
                        width: 2,
                    }),
                );
            }
            2 => {
                b.frame.temps.remove(&input);
            }
            3 => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 2,
                        width: 1,
                    }),
                );
            }
            4 => {
                b.frame.temps.insert(
                    input,
                    Location::DirectPage(Slot {
                        offset: 2,
                        width: 1,
                    }),
                );
            }
            5..=8 => {
                right = Mir65816Value::Param(r.frame.parameters[0].param);
                match problem {
                    5 => routine.frame.parameters.clear(),
                    6 => routine.frame.parameters[0].incoming = Mir65816AbiHome::Accumulator,
                    7 => {
                        routine.frame.objects.clear();
                    }
                    8 => {
                        routine.frame.parameters[0].frame_object =
                            Some(Mir65816FrameObjectId(u32::MAX))
                    }
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        b.routine = &routine;
        b.code.a16();
        let before = format!("{:?}", b.code);
        assert!(
            b.word_compare(dest, 2, false, NirCompareOp::Eq, &left, &right)
                .is_err(),
            "problem {problem}"
        );
        assert_eq!(format!("{:?}", b.code), before);
        if problem >= 2 {
            assert!(
                b.word_compare(
                    dest,
                    2,
                    false,
                    NirCompareOp::Eq,
                    &Mir65816Value::U24(1),
                    &right
                )
                .is_err()
            );
            assert_eq!(format!("{:?}", b.code), before);
        }
    }
}

#[test]
fn comparison_extent_checks_distinguish_the_byte_result_and_word_inputs() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, _) = operands(r);
    let Mir65816Value::Temp(input, _) = left else {
        panic!()
    };
    for (source, result, delta, valid) in [
        (254, 255, 0, true),
        (255, 254, 0, false),
        (253, 254, 1, true),
        (254, 254, 1, false),
        (253, 255, 1, false),
        (0, 1, 0, false),
        (1, 0, 0, false),
        (2, 1, u32::MAX, false),
        (2, 2, 0, true),
        (2, 3, 0, true),
    ] {
        let mut b = builder(r);
        b.frame.temps.insert(
            input,
            Location::Stack(Slot {
                offset: source,
                width: 2,
            }),
        );
        b.frame.temps.insert(
            dest,
            Location::Stack(Slot {
                offset: result,
                width: 1,
            }),
        );
        b.code.test_delta(delta);
        b.code.a8();
        let before = format!("{:?}", b.code);
        assert_eq!(
            b.word_compare(
                dest,
                2,
                false,
                NirCompareOp::Le,
                &left,
                &Mir65816Value::U8(255)
            )
            .is_ok(),
            valid,
            "{source}/{result}/{delta}"
        );
        if !valid {
            assert_eq!(format!("{:?}", b.code), before);
        } else {
            assert_eq!(
                &b.code.code().bytes[b.code.code().bytes.len() - 2..],
                [0x83, (u32::from(result) + delta) as u8]
            );
        }
    }
}
