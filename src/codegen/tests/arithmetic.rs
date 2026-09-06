use super::*;

#[test]
fn owned_narrow_division_bodies_exhaust_byte_pairs_and_word_boundaries() {
    for word_dividend in [false, true] {
        for remainder in [false, true] {
            let mut body = crate::integer6502::narrow_division_body(word_dividend, remainder);
            body.push(0x60);
            let mut memory = [0xCCu8; 65536];
            memory[0x3100..0x3100 + body.len()].copy_from_slice(&body);
            let dividends: Vec<u16> = if word_dividend {
                vec![
                    0, 1, 127, 128, 255, 256, 257, 513, 32767, 32768, 40000, 50000, 65535,
                ]
            } else {
                (0..=255).collect()
            };
            for left in dividends {
                for right in 1..=255u16 {
                    memory[0x80..0x90].fill(0xCC);
                    memory[0x84] = right as u8;
                    let input_x = if word_dividend {
                        (left >> 8) as u8
                    } else {
                        right as u8
                    };
                    let wrapper = [
                        0xA9, left as u8, 0xA2, input_x, 0x20, 0, 0x31, 0x8D, 0, 6, 0x8E, 1, 6,
                        0x60,
                    ];
                    memory[0x3000..0x3000 + wrapper.len()].copy_from_slice(&wrapper);
                    indexed_test_cpu::run_memory(&mut memory, 0x3000);
                    let expected = if remainder {
                        left % right
                    } else {
                        left / right
                    };
                    let actual = if word_dividend && !remainder {
                        u16::from_le_bytes([memory[0x600], memory[0x601]])
                    } else {
                        u16::from(memory[0x600])
                    };
                    assert_eq!(
                        actual, expected,
                        "word={word_dividend}/rem={remainder}/{left}/{right}"
                    );
                    for address in [0x80, 0x81, 0x85, 0x87, 0x88, 0x89] {
                        assert_eq!(memory[address], 0xCC, "undeclared scratch ${address:02X}");
                    }
                    if !word_dividend {
                        assert_eq!(memory[0x83], 0xCC);
                    }
                }
            }
        }
    }
}

#[test]
fn owned_shared_divmod_signs_both_results_independently() {
    for signed in [false, true] {
        let mut body = crate::integer6502::divmod_body(signed);
        body.push(0x60);
        for (left, right) in [
            (65529u16, 3u16),
            (7, 65533),
            (65529, 65533),
            (32768, 65535),
            (50000, 40000),
            (65535, 2),
            (513, 256),
        ] {
            let mut memory = [0xCCu8; 65536];
            memory[0x84..0x86].copy_from_slice(&right.to_le_bytes());
            memory[0x3100..0x3100 + body.len()].copy_from_slice(&body);
            let wrapper = [
                0xA9,
                left as u8,
                0xA2,
                (left >> 8) as u8,
                0x20,
                0,
                0x31,
                0x8D,
                0,
                6,
                0x8E,
                1,
                6,
                0x60,
            ];
            memory[0x3000..0x3000 + wrapper.len()].copy_from_slice(&wrapper);
            indexed_test_cpu::run_memory(&mut memory, 0x3000);
            let (a, b) = if signed {
                (i64::from(left as i16), i64::from(right as i16))
            } else {
                (i64::from(left), i64::from(right))
            };
            let q = a.abs() / b.abs() * if (a < 0) != (b < 0) { -1 } else { 1 };
            let r = a - q * b;
            assert_eq!(u16::from_le_bytes([memory[0x600], memory[0x601]]), q as u16);
            assert_eq!(u16::from_le_bytes([memory[0x86], memory[0x87]]), r as u16);
        }
    }
}

