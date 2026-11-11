//! Execute the exact measured TN images, including same/overlapping strings.
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

struct Observe {
    move_page: u16,
    arguments: Vec<Vec<u8>>,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.move_page {
            self.arguments.push(vec![
                r.a,
                r.x,
                r.y,
                vm.bus().ram().read(0xA3),
                vm.bus().ram().read(0xA4),
            ]);
        }
        Ok(())
    }
}

#[test]
#[ignore = "requires artifacts from argument_x_bitwise_measure_corpus"]
fn measured_tn_argument_x_bitwise_images_preserve_strings_and_parameter_aliases() {
    let artifacts = root().join("build/argument-x-bitwise/measured");
    const START: u16 = 0x6F00;
    for name in ["TN", "TNDBG"] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut baseline = Vec::new();
            for enabled in [false, true] {
                let stem = format!("{name}-{runtime:?}-{enabled}");
                let entries = std::fs::read_to_string(artifacts.join(format!("{stem}.entries")))
                    .expect("run argument_x_bitwise_measure_corpus first");
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
                let image = std::fs::read(artifacts.join(format!("{stem}.xex"))).unwrap();
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                let mut observations = Vec::new();
                // Source lengths are positive: MovePage's existing assembly
                // interprets zero as 256 bytes. This pass leaves that behavior
                // unchanged; the counted-string oracle does not redefine it.
                for (s, t, first, second) in [
                    (0x7000u16, 0x7200u16, 0u8, 1u8),
                    (0x7000, 0x7200, 3, 8),
                    (0x7000, 0x7200, 127, 128),
                    (0x7000, 0x7200, 1, 255),
                    (0x70FF, 0x72FF, 3, 128),
                    (0x7000, 0x7000, 1, 1),
                    (0x7000, 0x7000, 4, 4),
                    (0x7000, 0x7000, 127, 127),
                    (0x7000, 0x7000, 128, 128),
                    // The actual copy ranges overlap; MovePage copies down
                    // from the last byte, preserving this rightward overlap.
                    (0x7000, 0x7002, 4, 6),
                ] {
                    let mut expected = (0..0x700).map(|n| 32 + (n % 96) as u8).collect::<Vec<_>>();
                    let at = |p: u16| usize::from(p - START);
                    expected[at(s)] = first;
                    expected[at(t)] = second;
                    vm.bus_mut().ram_mut().map(START, &expected).unwrap();
                    let destination = s + u16::from(first) + 1;
                    let source = t + 1;
                    let payload = expected[at(source)..at(source) + usize::from(second)].to_vec();
                    expected[at(destination)..at(destination) + payload.len()]
                        .copy_from_slice(&payload);
                    expected[at(s)] = expected[at(s)].wrapping_add(expected[at(t)]);
                    let mut args = s.to_le_bytes().to_vec();
                    args.extend(t.to_le_bytes());
                    let mut hooks = Observe {
                        move_page: entry("MovePage"),
                        arguments: Vec::new(),
                    };
                    let started = vm.cpu().cycles();
                    vm = machine::call(vm, &mut hooks, entry("Strcat"), &args);
                    let cycles = vm.cpu().cycles() - started;
                    let mut expected_args = destination.to_le_bytes().to_vec();
                    expected_args.extend(source.to_le_bytes());
                    expected_args.push(second);
                    assert_eq!(
                        hooks.arguments,
                        vec![expected_args],
                        "{stem}/{s:04X}/{t:04X}/{first}/{second}"
                    );
                    assert_eq!(
                        machine::bytes(&vm, START, expected.len()),
                        expected,
                        "{stem}/{s:04X}/{t:04X}/{first}/{second}"
                    );
                    observations.push(cycles);
                    if first == 3 && second == 8 {
                        assert_eq!(cycles, 355, "{stem}");
                        eprintln!("bitwise-strcat,{stem},cycles={cycles}");
                    }
                }
                let parameters =
                    std::fs::read_to_string(artifacts.join(format!("{stem}.params"))).unwrap();
                let parse = |text: &str| {
                    text.lines()
                        .map(|line| {
                            let (k, a) = line.split_once('\t').unwrap();
                            (k.to_string(), u16::from_str_radix(a, 16).unwrap())
                        })
                        .collect::<std::collections::BTreeMap<_, _>>()
                };
                let params = parse(&parameters);
                assert_eq!(
                    params,
                    parse(&parameters.replace("\r\n", "\n").replace('\n', "\r\n"))
                );
                for field in ["s", "t"] {
                    let home = params[&format!("Strcat.{field}")];
                    let s = home - 0x100;
                    vm.bus_mut().ram_mut().write(s, 255);
                    vm.bus_mut().ram_mut().map(0x7200, &[2, 0, 0x73]).unwrap();
                    vm.bus_mut().ram_mut().write(0x7300, 5);
                    let mut args = s.to_le_bytes().to_vec();
                    args.extend(0x7200u16.to_le_bytes());
                    let mut hooks = Observe {
                        move_page: entry("MovePage"),
                        arguments: Vec::new(),
                    };
                    vm = machine::call(vm, &mut hooks, entry("Strcat"), &args);
                    assert_eq!(vm.bus().ram().read_word(home), 0x7300, "{stem}/{field}");
                    if field == "s" {
                        assert_eq!(vm.bus().ram().read(0x7300), 7);
                        assert_eq!(vm.bus().ram().read(s), 255);
                    } else {
                        assert_eq!(vm.bus().ram().read(s), 4);
                        assert_eq!(vm.bus().ram().read(0x7300), 5);
                    }
                }
                if enabled {
                    for (before, after) in baseline.iter().zip(&observations) {
                        assert_eq!(after, before, "Strcat control changed: {stem}");
                    }
                } else {
                    baseline = observations;
                }
            }
        }
    }
}

