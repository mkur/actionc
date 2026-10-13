//! General record sequences and source-cell aliasing through emitted code.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, VmRunHooks};
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
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

#[derive(Default)]
struct Traffic {
    scratch_writes: usize,
    accesses: Vec<(bool, u16)>,
    watched: Option<(u16, u16)>,
}
impl VmRunHooks for Traffic {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let registers = vm.cpu().registers();
        let ram = vm.bus().ram();
        let opcode = ram.read(registers.pc);
        let operand = registers.pc.wrapping_add(1);
        let direct = match opcode {
            0x85 | 0x86 | 0x84 => Some(u16::from(ram.read(operand))),
            0x8D | 0x8E | 0x8C => Some(ram.read_word(operand)),
            _ => None,
        };
        if direct.is_some_and(|a| (0xAA..=0xAF).contains(&a)) {
            self.scratch_writes += 1;
        }
        if let Some(address) = direct.filter(|a| {
            self.watched
                .is_some_and(|(start, bytes)| a.wrapping_sub(start) < bytes)
        }) {
            self.accesses.push((true, address));
        }
        if matches!(opcode, 0xB1 | 0x91) {
            let address = ram
                .read_word(u16::from(ram.read(operand)))
                .wrapping_add(u16::from(registers.y));
            self.accesses.push((opcode == 0x91, address));
        }
        Ok(())
    }
}

