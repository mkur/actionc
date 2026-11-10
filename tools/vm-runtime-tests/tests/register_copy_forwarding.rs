//! Source-level oracle for direct byte stores and counted pointer forwarding.
use actionc::{codegen, compiler::Runtime, mir6502};
use actionc_vm::{CompilerVm, VmRunHooks};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[derive(Default)]
struct Traffic {
    private: BTreeSet<u16>,
    reads: usize,
    writes: usize,
}
impl VmRunHooks for Traffic {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let ram = vm.bus().ram();
        let pc = vm.cpu().registers().pc;
        let (write, address) = match ram.read(pc) {
            0xA5 | 0xA6 | 0xA4 => (false, u16::from(ram.read(pc + 1))),
            0xAD | 0xAE | 0xAC => (false, ram.read_word(pc + 1)),
            0x85 | 0x86 | 0x84 => (true, u16::from(ram.read(pc + 1))),
            0x8D | 0x8E | 0x8C => (true, ram.read_word(pc + 1)),
            _ => return Ok(()),
        };
        if self.private.contains(&address) {
            if write {
                self.writes += 1;
            } else {
                self.reads += 1;
            }
        }
        Ok(())
    }
}

fn exercise(
    output: &codegen::CodegenOutput,
    private: &BTreeSet<u16>,
) -> BTreeMap<(String, u16, u8), (u64, usize, usize)> {
    let (mut vm, profile) = support::vm_for(output.map.runtime);
    vm.load_atari_object_for_execution(profile, &codegen::format_load_file(output))
        .unwrap();
    let routine = |name: &str| {
        output
            .map
            .routine_addresses
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .address
    };
    let observed = output
        .map
        .storage_symbols
        .iter()
        .find(|s| s.name == "observed")
        .unwrap()
        .address;
    let mut measurements = BTreeMap::new();
    let mut invoke = |vm, name: &str, args: &[u8], input, value| {
        let mut traffic = Traffic {
            private: private.clone(),
            ..Default::default()
        };
        let cycles = CompilerVm::cpu(&vm).cycles();
        let vm = machine::call(vm, &mut traffic, routine(name), args);
        measurements.insert(
            (name.to_owned(), input, value),
            (vm.cpu().cycles() - cycles, traffic.reads, traffic.writes),
        );
        vm
    };
    for value in [0, 1, 127, 128, 254, 255] {
        vm = invoke(vm, "SaveBytes", &[value, !value, value ^ 0x55], 0, value);
        assert_eq!(
            machine::bytes(&vm, observed, 3),
            [value ^ 0x55, value, !value]
        );
    }
    for name in ["CopyCounted", "SendString"] {
        for (input, output) in [(0x5000u16, 0x5300u16), (0x50FE, 0x53FE)] {
            for length in [0u8, 1, 127, 128, 254, 255] {
                let mut data: Vec<_> = (0..256).map(|i| (i as u8).wrapping_mul(37)).collect();
                data[0] = length;
                let count = usize::from(length) + 1;
                vm.bus_mut().ram_mut().map(input, &data).unwrap();
                vm.bus_mut()
                    .ram_mut()
                    .map(output - 1, &vec![0xCC; 258])
                    .unwrap();
                let mut args = output.to_le_bytes().to_vec();
                args.extend(input.to_le_bytes());
                vm = invoke(vm, name, &args, input, length);
                assert_eq!(
                    machine::bytes(&vm, output, count),
                    data[..count],
                    "{name}/{input:04X}/{length}"
                );
                assert_eq!(vm.bus().ram().read(output - 1), 0xCC);
                assert_eq!(vm.bus().ram().read(output + count as u16), 0xCC);
                assert_eq!(machine::bytes(&vm, input, 256), data);
            }
        }
    }
    measurements
}

#[test]
fn direct_bytes_and_counted_copies_preserve_values_and_guards() {
    let source =
        std::fs::read_to_string(root().join("fixtures/runtime/register_copy_forwarding.act"))
            .unwrap()
            .replace("\r\n", "\n");
    let mut lf_images = BTreeMap::new();
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
        let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
            &loaded.program,
            &model,
        ));
        for optimized_nir in [false, true] {
            let nir = if optimized_nir {
                actionc::nir::optimize_program(&raw).unwrap()
            } else {
                raw.clone()
            };
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                for origin in [0x3000, 0x41C7] {
                    let mut controls = None;
                    let mut old_private = None;
                    for enabled in [false, true] {
                        let mut config = mir6502::Mir6502Config::optimized();
                        config.enable_register_copy_forwarding = enabled;
                        let output = mir6502::generate_output_with_config_and_runtime(
                            &nir, origin, &config, runtime,
                        )
                        .unwrap();
                        let mir = mir6502::materialize_program_with_origin_and_runtime(
                            mir6502::lower_program(&nir).unwrap(),
                            &config,
                            origin,
                            runtime,
                        )
                        .unwrap();
                        let private: BTreeSet<_> = mir
                            .routines
                            .iter()
                            .flat_map(|r| &r.frame.zero_page_allocations)
                            .flat_map(|a| {
                                (0..a.size)
                                    .map(move |offset| u16::from(a.start.0) + u16::from(offset))
                            })
                            .collect();
                        let image = codegen::format_load_file(&output);
                        let key = format!("{runtime:?}/{optimized_nir}/{origin}/{enabled}");
                        if crlf {
                            assert_eq!(&image, &lf_images[&key]);
                        } else {
                            lf_images.insert(key, image);
                        }
                        let measured = exercise(&output, &private);
                        if let Some(old) = &controls {
                            let old: &BTreeMap<(String, u16, u8), (u64, usize, usize)> = old;
                            assert_eq!(
                                Some(&private),
                                old_private.as_ref(),
                                "occupied zero page changed"
                            );
                            for (key, &(cycles, reads, writes)) in &measured {
                                let (old_cycles, old_reads, old_writes) = old[key];
                                assert!(cycles < old_cycles, "{key:?}: {old_cycles} -> {cycles}");
                                if key.0 != "SaveBytes" {
                                    assert_eq!((reads, writes), (0, 0));
                                    assert!(reads <= old_reads && writes <= old_writes);
                                    if optimized_nir {
                                        assert_eq!(
                                            (old_cycles - cycles, old_reads, old_writes),
                                            (18, 2, 2)
                                        );
                                    }
                                }
                            }
                        } else {
                            old_private = Some(private);
                            controls = Some(measured.clone());
                        }
                        if !crlf && optimized_nir && origin == 0x3000 {
                            for ((name, input, length), (cycles, reads, writes)) in measured {
                                if input == 0x5000 && [0, 255].contains(&length)
                                    || name == "SaveBytes" && length == 0
                                {
                                    println!(
                                        "register-copy,{runtime:?},{enabled},{name},length={length},cycles={cycles},scratch-reads={reads},scratch-writes={writes}"
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
