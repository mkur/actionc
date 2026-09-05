use std::path::Path;

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};

struct Sources(std::path::PathBuf);
impl Drop for Sources {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn modern_embedded_arrays_preserve_initialization_addresses_and_whole_record_copies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let template =
        std::fs::read_to_string(root.join("fixtures/runtime/embedded_record_arrays.act")).unwrap();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "actionc-embedded-arrays-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let sources = Sources(directory);
    for count in [1usize, 2, 100, 127, 128, 129, 255, 256, 257] {
        let source = sources.0.join(format!("arrays-{count}.act"));
        std::fs::write(
            &source,
            template.replacen("CONST Count=257", &format!("CONST Count={count}"), 1),
        )
        .unwrap();
        let size = count * 6 + 8;
        let pattern = (0..size).map(|i| (i * 37 + 13) as u8).collect::<Vec<_>>();
        let mut expected = vec![0xCC; 0x1000];
        expected[1..1 + size].copy_from_slice(&pattern);
        expected[0x801..0x801 + size].copy_from_slice(&pattern);
        expected.copy_within(1..1 + size, 2);
        expected.copy_within(2..2 + size, 0);
        expected[0xF00..0xF03].copy_from_slice(&[0xA5, 0x34, 0x12]);
        expected[0xF10..0xF13].copy_from_slice(&[0xA5, 0x34, 0x12]);
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let label = format!("{mode:?}/{runtime:?}/Count={count}");
                let compiled = compile_file(
                    &source,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap_or_else(|error| panic!("{label}: {error}"));
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
                        .all(|segment| segment.end < 0x7000 || segment.start >= 0x8000),
                    "{label}: image overlaps oracle RAM"
                );
                for address in 0x7000..0x8000 {
                    vm.bus_mut().ram_mut().write(address, 0xCC);
                }
                for (i, byte) in pattern.iter().enumerate() {
                    vm.bus_mut().ram_mut().write(0x7001 + i as u16, *byte);
                }
                for address in 0x600..0x610 {
                    vm.bus_mut().ram_mut().write(address, 0xCC);
                }
                for (offset, byte) in [0xA5, 0x34, 0x12].into_iter().enumerate() {
                    vm.bus_mut().ram_mut().write(0x7F00 + offset as u16, byte);
                }
                let outcome = VmRunner::new(vm).run(RunRequest {
                    max_steps: 2_000_000,
                    history_len: 8,
                    ..RunRequest::default()
                });
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit {
                        max_steps: 2_000_000
                    },
                    "{label}"
                );
                let memory = outcome.memory();
                let actual = (0x7000..0x8000)
                    .map(|address| memory.read(address))
                    .collect::<Vec<_>>();
                assert_eq!(
                    actual, expected,
                    "{label}: complete copied/overlapping memory and guards"
                );
                assert_eq!(
                    (0x600..0x604)
                        .map(|address| memory.read(address))
                        .collect::<Vec<_>>(),
                    [12, 1, 1, 0xA5],
                    "{label}: ordered once-only calls and completion"
                );
                let word =
                    |address| u16::from_le_bytes([memory.read(address), memory.read(address + 1)]);
                let base = word(0x604);
                assert_eq!(
                    [word(0x606), word(0x608), word(0x60A)],
                    [
                        base + 1,
                        base + (4 * count - 1) as u16,
                        base + (6 * count - 1) as u16
                    ],
                    "{label}: static subobject addresses"
                );
                let mut initialized = vec![0; size];
                initialized[..3].copy_from_slice(&[1, 2, 3]);
                assert_eq!(
                    (0..size)
                        .map(|i| memory.read(base + i as u16))
                        .collect::<Vec<_>>(),
                    initialized,
                    "{label}: initialized source untouched"
                );
                assert_eq!(
                    (0x60C..0x610)
                        .map(|address| memory.read(address))
                        .collect::<Vec<_>>(),
                    [0xCC; 4],
                    "{label}: probe guards"
                );
            }
        }
    }
}
