//! Captured-bound motion through the real frontend, ABI and both native runtimes.
use actionc::compiler::Runtime;
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

#[derive(Default)]
struct Reads {
    addresses: Vec<u16>,
}
impl VmRunHooks for Reads {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let ram = vm.bus().ram();
        if ram.read(r.pc) == 0xB1 {
            let a = ram
                .read_word(u16::from(ram.read(r.pc.wrapping_add(1))))
                .wrapping_add(u16::from(r.y));
            if (0x50FF..=0x5102).contains(&a) {
                self.addresses.push(a);
            }
        }
        Ok(())
    }
}

#[test]
fn captured_bounds_preserve_zero_trip_wide_lanes_snapshots_and_fallbacks() {
    let source =
        include_str!("../../../surveys/tn/reproducers/captured-bounds.act").replace("\r\n", "\n");
    let values = [
        0u16, 1, 7, 8, 9, 255, 256, 257, 511, 512, 1088, 1089, 4095, 4096, 0x7FFF, 0x8000, 0xFFF8,
        0xFFFF,
    ];
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for enabled in [false, true] {
            let mut lf = None;
            for crlf in [false, true] {
                let file = support::Source::new(&if crlf {
                    source.replace('\n', "\r\n")
                } else {
                    source.clone()
                });
                let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
                let model = actionc::semantic::analyze_with_options(
                    &loaded.program,
                    actionc::semantic::SemanticOptions::modern(),
                )
                .unwrap();
                let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
                let nir =
                    actionc::nir::optimize_program(&actionc::nir::lower_program(&sem)).unwrap();
                let mut config = Mir6502Config::optimized();
                config.enable_captured_bound_motion = enabled;
                let output = mir6502::generate_output_with_config_and_runtime(
                    &nir, 0x3000, &config, runtime,
                )
                .unwrap();
                let image = actionc::codegen::format_load_file(&output);
                if let Some(old) = &lf {
                    assert_eq!(&image, old);
                } else {
                    lf = Some(image.clone());
                }
                let entry = |name: &str| {
                    output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                let touched = output
                    .map
                    .storage_symbols
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case("touched"))
                    .unwrap()
                    .address;
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                let mut hooks = Reads::default();
                for value in values {
                    for name in ["Count", "Repeated", "Mutable"] {
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x9F, &[0xA5, 0x55, 0xAA, 0x5A])
                            .unwrap();
                        let cycles = vm.cpu().cycles();
                        vm = machine::call(vm, &mut hooks, entry(name), &value.to_le_bytes());
                        let expected = match name {
                            "Repeated" => (value >> 3) & 511,
                            "Mutable" => {
                                let (mut ordinal, mut capacity) = (0, value);
                                while ordinal < (capacity >> 3) {
                                    ordinal += 1;
                                    capacity -= 1;
                                }
                                ordinal
                            }
                            _ => value >> 3,
                        };
                        assert_eq!(
                            vm.bus().ram().read_word(0xA0),
                            expected,
                            "{runtime:?}/{enabled}/{crlf}/{name}/{value}"
                        );
                        assert_eq!(vm.bus().ram().read(0x9F), 0xA5);
                        assert_eq!(vm.bus().ram().read(0xA2), 0x5A);
                        if !crlf && value == 1088 {
                            println!(
                                "captured-bound,{runtime:?},{enabled},{name},{}",
                                vm.cpu().cycles() - cycles
                            );
                        }
                    }
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x50FF, &[0xA5, value as u8, (value >> 8) as u8, 0x5A])
                        .unwrap();
                    hooks.addresses.clear();
                    vm = machine::call(vm, &mut hooks, entry("Snapshot"), &0x5100u16.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(0xA0), value >> 3);
                    assert_eq!(machine::bytes(&vm, 0x50FF, 4), [0xA5, 0, 0, 0x5A]);
                    assert_eq!(
                        hooks.addresses,
                        vec![0x5100, 0x5101],
                        "original capture remains ordered and occurs once"
                    );
                    if value <= 512 {
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x50FF, &[0xA5, value as u8, (value >> 8) as u8, 0x5A])
                            .unwrap();
                        hooks.addresses.clear();
                        vm = machine::call(
                            vm,
                            &mut hooks,
                            entry("Dynamic"),
                            &0x5100u16.to_le_bytes(),
                        );
                        assert_eq!(vm.bus().ram().read_word(0xA0), value >> 3);
                        let expected = (0..=(value >> 3))
                            .flat_map(|_| [0x5100, 0x5101])
                            .collect::<Vec<_>>();
                        assert_eq!(
                            hooks.addresses, expected,
                            "record/pointer reads stay inside the loop"
                        );
                        vm.bus_mut().ram_mut().write(touched, 0xF0);
                        vm = machine::call(vm, &mut hooks, entry("WithCall"), &value.to_le_bytes());
                        assert_eq!(vm.bus().ram().read_word(0xA0), value >> 3);
                        assert_eq!(
                            vm.bus().ram().read(touched),
                            0xF0u8.wrapping_add((value >> 3) as u8)
                        );
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x51FF, &[0xA5, 0xFF, 0x5A])
                            .unwrap();
                        let mut args = value.to_le_bytes().to_vec();
                        args.extend(0x5200u16.to_le_bytes());
                        vm = machine::call(vm, &mut hooks, entry("WithWrite"), &args);
                        assert_eq!(vm.bus().ram().read_word(0xA0), value >> 3);
                        assert_eq!(
                            machine::bytes(&vm, 0x51FF, 3),
                            [0xA5, if value >= 8 { 0 } else { 0xFF }, 0x5A]
                        );
                    }
                }
            }
        }
    }
}
