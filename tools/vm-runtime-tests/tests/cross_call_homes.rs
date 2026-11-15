//! Independent call-input, ordered read, scalar and repeated-invocation oracles.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

struct Observe {
    advance: u16,
    arguments: Vec<u16>,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.advance {
            self.arguments.push(u16::from_le_bytes([r.a, r.x]));
        }
        Ok(())
    }
}

#[test]
fn ordinary_calls_preserve_arguments_results_reads_and_persistent_locals() {
    let lf = include_str!("../../../fixtures/runtime/cross_call_homes.act").replace("\r\n", "\n");
    for optimize_nir in [false, true] {
        for enabled in [false, true] {
            for (runtime, origin) in [(Runtime::ActionCart, 0x3000), (Runtime::Standalone, 0x41c7)]
            {
                let mut reference = None;
                for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                    let file = support::Source::new(&source);
                    let loaded =
                        actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
                    let model = actionc::semantic::analyze_with_options(
                        &loaded.program,
                        actionc::semantic::SemanticOptions::modern(),
                    )
                    .unwrap();
                    let nir = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
                        &loaded.program,
                        &model,
                    ));
                    let nir = if optimize_nir {
                        actionc::nir::optimize_program(&nir).unwrap()
                    } else {
                        nir
                    };
                    let output = mir6502::generate_output_with_config_and_runtime(
                        &nir,
                        origin,
                        &Mir6502Config {
                            enable_small_leaf_inlining: false,
                            enable_cross_call_home_selection: enabled,
                            ..Mir6502Config::optimized()
                        },
                        runtime,
                    )
                    .unwrap();
                    let image = actionc::codegen::format_load_file(&output);
                    if let Some(old) = &reference {
                        assert_eq!(old, &image);
                    } else {
                        reference = Some(image.clone());
                    }
                    let entry = |name| {
                        output
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address
                    };
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    let mut observe = Observe {
                        advance: entry("AdvanceRecord"),
                        arguments: vec![],
                    };
                    for base in [0x7000u16, 0x70fe] {
                        let mut records = vec![base];
                        for length in [1u8, 254, 2] {
                            let p = *records.last().unwrap();
                            vm.bus_mut().ram_mut().write(p, length);
                            records.push(p.wrapping_add(u16::from(length) + 3));
                        }
                        vm.bus_mut().ram_mut().write(records[3], 0);
                        for &p in &records {
                            vm.bus_mut().add_watchpoint(p);
                        }
                        for start in [0usize, 3] {
                            for stop in start..=3 {
                                for name in ["FindRecord", "CountRecords"] {
                                    let end = if name == "CountRecords" { 3 } else { stop };
                                    let mut args = records[start].to_le_bytes().to_vec();
                                    args.extend(records[stop].to_le_bytes());
                                    observe.arguments.clear();
                                    let first = vm.bus().events().len();
                                    vm = machine::call(vm, &mut observe, entry(name), &args);
                                    assert_eq!(
                                        machine::bytes(&vm, 0xa0, 1),
                                        vec![(end - start) as u8],
                                        "{name}"
                                    );
                                    assert_eq!(observe.arguments, records[start..end]);
                                    let reads: Vec<_> = vm.bus().events()[first..]
                                        .iter()
                                        .filter(|e| {
                                            records.contains(&e.address)
                                                && e.access == BusAccess::Read
                                        })
                                        .map(|e| e.address)
                                        .collect();
                                    let expected: Vec<_> = records[start..end]
                                        .iter()
                                        .flat_map(|p| [*p, *p])
                                        .chain([records[end]])
                                        .collect();
                                    assert_eq!(reads, expected, "{name}");
                                }
                            }
                        }
                    }
                    for value in [0u16, 0x00ff, 0x1234, 0xffff] {
                        let keep = 0xa357u16;
                        let args: Vec<_> = value
                            .to_le_bytes()
                            .into_iter()
                            .chain(keep.to_le_bytes())
                            .collect();
                        vm = machine::call(vm, &mut observe, entry("KeepWord"), &args);
                        assert_eq!(
                            machine::bytes(&vm, 0xa0, 2),
                            value.wrapping_add(1).wrapping_add(keep).to_le_bytes()
                        );
                        for enabled in [0u8, 1] {
                            vm = machine::call(
                                vm,
                                &mut observe,
                                entry("Conditional"),
                                &[value as u8, enabled],
                            );
                            assert_eq!(
                                machine::bytes(&vm, 0xa0, 1),
                                vec![(value as u8).wrapping_add(enabled)]
                            );
                        }
                    }
                    let saved = output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| {
                            s.name == "saved"
                                && s.scope
                                    == actionc::codegen::CodegenSymbolScope::Routine(
                                        "Persistent".into(),
                                    )
                        })
                        .unwrap()
                        .address;
                    vm.bus_mut().ram_mut().write(saved, 31);
                    for expected in [32, 33, 34] {
                        vm = machine::call(vm, &mut observe, entry("Persistent"), &[0]);
                        assert_eq!(machine::bytes(&vm, 0xa0, 1), vec![expected]);
                    }
                }
            }
        }
    }
}

struct LocalObserve {
    byte_step: u16,
    word_step: u16,
    hop: u16,
    arguments: Vec<(&'static str, u16)>,
}
impl VmRunHooks for LocalObserve {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.byte_step {
            self.arguments.push(("byte", u16::from(r.a)));
        } else if r.pc == self.word_step {
            self.arguments
                .push(("word", u16::from_le_bytes([r.a, r.x])));
        } else if r.pc == self.hop {
            self.arguments.push(("hop", u16::from_le_bytes([r.a, r.x])));
        }
        Ok(())
    }
}

