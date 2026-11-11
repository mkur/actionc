//! Execute handwritten comparisons with the actual, unchanged TN MovePage.
use actionc::compiler::Runtime;
use actionc_vm::{BusAccess, CompilerVm, CpuStep, VmRunHooks};
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

#[derive(Debug, PartialEq, Eq)]
struct Control {
    name: String,
    runtime: String,
    variant: String,
    entry: u16,
    helper: u16,
    s_home: u16,
    t_home: u16,
    size: u16,
    patch: String,
    image: String,
    original: u16,
    patch_bytes: Vec<u8>,
}
fn controls(text: &str) -> Vec<Control> {
    text.lines()
        .map(|line| {
            let f = line.split('\t').collect::<Vec<_>>();
            assert_eq!(f.len(), 12);
            let hex = |i| u16::from_str_radix(f[i], 16).unwrap();
            Control {
                name: f[0].into(),
                runtime: f[1].into(),
                variant: f[2].into(),
                entry: hex(3),
                helper: hex(4),
                s_home: hex(5),
                t_home: hex(6),
                size: f[7].parse().unwrap(),
                patch: f[8].into(),
                image: f[9].into(),
                original: hex(10),
                patch_bytes: f[11]
                    .as_bytes()
                    .chunks_exact(2)
                    .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
                    .collect(),
            }
        })
        .collect()
}

struct Observe {
    entry: u16,
    size: u16,
    helper: u16,
    cycles: [u64; 3],
    previous_cycle: u64,
    arguments: Vec<[u8; 5]>,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.helper {
            self.arguments.push([
                r.a,
                r.x,
                r.y,
                vm.bus().ram().read(0xA3),
                vm.bus().ram().read(0xA4),
            ]);
        }
        Ok(())
    }
    fn after_step(&mut self, _vm: &CompilerVm, step: &CpuStep) -> Result<(), String> {
        let bucket = if (self.entry..self.entry + self.size).contains(&step.pc) {
            0
        } else if (self.helper..self.helper + 19).contains(&step.pc) {
            1
        } else {
            assert!(
                (machine::ENTRY..=machine::STOP).contains(&step.pc),
                "unexpected PC {:04X}",
                step.pc
            );
            2
        };
        self.cycles[bucket] += step.cycles - self.previous_cycle;
        self.previous_cycle = step.cycles;
        Ok(())
    }
}

const START: u16 = 0x6F00;
const LENGTH: usize = 0xC00;
fn seed(vm: &mut CompilerVm) -> Vec<u8> {
    let memory = (0..LENGTH).map(|i| 32 + (i % 96) as u8).collect::<Vec<_>>();
    vm.bus_mut().ram_mut().map(START, &memory).unwrap();
    vm.bus_mut().ram_mut().map(0xA0, &[0xCC; 18]).unwrap();
    vm.bus_mut().ram_mut().write(0xA4, 1);
    vm.bus_mut().ram_mut().write(0xA5, 0x55);
    memory
}
fn call(vm: CompilerVm, c: &Control, s: u16, t: u16) -> (CompilerVm, Observe) {
    let mut args = s.to_le_bytes().to_vec();
    args.extend(t.to_le_bytes());
    let mut hooks = Observe {
        entry: c.entry,
        size: c.size,
        helper: c.helper,
        cycles: [0; 3],
        previous_cycle: vm.cpu().cycles(),
        arguments: Vec::new(),
    };
    let started = vm.cpu().cycles();
    let vm = machine::call(vm, &mut hooks, c.entry, &args);
    assert_eq!(
        hooks.cycles.iter().sum::<u64>(),
        vm.cpu().cycles() - started
    );
    assert_eq!(hooks.cycles[2], 17);
    (vm, hooks)
}
fn load(c: &Control) -> CompilerVm {
    let runtime = match c.runtime.as_str() {
        "ActionCart" => Runtime::ActionCart,
        "Standalone" => Runtime::Standalone,
        _ => panic!(),
    };
    let (mut vm, profile) = support::vm_for(runtime);
    vm.load_atari_object_for_execution(profile, &std::fs::read(root().join(&c.image)).unwrap())
        .unwrap();
    // A callable fragment has no RUNAD. Map its sole assembler segment into
    // the loaded program without replacing the program's execution profile.
    let patch = std::fs::read(root().join(&c.patch)).unwrap();
    assert_eq!(patch, c.patch_bytes);
    assert_eq!(&patch[..2], &[0xFF, 0xFF]);
    assert_eq!(u16::from_le_bytes([patch[2], patch[3]]), c.entry);
    assert_eq!(
        u16::from_le_bytes([patch[4], patch[5]]) + 1,
        c.entry + c.size
    );
    assert_eq!(patch.len(), 6 + usize::from(c.size));
    vm.bus_mut().ram_mut().map(c.entry, &patch[6..]).unwrap();
    vm
}
fn signature(vm: &CompilerVm, c: &Control) -> (Vec<(u16, u8, bool)>, [u8; 5], Vec<(u16, u8)>) {
    let events = vm.bus().events();
    let buffers = events
        .iter()
        .filter(|e| (START..START + LENGTH as u16).contains(&e.address))
        .map(|e| (e.address, e.value, e.access == BusAccess::Write))
        .collect();
    let homes = events
        .iter()
        .filter(|e| {
            e.access == BusAccess::Write
                && ((c.s_home..c.s_home + 2).contains(&e.address)
                    || (c.t_home..c.t_home + 2).contains(&e.address))
        })
        .map(|e| (e.address, e.value))
        .collect();
    let r = vm.cpu().registers();
    (buffers, [r.a, r.x, r.y, r.status, r.sp], homes)
}
fn expected_status(a: u8, b: u8) -> u8 {
    let result = a.wrapping_add(b);
    u8::from(u16::from(a) + u16::from(b) > 255)
        | (result & 128)
        | (u8::from(result == 0) << 1)
        | (u8::from((!(a ^ b) & (a ^ result) & 128) != 0) << 6)
}

