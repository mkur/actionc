//! Producer-only comparison after late store deletion, through real source files.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, CompilerVm, VmRunHooks};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
struct Observe;
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, _: &mut CompilerVm) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn late_producers_preserve_bytes_public_writes_and_return_state() {
    let text =
        include_str!("../../../fixtures/runtime/late_producer_cleanup.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut reference = None;
            let mut old_size = 0;
            for enabled in [false, true] {
                let mut lf = None;
                for crlf in [false, true] {
                    let text = if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.clone()
                    };
                    let source = support::Source::new(&text);
                    let loaded =
                        actionc::includes::load_program_with_expanded_source(&source.0).unwrap();
                    let model = actionc::semantic::analyze_with_options(
                        &loaded.program,
                        actionc::semantic::SemanticOptions::modern(),
                    )
                    .unwrap();
                    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(
                        &actionc::semantic::ir::lower_program(&loaded.program, &model),
                    ))
                    .unwrap();
                    let output = mir6502::generate_output_with_config_and_runtime(
                        &nir,
                        origin,
                        &Mir6502Config {
                            enable_late_dead_register_cleanup: enabled,
                            ..Mir6502Config::optimized()
                        },
                        runtime,
                    )
                    .unwrap();
                    let image = actionc::codegen::format_load_file(&output);
                    if !enabled {
                        old_size = image.len();
                    } else {
                        assert!(image.len() < old_size, "producer removal must save bytes");
                    }
                    if let Some(lf) = &lf {
                        assert_eq!(lf, &image);
                    } else {
                        lf = Some(image.clone());
                    }
                    let entry = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == "Transform")
                        .unwrap()
                        .address;
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    for address in 0x7200..0x7401 {
                        vm.bus_mut().add_watchpoint(address);
                    }
                    let mut results = Vec::new();
                    for address in [0x7200u16, 0x72fe] {
                        for first in [0u8, 29, 31, 32, 93, 95, 96, 127, 252, 255] {
                            let input: Vec<_> = (0..4).map(|n| first.wrapping_add(n)).collect();
                            vm.bus_mut().ram_mut().map(address, &input).unwrap();
                            vm.bus_mut().clear_events();
                            vm = machine::call(vm, &mut Observe, entry, &address.to_le_bytes());
                            let expected: Vec<_> = input
                                .into_iter()
                                .map(|n| {
                                    if n < 32 {
                                        n + 64
                                    } else if n < 96 {
                                        n - 32
                                    } else {
                                        n
                                    }
                                })
                                .collect();
                            assert_eq!(machine::bytes(&vm, address, 4), expected);
                            let writes: Vec<_> = vm
                                .bus()
                                .events()
                                .iter()
                                .filter(|e| e.access == BusAccess::Write)
                                .map(|e| (e.address, e.value))
                                .collect();
                            let r = vm.cpu().registers();
                            results.push((writes, r.a, r.x, r.y, r.sp, r.status));
                        }
                    }
                    if let Some(reference) = &reference {
                        assert_eq!(&results, reference);
                    } else {
                        reference = Some(results);
                    }
                }
            }
        }
    }
}

#[test]
fn tn_directory_row_producers_preserve_conversion_writes_and_machine_state() {
    let loaded = actionc::includes::load_program_with_expanded_source(
        root().join("samples/tn/modern/TN.ACT"),
    )
    .unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&loaded.program, &model),
    ))
    .unwrap();
    // The independent fixture covers the full runtime/origin/newline matrix;
    // execute the intact TN routine in each runtime at distinct origins here.
    for (runtime, origin) in [(Runtime::ActionCart, 0x3000), (Runtime::Standalone, 0x41c7)] {
        let mut reference = None;
        let mut old_size = 0;
        for enabled in [false, true] {
            let output = mir6502::generate_output_with_config_and_runtime(
                &nir,
                origin,
                &Mir6502Config {
                    enable_late_dead_register_cleanup: enabled,
                    ..Mir6502Config::optimized()
                },
                runtime,
            )
            .unwrap();
            let image = actionc::codegen::format_load_file(&output);
            if !enabled {
                old_size = image.len();
            } else {
                assert_eq!(old_size - image.len(), 12);
            }
            let entry = output
                .routine_addresses
                .iter()
                .find(|r| r.name == "FormatDirectoryRow")
                .unwrap()
                .address;
            let pointer = output
                .map
                .storage_symbols
                .iter()
                .find(|s| s.name == "s" && s.scope == actionc::codegen::CodegenSymbolScope::Global)
                .unwrap()
                .address;
            let (mut vm, profile) = support::vm_for(runtime);
            vm.load_atari_object_for_execution(profile, &image).unwrap();
            for address in 0x8fff..0x9110 {
                vm.bus_mut().add_watchpoint(address);
            }
            let mut results = Vec::new();
            for address in [0x9000u16, 0x90f8] {
                for short in [false, true] {
                    for space in [false, true] {
                        let mut row: Vec<_> = (0..18).map(|n| (n * 17 + 3) as u8).collect();
                        row[1] = if space { 32 } else { 42 };
                        row[17] = if short { 155 } else { 49 };
                        let mut expected = row.clone();
                        let internal =
                            |c: u8| (c & 0x9f) | [0x40, 0, 0x20, 0x60][usize::from((c >> 5) & 3)];
                        if expected[1] == 32 {
                            expected[1] = expected[0];
                        }
                        expected[1] = internal(expected[1]);
                        expected[0] = 0;
                        for value in &mut expected[2..=9] {
                            *value = internal(*value);
                        }
                        let end = if expected[17] == 155 { 16 } else { 17 };
                        for i in 0..3 {
                            expected[13 - i] = internal(expected[12 - i]);
                            expected[17 - i] = internal(expected[end - i]);
                        }
                        expected[10] = 124;
                        expected[14] = 124;
                        vm.bus_mut().ram_mut().write_word(pointer, address);
                        vm.bus_mut().ram_mut().write(address - 1, 0xa5);
                        vm.bus_mut().ram_mut().map(address, &row).unwrap();
                        vm.bus_mut().ram_mut().write(address + 18, 0x5a);
                        vm.bus_mut().clear_events();
                        vm = machine::call(vm, &mut Observe, entry, &[]);
                        assert_eq!(machine::bytes(&vm, address, 18), expected);
                        assert_eq!(vm.bus().ram().read(address - 1), 0xa5);
                        assert_eq!(vm.bus().ram().read(address + 18), 0x5a);
                        let writes: Vec<_> = vm
                            .bus()
                            .events()
                            .iter()
                            .filter(|e| e.access == BusAccess::Write)
                            .map(|e| (e.address, e.value))
                            .collect();
                        let r = vm.cpu().registers();
                        results.push((writes, r.a, r.x, r.y, r.sp, r.status));
                    }
                }
            }
            if let Some(reference) = &reference {
                assert_eq!(&results, reference);
            } else {
                reference = Some(results);
            }
        }
    }
}