#[test]
fn record_sequences_keep_fields_snapshots_and_mutable_source_cells() {
    let original =
        std::fs::read_to_string(root().join("surveys/tn/reproducers/record-base-reuse.act"))
            .unwrap()
            .replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for crlf in [false, true] {
            let path = std::env::temp_dir().join(format!(
                "record-bases-{}-{runtime:?}-{crlf}.act",
                std::process::id()
            ));
            std::fs::write(
                &path,
                if crlf {
                    original.replace('\n', "\r\n")
                } else {
                    original.clone()
                },
            )
            .unwrap();
            let compiled = compile_file(
                &path,
                &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
            )
            .unwrap();
            let loaded = actionc::includes::load_program_with_expanded_source(&path).unwrap();
            std::fs::remove_file(path).unwrap();
            let model = actionc::semantic::analyze_with_options(
                &loaded.program,
                actionc::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
            let raw = actionc::nir::lower_program(&sem);
            for raw_nir in [false, true] {
                let normalized = if raw_nir {
                    raw.clone()
                } else {
                    actionc::nir::optimize_program(&raw).unwrap()
                };
                let mut read_controls = std::collections::BTreeMap::new();
                for (enabled, writes, origins) in [
                    (false, false, false),
                    (true, false, false),
                    (true, true, false),
                    (true, true, true),
                ] {
                    let mut config = actionc::mir6502::Mir6502Config::optimized();
                    config.enable_record_base_reuse = enabled;
                    config.enable_record_base_writes = writes;
                    config.enable_record_base_parameter_origins = origins;
                    let output = actionc::mir6502::generate_output_with_config_and_runtime(
                        &normalized,
                        0x2C00,
                        &config,
                        runtime,
                    )
                    .unwrap();
                    let routine = |name: &str| {
                        output
                            .map
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address
                    };
                    let global = |name: &str| {
                        output
                            .map
                            .storage_symbols
                            .iter()
                            .find(|s| {
                                s.name == name
                                    && s.scope == actionc::codegen::CodegenSymbolScope::Global
                            })
                            .unwrap()
                            .address
                    };
                    let mut vm = if runtime == Runtime::ActionCart {
                        machine::load(&compiled)
                    } else {
                        let mut vm = CompilerVm::default();
                        vm.load_atari_object_for_execution(
                            ExecutionProfile::StandaloneObject,
                            compiled.object_bytes(),
                        )
                        .unwrap();
                        vm
                    };
                    vm.bus_mut()
                        .ram_mut()
                        .map(output.origin, &output.bytes)
                        .unwrap();
                    for base in [0x6000u16, 0x60FF] {
                        vm.bus_mut()
                            .ram_mut()
                            .map(base - 1, &[0xA5, 0x34, 0x12, 8, 9, 10, 0xCD, 0xAB, 0xA5])
                            .unwrap();
                        vm.bus_mut()
                            .ram_mut()
                            .map(global("current"), &base.to_le_bytes())
                            .unwrap();
                        let mut traffic = Traffic::default();
                        let cycles = vm.cpu().cycles();
                        vm = machine::call(vm, &mut traffic, routine("ReadCurrent"), &[]);
                        let measured = (vm.cpu().cycles() - cycles, traffic.scratch_writes);
                        assert_eq!(
                            traffic.accesses,
                            [base, base + 1, base + 5, base + 6].map(|address| (false, address))
                        );
                        if enabled {
                            let old: &(u64, usize) = read_controls.get(&base).unwrap();
                            assert!(measured.0 < old.0);
                            assert_eq!(measured.1 + 2, old.1);
                        } else {
                            read_controls.insert(base, measured);
                        }
                        if !crlf && !raw_nir && base == 0x6000 {
                            println!(
                                "record-base,{runtime:?},{enabled}/{writes}/{origins},ReadCurrent,cycles={},scratch-writes={}",
                                measured.0, measured.1
                            );
                        }
                        assert_eq!(vm.bus().ram().read_word(0xA0), 0xBE01);
                        assert_eq!(vm.bus().ram().read_word(global("observed")), 0x1234);
                        vm = machine::call(
                            vm,
                            &mut Hooks,
                            routine("ReadFields"),
                            &base.to_le_bytes(),
                        );
                        assert_eq!(vm.bus().ram().read_word(0xA0), 0xBE01);
                        let mut args = base.to_le_bytes().to_vec();
                        args.extend(0xFEDCu16.to_le_bytes());
                        vm = machine::call(vm, &mut Hooks, routine("WriteFields"), &args);
                        assert_eq!(
                            machine::bytes(&vm, base - 1, 9),
                            [0xA5, 0xDC, 0xFE, 0, 0, 1, 0xCD, 0xAB, 0xA5]
                        );
                        vm = machine::call(vm, &mut Hooks, routine("MixedFields"), &args);
                        assert_eq!(vm.bus().ram().read_word(global("observed")), 0xFDB8);
                        let mut args = base.to_le_bytes().to_vec();
                        args.extend(base.to_le_bytes());
                        vm = machine::call(vm, &mut Hooks, routine("NestedFields"), &args);
                        assert_eq!(vm.bus().ram().read_word(base), 0xFEDC);
                        assert_eq!(vm.bus().ram().read(base + 2), 0);
                        assert_eq!(vm.bus().ram().read(base + 4), 0);
                        assert_eq!(vm.bus().ram().read(base - 1), 0xA5);
                        assert_eq!(vm.bus().ram().read(base + 7), 0xA5);
                    }
                    let data = global("data");
                    vm.bus_mut()
                        .ram_mut()
                        .map(data, &[0xA5, 0xA5, 0xA5, 0xA5, 0xA5, 0x43, 0x65])
                        .unwrap();
                    let next = vm.bus().ram().read(data + 7);
                    let mut traffic = Traffic {
                        watched: Some((data, 7)),
                        ..Traffic::default()
                    };
                    vm = machine::call(vm, &mut traffic, routine("KnownFields"), &[]);
                    assert_eq!(
                        traffic.accesses,
                        [3, 2, 4, 0, 1].map(|offset| (true, data + offset))
                    );
                    assert_eq!(
                        machine::bytes(&vm, data, 7),
                        [0xDC, 0xFE, 0, 0, 1, 0x43, 0x65]
                    );
                    vm = machine::call(
                        vm,
                        &mut Hooks,
                        routine("KnownWord"),
                        &0x7EEDu16.to_le_bytes(),
                    );
                    assert_eq!(
                        machine::bytes(&vm, data, 7),
                        [0xED, 0x7E, 0, 0, 1, 0x43, 0x65]
                    );
                    assert_eq!(vm.bus().ram().read(data + 7), next);
                    vm = machine::call(
                        vm,
                        &mut Hooks,
                        routine("RebindParameter"),
                        &0x6000u16.to_le_bytes(),
                    );
                    assert_eq!(
                        machine::bytes(&vm, data, 7),
                        [0xED, 0x7E, 2, 1, 0, 0x43, 0x65]
                    );
                    let rebound = output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| {
                            s.name == "p"
                                && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                                && s.scope
                                    == actionc::codegen::CodegenSymbolScope::Routine(
                                        "RebindParameter".into(),
                                    )
                        })
                        .unwrap()
                        .address;
                    assert_eq!(vm.bus().ram().read_word(rebound), data);
                    let cell = output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| {
                            s.name == "p"
                                && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                                && s.scope
                                    == actionc::codegen::CodegenSymbolScope::Routine(
                                        "MutateSource".into(),
                                    )
                        })
                        .unwrap()
                        .address;
                    vm.bus_mut().ram_mut().write(0x6002, 0xA5);
                    vm =
                        machine::call(vm, &mut Hooks, routine("MutateSource"), &cell.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(cell), 0x6000);
                    assert_eq!(
                        vm.bus().ram().read(0x6002),
                        0x12,
                        "{runtime:?}/{crlf}: must reread the changed source pointer"
                    );
                }
            }
        }
    }
}

