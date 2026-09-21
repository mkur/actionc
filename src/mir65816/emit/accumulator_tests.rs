use super::*;
fn builder(r: &Mir65816Routine) -> Builder<'_> {
    Builder {
        next_block: None,
        routine: r,
        frame: AllocatedFrame::new(r).unwrap(),
        code: TrackedEmitter65816::for_test(&AllocatedFrame::new(r).unwrap()),
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
