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
fn object_storage_native_swaps_preserve_calls_pages_and_stack() {
    let source = include_str!("../../../fixtures/runtime/object_storage.act").replace("\r\n", "\n");
    let file = support::Source::new(&source);
    let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
    let model = actionc::semantic::analyze_with_options(&loaded.program,
        actionc::semantic::SemanticOptions::modern()).unwrap();
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&loaded.program, &model))).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for enabled in [false, true] {
            let output = mir6502::generate_output_with_config_and_runtime(&nir, 0x3000,
                &Mir6502Config { enable_small_leaf_inlining: false,
                    enable_parameter_home_selection: enabled,
                    enable_cross_call_home_selection: enabled,
                    ..Mir6502Config::optimized() }, runtime).unwrap();
            for name in ["SwapWords", "SwapBytes"] {
                assert_eq!(output.map.storage_symbols.iter().any(|s|
                    s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                    && s.kind == actionc::codegen::CodegenSymbolKind::Local && s.address < 256), enabled, "{name}");
            }
            let entry = |name| output.routine_addresses.iter().find(|r| r.name == name).unwrap().address;
            let (mut vm, profile) = support::vm_for(runtime);
            vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
            let mut observe = Observe { advance: entry("Touch"), arguments: vec![] };
            vm.bus_mut().ram_mut().write(0xe4, 0x5a);
            for (name, word) in [("SwapWords", true), ("SwapBytes", false)] {
                for base in [0x7000u16, 0x70fd] {
                    for index in [2u8, 4] {
                        let mut values = vec![0x12abu16, 0x00ff, 0xff00, 0xa571, 0x8123, 0x0042];
                        if !word { values.iter_mut().for_each(|v| *v &= 255); }
                        let bytes = |values: &[u16]| values.iter().flat_map(|v|
                            if word { v.to_le_bytes().to_vec() } else { vec![*v as u8] }).collect::<Vec<_>>();
                        let initial = bytes(&values);
                        vm.bus_mut().ram_mut().write(base-1, 0xc3);
                        vm.bus_mut().ram_mut().write(base+initial.len() as u16, 0x3c);
                        for (i, byte) in initial.iter().enumerate() { vm.bus_mut().ram_mut().write(base+i as u16, *byte); }
                        for _ in 0..2 {
                            let other = usize::from(index / 2);
                            let saved = values[other];
                            values.swap(other, other+1);
                            observe.arguments.clear();
                            let [lo, hi] = base.to_le_bytes();
                            vm = machine::call(vm, &mut observe, entry(name), &[lo, hi, index]);
                            assert_eq!(observe.arguments.iter().map(|v| *v as u8).collect::<Vec<_>>(), vec![index]);
                            assert_eq!(machine::bytes(&vm, base, initial.len()), bytes(&values), "{runtime:?}, {enabled}, {name}, {base:04x}");
                            let expected = if word { saved.to_le_bytes().to_vec() } else { vec![(saved as u8) ^ (index+1)] };
                            assert_eq!(machine::bytes(&vm, 0xa0, expected.len()), expected);
                            assert_eq!(machine::bytes(&vm, base-1, 1), vec![0xc3]);
                            assert_eq!(machine::bytes(&vm, base+initial.len() as u16, 1), vec![0x3c]);
                            assert_eq!(machine::bytes(&vm, 0xe4, 1), vec![0x5a]);
                        }
                    }
                }
            }
            for x in [0u8, 0x80, 0xff] {
                for (name, expected) in [("ShiftZero", x ^ 0x65), ("ShiftEight", 0x65)] {
                    vm = machine::call(vm, &mut observe, entry(name), &[x, 0x65]);
                    assert_eq!(machine::bytes(&vm, 0xa0, 1), vec![expected]);
                }
            }
        }
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

struct StorageObserve {
    advance: u16,
    observe: u16,
    calls: Vec<u16>,
    arguments: Vec<[u8; 3]>,
}
impl VmRunHooks for StorageObserve {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.advance { self.calls.push(u16::from_le_bytes([r.a, r.x])); }
        if r.pc == self.observe { self.arguments.push([r.a, r.x, r.y]); }
        Ok(())
    }
}