#[test]
fn wide_field_windows_and_address_wrap_keep_original_byte_addresses() {
    for offset in [253u16, 254, 255, 256] {
        let source = format!(
            "TYPE Wide=[BYTE head BYTE ARRAY pad({}) CARD value BYTE tail]\nWide data\nBYTE guard\nCARD FUNC Read(Wide POINTER p) RETURN(p.value)\nPROC WriteKnown(CARD value) Wide POINTER p p=@data p.head=17 p.value=value p.tail=34 RETURN\nPROC Main() WriteKnown(0) guard=1 RETURN\n",
            offset - 1,
        );
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for raw in [false, true] {
                let ast =
                    actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
                let model = actionc::semantic::analyze_with_options(
                    &ast,
                    actionc::semantic::SemanticOptions::modern(),
                )
                .unwrap();
                let sem = actionc::semantic::ir::lower_program(&ast, &model);
                let nir = actionc::nir::lower_program(&sem);
                let nir = if raw {
                    nir
                } else {
                    actionc::nir::optimize_program(&nir).unwrap()
                };
                for enabled in [false, true] {
                    let mut config = actionc::mir6502::Mir6502Config::optimized();
                    config.enable_record_base_reuse = enabled;
                    config.enable_record_base_writes = enabled;
                    config.enable_record_base_parameter_origins = enabled;
                    let output = actionc::mir6502::generate_output_with_config_and_runtime(
                        &nir, 0x2C00, &config, runtime,
                    )
                    .unwrap();
                    let image = actionc::codegen::format_load_file(&output);
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    let data = output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| s.name == "data")
                        .unwrap()
                        .address;
                    let entry = |name| {
                        output
                            .map
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address
                    };
                    let mut expected = vec![0xA5; usize::from(offset) + 5];
                    vm.bus_mut().ram_mut().map(data - 1, &expected).unwrap();
                    expected[1] = 17;
                    expected[usize::from(offset) + 1..usize::from(offset) + 3]
                        .copy_from_slice(&0xFEDCu16.to_le_bytes());
                    expected[usize::from(offset) + 3] = 34;
                    vm = machine::call(
                        vm,
                        &mut Hooks,
                        entry("WriteKnown"),
                        &0xFEDCu16.to_le_bytes(),
                    );
                    assert_eq!(
                        machine::bytes(&vm, data - 1, expected.len()),
                        expected,
                        "{offset}/{runtime:?}/{raw}/{enabled}"
                    );
                    vm = machine::call(vm, &mut Hooks, entry("Read"), &data.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(0xA0), 0xFEDC);
                    if runtime == Runtime::Standalone {
                        // The ROM-free profile permits a read across $FFFF/$0000.
                        let base = 0xFFFFu16.wrapping_sub(offset);
                        vm.bus_mut().ram_mut().write(0xFFFF, 0xDC);
                        vm.bus_mut().ram_mut().write(0, 0xFE);
                        let mut traffic = Traffic::default();
                        vm = machine::call(vm, &mut traffic, entry("Read"), &base.to_le_bytes());
                        assert_eq!(vm.bus().ram().read_word(0xA0), 0xFEDC);
                        assert_eq!(traffic.accesses, [(false, 0xFFFF), (false, 0)]);
                    }
                }
            }
        }
    }
}
