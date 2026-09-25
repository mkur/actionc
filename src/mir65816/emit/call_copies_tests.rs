use super::*;

fn program() -> Mir65816Program {
    program_for(
        "LONGCARD FUNC Echo(LONGCARD x) x==+1 RETURN(x) PROC Main() LONGCARD x x=Echo(LONGCARD($ABCDEF12)) RETURN",
    )
}
fn program_for(source: &str) -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
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
    let frame = AllocatedFrame::new(routine).unwrap();
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
fn call_width_selection_minimizes_actual_encoding_in_mixed_sequences() {
    let p = program();
    let r = &p.routines[1];
    // All four-argument width combinations, constants/captured homes, and both
    // padding/transfer widths. Enumerate alternative
    // instruction streams independently of the selector's cost recurrence.
    for pattern in 0..256u32 {
        for (memory, first_word, next_word) in (0..12).map(|bits| {
            (
                bits & 1 != 0,
                [None, Some(false), Some(true)][bits / 4],
                bits & 2 != 0,
            )
        }) {
            let mut b = builder(r);
            let mut args = vec![];
            let mut values = vec![];
            for i in 0..4 {
                let bytes = ((pattern >> (2 * i)) & 3) as u8 + 1;
                let id = TempId(100 + i);
                let offset = 64 + 4 * i as u16;
                b.frame.temps.insert(
                    id,
                    Location::Stack(Slot {
                        offset,
                        width: bytes,
                    }),
                );
                values.push(if memory {
                    Mir65816Value::Temp(id, ByteSize::new(bytes.into()))
                } else {
                    match bytes {
                        1 => Mir65816Value::U8(0xef),
                        2 => Mir65816Value::U16(0xcdef),
                        3 => Mir65816Value::U24(0xabcdef),
                        _ => Mir65816Value::U32(0x89abcdef),
                    }
                });
                args.push(Argument {
                    source: if memory {
                        Source::Home(Memory::Stack(offset.into()))
                    } else {
                        Source::Immediate(0x89abcdef)
                    },
                    bytes,
                    displacement: 1 + 4 * i as u8,
                    copy: ArgumentCopy::Bytes,
                });
            }
            let encode = |args: &[Argument]| {
                let mut probe = builder(r);
                probe.frame = b.frame.clone();
                match first_word {
                    Some(true) => probe.code.a16(),
                    Some(false) => probe.code.a8(),
                    None => {
                        let join = probe.code.label();
                        probe.code.mark(join);
                    }
                }
                let start = probe.code.code().bytes.len();
                for (value, arg) in values.iter().zip(args) {
                    probe.copy_call_argument(value, arg).unwrap();
                }
                if next_word {
                    probe.code.a16();
                } else {
                    probe.code.a8();
                }
                probe.code.code().bytes.len() - start
            };
            let mut best = usize::MAX;
            for mut pattern in 0..81 {
                for arg in args.iter_mut() {
                    arg.copy = match pattern % 3 {
                        1 if arg.bytes >= 2 => ArgumentCopy::Native,
                        2 if arg.bytes == 3 => ArgumentCopy::OverlappingWords,
                        _ => ArgumentCopy::Bytes,
                    };
                    pattern /= 3;
                }
                best = best.min(encode(&args));
            }
            select_widths(&mut args, first_word, next_word);
            assert_eq!(
                encode(&args),
                best,
                "{pattern}/{memory}/{first_word:?}/{next_word}"
            );
        }
    }
}

