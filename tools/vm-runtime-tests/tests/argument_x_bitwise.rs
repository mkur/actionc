//! Independent bitwise values, preserved C/V, retained stores and ordered reads.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, VmRunHooks};
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
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}
fn compile(
    text: &str,
    opt: bool,
    rt: Runtime,
    origin: u16,
    _enabled: bool,
) -> actionc::codegen::CodegenOutput {
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
    let nir = if opt {
        actionc::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    mir6502::generate_output_with_config_and_runtime(&nir, origin, &Mir6502Config::optimized(), rt)
        .unwrap()
}
#[test]
fn argument_x_bitwise_values_flags_and_images_with_lf_and_crlf() {
    let lf = include_str!("../../../fixtures/runtime/argument_x_bitwise.act").replace("\r\n", "\n");
    for opt in [false, true] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut baseline = std::collections::BTreeMap::new();
                for enabled in [false] {
                    let mut lf_image = None;
                    for crlf in [false, true] {
                        let text = if crlf {
                            lf.replace('\n', "\r\n")
                        } else {
                            lf.clone()
                        };
                        let out = compile(&text, opt, rt, origin, enabled);
                        let image = actionc::codegen::format_load_file(&out);
                        if crlf {
                            assert_eq!(lf_image.as_ref(), Some(&image));
                        } else {
                            lf_image = Some(image.clone());
                        }
                        if opt && !crlf && !enabled {
                            if let Some(dir) =
                                std::env::var_os("ACTIONC_ARGUMENT_X_BITWISE_BASELINE")
                            {
                                let rtname = if rt == Runtime::ActionCart {
                                    "cart"
                                } else {
                                    "standalone"
                                };
                                assert_eq!(
                                    image,
                                    std::fs::read(
                                        PathBuf::from(dir)
                                            .join(format!("neutral-{rtname}-{origin:04X}.xex"))
                                    )
                                    .unwrap()
                                );
                            }
                        }
                        let (mut vm, profile) = support::vm_for(rt);
                        vm.load_atari_object_for_execution(profile, &image).unwrap();
                        let mut homes = Vec::new();
                        for s in &out.map.storage_symbols {
                            if matches!(&s.scope,actionc::codegen::CodegenSymbolScope::Routine(n) if n.starts_with("Forward"))
                            {
                                homes.extend(s.address..s.address + s.size);
                            }
                        }
                        for a in homes
                            .iter()
                            .copied()
                            .chain(0x680..=0x683)
                            .chain(0xE0..=0xE2)
                            .chain(0xAC..=0xAD)
                            .chain([0x7000, 0x7001])
                        {
                            vm.bus_mut().add_watchpoint(a);
                        }
                        for flags in [0, 1, 0x40, 0x41u8] {
                            for tag in [0u8, 1, 127, 128, 254, 255] {
                                for name in ["ForwardAnd", "ForwardOr", "ForwardXor"] {
                                    let address = out
                                        .routine_addresses
                                        .iter()
                                        .find(|r| r.name == name)
                                        .unwrap()
                                        .address;
                                    vm.bus_mut()
                                        .ram_mut()
                                        .map(
                                            0x740,
                                            &[
                                                0xA9,
                                                0x20 | flags,
                                                0x48,
                                                0xA9,
                                                0,
                                                0x28,
                                                0x20,
                                                address as u8,
                                                (address >> 8) as u8,
                                                0x60,
                                            ],
                                        )
                                        .unwrap();
                                    vm.bus_mut().ram_mut().map(0x7000, &[0x34, 0x12]).unwrap();
                                    vm.bus_mut().ram_mut().write(0x67F, 0xA5);
                                    vm.bus_mut().ram_mut().write(0x684, 0x5A);
                                    vm.bus_mut().clear_events();
                                    let cycles = vm.cpu().cycles();
                                    vm = machine::call(vm, &mut Hooks, 0x740, &[0, 0x70, tag]);
                                    let cycles = vm.cpu().cycles() - cycles;
                                    let expected = match name {
                                        "ForwardAnd" => tag & 127,
                                        "ForwardOr" => tag | 128,
                                        _ => tag ^ 127,
                                    };
                                    assert_eq!(
                                        machine::bytes(&vm, 0x680, 3),
                                        [0x34, 0x12, expected]
                                    );
                                    assert_eq!(
                                        vm.bus().ram().read(0x683) & 0xC3,
                                        flags | (expected & 128) | u8::from(expected == 0) * 2
                                    );
                                    assert_eq!(vm.bus().ram().read(0x67F), 0xA5);
                                    assert_eq!(vm.bus().ram().read(0x684), 0x5A);
                                    let r = vm.cpu().registers();
                                    let writes = vm
                                        .bus()
                                        .events()
                                        .iter()
                                        .filter(|e| e.access == BusAccess::Write)
                                        .map(|e| {
                                            let a = if let Some(i) =
                                                homes.iter().position(|a| *a == e.address)
                                            {
                                                0x10000 + i
                                            } else {
                                                e.address as usize
                                            };
                                            (a, e.value)
                                        })
                                        .collect::<Vec<_>>();
                                    let reads = vm
                                        .bus()
                                        .events()
                                        .iter()
                                        .filter(|e| {
                                            e.access == BusAccess::Read
                                                && [0x7000, 0x7001].contains(&e.address)
                                        })
                                        .map(|e| e.address)
                                        .collect::<Vec<_>>();
                                    assert_eq!(reads, [0x7000, 0x7001]);
                                    let state = (writes, (r.a, r.x, r.y, r.status, r.sp));
                                    let key = (crlf, flags, tag, name);
                                    if enabled {
                                        assert_eq!(baseline.get(&key), Some(&state));
                                    } else {
                                        baseline.insert(key, state);
                                    }
                                    if opt
                                        && !crlf
                                        && flags == 0
                                        && tag == 1
                                        && std::env::var_os("ACTIONC_ARGUMENT_X_BITWISE_METRICS")
                                            .is_some()
                                    {
                                        eprintln!("bitwise,{name},{rt:?},{origin:04X},{enabled},cycles={cycles}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
