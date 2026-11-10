//! External Oscar64 artifacts: build with tools/compare6502/tn/build.py first.
use actionc_vm::{CompilerVm, ExecutionProfile, RunRequest, StopReason, VmRunner};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn artifact(name: &str) -> PathBuf {
    root().join("build/tn-c-oscar64").join(name)
}
fn symbol(stem: &str, name: &str) -> u16 {
    std::fs::read_to_string(artifact(&format!("{stem}.symbols.tsv")))
        .unwrap()
        .lines()
        .find_map(|l| {
            let mut c = l.split('\t');
            (c.next() == Some(name)).then(|| c.next().unwrap().parse().unwrap())
        })
        .unwrap()
}
#[test]
fn oscar_atari_startup_layout_and_scalar_abi() {
    for profile in ["Os", "O3"] {
        let stem = format!("probe-{profile}");
        let image = std::fs::read(artifact(&format!("{stem}.xex"))).unwrap();
        for (a, b, c) in [(3u8, 0x7FFFu16, 155u8), (255, 0xFFFF, 255), (128, 0, 0)] {
            let mut vm = CompilerVm::default();
            vm.prepare_execution_profile(ExecutionProfile::StandaloneObject)
                .unwrap();
            vm.reset_cpu();
            vm.prepare_headless_program_environment();
            vm.load_atari_object(&image).unwrap();
            // Oscar emits standard RUNAD ($02E0); the pinned Action! VM's
            // convenience loader recognizes Action!'s $02E2 INITAD instead.
            let entry = vm.bus().ram().read_word(0x02E0);
            assert_eq!(entry, 0x2C00);
            vm.set_pc(entry);
            vm.bus_mut().ram_mut().write(0x0610, a);
            vm.bus_mut().ram_mut().write(0x0611, c);
            vm.bus_mut().ram_mut().write_word(0x0612, b);
            for addr in [0xE0, 0xE1, 0xE6, 0xE7, 0xE8, 0xE9, 0xFE, 0xFF] {
                vm.bus_mut().ram_mut().write(addr, 0x5A);
            }
            let end = symbol(&stem, "probe_done");
            let result = VmRunner::new(vm).run(RunRequest {
                max_steps: 10000,
                stop_after_pc: Some(end),
                history_len: 8,
            });
            assert_eq!(result.stop_reason(), StopReason::PcReached { pc: end });
            let vm = result.into_vm();
            let ram = vm.bus().ram();
            assert_eq!(
                (0..5).map(|i| ram.read(0x600 + i)).collect::<Vec<_>>(),
                [0xA5, 9, 18, 0x34, 0x12]
            );
            assert_eq!(
                ram.read_word(0x605),
                b.wrapping_add(a.into()).wrapping_add(c.into())
            );
            assert_eq!(
                (0..3).map(|i| ram.read(0x607 + i)).collect::<Vec<_>>(),
                [1, 0xC2, 0x7F]
            );
            for addr in [0xE0, 0xE1, 0xE6, 0xE7, 0xE8, 0xE9, 0xFE, 0xFF] {
                assert_eq!(ram.read(addr), 0x5A);
            }
        }
    }
}

