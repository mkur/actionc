//! Independent result oracles for shared callee facts.
use actionc::{compiler::Runtime, mir6502::{self, Mir6502Config}};
use actionc_vm::{CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..") }
#[allow(dead_code)]
#[path = "support/tn_machine.rs"] mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"] mod support;
struct Observe;
impl VmRunHooks for Observe { type Error = String; }

fn compile(source: &str, config: &Mir6502Config, runtime: Runtime) -> actionc::codegen::CodegenOutput {
    let file = support::Source::new(source);
    let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
    let model = actionc::semantic::analyze_with_options(&loaded.program, actionc::semantic::SemanticOptions::modern()).unwrap();
    let n = actionc::nir::optimize_program(&actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&loaded.program, &model))).unwrap();
    mir6502::generate_output_with_config_and_runtime(&n, 0x3000, config, runtime).unwrap()
}
fn load(out: &actionc::codegen::CodegenOutput, runtime: Runtime) -> CompilerVm {
    let (mut vm, profile) = support::vm_for(runtime);
    vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(out)).unwrap();
    vm
}
fn entry(out: &actionc::codegen::CodegenOutput, name: &str) -> u16 {
    out.routine_addresses.iter().find(|r| r.name == name).unwrap().address
}

#[test]
fn callee_facts_pointer_parameters_return_bytes_and_words_without_losing_caller_state() {
    let source = "BYTE observedByte=$0680
        CARD observedWord=$0681
        BYTE FUNC FetchByte(BYTE POINTER p) RETURN(p^)
        CARD FUNC FetchWord(CARD POINTER p) RETURN(p^)
        CARD FUNC Hold(CARD keep BYTE POINTER p)
          BYTE b
          CARD w
          b=FetchByte(p)
          w=FetchWord(CARD POINTER(p))
          observedByte=b
          observedWord=w
        RETURN(keep+b+w)
        PROC Main() RETURN".replace("\r\n", "\n");
    let optimized = Mir6502Config {
        enable_small_leaf_inlining: false,
        ..Mir6502Config::optimized()
    };
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let mut lf_image = None;
        for (config, crlf) in [
            (Mir6502Config::default(), false),
            (optimized.clone(), false),
            (optimized.clone(), true),
        ] {
            let text = if crlf { source.replace('\n', "\r\n") } else { source.clone() };
            let out = compile(&text, &config, runtime);
            if config == optimized {
                let image = actionc::codegen::format_load_file(&out);
                if crlf { assert_eq!(lf_image.as_ref(), Some(&image)); }
                else { lf_image = Some(image); }
            }
            let mut vm = load(&out, runtime);
            for address in [0x0700u16, 0x07ff] {
                for value in [0u16, 0x00ff, 0x0100, 0xffff] {
                    for keep in [0u16, 0x00ff, 0xff00, 0xffff] {
                        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
                            vm.bus_mut().ram_mut().write(address + i as u16, byte);
                        }
                        let mut args = keep.to_le_bytes().to_vec();
                        args.extend(address.to_le_bytes());
                        vm = machine::call(vm, &mut Observe, entry(&out, "Hold"), &args);
                        let expected = keep.wrapping_add(value & 0xff).wrapping_add(value);
                        assert_eq!(machine::bytes(&vm, 0xa0, 2), expected.to_le_bytes(),
                            "{runtime:?} p={address:04x} value={value:04x} keep={keep:04x} optimized={} CRLF={crlf}",
                            config == optimized);
                        assert_eq!(machine::bytes(&vm, 0x680, 1), [value as u8]);
                        assert_eq!(machine::bytes(&vm, 0x681, 2), value.to_le_bytes());
                        assert_eq!(machine::bytes(&vm, address, 2), value.to_le_bytes());
                    }
                }
            }
        }
    }
}


