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