#[test]
fn owned_word_division_bodies_match_oracles_and_scratch_contracts() {
    let words = [
        0u16, 1, 2, 3, 127, 128, 255, 256, 257, 513, 32767, 32768, 40000, 50000, 65529, 65533,
        65535,
    ];
    for signed in [false, true] {
        for remainder in [false, true] {
            let mut body = crate::integer6502::division_body(signed, remainder);
            body.push(0x60);
            for left in words {
                for right in words.into_iter().filter(|right| *right != 0) {
                    let mut memory = [0xCCu8; 65536];
                    memory[0x84..0x86].copy_from_slice(&right.to_le_bytes());
                    memory[0x3100..0x3100 + body.len()].copy_from_slice(&body);
                    let wrapper = [
                        0xA9,
                        left as u8,
                        0xA2,
                        (left >> 8) as u8,
                        0x20,
                        0,
                        0x31,
                        0x8D,
                        0,
                        6,
                        0x8E,
                        1,
                        6,
                        0x60,
                    ];
                    memory[0x3000..0x3000 + wrapper.len()].copy_from_slice(&wrapper);
                    let before = memory;
                    indexed_test_cpu::run_memory(&mut memory, 0x3000);
                    let (a, b) = if signed {
                        (i64::from(left as i16), i64::from(right as i16))
                    } else {
                        (i64::from(left), i64::from(right))
                    };
                    let q = (a.abs() / b.abs()) * if (a < 0) != (b < 0) { -1 } else { 1 };
                    let expected = if remainder { a - q * b } else { q } as u16;
                    assert_eq!(
                        u16::from_le_bytes([memory[0x600], memory[0x601]]),
                        expected,
                        "signed={signed}/rem={remainder}/{left}/{right}"
                    );
                    for address in 0..65536 {
                        if (0x82..=0x87).contains(&address)
                            || (signed && (0xC2..=0xC3).contains(&address))
                            || (0x100..0x200).contains(&address)
                            || (0x600..0x602).contains(&address)
                        {
                            continue;
                        }
                        assert_eq!(
                            memory[address], before[address],
                            "undeclared write ${address:04X}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn retargeted_compound_places_do_not_reuse_indirect_memory_aliases() {
    let ast = parse(
        &tokenize(
            "INT a=$06E0,cq=$0604,cr=$0606 CARD b=$06E2,q=$0600,r=$0602 \
         BYTE done=$06FF PROC Main() q=a/b r=a MOD b cq=a cq==/b \
         cr=a cr==MOD b done=$A5 RETURN",
        )
        .unwrap(),
    )
    .unwrap();
    let model = analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let output = match runtime {
                Runtime::ActionCart => generate_semir_profile_at_origin(&semir, 0x3000, profile),
                Runtime::Standalone => {
                    generate_semir_standalone_profile_at_origin(&semir, 0x3000, profile)
                }
            }
            .unwrap();
            for (left, right) in [(513u16, 3u16), (1000, 7), (32767, 3), (256, 3)] {
                let mut memory = [0u8; 65536];
                if runtime == Runtime::ActionCart {
                    let cart = include_bytes!("../../../roms/action.rom");
                    memory[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
                    memory[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
                }
                let origin = usize::from(output.origin);
                memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
                memory[0x600..0x700].fill(0xCC);
                memory[0x6E0..0x6E2].copy_from_slice(&left.to_le_bytes());
                memory[0x6E2..0x6E4].copy_from_slice(&right.to_le_bytes());
                let before = memory;
                indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                let actual: Vec<_> = memory[0x600..0x608]
                    .chunks_exact(2)
                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                    .collect();
                assert_eq!(
                    actual,
                    [left / right, left % right, left / right, left % right],
                    "{profile:?}/{runtime:?}/{left}/{right}"
                );
                assert_eq!(&memory[0x608..0x6FF], &before[0x608..0x6FF]);
                assert_eq!(memory[0x6FF], 0xA5);
            }
        }
    }
}

#[test]
fn addressing_templates_are_not_stable_cached_memory_locations() {
    for space in [AddressSpace::AbsoluteX, AddressSpace::IndirectIndexedY] {
        let mut state = ProcessorState::default();
        let mut slot = StorageSlot::absolute(0xAE, 2);
        slot.space = space;
        state.set_memory_byte(slot, 1, ValueFact::Immediate(0));
        assert_eq!(state.memory_value(slot, 1), None);
    }
}
