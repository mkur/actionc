use super::*;

fn program() -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(
        "BYTE FUNC Echo(BYTE value) RETURN(value) BYTE FUNC Mutate(BYTE value) value==+1 RETURN(value) PROC Main() RETURN",
    ).unwrap()).unwrap();
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
        borrowed: BTreeMap::new(),
    }
}

#[test]
fn captured_byte_returns_read_exactly_one_byte_then_share_native_teardown() {
    let p = program();
    for r in &p.routines[..2] {
        for parameter in [false, true] {
            for a8 in [false, true] {
                for zero_frame in [false, true] {
                    let mut b = builder(r);
                    let value = if parameter {
                        Mir65816Value::Param(r.frame.parameters[0].param)
                    } else {
                        b.frame.temps.insert(
                            TempId(999),
                            Location::Stack(Slot {
                                offset: 255,
                                width: 1,
                            }),
                        );
                        Mir65816Value::Temp(TempId(999), ByteSize::ONE)
                    };
                    if zero_frame {
                        b.frame.extent = 0;
                        b.code.test_frame(0);
                    }
                    let source = match b.byte_operand(&value).unwrap().unwrap() {
                        ByteOperand::Stack(offset) => offset,
                        _ => panic!(),
                    };
                    if parameter {
                        let param = &r.frame.parameters[0];
                        if let Some(object) = param.frame_object {
                            assert_eq!(u32::from(source), b.object(object).unwrap());
                        } else {
                            assert_eq!(u32::from(source), b.incoming(param.param).unwrap());
                        }
                    }
                    if a8 {
                        b.code.a8();
                    } else {
                        b.code.a16();
                    }
                    let prefix = b.code.position();
                    let frame = b.frame.clone();
                    b.return_value(Some(&value)).unwrap();
                    let mut expected = if a8 { vec![] } else { vec![0xe2, 0x20] };
                    expected.extend([0xa3, source, 0xc2, 0x20, 0x29, 0xff, 0]);
                    if !zero_frame {
                        expected.extend([
                            0xa8,
                            0x3b,
                            0x18,
                            0x69,
                            frame.extent as u8,
                            (frame.extent >> 8) as u8,
                            0x1b,
                            0x98,
                        ]);
                    }
                    expected.push(0x6b);
                    assert_eq!(&b.code.code().bytes[prefix..], expected);
                    assert_eq!(b.frame.temps, frame.temps);
                    assert_eq!(b.frame.edge_copies, frame.edge_copies);
                    assert_eq!(
                        (
                            b.frame.extent,
                            b.frame.spill_bytes,
                            b.frame.peak_below_entry
                        ),
                        (frame.extent, frame.spill_bytes, frame.peak_below_entry)
                    );
                    assert!(
                        b.code.code().fixups.is_empty() && b.code.code().return_fixups.is_empty()
                    );
                }
            }
        }
    }
}

#[test]
fn captured_byte_return_preflight_checks_width_last_displacement_and_delta() {
    let p = program();
    let r = &p.routines[0];
    for (offset, delta, width, valid) in [
        (255, 0, 1, true),
        (254, 1, 1, true),
        (255, 1, 1, false),
        (0, 0, 1, false),
        (1, u32::MAX, 1, false),
        (1, 0, 2, false),
    ] {
        let mut b = builder(r);
        b.frame
            .temps
            .insert(TempId(999), Location::Stack(Slot { offset, width }));
        b.code.test_delta(delta);
        b.code.a8();
        let prefix = b.code.code().bytes.clone();
        let result = b.captured_byte_return(&Mir65816Value::Temp(TempId(999), ByteSize::ONE));
        if valid {
            assert!(result.unwrap());
        } else {
            assert!(result.is_err());
            b.code.a8();
            assert_eq!(b.code.code().bytes, prefix);
        }
    }
    for problem in 0..4 {
        let mut r = r.clone();
        let mut b = builder(&p.routines[0]);
        match problem {
            0 => r.frame.parameters.clear(),
            1 => {
                r.frame.parameters[0].incoming =
                    Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)
            }
            2 => r.frame.parameters[0].frame_object = Some(Mir65816FrameObjectId(999)),
            _ => {}
        }
        b.routine = &r;
        b.code.a16();
        let prefix = b.code.code().bytes.clone();
        let value = if problem == 3 {
            Mir65816Value::Temp(TempId(999), ByteSize::ONE)
        } else {
            Mir65816Value::Param(p.routines[0].frame.parameters[0].param)
        };
        assert!(b.captured_byte_return(&value).is_err());
        b.code.a16();
        assert_eq!(b.code.code().bytes, prefix);
    }
}

#[test]
fn captured_byte_return_gate_leaves_other_homes_and_operands_untouched() {
    let p = program();
    for home in [
        None,
        Some(abi::ResultLocation::A16),
        Some(abi::ResultLocation::A16X8ZeroExtended),
        Some(abi::ResultLocation::A16X16),
    ] {
        let mut r = p.routines[0].clone();
        r.result_home = home.map(Mir65816AbiHome::NativeResult);
        let mut b = builder(&r);
        assert!(
            !b.captured_byte_return(&Mir65816Value::Param(r.frame.parameters[0].param))
                .unwrap()
        );
        assert!(b.code.code().bytes.is_empty());
    }
    for value in [
        Mir65816Value::U8(255),
        Mir65816Value::U16(255),
        Mir65816Value::Null(ByteSize::ONE),
        Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
        Mir65816Value::Temp(TempId(999), ByteSize::new(2)),
    ] {
        let mut b = builder(&p.routines[0]);
        b.frame.temps.insert(
            TempId(999),
            Location::Stack(Slot {
                offset: 1,
                width: 2,
            }),
        );
        assert!(!b.captured_byte_return(&value).unwrap());
        assert!(b.code.code().bytes.is_empty());
    }
    let mut b = builder(&p.routines[0]);
    b.frame.temps.insert(
        TempId(999),
        Location::DirectPage(Slot {
            offset: 32,
            width: 1,
        }),
    );
    assert!(
        !b.captured_byte_return(&Mir65816Value::Temp(TempId(999), ByteSize::ONE))
            .unwrap()
    );
    assert!(b.code.code().bytes.is_empty());
}
