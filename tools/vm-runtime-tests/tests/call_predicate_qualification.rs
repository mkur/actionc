//! Configuration comparisons, independent local sources and executed home traffic.
use actionc::compiler::Runtime;
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{CompilerVm, CpuStep, VmRunHooks};
use std::collections::BTreeMap;
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

fn compile(
    text: &str,
    config: &Mir6502Config,
    runtime: Runtime,
) -> actionc::codegen::CodegenOutput {
    let source = support::Source::new(text);
    let loaded = actionc::includes::load_program_with_expanded_source(&source.0).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(&sem)).unwrap();
    mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, config, runtime).unwrap()
}

struct Traffic {
    ranges: Vec<(u16, u16)>,
    homes: BTreeMap<u16, usize>,
    counts: [usize; 3],
    cycles: u64,
    pending: Option<(u64, Option<(u16, usize)>)>,
}
impl Traffic {
    fn new(output: &actionc::codegen::CodegenOutput, routine: &str) -> Self {
        let ranges = output
            .map
            .routine_ranges
            .iter()
            .filter(|r| r.name == routine)
            .map(|r| (r.start, r.end))
            .collect();
        let mut homes = BTreeMap::new();
        for s in &output.map.storage_symbols {
            if s.scope == actionc::codegen::CodegenSymbolScope::Routine(routine.to_owned()) {
                let kind = if s.name.starts_with("spill") {
                    0
                } else if s.kind == actionc::codegen::CodegenSymbolKind::Parameter {
                    1
                } else {
                    2
                };
                for n in 0..s.size {
                    homes.insert(s.address + n, kind);
                }
            }
        }
        Self {
            ranges,
            homes,
            counts: [0; 3],
            cycles: 0,
            pending: None,
        }
    }
}
impl VmRunHooks for Traffic {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if !self
            .ranges
            .iter()
            .any(|&(start, end)| (start..end).contains(&r.pc))
        {
            self.pending = None;
            return Ok(());
        }
        let ram = vm.bus().ram();
        let op = ram.read(r.pc);
        let arg = ram.read(r.pc + 1);
        let abs = ram.read_word(r.pc + 1);
        // Direct operand reads/writes only. Excludes instruction fetches,
        // indirect pointer fetches, stack and fixed runtime/ABI scratch.
        let operand = if op & 3 == 1 {
            match op & 0x1C {
                0x04 => Some((u16::from(arg), 1)),
                0x0C => Some((abs, 1)),
                0x14 => Some((u16::from(arg.wrapping_add(r.x)), 1)),
                0x18 => Some((abs.wrapping_add(u16::from(r.y)), 1)),
                0x1C => Some((abs.wrapping_add(u16::from(r.x)), 1)),
                _ => None,
            }
        } else {
            match op {
                0xAC | 0xAE | 0x8C | 0x8E | 0xCC | 0xEC | 0x2C => Some((abs, 1)),
                0xA4 | 0xA6 | 0x84 | 0x86 | 0xC4 | 0xE4 | 0x24 => Some((u16::from(arg), 1)),
                0xBC => Some((abs.wrapping_add(u16::from(r.x)), 1)),
                0xBE => Some((abs.wrapping_add(u16::from(r.y)), 1)),
                0x0E | 0x2E | 0x4E | 0x6E | 0xCE | 0xEE => Some((abs, 2)),
                _ => None,
            }
        };
        self.pending = Some((vm.cpu().cycles(), operand));
        Ok(())
    }
    fn after_step(&mut self, vm: &CompilerVm, _: &CpuStep) -> Result<(), String> {
        if let Some((start, operand)) = self.pending.take() {
            self.cycles += vm.cpu().cycles() - start;
            if let Some((address, count)) = operand {
                if let Some(&kind) = self.homes.get(&address) {
                    self.counts[kind] += count;
                }
            }
        }
        Ok(())
    }
}

