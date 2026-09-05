use super::*;

#[test]
fn scalar_slot_copy_stages_indirect_source_before_overwriting_its_pointer() {
    for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
        for segment_storage in [false, true] {
            for pointer in [
                runtime_zp::ARGS,
                runtime_zp::ARRAY_ADDR,
                runtime_zp::ELEMENT_ADDR,
                runtime_zp::ADDR,
            ] {
                for displacement in [-2i16, -1, 0, 1, 2] {
                    let address = (i16::from(pointer.address()) + displacement) as u16;
                    for absolute in [false, true] {
                        let target = if absolute {
                            StorageSlot::absolute(address, 2)
                        } else {
                            StorageSlot::zero_page(address as u8, 2)
                        };
                        for source_size in [1, 2] {
                            for volatile in [false, true] {
                                for offset in [0, 2, 127, 254] {
                                    let mut generator = test_generator(profile);
                                    generator.segment_storage = segment_storage;
                                    let source = StorageSlot {
                                        index_offset: offset,
                                        ..StorageSlot::indirect_indexed_y(pointer, source_size)
                                            .volatile(volatile)
                                    };
                                    generator.emit_lda_imm(0x5A);
                                    generator.emitter.emit_pha();
                                    generator.emit_ldx_imm(0x6D);
                                    assert!(generator.emit_copy_slot_to_slot(source, target));
                                    generator.emitter.emit_stx_absolute(Absolute::new(0x600));
                                    generator.emitter.emit_sty_absolute(Absolute::new(0x601));
                                    generator.emit_pla();
                                    generator.emit_sta_absolute(Absolute::new(0x602));
                                    generator.emitter.emit_rts();
                                    let bytes = generator.emitter.finish().unwrap();
                                    let mut memory = [0u8; 65536];
                                    memory[0x3000..0x3000 + bytes.len()].copy_from_slice(&bytes);
                                    memory[0x4F00..0x5200].fill(0xA5);
                                    let base = 0x4FF1usize;
                                    memory[base + usize::from(offset)
                                        ..base + usize::from(offset) + 2]
                                        .copy_from_slice(&[0xF3, 0x82]);
                                    let ptr = usize::from(pointer.address());
                                    memory[ptr..ptr + 2]
                                        .copy_from_slice(&(base as u16).to_le_bytes());
                                    let before = memory;
                                    indexed_test_cpu::run_memory(&mut memory, 0x3000);
                                    let label = format!(
                                        "{profile:?}/{segment_storage}/{source:?}/{target:?}"
                                    );
                                    assert_eq!(
                                        &memory[usize::from(address)..usize::from(address) + 2],
                                        &[0xF3, if source_size == 2 { 0x82 } else { 0 }],
                                        "{label}"
                                    );
                                    let y = offset as u8
                                        + u8::from(!segment_storage && source_size == 2);
                                    assert_eq!(&memory[0x600..0x602], &[0x6D, y], "{label}");
                                    assert_eq!(memory[0x602], 0x5A, "{label}: balanced stack");
                                    assert_eq!(
                                        &memory[0x4F00..0x5200],
                                        &before[0x4F00..0x5200],
                                        "{label}"
                                    );
                                    for zp in 0..256 {
                                        if !(usize::from(address)..usize::from(address) + 2)
                                            .contains(&zp)
                                        {
                                            assert_eq!(
                                                memory[zp], before[zp],
                                                "{label}/ZP={zp:02X}"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn record_field_arithmetic_preserves_materialized_word_pointer() {
    use crate::compiler::{CompileMode, CompileOptions, compile_file};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/classic_record_field_arithmetic.act");
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                &source,
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap();
            let output = &compiled.output;
            for index in [0u16, 1, 63, 64, 127, 128, 255, 256, 257] {
                for operand in [0u16, 1, 0xFF, 0x1234, 0x7FFF, 0x8000, 0xFFF3] {
                    let mut memory = [0u8; 65536];
                    if runtime == Runtime::ActionCart {
                        let cart = include_bytes!("../../../roms/action.rom");
                        memory[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
                        memory[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
                    }
                    let origin = usize::from(output.origin);
                    memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
                    memory[0x4F00..0x5500].fill(0xA5);
                    let field = 0x5003 + usize::from(index) * 4;
                    memory[field..field + 2].copy_from_slice(&operand.to_le_bytes());
                    memory[0x600..0x610].fill(0xCC);
                    memory[0x60E..0x610].copy_from_slice(&index.to_le_bytes());
                    let before = memory;
                    indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                    let expected = 0x2345u16.wrapping_sub(operand).to_le_bytes();
                    for offset in [0, 2, 4] {
                        assert_eq!(
                            &memory[0x600 + offset..0x602 + offset],
                            &expected,
                            "{mode:?}/{runtime:?}/index={index}/operand={operand:04X}/output={offset}"
                        );
                    }
                    assert_eq!(&memory[0x606..0x610], &before[0x606..0x610]);
                    assert_eq!(&memory[0x4F00..0x5500], &before[0x4F00..0x5500]);
                }
            }
        }
    }
}