#[test]
fn initialized_word_and_pointer_locals_and_entry_values_execute_independently() {
    let lf = include_str!("../../../fixtures/runtime/cross_call_locals.act").replace("\r\n", "\n");
    for (runtime, origin) in [(Runtime::ActionCart, 0x3000), (Runtime::Standalone, 0x41c7)] {
        for optimize_nir in [false, true] {
            for enabled in [false, true] {
                let mut reference = None;
                for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                    let file = support::Source::new(&source);
                    let loaded =
                        actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
                    let model = actionc::semantic::analyze_with_options(
                        &loaded.program,
                        actionc::semantic::SemanticOptions::modern(),
                    )
                    .unwrap();
                    let nir = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
                        &loaded.program,
                        &model,
                    ));
                    let nir = if optimize_nir {
                        actionc::nir::optimize_program(&nir).unwrap()
                    } else {
                        nir
                    };
                    let output = mir6502::generate_output_with_config_and_runtime(
                        &nir,
                        origin,
                        &Mir6502Config {
                            enable_small_leaf_inlining: false,
                            enable_cross_call_home_selection: enabled,
                            ..Mir6502Config::optimized()
                        },
                        runtime,
                    )
                    .unwrap();
                    let image = actionc::codegen::format_load_file(&output);
                    if let Some(old) = &reference {
                        assert_eq!(old, &image);
                    } else {
                        reference = Some(image.clone());
                    }
                    let entry = |name| {
                        output
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address
                    };
                    let local = |routine: &str, name: &str| {
                        output
                            .map
                            .storage_symbols
                            .iter()
                            .find(|s| {
                                s.name == name
                                    && s.scope
                                        == actionc::codegen::CodegenSymbolScope::Routine(
                                            routine.into(),
                                        )
                            })
                            .unwrap()
                            .address
                    };
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    let mut observe = LocalObserve {
                        byte_step: entry("ByteStep"),
                        word_step: entry("WordStep"),
                        hop: entry("Hop"),
                        arguments: vec![],
                    };
                    for value in [0u16, 0x00ff, 0x1234, 0xffff] {
                        observe.arguments.clear();
                        vm = machine::call(
                            vm,
                            &mut observe,
                            entry("KeptLocal"),
                            &value.to_le_bytes(),
                        );
                        assert_eq!(
                            machine::bytes(&vm, 0xa0, 2),
                            value.wrapping_mul(2).wrapping_add(1).to_le_bytes()
                        );
                        assert_eq!(observe.arguments, vec![("word", value)]);
                    }
                    for count in [0u8, 1, 2, 17, 255] {
                        observe.arguments.clear();
                        vm = machine::call(vm, &mut observe, entry("SumSteps"), &[count]);
                        let expected = (1..=u16::from(count)).fold(0x1234u16, |sum, x| {
                            sum.wrapping_add(u16::from((x as u8).wrapping_add(1)))
                        });
                        assert_eq!(machine::bytes(&vm, 0xa0, 2), expected.to_le_bytes());
                        assert_eq!(
                            observe.arguments,
                            (1..=u16::from(count))
                                .rev()
                                .map(|x| ("byte", x))
                                .collect::<Vec<_>>()
                        );
                    }
                    let base = 0x70feu16;
                    for offset in 0..32u16 {
                        vm.bus_mut()
                            .ram_mut()
                            .write(base + offset, (offset as u8).wrapping_mul(13));
                        vm.bus_mut().add_watchpoint(base + offset);
                    }
                    for count in [0u8, 1, 17] {
                        observe.arguments.clear();
                        let first = vm.bus().events().len();
                        vm = machine::call(
                            vm,
                            &mut observe,
                            entry("WalkLocal"),
                            &[base as u8, (base >> 8) as u8, count],
                        );
                        assert_eq!(machine::bytes(&vm, 0xa0, 1), vec![count.wrapping_mul(13)]);
                        assert_eq!(
                            observe.arguments,
                            (0..u16::from(count))
                                .map(|i| ("hop", base + i))
                                .collect::<Vec<_>>()
                        );
                        let reads: Vec<_> = vm.bus().events()[first..]
                            .iter()
                            .filter(|e| {
                                e.access == BusAccess::Read
                                    && (base..base + 32).contains(&e.address)
                            })
                            .map(|e| e.address)
                            .collect();
                        assert_eq!(reads, vec![base + u16::from(count)]);
                    }
                    for value in [0u8, 71, 255] {
                        vm.bus_mut().ram_mut().write(0x7100, value);
                        vm = machine::call(vm, &mut observe, entry("NoArgs"), &[]);
                        assert_eq!(
                            machine::bytes(&vm, 0xa0, 1),
                            vec![value.wrapping_mul(2).wrapping_add(1)]
                        );
                    }
                    let saved = local("EntryLocal", "saved");
                    let cell = local("AddressLocal", "cell");
                    assert!(saved >= 256 && cell >= 256);
                    vm.bus_mut().ram_mut().write(saved, 0x97);
                    for value in [0u8, 17, 255] {
                        vm = machine::call(vm, &mut observe, entry("EntryLocal"), &[value]);
                        assert_eq!(
                            machine::bytes(&vm, 0xa0, 1),
                            vec![value.wrapping_add(1).wrapping_add(0x97)]
                        );
                        assert_eq!(machine::bytes(&vm, saved, 1), vec![0x97]);
                        vm = machine::call(vm, &mut observe, entry("AddressLocal"), &[value]);
                        assert_eq!(machine::bytes(&vm, 0xa0, 2), cell.to_le_bytes());
                        assert_eq!(machine::bytes(&vm, cell, 1), vec![value]);
                    }
                }
            }
        }
    }
}