#[test]
fn callee_facts_global_state_reads_preserve_state_and_live_caller_values() {
    let optimized = Mir6502Config {
        enable_small_leaf_inlining: false,
        ..Mir6502Config::optimized()
    };
    // The VM call trampoline occupies $0600; keep the RAM control outside it.
    for address in [0xe8u16, 0x700] {
        let source = format!(
            "BYTE state=${address:04X}
             BYTE observed=$0680
             BYTE FUNC ReadState() RETURN(state)
             CARD FUNC Hold(CARD keep)
               BYTE value
               value=ReadState()
               observed=value
             RETURN(keep+value)
             PROC Main() RETURN"
        ).replace("\r\n", "\n");
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let mut lf_image = None;
            for (config, crlf) in [
                (Mir6502Config::default(), false),
                (optimized.clone(), false),
                (optimized.clone(), true),
            ] {
                let text = if crlf { source.replace('\n', "\r\n") } else { source.clone() };
                let out = compile(&text, &config, runtime);
                if config == optimized {
                    let image = actionc::codegen::format_load_file(&out);
                    if crlf { assert_eq!(lf_image.as_ref(), Some(&image)); }
                    else { lf_image = Some(image); }
                }
                let mut vm = load(&out, runtime);
                for state in [0u8, 0x81, 0xff] {
                    for keep in [0u16, 0x1234, 0xffff] {
                        vm.bus_mut().ram_mut().write(address, state);
                        vm = machine::call(vm, &mut Observe, entry(&out, "Hold"), &keep.to_le_bytes());
                        assert_eq!(machine::bytes(&vm, 0xa0, 2),
                            keep.wrapping_add(u16::from(state)).to_le_bytes(),
                            "{runtime:?} state@{address:04x}={state} keep={keep} optimized={} CRLF={crlf}",
                            config == optimized);
                        assert_eq!(machine::bytes(&vm, 0x680, 1), [state]);
                        assert_eq!(machine::bytes(&vm, address, 1), [state]);
                    }
                }
            }
        }
    }
}

#[test]
fn callee_facts_flags_store_only_entry_and_produced_result() {
    let lf = include_str!("../../../fixtures/runtime/callee_facts_flags.act").replace("\r\n", "\n");
    let mut optimized = None;
    for (config, source) in [(Mir6502Config::default(), lf.clone()),
        (Mir6502Config::optimized(), lf.clone()), (Mir6502Config::optimized(), lf.replace('\n', "\r\n"))] {
        let out = compile(&source, &config, Runtime::ActionCart);
        if config == Mir6502Config::optimized() {
            if let Some(old) = &optimized { assert_eq!(&out.bytes, old); } else { optimized = Some(out.bytes.clone()); }
        }
        let mut vm = load(&out, Runtime::ActionCart);
        vm = machine::call(vm, &mut Observe, entry(&out, "Check"), &[]);
        assert_eq!(machine::bytes(&vm, 0xa0, 1), [1]);
        assert_eq!(machine::bytes(&vm, 0xfe, 2), [128, 0]);
        for v in [0u8, 128, 1, 255] {
            vm = machine::call(vm, &mut Observe, entry(&out, "CheckProduced"), &[v]);
            assert_eq!(machine::bytes(&vm, 0xa0, 1), [u8::from(v & 127 == 0)]);
        }
    }
}

#[test]
fn callee_facts_machine_memory_keeps_indexed_reads_and_offset_stores() {
    let source = include_str!("../../../fixtures/runtime/callee_facts_memory.act");
    let out = compile(source, &Mir6502Config::optimized(), Runtime::ActionCart);
    let mut vm = load(&out, Runtime::ActionCart);
    vm.bus_mut().ram_mut().write(0x7080, 0x55);
    vm.bus_mut().ram_mut().write(0x7082, 0xaa);
    for (name, result) in [("CheckOffset", 42), ("CheckIndexed", 90)] {
        vm = machine::call(vm, &mut Observe, entry(&out, name), &[]);
        assert_eq!(machine::bytes(&vm, 0xa0, 1), [result]);
    }
    assert_eq!(machine::bytes(&vm, 0x7080, 3), [0x55, 42, 0xaa]);
    vm = machine::call(vm, &mut Observe, entry(&out, "Boundary"), &[]);
    assert_eq!(machine::bytes(&vm, 0xa0, 2), [0, 0x71]);
}