#[test]
fn overlapping_argument_words_cover_only_the_complete_three_byte_home() {
    let p = program_for(
        "ADDRESS FUNC Echo(ADDRESS x) x=ADDRESS(1) RETURN(x) PROC Main() Echo(ADDRESS($ABCDEF)) RETURN",
    );
    for memory in [Memory::Stack(250), Memory::DirectPage(61)] {
        let mut b = builder(&p.routines[0]);
        Builder::check_call_home(memory, 3, 3).unwrap();
        b.code.test_delta(3);
        b.code.a16();
        let start = b.code.code().bytes.len();
        b.copy_call_argument(
            &Mir65816Value::U24(0), // Native emission must use the preflighted home.
            &Argument {
                source: Source::Home(memory),
                displacement: 1,
                bytes: 3,
                copy: ArgumentCopy::OverlappingWords,
            },
        )
        .unwrap();
        let expected = match memory {
            Memory::Stack(_) => vec![0xa3, 253, 0x83, 1, 0xa3, 254, 0x83, 2],
            Memory::DirectPage(_) => vec![0xa5, 61, 0x83, 1, 0xa5, 62, 0x83, 2],
            _ => unreachable!(),
        };
        assert_eq!(&b.code.code().bytes[start..], expected);
    }
    for memory in [
        Memory::Stack(251),
        Memory::Stack(0),
        Memory::DirectPage(62),
        Memory::Absolute(0x123456),
    ] {
        assert!(Builder::check_call_home(memory, 3, 3).is_err());
    }
    let mut b = builder(&p.routines[0]);
    let param = &p.routines[0].frame.parameters[0];
    assert!(param.frame_object.is_some());
    let value = Mir65816Value::Param(param.param);
    let memory = b.value_memory(&value).unwrap().unwrap();
    assert!(
        matches!(memory, Memory::Stack(offset) if offset == b.parameter(param.param).unwrap().0)
    );
    Builder::check_call_home(memory, 3, 3).unwrap();
    b.copy_call_argument(
        &Mir65816Value::U24(0xabcdef),
        &Argument {
            source: Source::Immediate(0xabcdef),
            displacement: 253,
            bytes: 3,
            copy: ArgumentCopy::OverlappingWords,
        },
    )
    .unwrap();
    assert!(
        b.code
            .code()
            .bytes
            .ends_with(&[0xa9, 0xef, 0xcd, 0x83, 253, 0xa9, 0xcd, 0xab, 0x83, 254])
    );
}

#[test]
fn pointer_arguments_keep_symbolic_fixups_extension_and_cheaper_byte_tails() {
    let p = program_for(
        "ADDRESS FUNC Echo(ADDRESS x) RETURN(x) PROC Main() Echo(ADDRESS($ABCDEF)) RETURN",
    );
    let r = &p.routines[1];
    let b = builder(r);
    let (target, plan) = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| {
            if let Mir65816Op::Call { target, plan, .. } = op {
                Some((target, plan))
            } else {
                None
            }
        })
        .unwrap();
    for value in [
        Mir65816Value::U24(0xabcdef),
        Mir65816Value::Null(ByteSize::new(3)),
    ] {
        let args = b.call_arguments(&[value], plan, target, None).unwrap();
        assert_eq!(args[0].copy, ArgumentCopy::OverlappingWords);
    }
    for value in [
        Mir65816Value::U16(0xcdef),
        Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
    ] {
        let args = b.call_arguments(&[value], plan, target, None).unwrap();
        assert_eq!(args[0].copy, ArgumentCopy::Bytes);
    }
    for home in [
        Location::Stack(Slot {
            offset: 251,
            width: 3,
        }),
        Location::DirectPage(Slot {
            offset: 62,
            width: 3,
        }),
    ] {
        let mut b = builder(r);
        b.frame.temps.insert(TempId(999), home);
        let before = format!("{:?}", b.code);
        assert!(
            b.call(
                target,
                &[Mir65816Value::Temp(TempId(999), ByteSize::new(3))],
                None,
                plan
            )
            .is_err()
        );
        assert_eq!(format!("{:?}", b.code), before);
    }
    let mut args = [Argument {
        source: Source::Immediate(0xabcdef),
        displacement: 1,
        bytes: 3,
        copy: ArgumentCopy::Bytes,
    }];
    select_widths(&mut args, Some(true), false);
    assert_eq!(
        args[0].copy,
        ArgumentCopy::Native,
        "the A8 tail is smaller before an A8 consumer"
    );
}