#[derive(Default)]
struct CioProbe {
    calls: Vec<(u8, Vec<u8>)>,
}
impl actionc_vm::VmRunHooks for CioProbe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        if vm.cpu().registers().pc == 0 {
            return Err(format!(
                "pc=0 after {} CIO calls; out={:02X?}; sp={:04X}; jump={:04X}",
                self.calls.len(),
                (0..32)
                    .map(|i| vm.bus().ram().read(0x600 + i))
                    .collect::<Vec<_>>(),
                vm.bus().ram().read_word(0xA1),
                vm.bus().ram().read_word(0x62a)
            ));
        }
        if vm.cpu().registers().pc == 0xE456 {
            let index = vm.cpu().registers().x;
            let base = 0x340 + u16::from(index);
            self.calls.push((
                index,
                (0..16).map(|i| vm.bus().ram().read(base + i)).collect(),
            ));
            let status = if self.calls.len() <= 8 {
                0x90
            } else if self.calls.len() == 14 {
                136
            } else {
                1
            };
            vm.bus_mut()
                .ram_mut()
                .map(0x0700, &[0xA9, 0xC7, 0xA0, status, 0x60])
                .unwrap();
            vm.set_pc(0x0700);
        }
        Ok(())
    }
}
fn c_image(stem: &str) -> CompilerVm {
    let mut vm = CompilerVm::default();
    vm.prepare_execution_profile(ExecutionProfile::StandaloneObject)
        .unwrap();
    vm.reset_cpu();
    vm.prepare_headless_program_environment();
    vm.load_atari_object(&std::fs::read(artifact(&format!("{stem}.xex"))).unwrap())
        .unwrap();
    let entry = vm.bus().ram().read_word(0x2E0);
    vm.set_pc(entry);
    vm
}
#[test]
fn c_platform_recovers_nested_calls_and_preserves_machine_algorithms() {
    for profile in ["Os", "O3"] {
        for length in [0u8, 1, 127, 128, 255] {
            let mut vm = c_image(&format!("platform-{profile}"));
            let data: Vec<_> = (0..256)
                .map(|n| (n as u8).wrapping_mul(37).wrapping_add(3))
                .collect();
            vm.bus_mut().ram_mut().map(0x10FE, &data).unwrap();
            vm.bus_mut()
                .ram_mut()
                .map(0x12FF, &vec![0xCC; 258])
                .unwrap();
            vm.bus_mut()
                .ram_mut()
                .map(0x1400, &[4, b'D', b':', b'A', b'B'])
                .unwrap();
            vm.bus_mut().ram_mut().write(0x0610, length);
            vm.bus_mut().ram_mut().write_word(0x0620, 0x0760);
            vm.bus_mut().ram_mut().write(0x060E, 0);
            vm.bus_mut().ram_mut().write_word(0xE424, 0x073F);
            vm.bus_mut()
                .ram_mut()
                .map(0x0740, &[0xA9, 0x6B, 0x60])
                .unwrap();
            let mut hooks = CioProbe::default();
            let result = VmRunner::new(vm)
                .run_with_hooks(
                    RunRequest {
                        max_steps: 100000,
                        stop_after_pc: Some(0x0760),
                        history_len: 16,
                    },
                    &mut hooks,
                )
                .unwrap();
            assert_eq!(
                result.stop_reason(),
                StopReason::PcReached { pc: 0x0760 },
                "{:?}",
                result.report
            );
            let vm = result.into_vm();
            let ram = vm.bus().ram();
            assert_eq!(ram.read(0x600), 0xA5);
            assert_eq!(ram.read(0x601), 8);
            assert_eq!(ram.read(0x60E), 0, "abandoned C frame resumed");
            assert_eq!(ram.read(0x618), ram.read(0x61B), "CPU stack drift");
            assert_eq!(
                ram.read_word(0x619),
                ram.read_word(0x61C),
                "software stack drift"
            );
            assert_eq!(ram.read_word(0x602), 0xFE01);
            assert_eq!(ram.read_word(0x604), 0x1234);
            assert_eq!(ram.read_word(0x606), 0x2000);
            for n in 0u16..256 {
                let ch = n as u8;
                assert_eq!(
                    ram.read(0x800 + n),
                    (ch & 0x9F) | [0x40, 0, 0x20, 0x60][usize::from((ch >> 5) & 3)]
                );
                assert_eq!(
                    ram.read(0x900 + n),
                    (ch & 0x9F) | [0x20, 0x40, 0, 0x60][usize::from((ch >> 5) & 3)]
                );
            }
            let count = if length == 0 {
                256
            } else {
                usize::from(length)
            };
            assert_eq!(
                (0..count)
                    .map(|i| ram.read(0x1300 + i as u16))
                    .collect::<Vec<_>>(),
                data[..count]
            );
            assert_eq!(ram.read(0x12FF), 0xCC);
            assert_eq!(
                ram.read(0x1300 + count as u16),
                if count == 256 { 4 } else { 0xCC }
            );
            assert_eq!(ram.read(0x608), 0xC7);
            assert_eq!(ram.read(0x609), 136);
            assert_eq!(ram.read_word(0x60A), 0x7178);
            assert_eq!(ram.read(0x60C), 4);
            assert_eq!(ram.read(0x60D), 7);
            assert_eq!(ram.read(0x60F), 0x6B);
            assert_eq!(ram.read(0x02FC), 0xFF);
            assert_eq!(hooks.calls.len(), 16);
            assert!(
                hooks.calls[..8]
                    .iter()
                    .all(|(idx, b)| *idx == 0x10 && b[2] == 12)
            );
            let tail = &hooks.calls[8..];
            assert_eq!(
                tail.iter().map(|(_, b)| b[2]).collect::<Vec<_>>(),
                [3, 5, 7, 11, 33, 7, 11, 12]
            );
            assert!(tail[..5].iter().all(|(idx, _)| *idx == 0x20));
            assert!(tail[5..].iter().all(|(idx, _)| *idx == 0));
            for (n, expected_len) in [(1, 19), (2, 0x123), (3, 0x123)] {
                assert_eq!(
                    u16::from_le_bytes([tail[n].1[8], tail[n].1[9]]),
                    expected_len
                );
            }
            let name = u16::from_le_bytes([tail[0].1[4], tail[0].1[5]]);
            assert_eq!(
                (0..5).map(|i| ram.read(name + i)).collect::<Vec<_>>(),
                [b'D', b':', b'A', b'B', 155]
            );
        }
    }
}