#[allow(dead_code)]
#[path = "support/tn_symbols.rs"]
mod symbols;
struct TagObserve {
    store: u16,
    args: Vec<(u8, u8, u8, u8, u8)>,
}
impl VmRunHooks for TagObserve {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.store {
            self.args.push((r.a, r.x, r.y, r.status, r.sp));
        }
        Ok(())
    }
}
#[test]
#[ignore = "requires exact argument_x_bitwise_measure_corpus images"]
fn measured_tn_argument_x_bitwise_tag_updates_match_and_save_one_cycle() {
    let artifacts = root().join("build/argument-x-bitwise/measured");
    for name in ["TN", "TNDBG"] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            let mut before = std::collections::BTreeMap::new();
            for enabled in [false, true] {
                let stem = format!("{name}-{rt:?}-{enabled}");
                let asm = std::fs::read_to_string(artifacts.join(format!("{stem}.asm")))
                    .unwrap()
                    .replace("\r\n", "\n");
                let active = symbols::global_address(&asm, "active");
                let vector = symbols::global_address(&asm, "v");
                let entry = symbols::routine_address(&asm, "Tag");
                let store = symbols::routine_address(&asm, "Store");
                let crlf = asm.replace('\n', "\r\n");
                assert_eq!(symbols::global_address(&crlf, "active"), active);
                assert_eq!(symbols::global_address(&crlf, "v"), vector);
                assert_eq!(symbols::routine_address(&crlf, "Tag"), entry);
                let image = std::fs::read(artifacts.join(format!("{stem}.xex"))).unwrap();
                let (mut vm, profile) = support::vm_for(rt);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                for index in [0u8, 1] {
                    for old in [0u8, 127] {
                        for other in [0u8, 127] {
                            let target = 0x7600 + u16::from(index) * 32;
                            let mut rows = vec![0xD6; 64];
                            rows[usize::from(index) * 32] = old;
                            rows[usize::from(1 - index) * 32] = other;
                            vm.bus_mut().ram_mut().map(0x7600, &rows).unwrap();
                            vm.bus_mut()
                                .ram_mut()
                                .map(0x7400, &[0, 0x76, 0x20, 0x76])
                                .unwrap();
                            vm.bus_mut().ram_mut().write_word(vector, 0x7400);
                            let count = u8::from(old != 0) + u8::from(other != 0);
                            let initial_mask = if count == 2 { 127 } else { 0 };
                            let mut state = [0xD6; 9];
                            state[0] = 2;
                            state[3] = initial_mask;
                            state[8] = count;
                            vm.bus_mut().ram_mut().map(active, &state).unwrap();
                            let mut hooks = TagObserve {
                                store,
                                args: Vec::new(),
                            };
                            let cycles = vm.cpu().cycles();
                            vm = machine::call(vm, &mut hooks, entry, &[index]);
                            let cycles = vm.cpu().cycles() - cycles;
                            rows[usize::from(index) * 32] = old ^ 127;
                            state[8] = if old == 0 { count + 1 } else { count - 1 };
                            if state[8] == 2 {
                                state[3] = 127;
                            } else if state[8] == 0 {
                                state[3] = 0;
                            }
                            assert_eq!(machine::bytes(&vm, 0x7600, 64), rows, "{stem}");
                            assert_eq!(machine::bytes(&vm, active, 9), state, "{stem}");
                            assert_eq!(hooks.args.len(), 1);
                            let (a, x, y, _, _) = hooks.args[0];
                            assert_eq!([a, x, y], [target as u8, (target >> 8) as u8, old ^ 127]);
                            let key = (index, old, other);
                            if enabled {
                                let (states, old_cycles) = before.get(&key).unwrap();
                                assert_eq!(states, &hooks.args);
                                assert_eq!(old_cycles - cycles, 1, "{stem}/{key:?}");
                            } else {
                                before.insert(key, (hooks.args.clone(), cycles));
                            }
                            if index == 0 && old == 0 && other == 0 {
                                eprintln!("bitwise-Tag,{stem},cycles={cycles}");
                            }
                        }
                    }
                }
            }
        }
    }
}
