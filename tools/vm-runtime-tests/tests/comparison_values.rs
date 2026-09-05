use std::path::Path;

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};

#[test]
fn modern_comparison_values_preserve_widths_effects_and_destinations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                root.join("fixtures/runtime/comparison_values.act"),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
            for (case, (s, t, u, v, a, b)) in [
                (-32768i16, 32767i16, 0u16, 65535u16, 0u8, 255u8),
                (32767, -32768, 65535, 0, 255, 0),
                (-1, 0, 32768, 32767, 127, 128),
                (0, -1, 32767, 32768, 128, 127),
                (-129, -128, 255, 256, 1, 1),
                (256, 256, 32768, 32768, 255, 255),
            ]
            .into_iter()
            .enumerate()
            {
                let index = [0u16, 1, 127, 128, 255, 256][case];
                let mut vm = CompilerVm::default();
                let profile = match runtime {
                    Runtime::Standalone => ExecutionProfile::StandaloneObject,
                    Runtime::ActionCart => {
                        for (kind, name, base) in [
                            (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
                            (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
                        ] {
                            vm.load_image_bytes(
                                kind,
                                name,
                                base,
                                std::fs::read(root.join("roms").join(name)).unwrap(),
                            )
                            .unwrap();
                        }
                        ExecutionProfile::CartridgeObject
                    }
                };
                let loaded = vm
                    .load_atari_object_for_execution(profile, compiled.object_bytes())
                    .unwrap();
                let mut page = vec![0xCC; 256];
                for (offset, word) in [
                    (0xE0, s as u16),
                    (0xE2, t as u16),
                    (0xE4, u),
                    (0xE6, v),
                    (0xE8, index),
                    (0xEA, 0x5801),
                    (0xEC, 0x5C03),
                ] {
                    page[offset..offset + 2].copy_from_slice(&word.to_le_bytes());
                }
                page[0xEE] = a;
                page[0xEF] = b;
                let mut buffers = vec![0xCC; 0x1000];
                buffers[0x101] = 0x3B;
                buffers[0x102] = 0xC7;
                buffers[0x901] = a;
                buffers[0x902] = b;
                for (start, bytes) in [(0x0600u16, &page), (0x4F00, &buffers)] {
                    assert!(
                        loaded
                            .segments
                            .iter()
                            .all(|segment| usize::from(start) + bytes.len()
                                <= usize::from(segment.start)
                                || start > segment.end)
                    );
                    for (offset, byte) in bytes.iter().enumerate() {
                        vm.bus_mut().ram_mut().write(start + offset as u16, *byte);
                    }
                }
                for (group, (x, y)) in [
                    (i32::from(a), i32::from(b)),
                    (i32::from(u), i32::from(v)),
                    (i32::from(s), i32::from(t)),
                ]
                .into_iter()
                .enumerate()
                {
                    for (op, truth) in [x < y, x <= y, x >= y, x > y, x == y, x != y]
                        .into_iter()
                        .enumerate()
                    {
                        page[group * 6 + op] = u8::from(truth);
                    }
                }
                let (ab, uv, st) = (u8::from(a < b), u8::from(u < v), u8::from(s < t));
                page[18..32].copy_from_slice(&[
                    st,
                    ab + uv,
                    st & uv,
                    st | 2,
                    st ^ 1,
                    u8::from(ab == uv),
                    ab,
                    st,
                    ab & st,
                    ab,
                    ab,
                    if st == 0 { 0x3B } else { 0xC7 },
                    st + 5,
                    if a == 0 { uv } else { st },
                ]);
                page[0x40..0x42].copy_from_slice(&u16::from(st).to_le_bytes());
                page[0xF0] = 12;
                page[0xF1] = (0..6).fold(0u8, |order, _| order.wrapping_mul(100).wrapping_add(12));
                page[0xFF] = 0xA5;
                buffers[0x101 + usize::from(index)] = uv;
                let word_offset = 0x503 + usize::from(index) * 2;
                buffers[word_offset..word_offset + 2].copy_from_slice(&u16::from(st).to_le_bytes());
                buffers[0x901 + usize::from(index)] = ab;
                let outcome = VmRunner::new(vm).run(RunRequest {
                    max_steps: 40_000,
                    history_len: 8,
                    ..RunRequest::default()
                });
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps: 40_000 }
                );
                for (start, expected) in [(0x0600u16, page), (0x4F00, buffers)] {
                    let actual: Vec<_> = (0..expected.len())
                        .map(|offset| outcome.memory().read(start + offset as u16))
                        .collect();
                    assert_eq!(
                        actual, expected,
                        "{mode:?}/{runtime:?}/case {case}, region ${start:04X}"
                    );
                }
            }
        }
    }
}
