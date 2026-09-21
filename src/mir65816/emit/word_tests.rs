use super::*;

fn program() -> Mir65816Program {
    let ast = crate::parser::parse(
        &crate::lexer::tokenize("CARD FUNC Work(CARD a,b) RETURN(a+b) PROC Main() RETURN").unwrap(),
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

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    Builder {
        routine,
        frame: AllocatedFrame::new(routine).unwrap(),
        code: Code::default(),
        blocks: BTreeMap::new(),
        delta: 0,
    }
}

fn operands(routine: &Mir65816Routine) -> (TempId, Mir65816Value, Mir65816Value) {
    routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| {
            if let Mir65816Op::Binary {
                dest, left, right, ..
            } = op
            {
                Some((*dest, left.clone(), right.clone()))
            } else {
                None
            }
        })
        .unwrap()
}

#[test]
fn word_classification_preserves_immediate_bits_and_parameter_homes() {
    let p = program();
    let r = &p.routines[0];
    let b = builder(r);
    let (_, left, _) = operands(r);
    assert_eq!(
        b.word_operand(&Mir65816Value::U8(255)).unwrap(),
        Some(WordOperand::Immediate(255))
    );
    assert_eq!(
        b.word_operand(&Mir65816Value::U16(0x8000)).unwrap(),
        Some(WordOperand::Immediate(0x8000))
    );
    assert!(matches!(
        b.word_operand(&left).unwrap(),
        Some(WordOperand::Stack(_))
    ));
    for parameter in &r.frame.parameters {
        assert_eq!(
            b.word_operand(&Mir65816Value::Param(parameter.param))
                .unwrap(),
            Some(WordOperand::Stack(
                b.parameter(parameter.param).unwrap().0 as u8
            ))
        );
    }
}

#[test]
fn word_extent_checks_last_byte_and_transient_reservations() {
    let p = program();
    let mut b = builder(&p.routines[0]);
    for (delta, body, accepted) in [
        (0, 1, true),
        (0, 254, true),
        (0, 255, false),
        (0, 0, false),
        (1, 253, true),
        (1, 254, false),
        (2, 253, false),
        (u32::MAX, 2, false),
    ] {
        b.delta = delta;
        assert_eq!(
            b.word_displacement(body).is_ok(),
            accepted,
            "{delta}/{body}"
        );
    }
}

#[test]
fn unsupported_operands_do_not_emit_a_prefix_or_change_mode_knowledge() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, _) = operands(r);
    let Mir65816Value::Temp(input, _) = left else {
        panic!()
    };
    for unsupported in [
        Mir65816Value::U24(1),
        Mir65816Value::U32(1),
        Mir65816Value::Null(ByteSize::new(2)),
        Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
        Mir65816Value::Temp(input, ByteSize::ONE),
        Mir65816Value::Temp(input, ByteSize::new(2)),
    ] {
        let mut b = builder(r);
        match unsupported {
            Mir65816Value::Temp(_, w) if w == ByteSize::ONE => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 2,
                        width: 1,
                    }),
                );
            }
            Mir65816Value::Temp(_, _) => {
                b.frame.temps.insert(
                    input,
                    Location::DirectPage(Slot {
                        offset: 0,
                        width: 2,
                    }),
                );
            }
            _ => {}
        }
        b.code.a8();
        let before = b.code.bytes.clone();
        assert!(
            !b.word_binary(
                dest,
                2,
                NirBinaryOp::Add,
                &Mir65816Value::U16(1),
                &unsupported
            )
            .unwrap()
        );
        assert_eq!(b.code.bytes, before);
        b.code.a8();
        assert_eq!(
            b.code.bytes, before,
            "fallback changed local mode knowledge"
        );
        assert!(b.code.fixups.is_empty());
    }
    let mut b = builder(r);
    assert!(
        !b.word_binary(dest, 2, NirBinaryOp::Xor, &left, &left)
            .unwrap()
    );
    assert!(
        !b.word_binary(dest, 1, NirBinaryOp::Add, &left, &left)
            .unwrap()
    );
    assert!(b.code.bytes.is_empty());
}

#[test]
fn malformed_word_operands_fail_before_mutating_code() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let Mir65816Value::Temp(input, _) = right else {
        panic!()
    };
    for problem in 0..6 {
        let mut b = builder(r);
        match problem {
            0 => {
                b.frame.temps.remove(&dest);
            }
            1 => {
                b.frame.temps.insert(
                    dest,
                    Location::Stack(Slot {
                        offset: 6,
                        width: 1,
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
                        offset: 4,
                        width: 3,
                    }),
                );
            }
            4 => {
                b.frame.temps.insert(
                    input,
                    Location::Stack(Slot {
                        offset: 255,
                        width: 2,
                    }),
                );
            }
            5 => {
                b.frame.temps.insert(
                    dest,
                    Location::Stack(Slot {
                        offset: 255,
                        width: 2,
                    }),
                );
            }
            _ => unreachable!(),
        }
        b.code.a8();
        let before = b.code.bytes.clone();
        assert!(
            b.word_binary(dest, 2, NirBinaryOp::Sub, &left, &right)
                .is_err(),
            "problem {problem}"
        );
        assert_eq!(b.code.bytes, before);
        b.code.a8();
        assert_eq!(b.code.bytes, before);
    }
    // A legal fallback operand must not hide a malformed operand on the right.
    let mut b = builder(r);
    b.frame.temps.remove(&input);
    assert!(
        b.word_binary(dest, 2, NirBinaryOp::Add, &Mir65816Value::U24(1), &right)
            .is_err()
    );
    assert!(b.code.bytes.is_empty());
}

#[test]
fn operation_dispatch_keeps_native_width_without_changing_allocations() {
    let p = program();
    let r = &p.routines[0];
    let (dest, left, right) = operands(r);
    let mut b = builder(r);
    let before = b.frame.clone();
    b.code.a16();
    let prefix = b.code.bytes.len();
    b.operation(&Mir65816Op::Binary {
        dest,
        width: ByteSize::new(2),
        signed: false,
        operation: NirBinaryOp::Sub,
        left,
        right,
    })
    .unwrap();
    // Isolated selector output: operands are displacements, never scanned as
    // opcodes in a whole program. This covers the dispatcher's A8 trap.
    let code = &b.code.bytes[prefix..];
    assert_eq!(code.len(), 7);
    assert_eq!(
        [code[0], code[2], code[3], code[5]],
        [0xa3, 0x38, 0xe3, 0x83]
    );
    assert_eq!(b.frame.temps, before.temps);
    assert_eq!(b.frame.extent, before.extent);
    assert_eq!(b.frame.spill_bytes, before.spill_bytes);
    assert_eq!(b.frame.peak_below_entry, before.peak_below_entry);
    assert_eq!(b.frame.edge_copies, before.edge_copies);
}