#[test]
fn storage_observation_preserves_invocations_arguments_and_owned_memory() {
    let lf = include_str!("../../../fixtures/runtime/storage_observation.act").replace("\r\n", "\n");
    let protected = lf.replace("BYTE reserved0=$E0,reserved1=$E1,sourceOwned=$E6", "BYTE ARRAY ownedPool(16)=$E0");
    let mut reference = None;
    for (runtime, enabled, source) in [
        (Runtime::ActionCart, false, lf.clone()),
        (Runtime::ActionCart, true, lf.clone()),
        (Runtime::ActionCart, true, lf.replace('\n', "\r\n")),
        (Runtime::Standalone, true, lf),
        (Runtime::ActionCart, true, protected),
    ] {
        let file = support::Source::new(&source);
        let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
        let model = actionc::semantic::analyze_with_options(&loaded.program,
            actionc::semantic::SemanticOptions::modern()).unwrap();
        let n = actionc::nir::optimize_program(&actionc::nir::lower_program(
            &actionc::semantic::ir::lower_program(&loaded.program, &model))).unwrap();
        let output = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
            &Mir6502Config { enable_small_leaf_inlining: false,
                enable_scoped_storage_observation: enabled, ..Mir6502Config::optimized() }, runtime).unwrap();
        if runtime == Runtime::ActionCart && enabled && !source.contains("ownedPool") {
            // Source spans count the original newline bytes; compare executable
            // artifacts and storage placement, not LF/CRLF debug offsets.
            let artifact = (output.bytes.clone(), output.relocations.clone(),
                output.map.storage_symbols.clone(), output.map.routine_ranges.clone());
            if let Some(old) = &reference { assert_eq!(old, &artifact); }
            else { reference = Some(artifact); }
        }
        let entry = |name: &str| output.routine_addresses.iter().find(|r| r.name == name).unwrap().address;
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
        let mut hook = StorageObserve { advance: entry("AdvanceRecord"), observe: entry("Observe"), calls: vec![], arguments: vec![] };
        let records = [0x70feu16, 0x7103, 0x710a, 0x710e];
        let lengths = [2u8, 4, 1, 0];
        for (&p, &len) in records.iter().zip(&lengths) { vm.bus_mut().ram_mut().write(p, len); }
        let sentinels: Vec<_> = if source.contains("ownedPool") { (0xe0..=0xef).collect() } else { vec![0xe0, 0xe1, 0xe6] };
        for &address in &sentinels { vm.bus_mut().ram_mut().write(address, address as u8 ^ 0x53); }
        for _ in 0..2 {
            for count in [0usize, 1, 3] {
                hook.calls.clear(); hook.arguments.clear();
                let args: Vec<_> = records[0].to_le_bytes().into_iter().chain(records[count].to_le_bytes()).collect();
                vm = machine::call(vm, &mut hook, entry("FindRecord"), &args);
                assert_eq!(machine::bytes(&vm, 0xa0, 1), vec![count as u8]);
                assert_eq!(hook.calls, records[..count]);
                hook.calls.clear();
                let args = [records[0] as u8, (records[0] >> 8) as u8, count as u8];
                vm = machine::call(vm, &mut hook, entry("Walk"), &args);
                assert_eq!(machine::bytes(&vm, 0xa0, 2), records[count].to_le_bytes());
                assert_eq!(hook.calls, records[..count]);
                let sum = lengths[..count].iter().fold(0u8, |a, b| a.wrapping_add(b+1));
                assert_eq!(hook.arguments, vec![[sum, 0, 0], [128, lengths[count].wrapping_sub(1), 0]]);
                assert_eq!(machine::bytes(&vm, 0x7200, 3), vec![128, lengths[count].wrapping_sub(1), 0]);
                for &address in &sentinels { assert_eq!(machine::bytes(&vm, address, 1), vec![address as u8 ^ 0x53]); }
            }
        }
    }
}

