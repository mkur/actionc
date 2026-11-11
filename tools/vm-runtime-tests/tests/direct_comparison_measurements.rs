//! Execute the exact feature-off/on TN images from the opt-in corpus measurement.
use actionc::compiler::Runtime;
use actionc_vm::{CompilerVm, VmRunHooks};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

struct Output {
    putchar: u16,
    bytes: Vec<u8>,
}
impl VmRunHooks for Output {
    type Error = String;

    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        if vm.cpu().registers().pc == self.putchar {
            self.bytes.push(vm.cpu().registers().a);
            vm.set_pc(machine::RETURN);
        }
        Ok(())
    }
}

#[test]
#[ignore = "requires artifacts from captured_compare_measure_corpus"]
fn measured_tn_images_match_search_and_print_oracles() {
    let artifacts = root().join("build/direct-comparisons/measured");
    for name in ["TN", "TNDBG"] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut baseline = Vec::new();
            for enabled in [false, true] {
                let stem = format!("{name}-{runtime:?}-{enabled}");
                let entries = std::fs::read_to_string(artifacts.join(format!("{stem}.entries")))
                    .expect("run the opt-in captured_compare_measure_corpus test first");
                let entry = |routine: &str| {
                    entries
                        .lines()
                        .find_map(|line| {
                            let (name, address) = line.split_once('\t')?;
                            name.eq_ignore_ascii_case(routine)
                                .then(|| u16::from_str_radix(address, 16).unwrap())
                        })
                        .unwrap()
                };
                let address = entry("Instr");
                let image = std::fs::read(artifacts.join(format!("{stem}.xex"))).unwrap();
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                let mut observations = Vec::new();
                for pointer in [0x7000u16, 0x70FF] {
                    for length in [0, 1, 127, 128, 200, 254, 255u8] {
                        for matching in [None, Some(1), Some(length / 2), Some(length)] {
                            let mut text = vec![b'A'; usize::from(length)];
                            if let Some(position) = matching.filter(|&p| p > 0 && p <= length) {
                                text[usize::from(position - 1)] = b'B';
                            }
                            let expected = text
                                .iter()
                                .position(|&c| c == b'B')
                                .map_or(0, |i| i as u8 + 1);
                            vm.bus_mut().ram_mut().write(pointer, length);
                            if !text.is_empty() {
                                vm.bus_mut().ram_mut().map(pointer + 1, &text).unwrap();
                            }
                            let [lo, hi] = pointer.to_le_bytes();
                            let start = vm.cpu().cycles();
                            vm = machine::call(vm, &mut Hooks, address, &[lo, hi, b'B']);
                            let cycles = vm.cpu().cycles() - start;
                            assert_eq!(
                                vm.bus().ram().read(0xA0),
                                expected,
                                "{stem}/{length}/{matching:?}"
                            );
                            assert_eq!(
                                vm.bus().ram().read(0xE0),
                                if expected > 0 {
                                    expected
                                } else {
                                    length.saturating_add(1)
                                }
                            );
                            assert_eq!(machine::counted(&vm, pointer), text);
                            observations.push(cycles);
                            if length == 200 && matching.is_none() {
                                eprintln!(
                                    "tn-instr,{stem},pointer={pointer:04X},length={length},cycles={cycles}"
                                );
                            }
                        }
                    }
                }
                // Keep output deterministic while exercising the real Print
                // and PrintFname loops and their JSR/RTS calling convention.
                vm.bus_mut().ram_mut().write(machine::RETURN, 0x60);
                for (routine, first) in [("Print", 0usize), ("PrintFname", 2)] {
                    for pointer in [0x7000u16, 0x70FF] {
                        for length in [0, 1, 2, 3, 8, 128, 254, 255u8] {
                            let text: Vec<_> = (0..length).map(|i| 32 + i % 96).collect();
                            vm.bus_mut().ram_mut().write(pointer, length);
                            if !text.is_empty() {
                                vm.bus_mut().ram_mut().map(pointer + 1, &text).unwrap();
                            }
                            let mut output = Output {
                                putchar: entry("Putchar"),
                                bytes: Vec::new(),
                            };
                            let start = vm.cpu().cycles();
                            vm = machine::call(
                                vm,
                                &mut output,
                                entry(routine),
                                &pointer.to_le_bytes(),
                            );
                            let cycles = vm.cpu().cycles() - start;
                            assert_eq!(
                                output.bytes,
                                text[first.min(text.len())..],
                                "{stem}/{routine}/{length}"
                            );
                            assert_eq!(machine::counted(&vm, pointer), text);
                            observations.push(cycles);
                            if length == 8 {
                                eprintln!(
                                    "tn-print,{stem},{routine},pointer={pointer:04X},length={length},cycles={cycles}"
                                );
                            }
                        }
                    }
                }
                if enabled {
                    for (before, after) in baseline.iter().zip(observations) {
                        assert!(after < *before, "{stem}: {before} -> {after}");
                    }
                } else {
                    baseline = observations;
                }
            }
        }
    }
}
