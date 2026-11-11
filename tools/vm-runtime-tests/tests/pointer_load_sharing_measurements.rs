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
#[ignore = "requires artifacts from pointer_load_sharing_measure_corpus"]
fn measured_tn_pointer_load_sharing_images_preserve_strings_and_parameter_aliases() {
    let artifacts = root().join("build/pointer-load-sharing/measured");
    const START: u16 = 0x6F00;
    for name in ["TN", "TNDBG"] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut baseline = Vec::new();
            for enabled in [false, true] {
                let stem = format!("{name}-{runtime:?}-{enabled}");
                let entries = std::fs::read_to_string(artifacts.join(format!("{stem}.entries")))
                    .expect("run pointer_load_sharing_measure_corpus first");
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
                        eprintln!("pointer-load-sharing-strcat,{stem},cycles={cycles}");
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
                        assert!(*after < *before, "{stem}: {before} -> {after}");
                    }
                } else {
                    baseline = observations;
                }
            }
        }
    }
}
