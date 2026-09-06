use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn enum_values_returns_storage_and_case_execute_for_all_bytes() {
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                root().join("fixtures/runtime/enum_types.act"),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap();
            for input in 0..=255u8 {
                let mut vm = CompilerVm::default();
                let profile = if runtime == Runtime::ActionCart {
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
                } else {
                    ExecutionProfile::StandaloneObject
                };
                let load = vm
                    .load_atari_object_for_execution(profile, compiled.object_bytes())
                    .unwrap();
                for segment in &load.segments {
                    assert!(segment.end < 0x600 || segment.start > 0x6FF);
                    assert!(segment.end < 0x5000 || segment.start > 0x5106);
                }
                for address in 0x600..=0x6FF {
                    vm.bus_mut().ram_mut().write(address, 0xCC);
                }
                for address in 0x5000..=0x5106 {
                    vm.bus_mut().ram_mut().write(address, 0xA5);
                }
                vm.bus_mut().ram_mut().write(0x600, input);
                let result = VmRunner::new(vm).run(RunRequest {
                    max_steps: 5_000,
                    history_len: 4,
                    ..RunRequest::default()
                });
                assert_eq!(
                    result.stop_reason(),
                    StopReason::StepLimit { max_steps: 5_000 },
                    "{mode:?}/{runtime:?}/{input}: {:?}",
                    result.report
                );
                let memory = result.memory();
                let bytes = (0x600..=0x608)
                    .map(|address| memory.read(address))
                    .collect::<Vec<_>>();
                assert_eq!(
                    bytes,
                    [
                        input,
                        4,
                        match input {
                            0 | 128 => 1,
                            255 => 2,
                            _ => 3,
                        },
                        input,
                        if input == 255 { 72 } else { 71 },
                        255,
                        0,
                        input,
                        128
                    ],
                    "{mode:?}/{runtime:?}/{input}"
                );
                assert_eq!(memory.read(0x6FF), 0xA5, "completion marker");
                for address in 0x609..0x6FF {
                    assert_eq!(memory.read(address), 0xCC, "output guard {address:04X}");
                }
                for address in 0x5000..=0x5106 {
                    let expected = match address {
                        0x5102 => input,
                        0x5103 => 128,
                        0x5104 => 255,
                        _ => 0xA5,
                    };
                    assert_eq!(
                        memory.read(address),
                        expected,
                        "{mode:?}/{runtime:?}/{input}: record guard {address:04X}"
                    );
                }
            }
        }
    }
}
