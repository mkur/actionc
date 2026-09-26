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
        borrowed: BTreeMap::new(),
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

fn chain_program() -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize("BYTE ARRAY bytes(300) PROC Barrier() RETURN ADDRESS FUNC Work() RETURN(ADDRESS(@bytes(1))) PROC Main() Barrier() RETURN").unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::mir65816::lower_program(&nir).unwrap()
}

#[test]
fn chains_keep_other_uses_and_reject_loaded_or_cross_barrier_provenance() {
    let p = chain_program();
    let original = p.routines.iter().find(|r| r.name == "Work").unwrap();
    let frame = AllocatedFrame::new(original).unwrap();
    let Mir65816Op::AddressOf {
        dest: base, width, ..
    } = original.blocks[0].ops[0]
    else {
        panic!()
    };
    let value = Mir65816Value::Temp(base, width);
    let call = p
        .routines
        .iter()
        .flat_map(|r| &r.blocks)
        .flat_map(|b| &b.ops)
        .find(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap()
        .clone();
    for variant in 0..5 {
        let mut r = original.clone();
        match variant {
            0 => r.blocks[0].ops.push(Mir65816Op::Compare {
                dest: TempId(999),
                width,
                signed: false,
                operation: NirCompareOp::Eq,
                left: value.clone(),
                right: value.clone(),
            }),
            1 => {
                let Mir65816Terminator::Return {
                    value: returned, ..
                } = &mut r.blocks[0].terminator
                else {
                    panic!()
                };
                *returned = Some(value.clone());
            }
            2 => r.blocks[0].ops.insert(1, call.clone()),
            3 => {
                let Mir65816Op::AddressOf { address, .. } = r.blocks[0].ops[0].clone() else {
                    panic!()
                };
                r.blocks[0].ops[0] = Mir65816Op::Load {
                    dest: base,
                    width,
                    address,
                    volatile: false,
                };
            }
            _ => {
                let mut next = r.blocks[0].clone();
                next.id = BlockId(999);
                next.ops.remove(0);
                r.blocks[0].ops.truncate(1);
                r.blocks[0].terminator = Mir65816Terminator::Fallthrough;
                r.blocks.push(next);
            }
        }
        let plan = Plan::new(&r, &frame, &p.data).unwrap();
        assert!(!plan.omitted.contains(&base), "variant {variant}");
        if variant >= 2 {
            assert_eq!(plan.symbols.len(), usize::from(variant != 3));
        }
        if variant == 0 {
            assert_eq!(liveness::input_counts(&r)[&base], 3);
        }
    }
}

#[test]
fn only_nonzero_y_within_one_wide_scalar_transfer_is_advanced() {
    let p = word_tests::program();
    for bytes in [3u8, 4] {
        for offset in [0u16, 1, 7, 65535 - u16::from(bytes) + 1] {
            for wide in [false, true] {
                for store in [false, true] {
                    let mut b = builder(&p.routines[0], 16, bytes);
                    let pointer = Memory::Pointer { slot: PTR, offset };
                    let home = Memory::Stack(16);
                    let (source, dest) = if store {
                        (home, pointer)
                    } else {
                        (pointer, home)
                    };
                    b.transfer(source, dest, bytes, wide).unwrap();
                    assert_eq!(
                        b.code
                            .code()
                            .bytes
                            .windows(2)
                            .filter(|w| *w == [0xc8, 0xc8])
                            .count(),
                        usize::from(wide && offset != 0)
                    );
                }
                let mut b = builder(&p.routines[0], 16, bytes);
                b.constant_store(
                    Memory::Pointer { slot: PTR, offset },
                    &Mir65816Value::U32(0),
                    bytes,
                    !wide,
                )
                .unwrap();
                assert_eq!(
                    b.code
                        .code()
                        .bytes
                        .windows(2)
                        .filter(|w| *w == [0xc8, 0xc8])
                        .count(),
                    usize::from(wide && offset != 0)
                );
            }
        }
    }
}
