//! C/V-dependent nested observations and an immediate N/Z observer control.
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
    enabled: bool,
) -> actionc::codegen::CodegenOutput {
    compile_flags(text, opt, rt, origin, enabled, [true; 3])
}
fn compile_flags(
    text: &str,
    opt: bool,
    rt: Runtime,
    origin: u16,
    enabled: bool,
    predecessors: [bool; 3],
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
    mir6502::generate_output_with_config_and_runtime(
        &nir,
        origin,
        &Mir6502Config {
            enable_argument_x_bitwise_forwarding: false,
            enable_precise_callee_entry_flags: enabled,
            enable_argument_staging_reduction: predecessors[0],
            enable_pointer_byte_plus_one_fusion: predecessors[1],
            enable_pointer_load_sharing: predecessors[2],
            ..Mir6502Config::optimized()
        },
        rt,
    )
    .unwrap()
}
#[test]
fn callee_entry_flags_observers_match_host_oracle_with_lf_and_crlf() {
    let lf = include_str!("../../../fixtures/runtime/callee_entry_flags.act").replace("\r\n", "\n");
    for opt in [false, true] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut observations = std::collections::BTreeMap::new();
                for enabled in [false, true] {
                    let mut first = None;
                    for crlf in [false, true] {
                        let text = if crlf {
                            lf.replace('\n', "\r\n")
                        } else {
                            lf.clone()
                        };
                        let out = compile(&text, opt, rt, origin, enabled);
                        let image = actionc::codegen::format_load_file(&out);
                        if crlf {
                            assert_eq!(first.as_ref(), Some(&image));
                        } else {
                            first = Some(image.clone());
                        }
                        if opt && !crlf {
                            if let Some(dir) =
                                std::env::var_os("ACTIONC_CALLEE_ENTRY_FLAGS_MEASURED_IMAGES")
                            {
                                assert_eq!(
                                    image,
                                    std::fs::read(PathBuf::from(dir).join(format!(
                                        "entry-neutral-{origin:04X}-{rt:?}-{enabled}.xex"
                                    )))
                                    .unwrap()
                                );
                            }
                        }
                        if opt && !crlf && !enabled {
                            if let Some(dir) =
                                std::env::var_os("ACTIONC_CALLEE_ENTRY_FLAGS_BASELINE")
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
                        let address = |name: &str| {
                            out.routine_addresses
                                .iter()
                                .find(|r| r.name == name)
                                .unwrap()
                                .address
                        };
                        let (mut vm, profile) = support::vm_for(rt);
                        vm.load_atari_object_for_execution(profile, &image).unwrap();
                        let mut watched = vec![0x680u16, 0x681, 0x682];
                        for name in ["Forward", "UnsafeForward"] {
                            watched.push(
                                out.map
                                    .storage_symbols
                                    .iter()
                                    .find(|s| {
                                        s.name == "tag"
                                            && s.scope
                                                == actionc::codegen::CodegenSymbolScope::Routine(
                                                    name.into(),
                                                )
                                    })
                                    .unwrap()
                                    .address,
                            );
                        }
                        for &a in &watched {
                            vm.bus_mut().add_watchpoint(a);
                        }

                        // Called by the common harness with tag/status in A/X. PLP sets
                        // all tracked flags after the final tag load, immediately before JSR.
                        for mask in 0..16u8 {
                            for tag in
                                if opt && rt == Runtime::ActionCart && origin == 0x3000 && !crlf {
                                    (0..=255).collect::<Vec<u8>>()
                                } else {
                                    vec![0u8, 1, 127, 128, 254, 255]
                                }
                            {
                                for name in ["Forward", "UnsafeForward"] {
                                    let status = 0x20
                                        | (mask & 1)
                                        | (mask & 2)
                                        | ((mask & 4) << 5)
                                        | ((mask & 8) << 3);
                                    let a = address(name);
                                    vm.bus_mut()
                                        .ram_mut()
                                        .map(
                                            0x740,
                                            &[
                                                0x8D,
                                                0x60,
                                                7,
                                                0x8A,
                                                0x48,
                                                0xAD,
                                                0x60,
                                                7,
                                                0x28,
                                                0x20,
                                                a as u8,
                                                (a >> 8) as u8,
                                                0x60,
                                            ],
                                        )
                                        .unwrap();
                                    vm.bus_mut().ram_mut().write(0x67F, 0xA5);
                                    vm.bus_mut().ram_mut().write(0x683, 0x5A);
                                    vm.bus_mut().clear_events();
                                    let cycles_before = vm.cpu().cycles();
                                    vm = machine::call(vm, &mut Hooks, 0x740, &[tag, status]);
                                    if opt
                                        && !crlf
                                        && mask == 0
                                        && tag == 1
                                        && std::env::var_os("ACTIONC_CALLEE_ENTRY_FLAGS_METRICS")
                                            .is_some()
                                    {
                                        eprintln!("entry-flags-neutral,{name},{rt:?},{origin:04X},{enabled},cycles={}", vm.cpu().cycles()-cycles_before);
                                    }

                                    let writes = vm
                                        .bus()
                                        .events()
                                        .iter()
                                        .filter(|e| e.access == BusAccess::Write)
                                        .map(|e| {
                                            (
                                                watched
                                                    .iter()
                                                    .position(|a| *a == e.address)
                                                    .unwrap(),
                                                e.value,
                                            )
                                        })
                                        .collect::<Vec<_>>();
                                    let r = vm.cpu().registers();
                                    let state = (writes, (r.a, r.x, r.y, r.status, r.sp));
                                    let key = (crlf, mask, tag, name);
                                    if enabled {
                                        assert_eq!(observations.get(&key), Some(&state));
                                    } else {
                                        observations.insert(key, state);
                                    }

                                    let expected_nz = if name == "Forward" {
                                        if tag & 0x7F == 0 {
                                            2
                                        } else {
                                            0
                                        }
                                    } else {
                                        (tag & 0x80) | u8::from(tag == 0) * 2
                                    };
                                    assert_eq!(
                                        vm.bus().ram().read(0x682) & 0xC3,
                                        (status & 0x41) | expected_nz,
                                        "{name}/{tag}/{status:02X}"
                                    );
                                    if name == "Forward" {
                                        assert_eq!(machine::bytes(&vm, 0x680, 2), [0, tag & 0x7F]);
                                    }
                                    assert_eq!(vm.bus().ram().read(0x67F), 0xA5);
                                    assert_eq!(vm.bus().ram().read(0x683), 0x5A);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn callee_entry_flags_compose_with_preceding_feature_combinations() {
    let source =
        include_str!("../../../fixtures/runtime/callee_entry_flags.act").replace("\r\n", "\n");
    for bits in 0..8 {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            let mut old = None;
            for enabled in [false, true] {
                let out = compile_flags(
                    &source,
                    true,
                    rt,
                    0x3000,
                    enabled,
                    [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0],
                );
                let image = actionc::codegen::format_load_file(&out);
                if enabled {
                    assert_eq!(old.unwrap() - image.len(), 2);
                } else {
                    old = Some(image.len());
                }
                let (mut vm, profile) = support::vm_for(rt);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                for (name, expected) in [("Forward", 0x43), ("UnsafeForward", 0xC1)] {
                    let a = out
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
                                0x61,
                                0x48,
                                0xA9,
                                0x80,
                                0x28,
                                0x20,
                                a as u8,
                                (a >> 8) as u8,
                                0x60,
                            ],
                        )
                        .unwrap();
                    vm = machine::call(vm, &mut Hooks, 0x740, &[]);
                    assert_eq!(
                        vm.bus().ram().read(0x682) & 0xC3,
                        expected,
                        "{bits}/{rt:?}/{enabled}/{name}"
                    );
                }
            }
        }
    }
}
