use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(image: &[u8], runtime: Runtime, input: u8, word: u16, observe_error: bool) -> Vec<u8> {
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
                    std::fs::read(root().join("roms").join(name)).unwrap(),
                )
                .unwrap();
            }
            ExecutionProfile::CartridgeObject
        }
    };
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|segment| segment.end < 0x600 || segment.start > 0x790)
    );
    for address in 0x600..=0x700 {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    vm.bus_mut().ram_mut().write(0x600, input);
    vm.bus_mut().ram_mut().write(0x603, 0);
    vm.bus_mut().ram_mut().write_word(0x610, word);
    if observe_error {
        // Object execution does not boot DOS or the cartridge. Observe Error
        // using a returning handler; the compiler's fault fallback must still
        // prevent continuation. This handler is a test double, not cart ROM.
        vm.bus_mut()
            .ram_mut()
            .map(0x780, &[0x8D, 0, 7, 0x60])
            .unwrap(); // STA $0700; RTS
        match runtime {
            Runtime::ActionCart => vm
                .bus_mut()
                .ram_mut()
                .map(0x04CB, &[0x4C, 0x80, 7])
                .unwrap(),
            Runtime::Standalone => vm.bus_mut().ram_mut().write_word(0x000A, 0x0780),
        }
    }
    let result = VmRunner::new(vm).run(RunRequest {
        max_steps: 3_000,
        history_len: 4,
        ..RunRequest::default()
    });
    assert_eq!(
        result.stop_reason(),
        StopReason::StepLimit { max_steps: 3_000 },
        "{runtime:?}/{input}/{word}: {:?}",
        result.report
    );
    (0x600..=0x700)
        .map(|address| result.memory().read(address))
        .collect()
}

#[test]
fn case_dispatch_ranges_and_captures_execute_in_every_backend_and_runtime() {
    let words = [
        0, 1, 254, 255, 256, 257, 32766, 32767, 32768, 32769, 65534, 65535,
    ];
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                root().join("fixtures/runtime/case_statements.act"),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap();
            for input in 0..=255u8 {
                let word = words[usize::from(input) % words.len()];
                let bytes = run(compiled.object_bytes(), runtime, input, word, false);
                let expected_byte = match input {
                    0..=2 => 10,
                    127..=129 | 255 => 20,
                    _ => 30,
                };
                let expected_signed = if (word as i16) < 0 {
                    40
                } else if word == 0 {
                    41
                } else {
                    42
                };
                let expected_unsigned = match word {
                    0..=255 => 50,
                    256..=65534 => 51,
                    _ => 52,
                };
                assert_eq!(
                    [
                        bytes[2],
                        bytes[3],
                        bytes[4],
                        bytes[0x20],
                        bytes[0x21],
                        bytes[0x22],
                        bytes[0xFF]
                    ],
                    [
                        expected_byte,
                        1,
                        if input == 255 { 72 } else { 71 },
                        expected_signed,
                        expected_unsigned,
                        if input < 128 { 60 } else { 61 },
                        0xA5
                    ],
                    "{mode:?}/{runtime:?}/{input}/{word}"
                );
                for guard in [1, 5, 0x0F, 0x12, 0x1F, 0x23, 0xFE] {
                    assert_eq!(
                        bytes[guard], 0xCC,
                        "{mode:?}/{runtime:?}/{input}/{word}: guard {guard}"
                    );
                }
            }
        }
    }
}

#[test]
fn case_only_selected_arms_execute_calls_stores_and_faults() {
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                root().join("fixtures/runtime/case_effects.act"),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap();
            for input in [0, 1, 2, 3, 255] {
                let bytes = run(compiled.object_bytes(), runtime, input, 0, true);
                if matches!(input, 1 | 2) {
                    assert_eq!(
                        [bytes[1], bytes[2], bytes[255], bytes[256]],
                        [71, 0, 0xCC, 101],
                        "{mode:?}/{runtime:?}/{input}: expected non-returning Error(101), not a watchdog-only failure"
                    );
                } else {
                    assert_eq!(
                        [bytes[1], bytes[2], bytes[255], bytes[256]],
                        [
                            if input == 0 { 71 } else { 72 },
                            u8::from(input == 0),
                            0xA5,
                            0xCC
                        ],
                        "{mode:?}/{runtime:?}/{input}: unselected arm had effects"
                    );
                }
            }
        }
    }
}
