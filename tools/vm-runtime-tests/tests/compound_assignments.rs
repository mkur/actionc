use std::path::Path;

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};

#[test]
fn compound_integer_widths_and_signedness_across_profiles_and_runtimes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                root.join("fixtures/runtime/compound_assignments.act"),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
            for (case, (byte, word, divisor, signed, signed_divisor, shift)) in [
                (13u8, 513u16, 256u16, -513i16, 256i16, 2u8),
                (255, 32767, 3, -32768, -1, 7),
                (128, 16384, 32767, 32767, -256, 1),
                (0, 0, 32767, -1, 32767, 0),
                (1, 256, 257, -129, 128, 8),
            ]
            .into_iter()
            .enumerate()
            {
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
                let load = vm
                    .load_atari_object_for_execution(profile, compiled.object_bytes())
                    .unwrap();
                assert!(
                    load.segments
                        .iter()
                        .all(|segment| segment.end < 0x6000 || segment.start >= 0x6300)
                );
                for address in 0x6000..0x6300 {
                    vm.bus_mut().ram_mut().write(address, 0xCC);
                }
                vm.bus_mut().ram_mut().write(0x06E0, byte);
                vm.bus_mut().ram_mut().write(0x06E1, shift);
                for (address, value) in [
                    (0x06E2, word),
                    (0x06E4, divisor),
                    (0x06E6, signed as u16),
                    (0x06E8, signed_divisor as u16),
                ] {
                    for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
                        vm.bus_mut().ram_mut().write(address + offset as u16, byte);
                    }
                }
                let outcome = VmRunner::new(vm).run(RunRequest {
                    max_steps: 100_000,
                    history_len: 8,
                    ..RunRequest::default()
                });
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps: 100_000 }
                );
                assert_eq!(
                    outcome.memory().read(0x06FF),
                    0xA5,
                    "{mode:?}/{runtime:?}/{case}"
                );
                assert_eq!(
                    outcome.memory().read(0x06EA),
                    24,
                    "{mode:?}/{runtime:?}/{case}"
                );
                for (start, width, left, right) in [
                    (0x6000u16, 1usize, i32::from(byte), i32::from(divisor)),
                    (0x6100, 2, i32::from(word), i32::from(divisor)),
                    (0x6200, 2, i32::from(signed), i32::from(signed_divisor)),
                ] {
                    let bits = left as u16;
                    // Action! RSH shifts the stored bits logically, including INT.
                    let shifted_right = bits >> shift;
                    let remainder = if start == 0x6200 {
                        (left & 0x7FFF) % (right & 0x7FFF)
                    } else {
                        left % right
                    };
                    let values = [
                        left.wrapping_add(right) as u16,
                        left.wrapping_sub(right) as u16,
                        left.wrapping_mul(right) as u16,
                        (left / right) as u16,
                        remainder as u16,
                        (left & right) as u16,
                        (left | right) as u16,
                        (left ^ right) as u16,
                        bits.wrapping_shl(u32::from(shift)),
                        shifted_right,
                    ];
                    let mut expected = [0xCC; 256];
                    for (index, value) in values.into_iter().enumerate() {
                        let offset = 1 + index * 2 * width;
                        expected[offset..offset + width]
                            .copy_from_slice(&value.to_le_bytes()[..width]);
                    }
                    let actual: Vec<_> = (0..256)
                        .map(|offset| outcome.memory().read(start + offset))
                        .collect();
                    assert_eq!(
                        actual, expected,
                        "{mode:?}/{runtime:?}/case {case}/${start:04X}"
                    );
                }
            }
        }
    }
}
