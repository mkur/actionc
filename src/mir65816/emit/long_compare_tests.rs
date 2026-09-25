use super::*;

fn program(source: &str) -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
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

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::new(routine).unwrap();
    let mut code = TrackedEmitter65816::for_test(&frame);
    let blocks = routine
        .blocks
        .iter()
        .map(|b| (b.id, code.label()))
        .collect();
    Builder {
        routine,
        frame,
        code,
        blocks,
        next_block: None,
        loop_x: None,
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

#[test]
fn long_preflight_checks_all_four_bytes_and_mutable_parameter_homes() {
    let p = program("BYTE FUNC Work(LONGCARD a,b) a==+1 RETURN(a=b)");
    let r = &p.routines[0];
    let (_, left, _) = operands(r);
    let Mir65816Value::Temp(id, _) = left else {
        panic!()
    };
    for (offset, delta, valid) in [
        (1, 0, true),
        (252, 0, true),
        (253, 0, false),
        (251, 1, true),
        (252, 1, false),
        (0, 0, false),
        (1, u32::MAX, false),
        (u16::MAX, 0, false),
    ] {
        let mut b = builder(r);
        b.frame
            .temps
            .insert(id, Location::Stack(Slot { offset, width: 4 }));
        b.code.test_delta(delta);
        let before = format!("{:?}", b.code);
        let result = b.long_operand(&left);
        assert_eq!(result.is_ok(), valid, "{offset}/{delta}");
        if valid {
            assert_eq!(
                result.unwrap(),
                Some(LongOperand::Stack {
                    low: (u32::from(offset) + delta) as u8,
                    high: (u32::from(offset) + delta + 2) as u8
                })
            );
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
    let b = builder(r);
    assert!(r.frame.parameters[0].frame_object.is_some());
    for param in &r.frame.parameters {
        let (offset, width) = b.parameter(param.param).unwrap();
        assert_eq!(width, 4);
        assert_eq!(
            b.long_operand(&Mir65816Value::Param(param.param)).unwrap(),
            Some(LongOperand::Stack {
                low: offset as u8,
                high: (offset + 2) as u8
            })
        );
    }
    for value in [0, 1, 0x80000000, 0xffffffff] {
        assert_eq!(
            b.long_operand(&Mir65816Value::U32(value)).unwrap(),
            Some(LongOperand::Immediate(value))
        );
    }
}

#[test]
fn long_preflight_fallbacks_and_errors_do_not_partially_emit() {
    let p = program("BYTE FUNC Work(LONGCARD a,b) RETURN(a=b)");
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let Mir65816Value::Temp(input, _) = left else {
        panic!()
    };
    for problem in 0..17 {
        let mut b = builder(r);
        let mut a = left.clone();
        let mut c = right.clone();
        match problem {
            0 => a = Mir65816Value::U8(0),
            1 => a = Mir65816Value::U16(0),
            2 => a = Mir65816Value::U24(0),
            3 => a = Mir65816Value::RoutineAddress(0, ByteSize::new(4)),
            4 => a = Mir65816Value::Null(ByteSize::new(4)),
            5 => {
                b.frame.temps.insert(
                    input,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 4,
                    }),
                );
            }
            6 => {
                b.frame.temps.insert(
                    dest,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 1,
                    }),
                );
            }
            7 => {
                b.frame.temps.remove(&dest);
            }
            8 => {
                b.frame.temps.insert(
                    dest,
                    Location::Stack(Slot {
                        offset: 1,
                        width: 2,
                    }),
                );
            }
            9 => {
                b.frame.temps.remove(&input);
            }
            10 => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 1,
                        width: 3,
                    }),
                );
            }
            11 => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 253,
                        width: 4,
                    }),
                );
            }
            12 => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 252,
                        width: 4,
                    }),
                );
                b.code.test_delta(1);
            }
            13 => {
                b.frame.temps.insert(
                    dest,
                    Location::Stack(Slot {
                        offset: 255,
                        width: 1,
                    }),
                );
                b.code.test_delta(1);
            }
            14 => {
                a = Mir65816Value::U8(0);
                c = Mir65816Value::Temp(TempId(9999), ByteSize::new(4));
            }
            15 => a = Mir65816Value::Param(ParamId(9999)),
            16 => {
                b.frame.temps.insert(
                    input,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 3,
                    }),
                );
            }
            _ => unreachable!(),
        }
        b.code.a8();
        let before = format!("{:?}", b.code);
        let result = b.native_compare(dest, 4, false, NirCompareOp::Eq, &a, &c);
        if problem < 7 {
            assert_eq!(result, Ok(false));
        } else {
            assert!(result.is_err(), "{problem}");
        }
        assert_eq!(format!("{:?}", b.code), before, "{problem}");
    }
    for signed in [false, true] {
        for op in [
            NirCompareOp::Lt,
            NirCompareOp::Le,
            NirCompareOp::Gt,
            NirCompareOp::Ge,
        ] {
            let mut b = builder(r);
            let before = format!("{:?}", b.code);
            assert!(
                !b.native_compare(dest, 4, signed, op, &left, &right)
                    .unwrap()
            );
            assert_eq!(format!("{:?}", b.code), before);
        }
    }
}