#[test]
fn callee_facts_directional_reader_preserves_caller_and_pointee() {
    let optimized = Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() };
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for config in [Mir6502Config::default(), optimized.clone()] {
            let out = compile(include_str!("../../../fixtures/runtime/callee_facts_readers.act"), &config, runtime);
            let mut vm = load(&out, runtime);
            for base in [0x7000u16, 0x70ff] {
                vm.bus_mut().ram_mut().map(base, &[0x55, 0x81, 0xaa]).unwrap();
                for keep in [0u16, 0x1234, 0xffff] {
                    let args: Vec<_> = base.to_le_bytes().into_iter().chain(keep.to_le_bytes()).collect();
                    vm = machine::call(vm, &mut Observe, entry(&out, "KeepReader"), &args);
                    assert_eq!(machine::bytes(&vm, 0xa0, 2), keep.wrapping_add(0x81).to_le_bytes());
                    assert_eq!(machine::bytes(&vm, base, 3), [0x55, 0x81, 0xaa]);
                }
                vm = machine::call(vm, &mut Observe, entry(&out, "KeepWriter"), &[base as u8, (base >> 8) as u8, 0x34, 0x12]);
                assert_eq!(machine::bytes(&vm, 0xa0, 2), [0x34, 0x12]);
                assert_eq!(machine::bytes(&vm, base, 3), [0x55, 37, 0xaa]);
            }
            if config == optimized {
                for (name, resident) in [("KeepReader", true), ("KeepWriter", false)] {
                    let homes: Vec<_> = out.map.storage_symbols.iter().filter(|s|
                        s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                        && s.kind == actionc::codegen::CodegenSymbolKind::Parameter).collect();
                    assert!(!homes.is_empty());
                    assert!(homes.iter().all(|s| (s.address < 256) == resident), "{name}: {homes:?}");
                }
            }
        }
    }
}

#[test]
fn callee_facts_compare_entry_ignores_incoming_flags_and_preserves_caller() {
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for config in [Mir6502Config::default(), Mir6502Config {
            enable_small_leaf_inlining: false, ..Mir6502Config::optimized()
        }] {
            let out = compile(include_str!("../../../fixtures/runtime/callee_facts_compare.act"), &config, runtime);
            let mut vm = load(&out, runtime);
            for (left, right) in [(0u8, 0u8), (1, 255), (255, 1), (128, 127)] {
                let expected = if left == right { 0 } else if left > right { 1 } else { 2 };
                vm.bus_mut().ram_mut().map(0x70ff, &[7, left, 0xa5]).unwrap();
                vm.bus_mut().ram_mut().map(0x72ff, &[9, right, 0x5a]).unwrap();
                vm = machine::call(vm, &mut Observe, entry(&out, "KeepCompare"), &[0xff, 0x70, 0xff, 0x72]);
                assert_eq!(machine::bytes(&vm, 0xa0, 2), (1007u16 + expected).to_le_bytes());
                let callee = entry(&out, "CompareHead");
                for mask in 0..16u8 {
                    let status = 0x20 | (mask & 3) | ((mask & 4) << 5) | ((mask & 8) << 3);
                    // Establish flags after A/X/Y, immediately before the call.
                    vm.bus_mut().ram_mut().map(0x740, &[
                        0xa9, status, 0x48, 0xa9, 0xff, 0xa2, 0x70, 0xa0, 0xff,
                        0x28, 0x20, callee as u8, (callee >> 8) as u8, 0x60,
                    ]).unwrap();
                    vm.bus_mut().ram_mut().write(0xa3, 0x72);
                    vm = machine::call(vm, &mut Observe, 0x740, &[]);
                    assert_eq!(machine::bytes(&vm, 0xa0, 1), [expected as u8]);
                }
                assert_eq!(machine::bytes(&vm, 0x70ff, 3), [7, left, 0xa5]);
                assert_eq!(machine::bytes(&vm, 0x72ff, 3), [9, right, 0x5a]);
            }
        }
    }
}

#[test]
fn callee_facts_array_pointer_entry_preserves_caller_across_search() {
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for config in [Mir6502Config::default(), Mir6502Config {
            enable_small_leaf_inlining: false, ..Mir6502Config::optimized()
        }] {
            let out = compile(include_str!("../../../fixtures/runtime/callee_facts_array_entry.act"), &config, runtime);
            let mut vm = load(&out, runtime);
            for (buffer, needle, result) in [
                (vec![0, 0xa5, 0x5a, 0xcc], 42u8, 0u16),
                (vec![3, 17, 42, 99], 42, 2),
                (vec![3, 17, 42, 99], 128, 0),
                (vec![3, 17, 42, 99], 99, 3),
            ] {
                vm.bus_mut().ram_mut().map(0x70ff, &buffer).unwrap();
                vm.bus_mut().ram_mut().write(0x70fe, 0x55);
                vm.bus_mut().ram_mut().write(0x7103, 0xaa);
                vm = machine::call(vm, &mut Observe, entry(&out, "KeepLocate"), &[0xff, 0x70, needle]);
                assert_eq!(machine::bytes(&vm, 0xa0, 2), (1000 + u16::from(buffer[0]) + result).to_le_bytes());
                assert_eq!(machine::bytes(&vm, 0x70ff, buffer.len()), buffer);
                assert_eq!(machine::bytes(&vm, 0x70fe, 1), [0x55]);
                assert_eq!(machine::bytes(&vm, 0x7103, 1), [0xaa]);
            }
        }
    }
}

