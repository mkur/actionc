use super::*;
fn builder(r: &Mir65816Routine) -> Builder<'_> {
    Builder {
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
        routine: r,
        frame: AllocatedFrame::stack(r).unwrap(),
        code: TrackedEmitter65816::for_test(&AllocatedFrame::stack(r).unwrap()),
        blocks: BTreeMap::new(),
    }
}
fn inputs(r: &Mir65816Routine) -> (TempId, TempId, TempId) {
    r.blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| match op {
            Mir65816Op::Binary {
                dest,
                left: Mir65816Value::Temp(a, _),
                right: Mir65816Value::Temp(b, _),
                ..
            } => Some((*a, *b, *dest)),
            _ => None,
        })
        .unwrap()
}
fn seed(b: &mut Builder<'_>, id: TempId) {
    b.code.a16();
    b.code.word(WordOp::LdaImm, 0x8000);
    let Location::Stack(slot) = b.temp(id).unwrap() else {
        panic!()
    };
    b.code.register_home(slot);
    b.code.byte(ByteOp::StaStack, slot.offset as u8);
    b.remember_word(id);
    assert!(b.code.resident().is_some());
}
#[test]
fn adjacent_word_arithmetic_and_return_retain_homes_and_stores() {
    let p = word_tests::program();
    let r = &p.routines[0];
    let (a, _, dest) = inputs(r);
    let mut b = builder(r);
    let homes = b.frame.temps.clone();
    seed(&mut b, a);
    let start = b.code.code().bytes.len();
    assert!(
        b.word_binary(
            dest,
            2,
            NirBinaryOp::Add,
            &Mir65816Value::Temp(a, ByteSize::new(2)),
            &Mir65816Value::U16(1)
        )
        .unwrap()
    );
    assert_eq!(
        &b.code.code().bytes[start..],
        [
            0x18,
            0x69,
            1,
            0,
            0x83,
            b.temp(dest).unwrap().slot().offset as u8
        ]
    );
    let start = b.code.code().bytes.len();
    assert!(
        b.word_return(&Mir65816Value::Temp(dest, ByteSize::new(2)))
            .unwrap()
    );
    assert_eq!(b.code.code().bytes.len(), start);
    assert_eq!(b.frame.temps, homes);
    assert!(b.code.resident().is_none());
}
#[test]
fn stale_value_flags_width_label_and_stack_facts_cannot_remove_loads() {
    let p = word_tests::program();
    let r = &p.routines[0];
    let (a, _, _) = inputs(r);
    for case in 0..12 {
        let mut b = builder(r);
        seed(&mut b, a);
        let operand = b
            .word_operand(&Mir65816Value::Temp(a, ByteSize::new(2)))
            .unwrap()
            .unwrap();
        match case {
            0 => b.code.word(WordOp::CmpImm, 0), // A unchanged, N/Z no longer describe A.
            1 => b.code.word(WordOp::LdyImm, 0), // LDY changes N/Z only.
            2 => b.code.op(Implied::Nop),        // Unknown to forwarding, even if NOP.
            3 => b.code.byte(ByteOp::StaStack, 200), // Even a disjoint store is a barrier.
            4 => {
                let l = b.code.label();
                b.code.mark(l);
            }
            5 => {
                b.code.a8();
                b.code.a16();
            }
            6 => b.code.op(Implied::Tsc),
            7 => b.code.test_delta(2),
            8 => b.release(0, false), // Zero-byte path must also forget the fact.
            9 => b.check_stack(0),
            10 => b.reserve(2),
            11 => b.code.byte(ByteOp::Sep, 0x20), // Raw unmodelled mode instruction.
            _ => unreachable!(),
        }
        b.code.a16();
        let start = b.code.code().bytes.len();
        b.load_checked_word(operand, Some(a));
        assert_eq!(
            &b.code.code().bytes[start..],
            [
                0xa3,
                match operand {
                    WordOperand::Stack(s) => s,
                    _ => panic!(),
                }
            ],
            "case {case}"
        );
    }
}
#[test]
fn identity_exact_range_and_checked_extent_are_required_even_for_resident_words() {
    let p = word_tests::program();
    let r = &p.routines[0];
    let (a, other, dest) = inputs(r);
    for mismatch in 0..4 {
        let mut b = builder(r);
        seed(&mut b, a);
        let old = b.temp(a).unwrap();
        let slot = old.slot();
        let (operand, id) = match mismatch {
            0 => {
                b.frame.temps.insert(other, old);
                (WordOperand::Stack(slot.offset as u8), Some(other))
            }
            1 => {
                b.frame.temps.insert(
                    a,
                    Location::Stack(Slot {
                        offset: slot.offset + 2,
                        width: 2,
                    }),
                );
                (WordOperand::Stack(slot.offset as u8 + 2), Some(a))
            }
            2 => (WordOperand::Stack(slot.offset as u8), None),
            _ => (WordOperand::Immediate(0x8000), Some(a)),
        };
        let start = b.code.code().bytes.len();
        b.load_checked_word(operand, id);
        assert!(b.code.code().bytes.len() > start);
    }
    for offset in [254, 255] {
        let mut b = builder(r);
        b.frame
            .temps
            .insert(a, Location::Stack(Slot { offset, width: 2 }));
        seed(&mut b, a);
        let before = b.code.code().bytes.clone();
        let fact = b.code.resident();
        let result = b.word_binary(
            dest,
            2,
            NirBinaryOp::Sub,
            &Mir65816Value::Temp(a, ByteSize::new(2)),
            &Mir65816Value::U16(1),
        );
        assert_eq!(result.is_ok(), offset == 254);
        if offset == 255 {
            assert_eq!(b.code.code().bytes, before);
            assert_eq!(b.code.resident(), fact);
        }
    }
    let mut b = builder(r);
    seed(&mut b, a);
    let before = b.code.code().bytes.clone();
    let fact = b.code.resident();
    assert!(
        !b.word_binary(
            dest,
            2,
            NirBinaryOp::Add,
            &Mir65816Value::Temp(a, ByteSize::new(2)),
            &Mir65816Value::U24(0)
        )
        .unwrap()
    );
    assert_eq!(b.code.code().bytes, before);
    assert_eq!(b.code.resident(), fact);
}
#[test]
fn store_preflights_destination_and_preserves_only_the_original_write() {
    let p = word_tests::program();
    let r = &p.routines[0];
    let (a, _, _) = inputs(r);
    let mut b = builder(r);
    seed(&mut b, a);
    let address = Mir65816Address {
        base: Mir65816AddressBase::Parameter(r.frame.parameters[0].param),
        displacement: ByteOffset::new(0),
        index: None,
        mode: Mir65816AddressMode::Parameter,
    };
    let at = b.parameter(r.frame.parameters[0].param).unwrap().0 as u8;
    let start = b.code.code().bytes.len();
    assert!(
        b.word_store(
            &address,
            &Mir65816Value::Temp(a, ByteSize::new(2)),
            2,
            false
        )
        .unwrap()
    );
    assert_eq!(&b.code.code().bytes[start..], [0x83, at]);
    assert!(b.code.resident().is_none());
    seed(&mut b, a);
    let start = b.code.code().bytes.len();
    assert!(
        !b.word_store(&address, &Mir65816Value::Temp(a, ByteSize::new(2)), 2, true)
            .unwrap()
    );
    assert_eq!(b.code.code().bytes.len(), start);
    let mut bad = address;
    bad.displacement = ByteOffset::new(u32::MAX);
    let fact = b.code.resident();
    assert!(
        b.word_store(&bad, &Mir65816Value::Temp(a, ByteSize::new(2)), 2, false)
            .is_err()
    );
    assert_eq!(b.code.code().bytes.len(), start);
    assert_eq!(b.code.resident(), fact);
}

