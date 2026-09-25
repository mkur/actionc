use super::*;

fn builder(routine: &Mir65816Routine, offset: u16, width: u8) -> Builder<'_> {
    let mut frame = AllocatedFrame::stack(routine).unwrap();
    frame
        .temps
        .insert(TempId(999), Location::Stack(Slot { offset, width }));
    Builder {
        routine,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
    }
}

fn address(base: Mir65816AddressBase, displacement: u32) -> Mir65816Address {
    Mir65816Address {
        base,
        displacement: ByteOffset::new(displacement),
        index: None,
        mode: Mir65816AddressMode::Static,
    }
}

#[test]
fn symbol_addresses_write_exact_homes_and_keep_complete_fixup_addends() {
    let p = word_tests::program();
    for (base, data) in [
        (
            Mir65816AddressBase::Static(NirStorageId::Global(SymbolId(7))),
            Mir65816DataId::Global(SymbolId(7)),
        ),
        (
            Mir65816AddressBase::External(Mir65816ExternalAddress::Global(SymbolId(8))),
            Mir65816DataId::Global(SymbolId(8)),
        ),
        (
            Mir65816AddressBase::Indirect(Mir65816Value::StaticAddress(
                SymbolId(9),
                ByteSize::new(3),
            )),
            Mir65816DataId::Static(SymbolId(9)),
        ),
    ] {
        for offset in [0, 1, 65535] {
            if matches!(base, Mir65816AddressBase::Indirect(_)) && offset != 0 {
                continue;
            }
            for byte_mode in [false, true] {
                let mut b = builder(&p.routines[0], 253, 3);
                if byte_mode {
                    b.code.a8();
                }
                let at = b.code.position();
                assert!(
                    b.symbol_address(TempId(999), &address(base.clone(), offset))
                        .unwrap()
                );
                let mut expected = if byte_mode { vec![] } else { vec![0xe2, 0x20] };
                for byte in 0..3 {
                    expected.extend([0xa9, 0, 0x83, 253 + byte]);
                }
                assert_eq!(&b.code.code().bytes[at..], expected);
                let fixups = &b.code.code().fixups;
                assert_eq!(fixups.len(), 3);
                for (byte, fixup) in fixups.iter().enumerate() {
                    assert_eq!(fixup.target, Target::Data(data));
                    assert_eq!(fixup.addend, offset);
                    assert_eq!(fixup.byte, Some(byte as u8));
                }
            }
        }
    }
}

#[test]
fn symbol_address_declines_modular_indirection_and_checks_homes_before_emission() {
    let p = word_tests::program();
    for base in [
        Mir65816AddressBase::Indirect(Mir65816Value::GlobalAddress(SymbolId(7), ByteSize::new(3))),
        Mir65816AddressBase::Indirect(Mir65816Value::Temp(TempId(1), ByteSize::new(3))),
    ] {
        let mut b = builder(&p.routines[0], 20, 3);
        assert!(!b.symbol_address(TempId(999), &address(base, 1)).unwrap());
        assert!(b.code.code().bytes.is_empty());
    }
    for (offset, width) in [(0, 3), (254, 3), (20, 2)] {
        let mut b = builder(&p.routines[0], offset, width);
        assert!(
            b.symbol_address(
                TempId(999),
                &address(
                    Mir65816AddressBase::Static(NirStorageId::Global(SymbolId(7))),
                    0
                )
            )
            .is_err()
        );
        assert!(b.code.code().bytes.is_empty());
        assert!(b.code.code().fixups.is_empty());
    }
}
