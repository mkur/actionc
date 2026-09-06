use super::*;
use crate::compiler::{CompileMode, CompileOptions, compile_file};

#[test]
fn computed_record_field_comparisons_capture_operands_once() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/classic_record_field_comparisons.act");
    let source = std::fs::read_to_string(&path).unwrap();
    // Exercise both ordinary indexed fields (the port's regression) and
    // effectful indexes; the latter must not be evaluated once per byte.
    for effectful in [false, true] {
        let source = if effectful {
            source.clone()
        } else {
            source.replace("rows(Pick())", "rows(index)")
        };
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let model = analyze(&ast).unwrap();
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        for profile in [CodegenProfile::Compat, CodegenProfile::Modern] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let output = match runtime {
                    Runtime::ActionCart => {
                        generate_semir_profile_at_origin(&semir, 0x3000, profile)
                    }
                    Runtime::Standalone => {
                        generate_semir_standalone_profile_at_origin(&semir, 0x3000, profile)
                    }
                }
                .unwrap();
                for index in [0u16, 63, 127, 128, 255, 256, 257] {
                    for (left, right) in [
                        (0i16, 0i16),
                        (7, 7),
                        (0x1234, 0x1234),
                        (-1, -1),
                        (-32768, 32767),
                        (32767, -32768),
                        (256, 255),
                        (255, 256),
                    ] {
                        let mut memory = [0u8; 65536];
                        if runtime == Runtime::ActionCart {
                            let cart = include_bytes!("../../../roms/action.rom");
                            memory[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
                            memory[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
                        }
                        let origin = usize::from(output.origin);
                        memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
                        memory[0x4F00..0x5800].fill(0xA5);
                        let base = 0x5001 + usize::from(index) * 6;
                        memory[base..base + 2].copy_from_slice(&left.to_le_bytes());
                        memory[base + 2..base + 4].copy_from_slice(&right.to_le_bytes());
                        memory[0x600..0x630].fill(0xCC);
                        memory[0x620..0x622].copy_from_slice(&right.to_le_bytes());
                        memory[0x622..0x624].copy_from_slice(&index.to_le_bytes());
                        let before = memory;
                        indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                        let expected = [
                            left == right,
                            left != right,
                            left < right,
                            left <= right,
                            left > right,
                            left >= right,
                            right == left,
                            right != left,
                            right < left,
                            right <= left,
                            right > left,
                            right >= left,
                            left == right,
                            left != right,
                        ]
                        .map(u8::from);
                        assert_eq!(
                            &memory[0x600..0x60E],
                            &expected,
                            "{profile:?}/{runtime:?}/effectful={effectful}/index={index}/{left}/{right}"
                        );
                        assert_eq!(memory[0x624], if effectful { 16 } else { 0 });
                        assert_eq!(&memory[0x60E..0x624], &before[0x60E..0x624]);
                        assert_eq!(&memory[0x625..0x630], &before[0x625..0x630]);
                        assert_eq!(&memory[0x4F00..0x5800], &before[0x4F00..0x5800]);
                    }
                }
            }
        }
    }
}

#[test]
fn record_array_pointer_decay_preserves_backing_across_storage_kinds() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/record_array_pointer_decay.act");
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
            for large in [0u16, 63, 64, 127, 128, 255, 256, 257] {
                let small = usize::from(large % 3);
                let label = format!("{mode:?}/{runtime:?}/index={large}");
                let mut memory = [0u8; 65536];
                if runtime == Runtime::ActionCart {
                    let cart = include_bytes!("../../../roms/action.rom");
                    memory[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
                    memory[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
                }
                let origin = usize::from(output.origin);
                assert!(
                    origin + output.bytes.len() < 0x4F00,
                    "{label}: host RAM overlaps image"
                );
                memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
                memory[0x4F00..0x7000].fill(0xA5);
                memory[0x600..0x640].fill(0xCC);
                memory[0x60C..0x60E].copy_from_slice(&(small as u16).to_le_bytes());
                memory[0x60E..0x610].copy_from_slice(&large.to_le_bytes());
                let values = [0xFFF3u16, 0x1234, 0x8000];
                for (base, count, value) in [
                    (0x5001, 258, values[0]),
                    (0x5803, 258, values[1]),
                    (0x6F01, 1, values[2]),
                ] {
                    for i in 0..count {
                        memory[base + i * 6 + 2..base + i * 6 + 4]
                            .copy_from_slice(&value.to_le_bytes());
                    }
                }
                let mut expected = memory;
                for (base, index, value) in [
                    (0x5001, usize::from(large), values[0]),
                    (0x5803, usize::from(large), values[1]),
                    (0x6F01, 0, values[2]),
                ] {
                    expected[base + index * 6..base + index * 6 + 2]
                        .copy_from_slice(&value.to_le_bytes());
                }
                indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
                let word = |address| u16::from_le_bytes([memory[address], memory[address + 1]]);
                for (slot, value) in [
                    11 + small as u16 * 10,
                    41 + small as u16 * 10,
                    71 + small as u16,
                    values[0],
                    values[1],
                    values[2],
                ]
                .into_iter()
                .enumerate()
                {
                    assert_eq!(word(0x600 + slot * 2), value, "{label}/result={slot}");
                }
                assert_eq!(
                    [word(0x626), word(0x628), word(0x62A)],
                    [0x5001, 0x5803, 0x6F01],
                    "{label}: fixed and scalar bases"
                );
                for (slot, start, step, z_start) in
                    [(0, 11u16, 10u16, 12u16), (1, 41, 10, 42), (2, 71, 1, 81)]
                {
                    let base = usize::from(word(0x620 + slot * 2));
                    for i in 0..3 {
                        let y = start + i as u16 * step;
                        assert_eq!(
                            [
                                word(base + i * 6),
                                word(base + i * 6 + 2),
                                word(base + i * 6 + 4)
                            ],
                            [if i == small { y } else { 0 }, y, z_start + i as u16 * step],
                            "{label}/array={slot}/element={i}"
                        );
                    }
                }
                assert_eq!(
                    &memory[0x4F00..0x7000],
                    &expected[0x4F00..0x7000],
                    "{label}: fixed fields/guards"
                );
                assert_eq!(
                    &memory[0x60C..0x620],
                    &expected[0x60C..0x620],
                    "{label}: inputs/guards"
                );
                assert_eq!(
                    &memory[0x62C..0x640],
                    &expected[0x62C..0x640],
                    "{label}: result guards"
                );
            }
        }
    }
}
