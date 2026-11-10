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