#[test]
fn enabled_disabled_and_conservative_calls_preserve_effectful_snapshots() {
    struct Hooks;
    impl VmRunHooks for Hooks {
        type Error = String;
    }
    let text =
        std::fs::read_to_string(root().join("surveys/tn/reproducers/call-predicate-snapshots.act"))
            .unwrap()
            .replace("\r\n", "\n");
    let on = Mir6502Config::optimized();
    let mut off = on.clone();
    off.enable_call_predicate_arguments = false;
    off.enable_call_predicate_abi = false;
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for crlf in [false, true] {
            let text = if crlf {
                text.replace('\n', "\r\n")
            } else {
                text.clone()
            };
            for config in [&on, &off, &Mir6502Config::default()] {
                let output = compile(&text, config, runtime);
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(
                    profile,
                    &actionc::codegen::format_load_file(&output),
                )
                .unwrap();
                for name in [
                    "AliasWord",
                    "AliasHigh",
                    "AliasLaterHigh",
                    "Ordered",
                    "Absolute",
                ] {
                    let entry = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address;
                    for setting in [0u8, 1, 2, 255] {
                        vm.bus_mut().ram_mut().map(0x0800, &[0; 10]).unwrap();
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x0808, &0xFEDCu16.to_le_bytes())
                            .unwrap();
                        vm.bus_mut().ram_mut().write(0x0804, setting);
                        let mut args = if name == "Absolute" {
                            vec![]
                        } else {
                            0x6001u16.to_le_bytes().to_vec()
                        };
                        if name != "Absolute" {
                            if name != "Ordered" {
                                args.extend(0xFEDCu16.to_le_bytes());
                            }
                            args.push(setting);
                        }
                        vm = machine::call(vm, &mut Hooks, entry, &args);
                        assert_eq!(vm.bus().ram().read_word(0x0800), 0x6001);
                        assert_eq!(
                            vm.bus().ram().read_word(0x0802),
                            if name == "AliasHigh" { 0x12DC } else { 0xFEDC },
                            "{runtime:?}/{crlf}/{name}"
                        );
                        assert_eq!(vm.bus().ram().read(0x0804), u8::from(setting != 0));
                        assert_eq!(vm.bus().ram().read(0xA0), u8::from(setting != 0));
                        assert_eq!(
                            vm.bus().ram().read(0x0805),
                            u8::from(matches!(name, "AliasWord" | "AliasLaterHigh" | "Ordered"))
                        );
                        assert_eq!(
                            vm.bus().ram().read(0x0806),
                            if name == "Ordered" { 12 } else { 0 }
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn enabled_disabled_and_conservative_predicates_preserve_generic_and_local_results() {
    let mut off = Mir6502Config::optimized();
    off.enable_small_leaf_inlining = false;
    off.enable_call_predicate_arguments = false;
    off.enable_call_predicate_abi = false;
    let mut captures = off.clone();
    captures.enable_call_predicate_arguments = true;
    let mut on = captures.clone();
    on.enable_call_predicate_abi = true;
    for (fixture, routines) in [
        (
            "captured-call-arguments",
            &["Forward", "PredicateFirst"][..],
        ),
        ("call-predicate-layouts", &["LocalForward", "OtherHome"][..]),
    ] {
        let text =
            std::fs::read_to_string(root().join(format!("surveys/tn/reproducers/{fixture}.act")))
                .unwrap()
                .replace("\r\n", "\n");
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let text = if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text.clone()
                };
                let baseline = compile(&text, &off, runtime);
                for (label, config) in [
                    ("off", &off),
                    ("captures", &captures),
                    ("on", &on),
                    ("conservative", &Mir6502Config::default()),
                ] {
                    let output = compile(&text, config, runtime);
                    if label != "conservative" {
                        assert!(output.bytes.len() <= baseline.bytes.len());
                    }
                    let image = actionc::codegen::format_load_file(&output);
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    for &name in routines {
                        let entry = output
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address;
                        for value in [0u16, 1, 255, 256, 65535] {
                            for setting in [0u8, 1, 2, 255] {
                                vm.bus_mut().ram_mut().map(0x6000, &[0xA5; 6]).unwrap();
                                let mut args = if name != "OtherHome" {
                                    0x6001u16.to_le_bytes().to_vec()
                                } else {
                                    vec![]
                                };
                                args.extend(value.to_le_bytes());
                                args.push(setting);
                                let mut traffic = Traffic::new(&output, name);
                                let start = vm.cpu().cycles();
                                vm = machine::call(vm, &mut traffic, entry, &args);
                                let expected = if name == "LocalForward" {
                                    value.wrapping_add(u16::from(setting != 0)).wrapping_add(1)
                                } else {
                                    u16::from(setting != 0)
                                };
                                let result = if name == "LocalForward" {
                                    vm.bus().ram().read_word(0xA0)
                                } else {
                                    u16::from(vm.bus().ram().read(0xA0))
                                };
                                assert_eq!(
                                    result, expected,
                                    "{runtime:?}/{label}/{crlf}/{name}/{value}/{setting}"
                                );
                                if name == "OtherHome" || name == "LocalForward" {
                                    assert_eq!(vm.bus().ram().read_word(0x0802), value);
                                    assert_eq!(vm.bus().ram().read(0x0804), u8::from(setting != 0));
                                }
                                let mut expected = [0xA5; 6];
                                if name != "OtherHome" {
                                    expected[1..3].copy_from_slice(
                                        &(if setting == 0 { 0 } else { value }).to_le_bytes(),
                                    );
                                }
                                assert_eq!(machine::bytes(&vm, 0x6000, 6), expected);
                                if !crlf && value == 65535 {
                                    println!(
                                        "call-flow,{runtime:?},{label},{name},{setting},{},{},{},{},{}",
                                        traffic.cycles,
                                        vm.cpu().cycles() - start,
                                        traffic.counts[0],
                                        traffic.counts[1],
                                        traffic.counts[2]
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
