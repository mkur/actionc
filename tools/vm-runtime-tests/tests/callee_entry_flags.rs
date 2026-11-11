//! C/V-dependent nested observations and an immediate N/Z observer control.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::VmRunHooks;
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
            enable_precise_callee_entry_flags: enabled,
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
                        // Called by the common harness with tag/status in A/X. PLP sets
                        // all tracked flags after the final tag load, immediately before JSR.
                        for mask in 0..16u8 {
                            for tag in [0u8, 1, 127, 128, 254, 255] {
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
                                    vm = machine::call(vm, &mut Hooks, 0x740, &[tag, status]);
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
