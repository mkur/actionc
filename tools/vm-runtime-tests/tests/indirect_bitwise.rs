use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::Path;

#[test]
fn indirect_byte_and_word_bitwise_rhs_preserves_left_operand() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                root.join("fixtures/runtime/indirect_bitwise.act"),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap();
            for left in 0..=255u8 {
                let right = [0, 1, 0x55, 0xAA, 0x80, 0xFF][usize::from(left) % 6];
                let a = u16::from_le_bytes([left, right]);
                let b = u16::from_le_bytes([right, left.rotate_left(3)]);
                let label = format!("{mode:?}/{runtime:?}/{left:02X}/{right:02X}");
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
                assert!(
                    loaded
                        .segments
                        .iter()
                        .all(|s| s.end < 0x0600 || s.start > 0x08FF)
                );
                let mut expected = vec![0xCC; 0x300];
                expected[0x1F9] = left;
                expected[0x1FB] = right;
                expected[0x1FD..0x1FF].copy_from_slice(&a.to_le_bytes());
                expected[0x1FF..0x201].copy_from_slice(&b.to_le_bytes());
                vm.bus_mut().ram_mut().map(0x0600, &expected).unwrap();
                expected[1..5].copy_from_slice(&[
                    left | right,
                    left & right,
                    left ^ right,
                    u8::from(left | right != 0),
                ]);
                for (offset, value) in [(5, a | b), (7, a & b), (9, a ^ b)] {
                    expected[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
                }
                expected[0xFF] = 0xA5;
                for _ in 0..10_000 {
                    vm.step_cpu().unwrap_or_else(|e| panic!("{label}: {e:?}"));
                    if vm.bus().ram().read(0x06FF) == 0xA5 {
                        break;
                    }
                }
                for (offset, value) in expected.into_iter().enumerate() {
                    let address = 0x0600 + offset as u16;
                    assert_eq!(
                        vm.bus().ram().read(address),
                        value,
                        "{label}/${address:04X}"
                    );
                }
            }
        }
    }
}