#[test]
fn callee_facts_guaranteed_compare_definitions_match_emitted_carry_behavior() {
    let source = "BYTE input=$0680\nBYTE FUNC Rel()\nIF input>=40 THEN RETURN(1) FI RETURN(0)\nBYTE FUNC Zero()\nIF input=0 THEN RETURN(1) FI RETURN(0)\nPROC Main() RETURN\n";
    let optimized = Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() };
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let mut lf_image = None;
        for (config, crlf) in [(Mir6502Config::default(), false), (optimized.clone(), false), (optimized.clone(), true)] {
            // Both newline forms go through the real source loader.
            let out = compile(&if crlf { source.replace('\n', "\r\n") } else { source.to_owned() }, &config, runtime);
            if config == optimized {
                let image = actionc::codegen::format_load_file(&out);
                if crlf { assert_eq!(lf_image.as_ref(), Some(&image)); } else { lf_image = Some(image); }
            }
            let mut vm = load(&out, runtime);
            for name in ["Rel", "Zero"] {
                let range = out.map.routine_ranges.iter().find(|r| r.name == name).unwrap();
                let code = &out.bytes[usize::from(range.start - out.origin)..usize::from(range.end - out.origin)];
                let mut cursor = 0;
                let mut comparisons = vec![];
                loop {
                    // Bound the probe to the expected straight-line load/store
                    // and immediate-CMP recipe; reject unfamiliar instructions.
                    let len = match code[cursor] {
                        0x90 | 0xb0 | 0xf0 | 0xd0 | 0x30 | 0x10 | 0x70 | 0x50 => break,
                        0xad | 0x8d => 3, // LDA/STA absolute
                        0xa5 | 0x85 | 0xa9 => 2, // LDA/STA ZP; LDA immediate
                        0xc9 => { comparisons.push(code[cursor..cursor + 2].to_vec()); 2 }
                        opcode => panic!("unexpected comparison prefix opcode {opcode:02x}"),
                    };
                    cursor += len;
                }
                if name == "Rel" { assert_eq!(comparisons, vec![vec![0xc9, 40]]); }
                else { assert!(comparisons.is_empty(), "zero comparison should reuse LDA flags: {comparisons:?}"); }
                // Execute the exact generated prefix, then observe C using ADC.
                // No instruction in this appended observation initializes C.
                let mut prefix = code[..cursor].to_vec();
                prefix.extend([0xa9, 0, 0x69, 0, 0x8d, 0x81, 0x06, 0x60]);
                vm.bus_mut().ram_mut().map(0x780, &prefix).unwrap();
                for value in [0u8, 1, 39, 40, 41, 127, 128, 255] {
                    vm.bus_mut().ram_mut().write(0x680, value);
                    for mask in 0..16u8 {
                        let status = 0x20 | (mask & 3) | ((mask & 4) << 5) | ((mask & 8) << 3);
                        for target in [entry(&out, name), 0x780] {
                            vm.bus_mut().ram_mut().map(0x740, &[
                                0xa9, status, 0x48, 0x28, 0x20, target as u8, (target >> 8) as u8, 0x60,
                            ]).unwrap();
                            vm = machine::call(vm, &mut Observe, 0x740, &[]);
                            if target == 0x780 {
                                let carry = if name == "Rel" { u8::from(value >= 40) } else { mask & 1 };
                                assert_eq!(machine::bytes(&vm, 0x681, 1), [carry], "{name} value={value} flags={mask}");
                            } else {
                                let expected = if name == "Rel" { value >= 40 } else { value == 0 };
                                assert_eq!(machine::bytes(&vm, 0xa0, 1), [u8::from(expected)]);
                            }
                        }
                    }
                }
            }
        }
    }
}
