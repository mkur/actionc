//! Independent data/guard oracle for retained bases and changing pointers.
use actionc::{codegen, compiler::Runtime, mir6502};
use actionc_vm::{CompilerVm, VmRunHooks};
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
struct Traffic(usize, Vec<u16>);
impl VmRunHooks for Traffic {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let pc = vm.cpu().registers().pc;
        let ram = vm.bus().ram();
        if ram.read(pc) == 0xB1 {
            self.1.push(
                ram.read_word(u16::from(ram.read(pc + 1)))
                    .wrapping_add(u16::from(vm.cpu().registers().y)),
            );
        }
        if matches!(ram.read(pc), 0x85 | 0x86 | 0x84) && (0xAA..=0xAF).contains(&ram.read(pc + 1)) {
            self.0 += 1;
        }
        Ok(())
    }
}
fn exercise(output: &codegen::CodegenOutput) -> Vec<(String, u8, u64, usize)> {
    let (mut vm, profile) = support::vm_for(output.map.runtime);
    vm.load_atari_object_for_execution(profile, &codegen::format_load_file(output))
        .unwrap();
    let routine = |name| {
        output
            .map
            .routine_addresses
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .address
    };
    let global = |name| {
        output
            .map
            .storage_symbols
            .iter()
            .find(|s| s.name == name && s.scope == codegen::CodegenSymbolScope::Global)
            .unwrap()
            .address
    };
    let buffer = global("buffer");
    let data: Vec<_> = (0..512)
        .map(|n| (n as u8).wrapping_mul(37).wrapping_add(3))
        .collect();
    let mut measurements = vec![];
    for (name, counts) in [
        ("ReadAcrossCall", vec![0]),
        ("ConvertSpan", vec![0, 1, 127, 128, 254, 255]),
        ("ScanFixed", vec![0]),
        ("ReadFixed", vec![0]),
    ] {
        if !output.map.routine_addresses.iter().any(|r| r.name == name) {
            continue;
        }
        for count in counts {
            vm.bus_mut().ram_mut().map(buffer, &data).unwrap();
            let mut traffic = Traffic::default();
            let cycles = vm.cpu().cycles();
            let args = if name == "ReadAcrossCall" || name == "ReadFixed" {
                buffer.to_le_bytes().to_vec()
            } else {
                vec![count]
            };
            vm = machine::call(vm, &mut traffic, routine(name), &args);
            if name == "ReadFixed" {
                assert_eq!(
                    traffic.1,
                    (0..32)
                        .flat_map(|i| [buffer + i, buffer + i])
                        .collect::<Vec<_>>()
                );
            }
            measurements.push((name.into(), count, vm.cpu().cycles() - cycles, traffic.0));
            let mut expected = data.clone();
            match name {
                "ReadAcrossCall" => assert_eq!(vm.bus().ram().read(global("observed")), data[2]),
                "ReadFixed" => assert_eq!(vm.bus().ram().read(global("observed")), data[31]),
                "ConvertSpan" => {
                    expected[0] = 0;
                    for byte in &mut expected[..usize::from(count)] {
                        *byte ^= 0x5A;
                    }
                }
                "ScanFixed" => {
                    assert_eq!(vm.bus().ram().read(global("observed")), data[31]);
                    assert_eq!(
                        vm.bus().ram().read_word(global("finalPointer")),
                        buffer + 32
                    );
                }
                _ => unreachable!(),
            }
            assert_eq!(machine::bytes(&vm, buffer, 512), expected, "{name}/{count}");
        }
    }
    measurements
}

#[test]
fn pointer_calls_loops_and_scans_preserve_data_and_final_state() {
    qualify("pointer_retention.act");
}
#[test]
fn invariant_pointer_loops_preserve_both_reads_through_calls() {
    qualify("pointer_invariant_loop.act");
}
fn qualify(name: &str) {
    let source = std::fs::read_to_string(root().join("fixtures/runtime").join(name))
        .unwrap()
        .replace("\r\n", "\n");
    let mut lf_images = std::collections::BTreeMap::new();
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
                    let mut baseline_size = None;
                    for stage in 0..=2 {
                        let mut config = mir6502::Mir6502Config::optimized();
                        config.enable_pointer_call_retention = stage > 0;
                        config.enable_pointer_loop_retention = stage > 1;
                        config.enable_pointer_promotion = false;
                        let output = mir6502::generate_output_with_config_and_runtime(
                            &nir, origin, &config, runtime,
                        )
                        .unwrap();
                        let image = codegen::format_load_file(&output);
                        let key = format!("{runtime:?}/{optimized_nir}/{origin}/{stage}");
                        if crlf {
                            assert_eq!(&image, &lf_images[&key]);
                        } else {
                            lf_images.insert(key, image);
                        }
                        let measured = exercise(&output);
                        if let Some(old) = &controls {
                            let old: &Vec<(String, u8, u64, usize)> = old;
                            assert!(output.bytes.len() <= baseline_size.unwrap());
                            assert!(measured[0].2 < old[0].2);
                            assert!(measured[0].3 < old[0].3);
                            if stage == 2 && name == "pointer_invariant_loop.act" {
                                assert_eq!(measured[0].3, 2);
                            }
                        } else {
                            baseline_size = Some(output.bytes.len());
                            controls = Some(measured.clone());
                        }
                        if !crlf && optimized_nir && origin == 0x3000 {
                            for (name, count, cycles, stores) in measured {
                                println!(
                                    "pointer-retention,{runtime:?},stage={stage},{name},count={count},cycles={cycles},pair-stores={stores}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
