//! Compare public writes and ABI state with only terminal cleanup toggled.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, CompilerVm, VmRunHooks};
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
struct Observe;
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, _: &mut CompilerVm) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn terminal_cleanup_preserves_argument_bytes_public_writes_and_returns() {
    let text = include_str!("../../../fixtures/runtime/argument_staging.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut baseline = BTreeMap::new();
            let mut sizes = Vec::new();
            for enabled in [false, true] {
                let mut lf = None;
                for crlf in [false, true] {
                    let text = if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.clone()
                    };
                    // Exercise CRLF through an actual source file and include loader.
                    let source = support::Source::new(&text);
                    let loaded =
                        actionc::includes::load_program_with_expanded_source(&source.0).unwrap();
                    let model = actionc::semantic::analyze_with_options(
                        &loaded.program,
                        actionc::semantic::SemanticOptions::modern(),
                    )
                    .unwrap();
                    let input = actionc::nir::optimize_program(&actionc::nir::lower_program(
                        &actionc::semantic::ir::lower_program(&loaded.program, &model),
                    ))
                    .unwrap();
                    let output = mir6502::generate_output_with_config_and_runtime(
                        &input,
                        origin,
                        &Mir6502Config {
                            enable_late_private_store_cleanup: enabled,
                            ..Mir6502Config::optimized()
                        },
                        runtime,
                    )
                    .unwrap();
                    let image = actionc::codegen::format_load_file(&output);
                    if let Some(lf) = &lf {
                        assert_eq!(lf, &image);
                    } else {
                        sizes.push(image.len());
                        lf = Some(image.clone());
                    }
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    for address in 0x680..=0x684 {
                        vm.bus_mut().add_watchpoint(address);
                    }
                    for name in ["ForwardAdd", "ForwardSub", "FourByte", "FivePlain"] {
                        let entry = output
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address;
                        for first in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                            let second = first ^ 0xa55a;
                            let tag = (first >> 8) as u8;
                            let mut args = first.to_le_bytes().to_vec();
                            args.extend(second.to_le_bytes());
                            if name != "FourByte" {
                                args.push(tag);
                            }
                            let value = match name {
                                "ForwardAdd" | "FourByte" => first.wrapping_add(1),
                                "ForwardSub" => first.wrapping_sub(1),
                                _ => first,
                            };
                            let mut expected = value.to_le_bytes().to_vec();
                            expected.extend(second.to_le_bytes());
                            if name != "FourByte" {
                                expected.push(tag);
                            }
                            vm.bus_mut().clear_events();
                            vm = machine::call(vm, &mut Observe, entry, &args);
                            assert_eq!(machine::bytes(&vm, 0x680, expected.len()), expected);
                            let writes = vm
                                .bus()
                                .events()
                                .iter()
                                .filter(|e| e.access == BusAccess::Write)
                                .map(|e| (e.address, e.value))
                                .collect::<Vec<_>>();
                            let r = vm.cpu().registers();
                            let state = (writes, r.a, r.x, r.y, r.sp, r.status);
                            let key = (name, first);
                            if !enabled && !crlf {
                                baseline.insert(key, state);
                            } else {
                                assert_eq!(
                                    state, baseline[&key],
                                    "{runtime:?}/{origin:04x}/{name}/{first}"
                                );
                            }
                        }
                    }
                }
            }
            assert!(sizes[1] < sizes[0], "cleanup must actually save bytes");
        }
    }
}