#[test]
fn long_equality_uses_two_a16_parts_and_normalizes_zero_on_either_side() {
    let p = program("BYTE FUNC Work(LONGCARD a,b) RETURN(a=b)");
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    for signed in [false, true] {
        for op in [NirCompareOp::Eq, NirCompareOp::Ne] {
            for rhs in [
                right.clone(),
                Mir65816Value::U32(0),
                Mir65816Value::U32(0x80010001),
            ] {
                let mut b = builder(r);
                let Some(Condition::Long(c)) =
                    b.condition(dest, 4, signed, op, &left, &rhs).unwrap()
                else {
                    panic!()
                };
                if rhs == Mir65816Value::U32(0) {
                    let Some(Condition::Long(reversed)) =
                        b.condition(dest, 4, signed, op, &rhs, &left).unwrap()
                    else {
                        panic!()
                    };
                    assert_eq!(c.left, reversed.left);
                    assert_eq!(reversed.right, LongOperand::Immediate(0));
                }
                let LongOperand::Stack { low, high } = c.left else {
                    panic!()
                };
                let homes = b.frame.temps.clone();
                let frame = b.frame.extent;
                assert!(b.native_compare(dest, 4, signed, op, &left, &rhs).unwrap());
                let bytes = &b.code.code().bytes;
                assert_eq!(&bytes[..4], &[0xc2, 0x20, 0xa3, low]);
                let high_at = if rhs == Mir65816Value::U32(0) {
                    10
                } else if matches!(rhs, Mir65816Value::U32(_)) {
                    13
                } else {
                    12
                };
                assert_eq!(&bytes[high_at..high_at + 2], &[0xa3, high]);
                assert_eq!(b.frame.temps, homes);
                assert_eq!(b.frame.extent, frame);
            }
        }
    }
}

#[test]
fn long_fusion_uses_existing_sole_use_proof_and_keeps_both_edges() {
    let p = program("CARD FUNC Work(LONGCARD a,b) IF a=b THEN RETURN(1) FI RETURN(0)");
    let r = &p.routines[0];
    let block = &r.blocks[0];
    let sole = liveness::sole_branch_conditions(r);
    for op in [NirCompareOp::Eq, NirCompareOp::Ne] {
        let mut operation = block.ops.last().unwrap().clone();
        let Mir65816Op::Compare {
            operation: predicate,
            ..
        } = &mut operation
        else {
            panic!()
        };
        *predicate = op;
        let mut b = builder(r);
        let before = format!("{:?}", b.code);
        assert!(
            !b.compare_branch(&operation, &block.terminator, &BTreeSet::new())
                .unwrap()
        );
        assert_eq!(format!("{:?}", b.code), before);
        let homes = b.frame.temps.clone();
        let frame = b.frame.extent;
        assert!(
            b.compare_branch(&operation, &block.terminator, &sole)
                .unwrap()
        );
        assert_eq!(
            b.code
                .code()
                .conditional_branches
                .iter()
                .filter(|s| s.dispatch)
                .count(),
            if op == NirCompareOp::Eq { 1 } else { 2 }
        );
        let Mir65816Terminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &block.terminator
        else {
            panic!()
        };
        for target in [then_edge.target, else_edge.target] {
            assert!(
                b.code
                    .code()
                    .fixups
                    .iter()
                    .any(|f| f.target == Target::Label(b.blocks[&target]))
            );
        }
        assert_eq!(b.frame.temps, homes);
        assert_eq!(b.frame.extent, frame);
    }
}

#[test]
fn long_sign_checks_complete_homes_and_only_admits_sign_only_predicates() {
    let p = program("BYTE FUNC Work(LONGINT a,b) RETURN(a<b)");
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let zero = Mir65816Value::U32(0);
    for signed in [false, true] {
        for op in [
            NirCompareOp::Lt,
            NirCompareOp::Le,
            NirCompareOp::Gt,
            NirCompareOp::Ge,
        ] {
            for reverse in [false, true] {
                let mut b = builder(r);
                let (a, c) = if reverse {
                    (&zero, &left)
                } else {
                    (&left, &zero)
                };
                let admitted = signed
                    && if reverse {
                        matches!(op, NirCompareOp::Gt | NirCompareOp::Le)
                    } else {
                        matches!(op, NirCompareOp::Lt | NirCompareOp::Ge)
                    };
                let before = format!("{:?}", b.code);
                assert_eq!(
                    b.native_compare(dest, 4, signed, op, a, c).unwrap(),
                    admitted
                );
                if admitted {
                    assert!(b.code.code().bytes.len() <= 14);
                } else {
                    assert_eq!(format!("{:?}", b.code), before);
                }
            }
        }
    }
    let Mir65816Value::Temp(id, _) = left else {
        panic!()
    };
    for (offset, valid) in [(1, true), (252, true), (253, false), (0, false)] {
        let mut b = builder(r);
        b.frame
            .temps
            .insert(id, Location::Stack(Slot { offset, width: 4 }));
        let before = format!("{:?}", b.code);
        let c = b.condition(dest, 4, true, NirCompareOp::Lt, &left, &zero);
        assert_eq!(c.is_ok(), valid);
        if valid {
            let Some(Condition::LongSign(c)) = c.unwrap() else {
                panic!()
            };
            assert_eq!(c.source, ByteOperand::Stack(offset as u8 + 3));
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
    let mut b = builder(r);
    b.frame.temps.insert(
        id,
        Location::DirectPage(Slot {
            offset: 0,
            width: 4,
        }),
    );
    let before = format!("{:?}", b.code);
    assert!(
        !b.native_compare(dest, 4, true, NirCompareOp::Lt, &left, &zero)
            .unwrap()
    );
    assert_eq!(format!("{:?}", b.code), before);
    assert!(
        b.condition(dest, 4, true, NirCompareOp::Lt, &right, &zero)
            .unwrap()
            .is_some()
    );
}
