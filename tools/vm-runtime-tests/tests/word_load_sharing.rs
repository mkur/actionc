use actionc::{
    codegen::{self, CodegenOutput, CodegenSymbolScope},
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, CompilerVm, RunRequest, StopReason, VmRunHooks, VmRunner};
use std::collections::BTreeSet;

#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

fn compile(
    text: &str,
    optimized: bool,
    runtime: Runtime,
    origin: u16,
    enabled: bool,
) -> (mir6502::MirProgram, CodegenOutput) {
    // Use real source files so CRLF goes through the loader and parser.
    let source = support::Source::new(text);
    let loaded = actionc::includes::load_program_with_expanded_source(&source.0).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
        &loaded.program,
        &model,
    ));
    actionc::nir::verify_program(&raw).unwrap();
    let nir = if optimized {
        actionc::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    let config = Mir6502Config {
        enable_word_load_sharing: enabled,
        ..Mir6502Config::optimized()
    };
    let p = mir6502::materialize_program_with_origin_and_runtime(
        mir6502::lower_program(&nir).unwrap(),
        &config,
        origin,
        runtime,
    )
    .unwrap();
    mir6502::verify_program(&p, mir6502::MirPhase::PreEmission).unwrap();
    let output =
        mir6502::generate_output_with_config_and_runtime(&nir, origin, &config, runtime).unwrap();
    (p, output)
}

fn home(out: &CodegenOutput, owner: Option<&str>, name: &str) -> u16 {
    out.map
        .storage_symbols
        .iter()
        .find(|s| {
            s.name.eq_ignore_ascii_case(name)
                && match (&s.scope, owner) {
                    (CodegenSymbolScope::Global, None) => true,
                    (CodegenSymbolScope::Routine(r), Some(owner)) => r.eq_ignore_ascii_case(owner),
                    _ => false,
                }
        })
        .unwrap_or_else(|| panic!("missing {owner:?}/{name}"))
        .address
}

fn access(op: u8, address: u16) -> Vec<u8> {
    if address < 256 {
        vec![if op == 0xAD { 0xA5 } else { 0x85 }, address as u8]
    } else {
        vec![op, address as u8, (address >> 8) as u8]
    }
}

fn fragment(
    out: &CodegenOutput,
    name: &str,
    source: u16,
    first: u16,
    second: u16,
    enabled: bool,
) -> (u16, u16) {
    let original = [
        access(0xAD, source),
        access(0x8D, first),
        access(0xAD, source + 1),
        access(0x8D, first + 1),
        access(0xAD, source),
        access(0x8D, second),
        access(0xAD, source + 1),
        access(0x8D, second + 1),
    ];
    let bytes = if enabled {
        [0, 1, 5, 2, 3, 7]
            .into_iter()
            .flat_map(|i| original[i].clone())
            .collect::<Vec<_>>()
    } else {
        original.concat()
    };
    let routine = out
        .map
        .routine_ranges
        .iter()
        .find(|r| r.name == name)
        .unwrap();
    let start = usize::from(routine.start - out.origin);
    let end = usize::from(routine.end - out.origin);
    let matches = out.bytes[start..end]
        .windows(bytes.len())
        .enumerate()
        .filter_map(|(i, b)| (b == bytes).then_some(start + i))
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "{name}: expected exact emitted schedule enabled={enabled}"
    );
    let address = out.origin + matches[0] as u16;
    (address, address + bytes.len() as u16)
}

#[derive(Default)]
struct Entry {
    address: u16,
    status: Option<u8>,
    cycles: u64,
}
impl VmRunHooks for Entry {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        if vm.cpu().registers().pc == self.address {
            self.status = Some(vm.cpu().registers().status);
            self.cycles = vm.cpu().cycles();
            vm.bus_mut().clear_events();
        }
        Ok(())
    }
}