fn frame_program() -> Mir65816Program {
    let ast = crate::parser::parse(
        &crate::lexer::tokenize(
            "CARD FUNC Work(CARD x) CARD a,b a=x+1 b=a RETURN(b) PROC Main() RETURN",
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
    crate::nir::verify_program(&nir).unwrap();
    super::super::super::lower_program(&nir).unwrap()
}

#[test]
fn frame_store_load_omits_only_the_reload_and_can_forward_the_capture() {
    let p = frame_program();
    let r = &p.routines[0];
    let mut b = builder(r);
    let homes = b.frame.temps.clone();
    let ops = &r.blocks[0].ops;
    let i = ops
        .windows(2)
        .position(|w| {
            matches!((&w[0], &w[1]),
        (Mir65816Op::Store { address: a, .. }, Mir65816Op::Load { address: b, .. }) if a == b)
        })
        .unwrap();
    for op in &ops[..=i] {
        b.operation(op).unwrap();
    }
    let Mir65816Op::Load { dest, .. } = &ops[i + 1] else {
        panic!()
    };
    let at = b.code.position();
    b.operation(&ops[i + 1]).unwrap();
    assert_eq!(
        &b.code.code().bytes[at..],
        &[0x83, b.temp(*dest).unwrap().slot().offset as u8]
    );
    assert_eq!(b.frame.temps, homes);
    let at = b.code.position();
    assert!(
        b.word_return(&Mir65816Value::Temp(*dest, ByteSize::new(2)))
            .unwrap()
    );
    assert_eq!(b.code.position(), at);
}

#[test]
fn frame_consumer_keeps_loads_for_aliases_offsets_volatile_and_intervening_effects() {
    for case in 0..13 {
        let mut p = frame_program();
        let r = &mut p.routines[0];
        let i = r.blocks[0]
            .ops
            .windows(2)
            .position(|w| {
                matches!((&w[0], &w[1]),
            (Mir65816Op::Store { address: a, .. }, Mir65816Op::Load { address: b, .. }) if a == b)
            })
            .unwrap();
        let mut consumer = r.blocks[0].ops[i + 1].clone();
        let Mir65816Op::Load {
            address, volatile, ..
        } = &mut consumer
        else {
            panic!()
        };
        if case == 0 {
            let Mir65816AddressBase::AutomaticFrame(id) = address.base else {
                panic!()
            };
            r.frame
                .objects
                .iter_mut()
                .find(|o| o.id == id)
                .unwrap()
                .addressable = true;
        }
        if case == 1 {
            *volatile = true;
        }
        if case == 2 {
            let other = r
                .frame
                .objects
                .iter()
                .find(|o| address.base != Mir65816AddressBase::AutomaticFrame(o.id))
                .unwrap();
            address.base = Mir65816AddressBase::AutomaticFrame(other.id);
        }
        let mut b = builder(r);
        for op in &r.blocks[0].ops[..=i] {
            b.operation(op).unwrap();
        }
        match case {
            3 => b.code.word(WordOp::CmpImm, 0),
            4 => b.code.word(WordOp::LdyImm, 0),
            5 => b.code.op(Implied::Nop),
            6 => b.code.byte(ByteOp::StaStack, 200),
            7 => {
                let label = b.code.label();
                b.code.mark(label);
            }
            8 => {
                b.code.a8();
                b.code.a16();
            }
            9 => b.code.test_delta(2),
            10 => b.code.barrier(),
            11 => b
                .code
                .reference(ReferenceOp::Jsl, Target::Routine(r.id), 0, None),
            12 => b.code.byte(ByteOp::StaIndirect, PTR),
            _ => {}
        }
        let at = b.code.position();
        b.operation(&consumer).unwrap();
        assert!(b.code.code().bytes[at..].contains(&0xa3), "case {case}");
    }
}

#[test]
fn frame_forwarding_preflights_object_and_destination_before_omission() {
    let p = frame_program();
    let r = &p.routines[0];
    let i = r.blocks[0]
        .ops
        .windows(2)
        .position(|w| {
            matches!((&w[0], &w[1]),
        (Mir65816Op::Store { address: a, .. }, Mir65816Op::Load { address: b, .. }) if a == b)
        })
        .unwrap();
    for case in 0..4 {
        let mut consumer = r.blocks[0].ops[i + 1].clone();
        let Mir65816Op::Load { address, dest, .. } = &mut consumer else {
            panic!()
        };
        let mut b = builder(r);
        for op in &r.blocks[0].ops[..=i] {
            b.operation(op).unwrap();
        }
        if case < 2 {
            address.displacement = ByteOffset::new(if case == 0 { 1 } else { u32::MAX });
        } else {
            b.frame.temps.insert(
                *dest,
                Location::Stack(Slot {
                    offset: if case == 2 { 255 } else { 0 },
                    width: 2,
                }),
            );
        }
        let at = b.code.position();
        assert!(b.operation(&consumer).is_err());
        assert_eq!(b.code.position(), at);
    }
}

#[test]
fn incoming_loads_coexist_with_temp_forwarding_and_do_not_rearm() {
    let p = word_tests::program();
    let r = &p.routines[0];
    let mut b = builder(r);
    let Mir65816Op::Load { address, dest, .. } = &r.blocks[0].ops[0] else {
        panic!()
    };
    for (n, expected) in [(0, 4), (1, 2), (2, 4), (3, 2)] {
        let at = b.code.position();
        b.incoming_word_load(*dest, 2, address, false).unwrap();
        let size = b.code.position() - at;
        assert_eq!(size, expected + if n == 0 { 2 } else { 0 }); // First explicit A16 permission.
        assert!(b.code.resident().is_some());
    }
}

#[test]
fn incoming_word_rejects_stale_cursors_flags_calls_and_transient_stack() {
    let p = word_tests::program();
    let r = &p.routines[0];
    let op = &r.blocks[0].ops[0];
    for case in 0..12 {
        let mut b = builder(r);
        b.operation(op).unwrap();
        match case {
            0 => b.code.op(Implied::Nop),
            1 => b.code.op(Implied::Clc),
            2 => b.code.word(WordOp::CmpImm, 0),
            3 => b.code.word(WordOp::LdyImm, 0),
            4 => b.code.byte(ByteOp::StaStack, 200),
            5 => b.code.byte(ByteOp::StaIndirect, PTR),
            6 => {
                b.code.a8();
                b.code.a16();
            }
            7 => {
                let l = b.code.label();
                b.code.mark(l);
            }
            8 => b.code.barrier(),
            9 => b.code.test_delta(2),
            10 => b
                .code
                .reference(ReferenceOp::Jsl, Target::Routine(r.id), 0, None),
            11 => b.reserve(0),
            _ => unreachable!(),
        }
        let at = b.code.position();
        b.operation(op).unwrap();
        assert!(b.code.code().bytes[at..].contains(&0xa3), "case {case}");
    }
}

#[test]
fn incoming_classifier_rejects_address_escape_writes_and_bad_extent_before_emission() {
    for case in 0..8 {
        let mut p = word_tests::program();
        let r = &mut p.routines[0];
        let frame = AllocatedFrame::stack(r).unwrap();
        let original = r.blocks[0].ops[0].clone();
        let Mir65816Op::Load { address, dest, .. } = &original else {
            panic!()
        };
        match case {
            0 => r.blocks[0].ops.push(Mir65816Op::AddressOf {
                dest: *dest,
                address: address.clone(),
                width: ByteSize::new(3),
            }),
            1 => r.blocks[0].ops.push(Mir65816Op::Store {
                address: address.clone(),
                value: Mir65816Value::U16(0),
                width: ByteSize::new(2),
                volatile: false,
            }),
            2 => r.blocks[0].ops.push(Mir65816Op::Copy {
                source: address.clone(),
                destination: address.clone(),
                bytes: ByteSize::new(2),
                overlap_safe: true,
                source_volatile: false,
                destination_volatile: false,
            }),
            3 => {
                let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[0] else {
                    panic!()
                };
                *volatile = true;
            }
            4 => {
                let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[0] else {
                    panic!()
                };
                address.displacement = ByteOffset::new(1);
            }
            _ => {}
        }
        let mut b = Builder {
            next_block: None,
            loop_x: None,
            borrowed: BTreeMap::new(),
            routine: r,
            code: TrackedEmitter65816::for_test(&frame),
            frame,
            blocks: BTreeMap::new(),
        };
        if case < 5 {
            assert!(!b.incoming_word_load(*dest, 2, address, false).unwrap());
        } else {
            let offset = if case == 5 {
                255
            } else if case == 6 {
                0
            } else {
                b.incoming(r.frame.parameters[0].param).unwrap() as u16
            };
            b.frame
                .temps
                .insert(*dest, Location::Stack(Slot { offset, width: 2 }));
            assert!(b.incoming_word_load(*dest, 2, address, false).is_err());
        }
        assert!(b.code.code().bytes.is_empty());
    }
}

#[test]
fn incoming_capture_crosses_exactly_one_matching_private_store() {
    let p = frame_program();
    let r = &p.routines[0];
    let load = r.blocks[0]
        .ops
        .iter()
        .find(|op| {
            matches!(
                op,
                Mir65816Op::Load {
                    address: Mir65816Address {
                        base: Mir65816AddressBase::Parameter(_),
                        ..
                    },
                    ..
                }
            )
        })
        .unwrap()
        .clone();
    let Mir65816Op::Load { dest, .. } = load else {
        panic!()
    };
    let mut store = r.blocks[0]
        .ops
        .iter()
        .find(|op| matches!(op, Mir65816Op::Store { .. }))
        .unwrap()
        .clone();
    let Mir65816Op::Store { value, .. } = &mut store else {
        panic!()
    };
    *value = Mir65816Value::Temp(dest, ByteSize::new(2));
    for count in [0, 1, 2] {
        let mut b = builder(r);
        b.operation(&load).unwrap();
        for _ in 0..count {
            b.operation(&store).unwrap();
        }
        let at = b.code.position();
        b.operation(&load).unwrap();
        assert_eq!(b.code.position() - at, if count < 2 { 2 } else { 4 });
    }
}

#[test]
fn incoming_uses_final_frame_displacement_and_checks_the_last_byte() {
    let p = word_tests::program();
    let r = &p.routines[0];
    let Mir65816Op::Load { address, dest, .. } = &r.blocks[0].ops[0] else {
        panic!()
    };
    for extent in [250, 251, u16::MAX] {
        let mut b = builder(r);
        b.frame.extent = extent;
        if extent == 250 {
            assert!(b.incoming_word_load(*dest, 2, address, false).unwrap());
            assert_eq!(&b.code.code().bytes[2..4], &[0xa3, 254]);
            let at = b.code.position();
            assert!(b.incoming_word_load(*dest, 2, address, false).unwrap());
            assert_eq!(b.code.position() - at, 2);
        } else {
            assert!(b.incoming_word_load(*dest, 2, address, false).is_err());
            assert_eq!(b.code.position(), 0);
        }
    }
}

#[test]
fn incoming_metadata_and_unrelated_stores_cannot_grant_permission() {
    for case in 0..5 {
        let mut p = frame_program();
        let r = &mut p.routines[0];
        let frame = AllocatedFrame::stack(r).unwrap();
        let op = r.blocks[0].ops[0].clone();
        let Mir65816Op::Load { address, dest, .. } = &op else {
            panic!()
        };
        if case == 0 {
            r.frame.parameters[0].frame_object = Some(r.frame.objects[0].id);
        }
        if case == 1 {
            r.frame.objects[0].owner = Mir65816FrameObjectOwner::Param(r.frame.parameters[0].param);
        }
        if case == 2 {
            let Mir65816AbiHome::StackArgument { size, .. } = &mut r.frame.parameters[0].incoming
            else {
                panic!()
            };
            *size = ByteSize::new(1);
        }
        let mut store = r.blocks[0]
            .ops
            .iter()
            .find(|op| matches!(op, Mir65816Op::Store { .. }))
            .unwrap()
            .clone();
        if case == 4 {
            r.frame.objects[0].addressable = true;
        }
        let mut b = Builder {
            next_block: None,
            loop_x: None,
            borrowed: BTreeMap::new(),
            routine: r,
            code: TrackedEmitter65816::for_test(&frame),
            frame,
            blocks: BTreeMap::new(),
        };
        if case < 3 {
            assert!(!b.incoming_word_load(*dest, 2, address, false).unwrap());
            assert_eq!(b.code.position(), 0);
        } else {
            b.operation(&op).unwrap();
            let Mir65816Op::Store { value, .. } = &mut store else {
                panic!()
            };
            *value = if case == 3 {
                Mir65816Value::U16(0)
            } else {
                Mir65816Value::Temp(*dest, ByteSize::new(2))
            };
            b.operation(&store).unwrap();
            let at = b.code.position();
            b.operation(&op).unwrap();
            let bytes = &b.code.code().bytes[at..];
            let start = usize::from(bytes.starts_with(&[0xc2, 0x20])) * 2;
            assert_eq!(bytes.len() - start, 4);
            assert_eq!(bytes[start], 0xa3);
        }
    }
}

#[test]
fn incoming_compare_requires_adjacent_sole_use_and_complete_disjoint_homes() {
    for case in 0..6 {
        let p = word_tests::program();
        let r = &p.routines[0];
        let mut b = builder(r);
        let mut block = r.blocks[0].clone();
        let load = block.ops[0].clone();
        let Mir65816Op::Load { dest: loaded, .. } = load else {
            panic!()
        };
        block.ops = vec![
            load,
            Mir65816Op::Compare {
                dest: TempId(999),
                width: ByteSize::new(2),
                signed: false,
                operation: NirCompareOp::Lt,
                left: Mir65816Value::Temp(loaded, ByteSize::new(2)),
                right: Mir65816Value::U16(0x8000),
            },
        ];
        b.frame.temps.insert(
            TempId(999),
            Location::Stack(Slot {
                offset: 1,
                width: 1,
            }),
        );
        let mut counts = BTreeMap::from([(loaded, 1)]);
        match case {
            1 => {
                counts.insert(loaded, 2);
            }
            2 => {
                let Mir65816Op::Load { volatile, .. } = &mut block.ops[0] else {
                    panic!()
                };
                *volatile = true;
            }
            3 => {
                block.ops.insert(1, r.blocks[0].ops.last().unwrap().clone());
            }
            4 => {
                b.frame.temps.insert(
                    loaded,
                    Location::Stack(Slot {
                        offset: 255,
                        width: 2,
                    }),
                );
            }
            5 => {
                let incoming = b.incoming(r.frame.parameters[0].param).unwrap() as u16;
                b.frame.temps.insert(
                    loaded,
                    Location::Stack(Slot {
                        offset: incoming,
                        width: 2,
                    }),
                );
            }
            _ => {}
        }
        let before = format!("{:?}", b.code);
        let plan = b.incoming_comparisons(&block, &counts);
        if case >= 4 {
            assert!(plan.is_err());
        } else {
            assert_eq!(plan.unwrap().len(), usize::from(case == 0));
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
}
