//! Attribute executed costs to the frozen byte accounting and probe aliases.
use actionc::compiler::Runtime;
use actionc_vm::{CompilerVm, VmRunHooks};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

struct Profile {
    buckets: BTreeMap<u16, String>,
    move_page: u16,
    previous: Option<(String, u64)>,
    cycles: BTreeMap<String, u64>,
}
impl Profile {
    fn finish_step(&mut self, now: u64) {
        if let Some((bucket, before)) = self.previous.take() {
            *self.cycles.entry(bucket).or_default() += now - before;
        }
    }
}
impl VmRunHooks for Profile {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let now = vm.cpu().cycles();
        self.finish_step(now);
        let pc = vm.cpu().registers().pc;
        let bucket = if let Some(bucket) = self.buckets.get(&pc) {
            bucket.clone()
        } else if (self.move_page..self.move_page + 19).contains(&pc) {
            "MovePage".into()
        } else {
            assert!(
                (machine::ENTRY..=machine::STOP).contains(&pc),
                "unexpected PC {pc:04X}"
            );
            "wrapper".into()
        };
        self.previous = Some((bucket, now));
        Ok(())
    }
}
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

fn entries(text: &str) -> BTreeMap<String, u16> {
    text.lines()
        .map(|line| {
            let (name, address) = line.split_once('\t').unwrap();
            (name.into(), u16::from_str_radix(address, 16).unwrap())
        })
        .collect()
}

#[test]
#[ignore = "requires frozen argument-staging images and generated instruction roles"]
fn strcat_byte_accounting_matches_cycles_and_parameter_aliases() {
    let artifacts = root().join("build/argument-staging/measured");
    let text = std::fs::read_to_string(root().join("build/argument-staging/strcat-byte-roles.tsv"))
        .expect("run strcat_breakdown.py with --roles-output first");
    let parse_roles = |text: &str| {
        text.lines()
            .map(|line| {
                let (offset, bucket) = line.split_once('\t').unwrap();
                (u16::from_str_radix(offset, 16).unwrap(), bucket.to_string())
            })
            .collect::<Vec<_>>()
    };
    let roles = parse_roles(&text);
    assert_eq!(
        roles,
        parse_roles(&text.replace("\r\n", "\n").replace('\n', "\r\n"))
    );
    for name in ["TN", "TNDBG"] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for enabled in [false, true] {
                let stem = format!("{name}-{runtime:?}-{enabled}");
                let text =
                    std::fs::read_to_string(artifacts.join(format!("{stem}.entries"))).unwrap();
                let entries = entries(&text);
                assert_eq!(
                    entries,
                    self::entries(&text.replace("\r\n", "\n").replace('\n', "\r\n"))
                );
                let entry = entries["Strcat"];
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(
                    profile,
                    &std::fs::read(artifacts.join(format!("{stem}.xex"))).unwrap(),
                )
                .unwrap();
                let buckets = roles
                    .iter()
                    .map(|(offset, bucket)| (entry + offset, bucket.clone()))
                    .collect::<BTreeMap<_, _>>();
                for (s, first, second, computation, helper, total) in [
                    (0x7000u16, 3u8, 8u8, 77, 163, 370),
                    (0x70FF, 0, 1, 81, 37, 248),
                ] {
                    vm.bus_mut().ram_mut().write(s, first);
                    for i in 1..=u16::from(first) {
                        vm.bus_mut().ram_mut().write(s + i, b'A');
                    }
                    vm.bus_mut().ram_mut().write(0x7200, second);
                    let payload = (0..second).map(|i| b'0' + i).collect::<Vec<_>>();
                    vm.bus_mut().ram_mut().map(0x7201, &payload).unwrap();
                    let mut args = s.to_le_bytes().to_vec();
                    args.extend(0x7200u16.to_le_bytes());
                    let mut hook = Profile {
                        buckets: buckets.clone(),
                        move_page: entries["MovePage"],
                        previous: None,
                        cycles: BTreeMap::new(),
                    };
                    let started = vm.cpu().cycles();
                    vm = machine::call(vm, &mut hook, entry, &args);
                    hook.finish_step(vm.cpu().cycles());
                    let expected = [
                        ("entry_homing", 19),
                        ("pointer_setup", 52),
                        ("computation", computation),
                        ("outgoing_arguments", 30),
                        ("call_return", 12),
                        ("MovePage", helper),
                        ("wrapper", 17),
                    ]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect::<BTreeMap<_, _>>();
                    assert_eq!(hook.cycles, expected, "{stem}/{s:04X}");
                    assert_eq!(
                        hook.cycles.values().sum::<u64>(),
                        vm.cpu().cycles() - started
                    );
                    assert_eq!(vm.cpu().cycles() - started, total);
                    let mut expected = vec![b'A'; usize::from(first)];
                    expected.extend(payload);
                    assert_eq!(machine::counted(&vm, s), expected);
                    eprintln!(
                        "strcat-cost,{stem},s={s:04X},first={first},second={second},cycles={:?}",
                        hook.cycles
                    );
                }
                // Resolve the parameter homes from frozen LDA absolute operands,
                // including TNDBG/standalone relocation. Both reloads are real
                // semantic dependencies when the copy overwrites those homes.
                for (offset, changes_s) in [(95u16, true), (105, false)] {
                    assert_eq!(vm.bus().ram().read(entry + offset), 0xAD);
                    let home = vm.bus().ram().read_word(entry + offset + 1);
                    let s = home - 0x100;
                    vm.bus_mut().ram_mut().write(s, 255);
                    vm.bus_mut().ram_mut().map(0x7200, &[2, 0, 0x73]).unwrap();
                    vm.bus_mut().ram_mut().write(0x7300, 5);
                    let mut args = s.to_le_bytes().to_vec();
                    args.extend(0x7200u16.to_le_bytes());
                    vm = machine::call(vm, &mut Hooks, entry, &args);
                    assert_eq!(vm.bus().ram().read_word(home), 0x7300, "{stem}");
                    if changes_s {
                        assert_eq!(vm.bus().ram().read(0x7300), 7, "{stem}");
                        assert_eq!(vm.bus().ram().read(s), 255, "{stem}");
                    } else {
                        assert_eq!(vm.bus().ram().read(s), 4, "{stem}");
                        assert_eq!(vm.bus().ram().read(0x7300), 5, "{stem}");
                    }
                }
            }
        }
    }
}