#[test]
fn call_preflight_rejects_bad_sources_results_and_contracts_before_emission() {
    let p = program();
    let r = &p.routines[1];
    let (target, args, result, plan) = r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| {
            if let Mir65816Op::Call {
                target,
                args,
                result,
                plan,
                ..
            } = op
            {
                Some((target, args, result, plan))
            } else {
                None
            }
        })
        .unwrap();
    for case in 0..12 {
        let mut b = builder(r);
        let mut args = args.clone();
        let mut plan = plan.clone();
        let mut target = target.clone();
        let mut result = *result;
        match case {
            0 => args.clear(),
            1 => args[0] = Mir65816Value::Temp(TempId(9999), ByteSize::new(4)),
            2 => {
                b.frame.temps.insert(
                    TempId(999),
                    Location::Stack(Slot {
                        offset: 248,
                        width: 4,
                    }),
                );
                args[0] = Mir65816Value::Temp(TempId(999), ByteSize::new(4));
            }
            3 => {
                b.frame.temps.remove(&result.unwrap().0);
            }
            4 => result.as_mut().unwrap().1 = ByteSize::new(2),
            5 => plan.result = Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)),
            6 => plan.result = None,
            7 => target = Mir65816CallTarget::Indirect(Mir65816Value::U16(0), ByteSize::new(3)),
            8 => plan.native.as_mut().unwrap().transfer = abi::FarTransfer::StackRtl,
            9 => b.code.test_delta(1),
            10 => {
                b.frame.temps.insert(
                    result.unwrap().0,
                    Location::Stack(Slot {
                        offset: 253,
                        width: 4,
                    }),
                );
            }
            11 => {
                b.frame.temps.insert(
                    result.unwrap().0,
                    Location::DirectPage(Slot {
                        offset: 63,
                        width: 4,
                    }),
                );
            }
            _ => unreachable!(),
        }
        let before = format!("{:?}", b.code);
        assert!(b.call(&target, &args, result, &plan).is_err(), "{case}");
        assert_eq!(format!("{:?}", b.code), before, "{case}");
    }
}

#[test]
fn argument_and_result_extents_include_the_final_byte_and_outgoing_delta() {
    for bytes in 1..=4u8 {
        for delta in [0, 1, 13, 253, u32::MAX] {
            for offset in [0u32, 1, 251, 252, 253, 254, 255, u32::MAX] {
                let valid = offset >= 1
                    && u64::from(offset) + u64::from(delta) + u64::from(bytes) - 1 <= 255;
                assert_eq!(
                    Builder::check_call_home(Memory::Stack(offset), bytes, delta).is_ok(),
                    valid
                );
            }
        }
    }
    let p = program();
    let b = builder(&p.routines[0]);
    let param = &p.routines[0].frame.parameters[0];
    assert!(param.frame_object.is_some());
    let memory = b
        .value_memory(&Mir65816Value::Param(param.param))
        .unwrap()
        .unwrap();
    assert!(
        matches!(memory, Memory::Stack(offset) if offset == b.parameter(param.param).unwrap().0)
    );
    Builder::check_call_home(memory, 4, 5).unwrap();
    // A three-byte tail ending at 255 must not overflow intermediate u8
    // arithmetic or write a fourth byte.
    let mut b = builder(&p.routines[0]);
    b.copy_call_argument(
        &Mir65816Value::U24(0xabcdef),
        &Argument {
            source: Source::Immediate(0xabcdef),
            displacement: 253,
            bytes: 3,
            copy: ArgumentCopy::Native,
        },
    )
    .unwrap();
    assert!(b.code.code().bytes.ends_with(&[0xa9, 0xab, 0x83, 255]));
}

#[test]
fn native_result_capture_uses_only_declared_lanes_and_exact_owned_bytes() {
    let p = program();
    for (bytes, expected) in [
        (1, vec![0xe2, 0x20, 0x83, 1, 0xc2, 0x20]),
        (2, vec![0x83, 1]),
        (3, vec![0x83, 1, 0x8a, 0xe2, 0x20, 0x83, 3, 0xc2, 0x20]),
        (4, vec![0x83, 1, 0x8a, 0x83, 3]),
    ] {
        let mut b = builder(&p.routines[1]);
        b.code.a16();
        let start = b.code.code().bytes.len();
        b.capture_call_result(Memory::Stack(1), bytes).unwrap();
        assert_eq!(&b.code.code().bytes[start..], expected);
    }
}
