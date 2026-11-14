//! Opt-in private spill compaction through real LF/CRLF source files.
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
fn private_compaction_preserves_all_bytes_public_writes_and_return_state() {
    let text =
        include_str!("../../../fixtures/runtime/late_producer_cleanup.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut reference = None;
            let mut old_size = 0;
            let mut storage = None;
            let mut private_before = None;
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
                            enable_late_spill_pruning: enabled,
                            ..Mir6502Config::optimized()
                        },
                        runtime,
                    )
                    .unwrap();
                    // This fixture has two source globals and three private
                    // spill symbols. Only the surviving private byte may move.
                    let source_symbols: Vec<_> = output
                        .map
                        .storage_symbols
                        .iter()
                        .filter(|s| s.scope == actionc::codegen::CodegenSymbolScope::Global)
                        .cloned()
                        .collect();
                    assert_eq!(source_symbols.len(), 2);
                    if let Some(storage) = &storage {
                        assert_eq!(storage, &source_symbols);
                    } else {
                        storage = Some(source_symbols);
                    }
                    let private: Vec<_> = output
                        .map
                        .storage_symbols
                        .iter()
                        .filter(|s| s.scope != actionc::codegen::CodegenSymbolScope::Global)
                        .cloned()
                        .collect();
                    if !enabled {
                        assert_eq!(private.len(), 3);
                        private_before = Some(private);
                    } else {
                        assert_eq!(private.len(), 1);
                        let before = private_before
                            .as_ref()
                            .unwrap()
                            .iter()
                            .find(|s| s.name == private[0].name && s.scope == private[0].scope)
                            .unwrap();
                        assert_eq!(before.address - private[0].address, 2);
                        assert_eq!(private[0].size, 1);
                    }
                    let image = actionc::codegen::format_load_file(&output);
                    if !enabled {
                        old_size = image.len();
                    } else {
                        assert_eq!(
                            old_size - image.len(),
                            2,
                            "two unused declarations must disappear"
                        );
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
                        for first in 0u8..=255 {
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