#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[path = "support/tn_symbols.rs"]
mod symbols;
use symbols::{global_address, routine_address};
#[allow(dead_code)]
#[path = "support/tn_directory.rs"]
mod directory;
struct NoHooks;
impl actionc_vm::VmRunHooks for NoHooks {
    type Error = String;
}
#[derive(Default)]
struct KernelTime {
    entry: u16,
    start: Option<u64>,
    return_pc: u16,
    cycles: Option<u64>,
}
impl actionc_vm::VmRunHooks for KernelTime {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let pc = vm.cpu().registers().pc;
        if self.start.is_none() && pc == self.entry {
            self.start = Some(vm.cpu().cycles());
            let stack = 0x100 + u16::from(vm.cpu().registers().sp);
            self.return_pc = vm.bus().ram().read_word(stack + 1).wrapping_add(1);
        } else if self.cycles.is_none() && pc == self.return_pc {
            if let Some(start) = self.start {
                self.cycles = Some(vm.cpu().cycles() - start);
            }
        }
        Ok(())
    }
}
fn metric(language: &str, name: &str, left: &[u8], right: &[u8], arg: u8, cycles: Option<u64>) {
    if std::env::var_os("TN_C_METRICS").is_some() {
        let mut hash = 0xcbf29ce484222325u64;
        for b in name
            .bytes()
            .chain(left.iter().copied())
            .chain(right.iter().copied())
            .chain([arg])
        {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        println!(
            "tn-c-kernel,{language},{name},{hash:016X},{}",
            cycles.expect("measured callee returned")
        );
    }
}
fn kernel(profile: &str, command: u8, left: &[u8], right: &[u8], arg: u8) -> CompilerVm {
    let stem = format!("kernels-{profile}");
    let mut vm = c_image(&stem);
    if !left.is_empty() {
        vm.bus_mut().ram_mut().map(0x6000, left).unwrap();
    }
    if !right.is_empty() {
        vm.bus_mut().ram_mut().map(0x6400, right).unwrap();
    }
    vm.bus_mut().ram_mut().write(0x600, command);
    vm.bus_mut().ram_mut().write_word(0x602, 0x6000);
    vm.bus_mut().ram_mut().write_word(0x604, 0x6400);
    vm.bus_mut().ram_mut().write(0x609, arg);
    let end = symbol(&stem, "kernel_done");
    let name = [
        "",
        "Strcpy",
        "Strcat",
        "Instr",
        "Fnamecmp",
        "Sort",
        "Convert",
        "SetWinConversion",
        "Print",
        "PrintFname",
    ][usize::from(command)];
    let mut timing = KernelTime {
        entry: symbol(&stem, name),
        ..Default::default()
    };
    let result = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: 500000,
                stop_after_pc: Some(end),
                history_len: 12,
            },
            &mut timing,
        )
        .unwrap();
    metric(profile, name, left, right, arg, timing.cycles);
    assert_eq!(
        result.stop_reason(),
        StopReason::PcReached { pc: end },
        "{profile}/{command}: {:?}",
        result.report
    );
    result.into_vm()
}
fn action_kernel(name: &str, left: &[u8], right: &[u8], arg: u8) -> CompilerVm {
    let listing = std::fs::read_to_string(artifact("reference/TN-standalone.asm")).unwrap();
    let image = std::fs::read(artifact("reference/TN-standalone.xex")).unwrap();
    let mut vm = CompilerVm::default();
    vm.load_atari_object_for_execution(ExecutionProfile::StandaloneObject, &image)
        .unwrap();
    if !left.is_empty() {
        vm.bus_mut().ram_mut().map(0x6000, left).unwrap();
    }
    if !right.is_empty() {
        vm.bus_mut().ram_mut().map(0x6400, right).unwrap();
    }
    if name == "Print" || name == "PrintFname" {
        let table = global_address(&listing, "quickmul");
        for row in 0..24u16 {
            vm.bus_mut().ram_mut().write(table + row, (row * 40) as u8);
            vm.bus_mut()
                .ram_mut()
                .write(table + row + 24, (row * 40 >> 8) as u8);
        }
        vm.bus_mut().ram_mut().write_word(0x58, 0x7800);
        vm.bus_mut().ram_mut().write(0xFF, 0);
        vm.bus_mut().ram_mut().write(0xFE, 0);
    }
    let args = if ["Strcpy", "Strcat", "Fnamecmp"].contains(&name) {
        vec![0, 0x60, 0, 0x64]
    } else if name == "Instr" {
        vec![0, 0x60, arg]
    } else {
        vec![0, 0x60]
    };
    let entry = routine_address(&listing, name);
    let mut timing = KernelTime {
        entry,
        ..Default::default()
    };
    let vm = machine::call(vm, &mut timing, entry, &args);
    metric("Action", name, left, right, arg, timing.cycles);
    vm
}
fn screen_byte(ch: u8) -> u8 {
    (ch & 0x9F) | [0x40, 0, 0x20, 0x60][usize::from((ch >> 5) & 3)]
}
#[test]
fn c_and_action_string_kernels_match_independent_buffers_and_results() {
    for profile in ["Os", "O3"] {
        for length in [0u8, 1, 127, 128, 254, 255] {
            let mut source: Vec<_> = (0..512)
                .map(|i| (i as u8).wrapping_mul(37).wrapping_add(3))
                .collect();
            source[0] = length;
            let target = vec![0xCC; 512];
            let mut expected = target.clone();
            expected[..usize::from(length) + 1].copy_from_slice(&source[..usize::from(length) + 1]);
            for vm in [
                kernel(profile, 1, &target, &source, 0),
                action_kernel("Strcpy", &target, &source, 0),
            ] {
                assert_eq!(machine::bytes(&vm, 0x6000, 512), expected);
                assert_eq!(machine::bytes(&vm, 0x6400, 512), source);
            }
            for found in [false, true] {
                let mut text = vec![0; 512];
                text[0] = length;
                text[1..usize::from(length) + 1].fill(b'A');
                if found && length != 0 {
                    text[usize::from(length)] = b'Z';
                }
                let expected = if found { length } else { 0 };
                let c = kernel(profile, 3, &text, &[], b'Z');
                let a = action_kernel("Instr", &text, &[], b'Z');
                assert_eq!(c.bus().ram().read(0x601), expected);
                assert_eq!(a.bus().ram().read(0xA0), expected);
                let final_index = if found && length != 0 {
                    length
                } else {
                    length.saturating_add(1)
                };
                assert_eq!(c.bus().ram().read(0x60A), final_index);
                assert_eq!(a.bus().ram().read(0xE0), final_index);
                assert_eq!(machine::bytes(&c, 0x6000, 512), text);
                assert_eq!(machine::bytes(&a, 0x6000, 512), text);
            }
        }
        for (n, m) in [(0u8, 1u8), (1, 0), (1, 1), (127, 128), (254, 1)] {
            let mut left = vec![0xCC; 600];
            let mut right = vec![0x55; 600];
            left[0] = n;
            right[0] = m;
            let count = if m == 0 { 256 } else { usize::from(m) };
            let start = usize::from(n) + 1;
            let mut expected = left.clone();
            expected[start..start + count].copy_from_slice(&right[1..1 + count]);
            expected[0] = n.wrapping_add(m);
            for vm in [
                kernel(profile, 2, &left, &right, 0),
                action_kernel("Strcat", &left, &right, 0),
            ] {
                assert_eq!(machine::bytes(&vm, 0x6000, 600), expected);
                assert_eq!(machine::bytes(&vm, 0x6400, 600), right);
            }
        }
        for (marker_l, marker_r, at, l, r, expected) in [
            (b':', 32, 2, 65, 66, 0),
            (32, b':', 2, 65, 66, 1),
            (32, 32, 2, 65, 66, 0),
            (32, 32, 128, 90, 65, 1),
            (32, 32, 254, 0, 255, 0),
        ] {
            let mut left = vec![b'A'; 512];
            let mut right = left.clone();
            left[1] = marker_l;
            right[1] = marker_r;
            left[at] = l;
            right[at] = r;
            let c = kernel(profile, 4, &left, &right, 0);
            let a = action_kernel("Fnamecmp", &left, &right, 0);
            assert_eq!(c.bus().ram().read(0x601), expected);
            assert_eq!(a.bus().ram().read(0xA0), expected);
            assert_eq!(machine::bytes(&c, 0x6000, 512), left);
            assert_eq!(machine::bytes(&a, 0x6400, 512), right);
        }
        for count in [0usize, 1, 17, 64] {
            let mut files = directory::files(count);
            files.reverse();
            let mut rows = vec![0; count * 18];
            for (i, file) in files.iter().enumerate() {
                rows[i * 18..(i + 1) * 18].copy_from_slice(&file.input()[..18]);
            }
            let mut sorted = files.clone();
            sorted.sort_by_key(|f| (!f.directory, f.name.clone(), f.extension.clone()));
            let listing = std::fs::read_to_string(artifact("reference/TN-standalone.asm")).unwrap();
            for language in ["C", "Action"] {
                let mut vm = if language == "C" {
                    c_image(&format!("kernels-{profile}"))
                } else {
                    let mut vm = CompilerVm::default();
                    vm.load_atari_object_for_execution(
                        ExecutionProfile::StandaloneObject,
                        &std::fs::read(artifact("reference/TN-standalone.xex")).unwrap(),
                    )
                    .unwrap();
                    vm
                };
                if !rows.is_empty() {
                    vm.bus_mut().ram_mut().map(0x6000, &rows).unwrap();
                }
                for i in 0..count {
                    vm.bus_mut()
                        .ram_mut()
                        .write_word(0x7000 + i as u16 * 2, 0x6000 + i as u16 * 18);
                }
                if language == "C" {
                    vm.bus_mut().ram_mut().write(0x600, 5);
                    vm.bus_mut().ram_mut().write_word(0x606, 0x7000);
                    vm.bus_mut().ram_mut().write(0x608, count as u8);
                    let end = symbol(&format!("kernels-{profile}"), "kernel_done");
                    let run = VmRunner::new(vm).run(RunRequest {
                        max_steps: 1000000,
                        stop_after_pc: Some(end),
                        history_len: 8,
                    });
                    assert_eq!(run.stop_reason(), StopReason::PcReached { pc: end });
                    vm = run.into_vm();
                } else {
                    vm.bus_mut()
                        .ram_mut()
                        .write_word(global_address(&listing, "v"), 0x7000);
                    vm.bus_mut()
                        .ram_mut()
                        .write(global_address(&listing, "active"), count as u8);
                    vm = machine::call(vm, &mut NoHooks, routine_address(&listing, "Sort"), &[]);
                }
                for (i, file) in sorted.iter().enumerate() {
                    let addr = vm.bus().ram().read_word(0x7000 + i as u16 * 2);
                    assert_eq!(
                        machine::bytes(&vm, addr, 18),
                        file.input()[..18],
                        "{language}/{profile}/{count}"
                    );
                }
            }
        }
        for file in directory::files(64) {
            let row = file.row(false);
            let c = kernel(profile, 6, &row, &[], 0);
            let a = action_kernel("Convert", &row, &[], 0);
            let expected = file.filename();
            let length = expected.len();
            assert_eq!(c.bus().ram().read(0xA00), length as u8);
            assert_eq!(machine::bytes(&c, 0xA01, length), expected);
            let listing = std::fs::read_to_string(artifact("reference/TN-standalone.asm")).unwrap();
            let fname = global_address(&listing, "fname");
            assert_eq!(c.bus().ram().read(0x60A), 12 + file.extension.len() as u8);
            assert_eq!(c.bus().ram().read(0x60B), length as u8 + 1);
            assert_eq!(a.bus().ram().read(0xE0), 12 + file.extension.len() as u8);
            assert_eq!(a.bus().ram().read(0xE1), length as u8 + 1);
            assert_eq!(machine::bytes(&a, fname + 1, length), expected);
            assert_eq!(machine::bytes(&c, 0x6000, 18), row);
            assert_eq!(machine::bytes(&a, 0x6000, 18), row);
        }
        for command in [8u8, 9] {
            let text = [8, b'D', b':', b'A', b'B', 0x80, 0x9B, 0xFF, b'Z'];
            let first = if command == 8 { 1 } else { 3 };
            let expected: Vec<_> = text[first..].iter().copied().map(screen_byte).collect();
            for vm in [
                kernel(profile, command, &text, &[], 0),
                action_kernel(
                    if command == 8 { "Print" } else { "PrintFname" },
                    &text,
                    &[],
                    0,
                ),
            ] {
                assert_eq!(machine::bytes(&vm, 0x7800, expected.len()), expected);
                assert_eq!(vm.bus().ram().read(0xFF), expected.len() as u8);
            }
        }
        for seed in [0u8, 1, 127, 128, 254, 255] {
            let row: Vec<_> = (0..32)
                .map(|n| (n as u8).wrapping_mul(37).wrapping_add(seed))
                .collect();
            let mut expected = row.clone();
            expected[1] = screen_byte(if row[1] == 32 { row[0] } else { row[1] });
            expected[0] = 0;
            for byte in &mut expected[2..10] {
                *byte = screen_byte(*byte);
            }
            let c = kernel(profile, 7, &row, &[], 0);
            assert_eq!(machine::bytes(&c, 0x6000, 32), expected);
            assert_eq!(c.bus().ram().read(0x60A), 10);
        }
    }
}