struct MultipleLeafObserve {
    bump: u16,
    hop: u16,
    step: u16,
    calls: Vec<(&'static str, u16)>,
}
impl VmRunHooks for MultipleLeafObserve {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let kind = if r.pc == self.bump { Some("bump") }
            else if r.pc == self.hop { Some("hop") }
            else if r.pc == self.step { Some("step") }
            else { None };
        if let Some(kind) = kind {
            self.calls.push((kind, if kind == "bump" { u16::from(r.a) }
                else { u16::from_le_bytes([r.a, r.x]) }));
        }
        Ok(())
    }
}

#[test]
fn multiple_leaf_calls_preserve_arguments_state_and_source_memory() {
    let lf = include_str!("../../../fixtures/runtime/multiple_leaf_calls.act").replace("\r\n", "\n");
    let mut reference = None;
    for (runtime, enabled, source) in [
        (Runtime::ActionCart, false, lf.clone()),
        (Runtime::ActionCart, true, lf.clone()),
        (Runtime::ActionCart, true, lf.replace('\n', "\r\n")),
        (Runtime::Standalone, true, lf),
    ] {
        let file = support::Source::new(&source);
        let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
        let model = actionc::semantic::analyze_with_options(&loaded.program,
            actionc::semantic::SemanticOptions::modern()).unwrap();
        let n = actionc::nir::optimize_program(&actionc::nir::lower_program(
            &actionc::semantic::ir::lower_program(&loaded.program, &model))).unwrap();
        let output = mir6502::generate_output_with_config_and_runtime(&n, 0x3000,
            &Mir6502Config { enable_small_leaf_inlining: false,
                enable_multiple_leaf_call_homes: enabled, ..Mir6502Config::optimized() }, runtime).unwrap();
        if runtime == Runtime::ActionCart && enabled {
            let artifact = (output.bytes.clone(), output.relocations.clone(),
                output.map.storage_symbols.clone(), output.map.routine_ranges.clone());
            if let Some(old) = &reference { assert_eq!(old, &artifact); }
            else { reference = Some(artifact); }
        }
        let entry = |name: &str| output.routine_addresses.iter().find(|r| r.name == name).unwrap().address;
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
        let mut hook = MultipleLeafObserve { bump: entry("Bump"), hop: entry("Hop"),
            step: entry("AddOne"), calls: vec![] };
        let base = 0x70feu16;
        let values = [7u8, 0, 250, 4];
        vm.bus_mut().ram_mut().map(base, &values).unwrap();
        for (address, value) in [(0xe6, 0x93), (base-1, 0x55), (base+4, 0xa5)] {
            vm.bus_mut().ram_mut().write(address, value);
        }
        for _ in 0..2 {
            for count in [0u8, 1, 4] {
                hook.calls.clear();
                vm = machine::call(vm, &mut hook, entry("SumWalk"), &[base as u8, (base>>8) as u8, count]);
                let sum = values[..usize::from(count)].iter().filter(|&&v| v != 0)
                    .fold(0u8, |a, v| a.wrapping_add(v.wrapping_add(1)));
                assert_eq!(machine::bytes(&vm, 0xa0, 1), vec![sum]);
                let mut expected = Vec::new();
                for i in 0..usize::from(count) {
                    let p = base+i as u16;
                    if values[i] != 0 { expected.push(("bump", u16::from(values[i]))); }
                    expected.push(("hop", p));
                }
                assert_eq!(hook.calls, expected);
                assert_eq!(machine::bytes(&vm, base, 4), values);
                for (address, value) in [(0xe6, 0x93), (base-1, 0x55), (base+4, 0xa5)] {
                    assert_eq!(machine::bytes(&vm, address, 1), vec![value]);
                }
            }
            for value in [0u16, 255, 65535] {
                for (name, steps) in [("Twice", 2u16), ("Once", 1)] {
                    hook.calls.clear();
                    vm = machine::call(vm, &mut hook, entry(name), &value.to_le_bytes());
                    assert_eq!(machine::bytes(&vm, 0xa0, 2), value.wrapping_add(steps).to_le_bytes());
                    assert_eq!(hook.calls, (0..steps).map(|i| ("step", value.wrapping_add(i))).collect::<Vec<_>>());
                    assert_eq!(machine::bytes(&vm, 0xe6, 1), vec![0x93]);
                }
            }
        }
    }
}
