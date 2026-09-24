use super::*;

fn program(bytes: u8) -> Mir65816Program {
    let ty = if bytes == 3 { "ADDRESS" } else { "LONGCARD" };
    let source = format!(
        "{ty} FUNC Echo({ty} value) RETURN(value) {ty} FUNC Mutate({ty} value) value={ty}(0) RETURN(value) PROC Main() RETURN"
    );
    let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
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

#[test]
fn wide_returns_select_native_lanes_and_preserve_shared_teardown() {
    for bytes in [3, 4] {
        let p = program(bytes);
        for r in &p.routines[..2] {
            for kind in 0..4 {
                for a8 in [false, true] {
                    let mut b = builder(r);
                    let value = match kind {
                        0 => {
                            if bytes == 3 {
                                Mir65816Value::U24(0xabcdef)
                            } else {
                                Mir65816Value::U32(0x89abcdef)
                            }
                        }
                        1 => Mir65816Value::Param(r.frame.parameters[0].param),
                        _ => {
                            let slot = Slot {
                                offset: if kind == 2 {
                                    256 - u16::from(bytes)
                                } else {
                                    64 - u16::from(bytes)
                                },
                                width: bytes,
                            };
                            b.frame.temps.insert(
                                TempId(999),
                                if kind == 2 {
                                    Location::Stack(slot)
                                } else {
                                    Location::DirectPage(slot)
                                },
                            );
                            Mir65816Value::Temp(TempId(999), ByteSize::new(bytes.into()))
                        }
                    };
                    let memory = b.value_memory(&value).unwrap();
                    if a8 {
                        b.code.a8();
                    } else {
                        b.code.a16();
                    }
                    let start = b.code.position();
                    let frame = b.frame.clone();
                    b.return_value(Some(&value)).unwrap();
                    let mut expected = if a8 { vec![0xc2, 0x20] } else { vec![] };
                    if let Some(memory) = memory {
                        let (op, at) = match memory {
                            Memory::Stack(at) => (0xa3, at as u8),
                            Memory::DirectPage(at) => (0xa5, at as u8),
                            _ => panic!(),
                        };
                        expected.extend([op, at + (bytes - 2)]);
                        if bytes == 3 {
                            expected.extend([0xeb, 0x29, 0xff, 0]);
                        }
                        expected.extend([0xaa, op, at]);
                    } else {
                        expected.extend([
                            0xa2,
                            0xab,
                            if bytes == 3 { 0 } else { 0x89 },
                            0xa9,
                            0xef,
                            0xcd,
                        ]);
                    }
                    if frame.extent != 0 {
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
                    assert_eq!(&b.code.code().bytes[start..], expected);
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
fn wide_return_preflight_rejects_incomplete_homes_without_emitting() {
    for bytes in [3, 4] {
        let p = program(bytes);
        for (dp, at, delta, slot_width, valid) in [
            (false, 256 - u16::from(bytes), 0, bytes, true),
            (false, 255 - u16::from(bytes), 1, bytes, true),
            (false, 257 - u16::from(bytes), 0, bytes, false),
            (false, 256 - u16::from(bytes), 1, bytes, false),
            (false, 0, 0, bytes, false),
            (false, 1, u32::MAX, bytes, false),
            (false, 1, 0, bytes - 1, false),
            (true, 64 - u16::from(bytes), 0, bytes, true),
            (true, 65 - u16::from(bytes), 0, bytes, false),
        ] {
            let mut b = builder(&p.routines[0]);
            let slot = Slot {
                offset: at,
                width: slot_width,
            };
            b.frame.temps.insert(
                TempId(999),
                if dp {
                    Location::DirectPage(slot)
                } else {
                    Location::Stack(slot)
                },
            );
            b.code.test_delta(delta);
            b.code.a8();
            let prefix = b.code.code().bytes.clone();
            let result = b.wide_return(&Mir65816Value::Temp(
                TempId(999),
                ByteSize::new(bytes.into()),
            ));
            if valid {
                assert!(result.unwrap());
            } else {
                assert!(result.is_err());
                b.code.a8();
                assert_eq!(b.code.code().bytes, prefix);
            }
        }
        for problem in 0..4 {
            let mut r = p.routines[0].clone();
            match problem {
                0 => r.frame.parameters.clear(),
                1 => {
                    r.frame.parameters[0].incoming =
                        Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)
                }
                2 => r.frame.parameters[0].frame_object = Some(Mir65816FrameObjectId(999)),
                _ => {}
            }
            let mut b = builder(&p.routines[0]);
            b.routine = &r;
            b.code.a16();
            let prefix = b.code.code().bytes.clone();
            let value = if problem == 3 {
                Mir65816Value::Temp(TempId(999), ByteSize::new(bytes.into()))
            } else {
                Mir65816Value::Param(p.routines[0].frame.parameters[0].param)
            };
            assert!(b.wide_return(&value).is_err());
            b.code.a16();
            assert_eq!(b.code.code().bytes, prefix);
        }
    }
}

#[test]
fn wide_return_gates_keep_symbolic_and_mixed_width_fallbacks() {
    for bytes in [3, 4] {
        let p = program(bytes);
        for value in [
            Mir65816Value::U8(255),
            Mir65816Value::U16(0xffff),
            Mir65816Value::Null(ByteSize::ONE),
            Mir65816Value::RoutineAddress(0, ByteSize::new(bytes.into())),
        ] {
            let mut b = builder(&p.routines[0]);
            assert!(!b.wide_return(&value).unwrap());
            assert!(b.code.code().bytes.is_empty());
        }
        let mut b = builder(&p.routines[0]);
        assert!(
            b.wide_return(&Mir65816Value::Null(ByteSize::new(bytes.into())))
                .unwrap()
        );
        assert_eq!(b.code.code().bytes, [0xc2, 0x20, 0xa2, 0, 0, 0xa9, 0, 0]);
        for home in [
            None,
            Some(abi::ResultLocation::A8ZeroExtended),
            Some(abi::ResultLocation::A16),
        ] {
            let mut r = p.routines[0].clone();
            r.result_home = home.map(Mir65816AbiHome::NativeResult);
            let mut b = builder(&r);
            assert!(!b.wide_return(&Mir65816Value::U32(0xffffffff)).unwrap());
            assert!(b.code.code().bytes.is_empty());
        }
    }
}