#[test]
fn word_load_sharing_fragments_preserve_memory_registers_flags_and_read_counts() {
    let mut executions = 0;
    for sources in [false, true] {
        let lf = if sources {
            include_str!("../../../fixtures/runtime/word_load_sharing_sources.act")
        } else {
            include_str!("../../../fixtures/runtime/word_load_sharing.act")
        }
        .replace("\r\n", "\n");
        for crlf in [false, true] {
            let text = if crlf {
                lf.replace('\n', "\r\n")
            } else {
                lf.clone()
            };
            for optimized in [false, true] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    for origin in [0x3000, 0x41C7] {
                        let (baseline, old) = compile(&text, optimized, runtime, origin, false);
                        for enabled in [false, true] {
                            let (selected, out) = if enabled {
                                compile(&text, optimized, runtime, origin, true)
                            } else {
                                (baseline.clone(), old.clone())
                            };
                            for (a, b) in baseline.routines.iter().zip(&selected.routines) {
                                assert_eq!(a.frame, b.frame);
                                if matches!(a.name.as_str(), "Volatile" | "Absolute") {
                                    assert_eq!(a, b, "unsafe storage changed");
                                }
                            }
                            let image = codegen::format_load_file(&out);
                            let (mut vm, profile) = support::vm_for(runtime);
                            vm.load_atari_object_for_execution(profile, &image).unwrap();
                            let names = if sources {
                                ["Parameter", "Local"]
                            } else {
                                ["Unlink", "Reset"]
                            };
                            for name in names {
                                if name == "Reset" && !optimized {
                                    // Raw NIR retains the separate pointer
                                    // cast/load; it has no eight-op copy pair.
                                    assert_eq!(
                                        baseline.routines.iter().find(|r| r.name == name),
                                        selected.routines.iter().find(|r| r.name == name)
                                    );
                                    continue;
                                }
                                let source = home(
                                    &out,
                                    if sources { Some(name) } else { None },
                                    if sources {
                                        "head"
                                    } else if name == "Reset" {
                                        "startAddress"
                                    } else {
                                        "anchor"
                                    },
                                );
                                let (first, second) = if name == "Reset" {
                                    let entry = out
                                        .routine_addresses
                                        .iter()
                                        .find(|r| r.name == name)
                                        .unwrap()
                                        .address;
                                    let i = usize::from(entry - out.origin);
                                    assert_eq!(
                                        out.bytes[i + 3],
                                        0x85,
                                        "first private capture must be zero page"
                                    );
                                    (u16::from(out.bytes[i + 4]), home(&out, Some(name), "p"))
                                } else {
                                    (home(&out, Some(name), "previous"), 0xAC)
                                };
                                let (entry, end) =
                                    fragment(&out, name, source, first, second, enabled);
                                // stop_after_pc executes the instruction at
                                // that address. Stop after the last store,
                                // before the following indirect-load setup.
                                let stop = end - access(0x8D, second + 1).len() as u16;
                                let core =
                                    [source, source + 1, first, first + 1, second, second + 1]
                                        .into_iter()
                                        .collect::<BTreeSet<_>>();
                                assert_eq!(core.len(), 6);
                                let guards = [source, first, second]
                                    .into_iter()
                                    .flat_map(|a| [a - 1, a + 2])
                                    .filter(|a| !core.contains(a))
                                    .collect::<BTreeSet<_>>();
                                for &address in core.iter().chain(&guards) {
                                    vm.bus_mut().add_watchpoint(address);
                                }
                                for value in [0u16, 1, 0xFF, 0x100, 0x7FFF, 0x8000, 0xFFFF] {
                                    for status in [0u8, 1, 0x40, 0x49, 0x0C, 0x4D] {
                                        let [lo, hi] = value.to_le_bytes();
                                        let x = lo ^ 0x5A;
                                        let y = hi ^ 0xA5;
                                        let sp = if lo & 1 == 0 { 0xFD } else { 0xD3 };
                                        let ram = vm.bus_mut().ram_mut();
                                        for &address in &guards {
                                            ram.write(address, 0xD6);
                                        }
                                        ram.write_word(source, value);
                                        ram.write_word(first, 0xA537);
                                        ram.write_word(second, 0xC619);
                                        ram.write(0x68F, status & 0x40);
                                        ram.map(
                                            0x800,
                                            &[
                                                0xA2,
                                                sp,
                                                0x9A,
                                                0xA2,
                                                x,
                                                0xA0,
                                                y,
                                                0xA9,
                                                0x37,
                                                0x2C,
                                                0x8F,
                                                0x06,
                                                if status & 8 != 0 { 0xF8 } else { 0xD8 },
                                                if status & 4 != 0 { 0x78 } else { 0x58 },
                                                if status & 1 != 0 { 0x38 } else { 0x18 },
                                                0x4C,
                                                entry as u8,
                                                (entry >> 8) as u8,
                                            ],
                                        )
                                        .unwrap();
                                        vm.set_pc(0x800);
                                        let mut hooks = Entry {
                                            address: entry,
                                            ..Entry::default()
                                        };
                                        let result = VmRunner::new(vm)
                                            .run_with_hooks(
                                                RunRequest {
                                                    max_steps: 200,
                                                    stop_after_pc: Some(stop),
                                                    history_len: 8,
                                                },
                                                &mut hooks,
                                            )
                                            .unwrap();
                                        assert_eq!(
                                            result.stop_reason(),
                                            StopReason::PcReached { pc: stop },
                                            "{name}: {:?}",
                                            result.report
                                        );
                                        let r = &result.report.registers;
                                        assert_eq!([r.a, r.x, r.y, r.sp], [hi, x, y, sp]);
                                        assert_eq!(
                                            r.status,
                                            (hooks.status.unwrap() & !0x82)
                                                | (hi & 0x80)
                                                | if hi == 0 { 2 } else { 0 }
                                        );
                                        vm = result.into_vm();
                                        assert_eq!(
                                            vm.cpu().cycles() - hooks.cycles,
                                            if enabled { 22 } else { 30 }
                                        );
                                        for address in [source, first, second] {
                                            assert_eq!(vm.bus().ram().read_word(address), value);
                                        }
                                        for &address in &guards {
                                            assert_eq!(vm.bus().ram().read(address), 0xD6);
                                        }
                                        let events = vm.bus().events();
                                        assert_eq!(
                                            events
                                                .iter()
                                                .filter(|e| e.access == BusAccess::Read
                                                    && (e.address == source
                                                        || e.address == source + 1))
                                                .map(|e| e.address)
                                                .collect::<Vec<_>>(),
                                            if enabled {
                                                vec![source, source + 1]
                                            } else {
                                                vec![source, source + 1, source, source + 1]
                                            }
                                        );
                                        for address in [first, second] {
                                            assert_eq!(
                                                events
                                                    .iter()
                                                    .filter(|e| e.access == BusAccess::Write
                                                        && (e.address == address
                                                            || e.address == address + 1))
                                                    .map(|e| (e.address, e.value))
                                                    .collect::<Vec<_>>(),
                                                [(address, lo), (address + 1, hi)]
                                            );
                                        }
                                        assert!(
                                            !events.iter().any(|e| e.access == BusAccess::Write
                                                && !core.contains(&e.address))
                                        );
                                        executions += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(executions, 4704);
    eprintln!("word load sharing: {executions} emitted fragment executions");
}

#[test]
fn word_load_sharing_neutral_program_runs_through_both_runtimes_and_nir_modes() {
    let lf = include_str!("../../../fixtures/runtime/word_load_sharing.act").replace("\r\n", "\n");
    for optimized in [false, true] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for enabled in [false, true] {
                let (_, out) = compile(&lf, optimized, runtime, 0x3000, enabled);
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &codegen::format_load_file(&out))
                    .unwrap();
                let result = VmRunner::new(vm).run(RunRequest {
                    max_steps: 3000,
                    history_len: 8,
                    ..RunRequest::default()
                });
                assert_eq!(
                    result.stop_reason(),
                    StopReason::StepLimit { max_steps: 3000 },
                    "{:?}",
                    result.report
                );
                assert_eq!(result.memory().read_word(0x600), 0x7000);
                assert_eq!(result.memory().read_word(0x7002), 0x7200);
                assert_eq!(result.memory().read_word(0x7200), 64);
                assert_eq!(result.memory().read_word(0x7202), 0);
            }
        }
    }
}

fn invoke(mut vm: CompilerVm, entry: u16, first: u16, second: u16) -> (CompilerVm, u64) {
    vm.bus_mut()
        .ram_mut()
        .map(
            0x700,
            &[
                0xA9,
                (second >> 8) as u8,
                0x85,
                0xA3,
                0xA0,
                second as u8,
                0xA9,
                first as u8,
                0xA2,
                (first >> 8) as u8,
                0x20,
                entry as u8,
                (entry >> 8) as u8,
                0x4C,
                0x20,
                0x07,
            ],
        )
        .unwrap();
    vm.set_pc(0x700);
    let started = vm.cpu().cycles();
    let result = VmRunner::new(vm).run(RunRequest {
        max_steps: 10000,
        stop_after_pc: Some(0x70D),
        history_len: 8,
    });
    assert_eq!(
        result.stop_reason(),
        StopReason::PcReached { pc: 0x70D },
        "{:?}",
        result.report
    );
    let vm = result.into_vm();
    let cycles = vm.cpu().cycles() - started - 20;
    (vm, cycles)
}

#[test]
fn word_load_sharing_fallback_executes_partial_aliases_against_a_byte_oracle() {
    for origin in [0x3000u16, 0x41C7] {
        for (first, second) in [
            (origin - 1, 0x680),
            (origin, 0x680),
            (origin + 1, 0x680),
            (0x680, origin - 1),
            (0x680, origin + 1),
            (0x680, 0x680),
            (0x680, 0x681),
            (0x681, 0x680),
        ] {
            let text = format!(
                "TYPE Pair=[BYTE lo,hi]\nPair source\nBYTE ARRAY padding(16)\nPair first=${first:04X},second=${second:04X}\nPROC Probe()\n ASM OPAQUE\n NOP\n ENDASM\n first.lo=source.lo first.hi=source.hi second.lo=source.lo second.hi=source.hi padding(7)=source.lo\nRETURN\nPROC Main() Probe() DO OD RETURN\n"
            );
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                for optimized in [false, true] {
                    let (_, baseline) = compile(&text, optimized, runtime, origin, false);
                    for enabled in [false, true] {
                        let out = if enabled {
                            compile(&text, optimized, runtime, origin, true).1
                        } else {
                            baseline.clone()
                        };
                        assert_eq!(
                            out.bytes, baseline.bytes,
                            "aliased absolute stores must retain fallback"
                        );
                        let source = home(&out, None, "source");
                        let pad = home(&out, None, "padding");
                        assert_eq!(source, origin);
                        assert!(pad >= source + 2);
                        let entry = out
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == "Probe")
                            .unwrap()
                            .address;
                        assert!(
                            entry > source + 4,
                            "alias fixtures must not write executing code"
                        );
                        for value in [0u16, 1, 0xFF, 0x100, 0x7FFF, 0x8000, 0xFFFF] {
                            let (mut vm, profile) = support::vm_for(runtime);
                            vm.load_atari_object_for_execution(
                                profile,
                                &codegen::format_load_file(&out),
                            )
                            .unwrap();
                            let addresses = (source - 1..=source + 4)
                                .chain(0x67F..=0x684)
                                .chain(pad..pad + 16)
                                .collect::<BTreeSet<_>>();
                            for &a in &addresses {
                                vm.bus_mut().ram_mut().write(a, 0x6B);
                            }
                            vm.bus_mut().ram_mut().write_word(source, value);
                            let mut expected = addresses
                                .iter()
                                .map(|&a| (a, vm.bus().ram().read(a)))
                                .collect::<std::collections::BTreeMap<_, _>>();
                            for (s, d) in [
                                (source, first),
                                (source + 1, first + 1),
                                (source, second),
                                (source + 1, second + 1),
                                (source, pad + 7),
                            ] {
                                let v = expected[&s];
                                expected.insert(d, v);
                            }
                            let (vm, _) = invoke(vm, entry, 0, 0);
                            for (a, v) in expected {
                                assert_eq!(
                                    vm.bus().ram().read(a),
                                    v,
                                    "{first:04X}/{second:04X}/{value:04X}/{a:04X}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn word_load_sharing_keeps_volatile_and_absolute_source_reads() {
    let text = include_str!("../../../fixtures/runtime/word_load_sharing_sources.act")
        .replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            for enabled in [false, true] {
                let (_, out) = compile(&text, optimized, runtime, 0x3000, enabled);
                for (name, source) in [
                    ("Volatile", home(&out, None, "hotAddress")),
                    ("Absolute", 0x698),
                ] {
                    let entry = out
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address;
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &codegen::format_load_file(&out))
                        .unwrap();
                    vm.bus_mut().ram_mut().write_word(source, 0x7000);
                    vm.bus_mut().ram_mut().write_word(0x7002, 0);
                    vm.bus_mut().add_watchpoint(source);
                    vm.bus_mut().add_watchpoint(source + 1);
                    vm.bus_mut().clear_events();
                    let (vm, _) = invoke(vm, entry, 0, 0);
                    assert_eq!(vm.bus().ram().read_word(0xA0), 0x7000);
                    let reads = vm
                        .bus()
                        .events()
                        .iter()
                        .filter(|e| {
                            e.access == BusAccess::Read
                                && (source..=source + 1).contains(&e.address)
                        })
                        .map(|e| e.address)
                        .collect::<Vec<_>>();
                    assert_eq!(reads, [source, source + 1, source, source + 1]);
                }
            }
        }
    }
}

#[test]
fn word_load_sharing_later_indirect_reads_can_alias_completed_copies() {
    let text =
        include_str!("../../../fixtures/runtime/word_load_sharing.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            for enabled in [false, true] {
                let (_, out) = compile(&text, optimized, runtime, 0x3000, enabled);
                let source = home(&out, None, "anchor");
                let previous = home(&out, Some("Unlink"), "previous");
                let entry = out
                    .routine_addresses
                    .iter()
                    .find(|r| r.name == "Unlink")
                    .unwrap()
                    .address;
                for field in [source, previous, 0xAC] {
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &codegen::format_load_file(&out))
                        .unwrap();
                    let pointer = field - 2;
                    vm.bus_mut().ram_mut().write_word(source, pointer);
                    let (vm, _) = invoke(vm, entry, 0, 0);
                    assert_eq!(vm.bus().ram().read_word(0xA0), pointer);
                    assert_eq!(vm.bus().ram().read_word(source), pointer);
                    assert_eq!(vm.bus().ram().read_word(previous), pointer);
                }
            }
        }
    }
}

#[test]
fn word_load_sharing_real_allocate_cases_preserve_results_and_measured_cycles() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runtime/allocate_word_load_sharing.act");
    let loaded = actionc::includes::load_program_with_expanded_source(&path).unwrap();
    let text = loaded.source.replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for enabled in [false, true] {
            let (_, out) = compile(&text, true, runtime, 0x3000, enabled);
            let entry = |name: &str| {
                out.routine_addresses
                    .iter()
                    .find(|r| r.name == name)
                    .unwrap()
                    .address
            };
            let fresh = || {
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &codegen::format_load_file(&out))
                    .unwrap();
                vm.bus_mut()
                    .ram_mut()
                    .write_word(home(&out, None, "FreeList"), 0x6000);
                vm
            };
            for (name, size, amount, expected, old_cycles, new_cycles) in [
                ("empty", 0u16, 4u16, 0u16, 95, 87),
                ("exact", 0x20, 0x20, 0x7000, 241, 233),
                ("split", 0x40, 0x10, 0x7030, 318, 310),
                ("boundary", 0xFF, 0x100, 0x7408, 407, 399),
            ] {
                let mut vm = fresh();
                let ram = vm.bus_mut().ram_mut();
                ram.write_word(0x6002, if name == "empty" { 0 } else { 0x7000 });
                ram.write_word(0x7000, size);
                ram.write_word(0x7002, if name == "boundary" { 0x7400 } else { 0 });
                ram.write_word(0x7400, 0x108);
                ram.write_word(0x7402, 0);
                let (vm, cycles) = invoke(vm, entry("Alloc"), amount, 0);
                assert_eq!(vm.bus().ram().read_word(0xA0), expected);
                if runtime == Runtime::ActionCart {
                    assert_eq!(
                        cycles,
                        if enabled { new_cycles } else { old_cycles },
                        "{name}"
                    );
                }
            }
            for (name, left, target, right, old_cycles, new_cycles) in [
                ("insert", 0x7000u16, 0x7080u16, 0x7100u16, 407, 391),
                ("left", 0x7000, 0x7020, 0, 345, 329),
                ("right", 0, 0x7010, 0x7020, 474, 466),
                ("both", 0x7000, 0x7020, 0x7030, 521, 505),
            ] {
                let mut vm = fresh();
                let ram = vm.bus_mut().ram_mut();
                ram.write_word(0x6000, 0);
                ram.write_word(0x6002, if left == 0 { right } else { left });
                if left != 0 {
                    ram.write_word(left, 0x20);
                    ram.write_word(left + 2, right);
                }
                if right != 0 {
                    ram.write_word(right, 0x20);
                    ram.write_word(right + 2, 0);
                }
                let (vm, cycles) = invoke(vm, entry("Free"), target, 0x10);
                if runtime == Runtime::ActionCart {
                    assert_eq!(
                        cycles,
                        if enabled { new_cycles } else { old_cycles },
                        "{name}"
                    );
                }
                let ram = vm.bus().ram();
                match name {
                    "insert" => {
                        assert_eq!(ram.read_word(left + 2), target);
                        assert_eq!(ram.read_word(target), 0x10);
                        assert_eq!(ram.read_word(target + 2), right);
                    }
                    "left" => {
                        assert_eq!(ram.read_word(left), 0x30);
                        assert_eq!(ram.read_word(left + 2), 0);
                    }
                    "right" => {
                        assert_eq!(ram.read_word(0x6002), target);
                        assert_eq!(ram.read_word(target), 0x30);
                        assert_eq!(ram.read_word(target + 2), 0);
                    }
                    "both" => {
                        assert_eq!(ram.read_word(left), 0x30);
                        assert_eq!(ram.read_word(left + 2), right);
                        assert_eq!(ram.read_word(right), 0x20);
                    }
                    _ => unreachable!(),
                }
                if name == "both" {
                    let (vm, _) = invoke(vm, entry("Alloc"), 0x50, 0);
                    assert_eq!(
                        vm.bus().ram().read_word(0xA0),
                        0,
                        "existing coalescing behavior changed"
                    );
                }
            }
            let mut vm = fresh();
            vm.bus_mut()
                .ram_mut()
                .write_word(home(&out, None, "EndProg"), 0x7000);
            vm.bus_mut().ram_mut().write_word(0x2E5, 0x7800);
            let (vm, cycles) = invoke(vm, entry("AllocInit"), 0, 0);
            assert_eq!(vm.bus().ram().read_word(0x6002), 0x7000);
            assert_eq!(vm.bus().ram().read_word(0x7000), 0x800);
            assert_eq!(vm.bus().ram().read_word(0x7002), 0);
            if runtime == Runtime::ActionCart {
                assert_eq!(cycles, if enabled { 155 } else { 163 });
            }
        }
    }
}
