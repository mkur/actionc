use super::*;

fn program() -> Mir65816Program {
    let ast = crate::parser::parse(
        &crate::lexer::tokenize(
            "CARD FUNC Work(CARD a,b) IF a<b THEN RETURN(a) FI RETURN(b) PROC Main() RETURN",
        )
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
    super::super::super::lower_program(&nir).unwrap()
}
fn builder(r: &Mir65816Routine) -> Builder<'_> {
    let mut b = Builder {
        routine: r,
        frame: AllocatedFrame::new(r).unwrap(),
        code: TrackedEmitter65816::for_test(&AllocatedFrame::new(r).unwrap()),
        blocks: BTreeMap::new(),
    };
    for block in &r.blocks {
        b.blocks.insert(block.id, b.code.label());
    }
    b
}
#[test]
fn fused_selection_uses_one_dispatch_and_retains_both_edges_and_frame() {
    let p = program();
    let r = &p.routines[0];
    let block = &r.blocks[0];
    let sole = liveness::sole_branch_conditions(r);
    for (predicate, opcode, swap) in [
        (NirCompareOp::Eq, 0xf0, false),
        (NirCompareOp::Ne, 0xd0, false),
        (NirCompareOp::Lt, 0x90, false),
        (NirCompareOp::Ge, 0xb0, false),
        (NirCompareOp::Gt, 0x90, true),
        (NirCompareOp::Le, 0xb0, true),
    ] {
        for signed in [false, true] {
            if signed && !matches!(predicate, NirCompareOp::Eq | NirCompareOp::Ne) {
                continue;
            }
            let mut b = builder(r);
            let mut op = block.ops.last().unwrap().clone();
            let Mir65816Op::Compare {
                operation,
                signed: is_signed,
                left,
                right,
                ..
            } = &mut op
            else {
                panic!()
            };
            *operation = predicate;
            *is_signed = signed;
            let a = b.word_operand(left).unwrap().unwrap();
            let c = b.word_operand(right).unwrap().unwrap();
            let (a, c) = if swap { (c, a) } else { (a, c) };
            let encode = |operand, load| match operand {
                WordOperand::Stack(d) => vec![if load { 0xa3 } else { 0xc3 }, d],
                WordOperand::Immediate(n) => {
                    vec![if load { 0xa9 } else { 0xc9 }, n as u8, (n >> 8) as u8]
                }
            };
            b.code.a16();
            let prefix = b.code.code().bytes.len();
            let frame = b.frame.clone();
            let mut expected = encode(a, true);
            expected.extend(encode(c, false));
            expected.extend([opcode ^ 0x20, 4, 0x5c, 0, 0, 0]);
            expected.extend([0x5c, 0, 0, 0]); // false edge already knows A16
            expected.extend([0xc2, 0x20, 0x5c, 0, 0, 0]); // true label resets knowledge
            assert!(b.compare_branch(&op, &block.terminator, &sole).unwrap());
            assert_eq!(&b.code.code().bytes[prefix..], expected);
            assert_eq!(b.code.code().fixups.len(), 3);
            let Mir65816Terminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &block.terminator
            else {
                panic!()
            };
            assert_eq!(
                b.code.code().fixups[1].target,
                Target::Label(b.blocks[&else_edge.target])
            );
            assert_eq!(
                b.code.code().fixups[2].target,
                Target::Label(b.blocks[&then_edge.target])
            );
            assert_eq!(b.frame.temps, frame.temps);
            assert_eq!(b.frame.extent, frame.extent);
            let end = b.code.code().bytes.clone();
            b.code.a16();
            assert_eq!(end, b.code.code().bytes);
        }
    }
}
#[test]
fn pair_gates_and_checked_homes_fail_without_partial_comparison_emission() {
    let p = program();
    let r = &p.routines[0];
    let block = &r.blocks[0];
    for problem in 0..12 {
        let mut b = builder(r);
        let mut op = block.ops.last().unwrap().clone();
        let mut term = block.terminator.clone();
        let mut sole = liveness::sole_branch_conditions(r);
        let Mir65816Op::Compare {
            dest,
            width,
            signed,
            operation,
            left,
            ..
        } = &mut op
        else {
            panic!()
        };
        match problem {
            0 => sole.clear(),
            1 => {
                let Mir65816Terminator::Branch { condition, .. } = &mut term else {
                    panic!()
                };
                *condition = Mir65816Value::U8(1);
            }
            2 => {
                let Mir65816Terminator::Branch { condition, .. } = &mut term else {
                    panic!()
                };
                *condition = Mir65816Value::Temp(TempId(999), ByteSize::ONE);
            }
            3 => *width = ByteSize::ONE,
            4 => {
                *signed = true;
                *operation = NirCompareOp::Lt;
            }
            5 => {
                b.frame.temps.insert(
                    *dest,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 1,
                    }),
                );
            }
            6 => *left = Mir65816Value::U24(0),
            7 => {
                b.frame.temps.remove(dest);
            }
            8 => {
                b.frame.temps.insert(
                    *dest,
                    Location::Stack(Slot {
                        offset: 1,
                        width: 2,
                    }),
                );
            }
            9 => {
                b.frame.temps.insert(
                    *dest,
                    Location::Stack(Slot {
                        offset: 255,
                        width: 1,
                    }),
                );
                b.code.test_delta(1);
            }
            10 => {
                let Mir65816Value::Temp(id, ..) = left else {
                    panic!()
                };
                b.frame.temps.insert(
                    *id,
                    Location::Stack(Slot {
                        offset: 254,
                        width: 2,
                    }),
                );
                b.code.test_delta(1);
            }
            11 => {
                let Mir65816Value::Temp(id, ..) = left else {
                    panic!()
                };
                b.frame.temps.remove(id);
            }
            _ => unreachable!(),
        }
        b.code.a8();
        let before = format!("{:?}", b.code);
        let result = b.compare_branch(&op, &term, &sole);
        if problem < 7 {
            assert_eq!(result, Ok(false));
        } else {
            assert!(result.is_err(), "{problem}");
        }
        assert_eq!(format!("{:?}", b.code), before, "{problem}");
    }
}
#[test]
fn fusion_obeys_byte_and_word_limits_after_transient_stack_movement() {
    let p = program();
    let r = &p.routines[0];
    let block = &r.blocks[0];
    let mut b = builder(r);
    let mut op = block.ops.last().unwrap().clone();
    let Mir65816Op::Compare {
        dest, left, right, ..
    } = &mut op
    else {
        panic!()
    };
    let Mir65816Value::Temp(input, ..) = left else {
        panic!()
    };
    b.frame.temps.insert(
        *input,
        Location::Stack(Slot {
            offset: 253,
            width: 2,
        }),
    );
    b.frame.temps.insert(
        *dest,
        Location::Stack(Slot {
            offset: 254,
            width: 1,
        }),
    );
    *right = Mir65816Value::U8(255);
    b.code.test_delta(1);
    assert!(
        b.compare_branch(&op, &block.terminator, &liveness::sole_branch_conditions(r))
            .unwrap()
    );
    assert_eq!(
        &b.code.code().bytes[..7],
        &[0xc2, 0x20, 0xa3, 254, 0xc9, 255, 0]
    );
}