#[test]
#[ignore = "run tools/compare6502/tn/strcat_idiomatic.py --assemble-only first"]
fn idiomatic_strcat_matches_buffers_and_exposes_storage_assumptions() {
    let manifest =
        std::fs::read_to_string(root().join("build/strcat-idiomatic/manifest.tsv")).unwrap();
    let all = controls(&manifest);
    assert_eq!(
        all,
        controls(&manifest.replace("\r\n", "\n").replace('\n', "\r\n"))
    );
    assert_eq!(all.len(), 40);
    let boundary = vec![
        ("3+8", 0x7000u16, 0x7200u16, 3u8, 8u8),
        ("empty-destination", 0x7000, 0x7200, 0, 1),
        ("low-carry", 0x70FF, 0x72FF, 0, 1),
        ("both-pages", 0x70FF, 0x72FF, 3, 128),
        ("large-destination", 0x70FF, 0x72FF, 255, 255),
        ("large-source", 0x7000, 0x7200, 1, 255),
        ("length-wrap", 0x7000, 0x7200, 127, 129),
        ("zero-means-256", 0x7000, 0x7200, 3, 0),
        ("zero-page-boundary", 0x70FF, 0x72FF, 0, 0),
        ("self-1", 0x7000, 0x7000, 1, 1),
        ("self-127", 0x7000, 0x7000, 127, 127),
        ("self-128", 0x7000, 0x7000, 128, 128),
        ("self-zero", 0x7000, 0x7000, 0, 0),
        ("overlap-right", 0x7000, 0x7002, 4, 6),
        ("overlap-left", 0x7000, 0x7002, 0, 6),
        ("overwrite-source-length", 0x7000, 0x7004, 3, 8),
    ];
    let mut reference = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    let mut alias_states = BTreeMap::new();
    let mut scratch = BTreeMap::new();
    let mut output = String::new();
    let (mut normal_calls, mut alias_calls, mut private_counterexamples, mut scratch_calls) =
        (0, 0, 0, 0);
    for c in &all {
        let mut cases = boundary
            .iter()
            .enumerate()
            .map(|(i, (_, s, t, a, b))| (i, *s, *t, *a, *b))
            .collect::<Vec<_>>();
        // All destination lengths at the six low-byte boundaries, then every
        // source count including zero/256. Concentrate the exhaustive axis in
        // one runtime/origin; boundaries cover the other seven contexts.
        if c.name == "TN" && c.runtime == "ActionCart" && c.entry == 0x0800 {
            for low in [0u16, 1, 127, 128, 254, 255] {
                for first in 0..=255u8 {
                    cases.push((cases.len(), 0x7000 + low, 0x7300, first, 1));
                }
            }
            for second in 0..=255u8 {
                cases.push((cases.len(), 0x70FF, 0x73FF, 3, second));
            }
        }
        let mut vm = load(c);
        for (i, s, t, first, second) in cases {
            let mut expected = seed(&mut vm);
            let at = |p: u16| usize::from(p - START);
            expected[at(s)] = first;
            expected[at(t)] = second;
            vm.bus_mut().ram_mut().map(START, &expected).unwrap();
            let destination = s.wrapping_add(u16::from(expected[at(s)])).wrapping_add(1);
            let count = expected[at(t)];
            let size = if count == 0 { 256u16 } else { u16::from(count) };
            for n in (0..size).rev() {
                expected[at(destination + n)] = expected[at(t + 1 + n)];
            }
            let a = expected[at(s)];
            let b = expected[at(t)];
            expected[at(s)] = a.wrapping_add(b);
            vm.bus_mut().clear_events();
            let hooks;
            (vm, hooks) = call(vm, c, s, t);
            let label = format!("{}/{}/{:04X}/{}/{i}", c.name, c.runtime, c.entry, c.variant);
            assert_eq!(machine::bytes(&vm, START, LENGTH), expected, "{label}");
            let d = destination.to_le_bytes();
            let source = (t + 1).to_le_bytes();
            assert_eq!(
                hooks.arguments,
                [[d[0], d[1], source[0], source[1], count]],
                "{label}"
            );
            let state = signature(&vm, c);
            assert_eq!(state.1[0], a.wrapping_add(b), "{label}");
            assert_eq!(state.1[1], d[1], "{label}");
            assert_eq!(state.1[2], 0, "{label}");
            assert_eq!(state.1[3] & 0xC3, expected_status(a, b), "{label}");
            let key = (c.name.clone(), c.runtime.clone(), c.entry, i);
            if c.variant == "compiler" {
                reference.insert(key, state);
            } else {
                let before = &reference[&key];
                assert_eq!(state.0, before.0, "ordered buffer accesses: {label}");
                assert_eq!(state.1, before.1, "returned state: {label}");
                if c.variant != "private-pointers" {
                    assert_eq!(state.2, before.2, "parameter writes: {label}");
                }
            }
            if i < boundary.len() && ["3+8", "low-carry", "zero-means-256"].contains(&boundary[i].0)
            {
                output.push_str(&format!(
                    "{}\t{}\t{}\t{:04X}\t{}\t{}\t{}\t{}\n",
                    c.name,
                    c.runtime,
                    c.variant,
                    c.entry,
                    boundary[i].0,
                    hooks.cycles[0],
                    hooks.cycles[1],
                    hooks.cycles[2]
                ));
                if boundary[i].0 == "3+8" && c.variant == "compiler" {
                    assert_eq!(hooks.cycles.iter().sum::<u64>(), 355);
                }
            }
            normal_calls += 1;
        }
        for (field, home) in [("s", c.s_home), ("t", c.t_home)] {
            seed(&mut vm);
            let s = home - 0x100;
            vm.bus_mut().ram_mut().write(s, 255);
            vm.bus_mut().ram_mut().map(0x7200, &[2, 0, 0x73]).unwrap();
            vm.bus_mut().ram_mut().write(0x7300, 5);
            // Seed the same values even for the private control, so its alias
            // counterexample isolates post-call pointer retention.
            vm.bus_mut().ram_mut().write_word(c.s_home, s);
            vm.bus_mut().ram_mut().write_word(c.t_home, 0x7200);
            vm.bus_mut().clear_events();
            let hooks;
            (vm, hooks) = call(vm, c, s, 0x7200);
            assert_eq!(vm.bus().ram().read_word(home), 0x7300);
            let result = (vm.bus().ram().read(s), vm.bus().ram().read(0x7300));
            let key = (c.name.clone(), c.runtime.clone(), c.entry, field);
            if c.variant == "private-pointers" {
                assert_eq!(result, (1, 5));
                assert_ne!(result, aliases[&key]);
                private_counterexamples += 1;
            } else {
                assert_eq!(result, if field == "s" { (255, 7) } else { (4, 5) });
                let state = (
                    machine::bytes(&vm, s - 1, 0x104),
                    machine::bytes(&vm, START, LENGTH),
                    signature(&vm, c),
                    hooks.arguments,
                );
                if c.variant == "compiler" {
                    alias_states.insert(key.clone(), state);
                    aliases.insert(key, result);
                } else {
                    assert_eq!(result, aliases[&key]);
                    assert_eq!(
                        state, alias_states[&key],
                        "parameter alias {}/{field}",
                        c.variant
                    );
                }
                alias_calls += 1;
            }
        }
        if c.variant != "private-pointers" {
            // Observe the ABI destination/count cells and original-s scratch
            // through t. These cases distinguish direct capture from deleting
            // the required A0/A1 stores or changing source access order.
            for (s, t, expected_length) in
                [(0x7000, 0xA0, 7), (0x7000, 0xA4, 4), (0x7003, 0xAE, 177)]
            {
                seed(&mut vm);
                vm.bus_mut().ram_mut().write(s, 3);
                vm.bus_mut().clear_events();
                let hooks;
                (vm, hooks) = call(vm, c, s, t);
                assert_eq!(vm.bus().ram().read(s), expected_length);
                let state = (
                    machine::bytes(&vm, START, LENGTH),
                    signature(&vm, c),
                    hooks.arguments,
                );
                let key = (c.name.clone(), c.runtime.clone(), c.entry, t);
                if c.variant == "compiler" {
                    scratch.insert(key, state);
                } else {
                    assert_eq!(state, scratch[&key], "scratch alias {}/{t:04X}", c.variant);
                }
                scratch_calls += 1;
            }
        }
    }
    output.push_str(&format!("count\tnormal_calls\t{normal_calls}\ncount\tparameter_alias_calls\t{alias_calls}\ncount\tprivate_counterexamples\t{private_counterexamples}\ncount\tscratch_alias_calls\t{scratch_calls}\n"));
    std::fs::write(root().join("build/strcat-idiomatic/vm-results.tsv"), output).unwrap();
    std::fs::write(
        root().join("build/strcat-idiomatic/vm-manifest.tsv"),
        manifest,
    )
    .unwrap();
    eprintln!(
        "Strcat: {normal_calls} normal calls, {alias_calls} parameter aliases, {scratch_calls} scratch aliases, {private_counterexamples} private-pointer counterexamples"
    );
}
