//! Independent values, retained writes and full state at the callee observer.
use actionc::{
    codegen::CodegenOutput,
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
fn compile(text: &str, opt: bool, rt: Runtime, origin: u16, enabled: bool) -> CodegenOutput {
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
            enable_argument_x_forwarding: enabled,
            ..Mir6502Config::optimized()
        },
        rt,
    )
    .unwrap()
}
#[derive(Default)]
struct Observe {
    entry: u16,
    byte_entry: u16,
    args: Vec<Vec<u8>>,
    byte_args: Vec<[u8; 2]>,
    states: Vec<(u8, u8)>,
    writes: Vec<Vec<(u16, u8)>>,
    targets: Vec<u16>,
    reads: Vec<Vec<u16>>,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.byte_entry {
            self.byte_args.push([r.a, r.x]);
        }
        if r.pc == self.entry {
            self.args.push(vec![
                r.a,
                r.x,
                r.y,
                vm.bus().ram().read(0xA3),
                vm.bus().ram().read(0xA4),
            ]);
            self.states.push((r.sp, r.status));
            self.writes.push(
                vm.bus()
                    .events()
                    .iter()
                    .filter(|e| e.access == BusAccess::Write)
                    .map(|e| (e.address, e.value))
                    .collect(),
            );
            self.reads.push(
                vm.bus()
                    .events()
                    .iter()
                    .filter(|e| e.access == BusAccess::Read && self.targets.contains(&e.address))
                    .map(|e| e.address)
                    .collect(),
            );
        }
        Ok(())
    }
}
#[test]
fn argument_x_forwarding_matches_runtime_oracles_with_lf_and_crlf() {
    let lf =
        include_str!("../../../fixtures/runtime/argument_x_forwarding.act").replace("\r\n", "\n");
    for opt in [false, true] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut baseline = std::collections::BTreeMap::new();
                for enabled in [false, true] {
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
                            if let Some(dir) = std::env::var_os("ACTIONC_ARGUMENT_X_BASELINE") {
                                let rt = if rt == Runtime::ActionCart {
                                    "cart"
                                } else {
                                    "standalone"
                                };
                                assert_eq!(
                                    image,
                                    std::fs::read(
                                        PathBuf::from(dir)
                                            .join(format!("neutral-{rt}-{origin:04X}.xex"))
                                    )
                                    .unwrap()
                                );
                            }
                        }
                        let entry = |n: &str| {
                            out.routine_addresses
                                .iter()
                                .find(|r| r.name == n)
                                .unwrap()
                                .address
                        };
                        let (mut vm, profile) = support::vm_for(rt);
                        vm.load_atari_object_for_execution(profile, &image).unwrap();
                        for a in (0xA0..=0xAF).chain([0x7000, 0x7001, 0x7200, 0x72FF]) {
                            vm.bus_mut().add_watchpoint(a);
                        }
                        for q in [0x7200u16, 0x72FF] {
                            for b in [0u8, 1, 127, 128, 254, 255] {
                                for name in ["Forward", "ForwardByte", "ForwardPrivate", "Plain"] {
                                    let p = 0x7000u16;
                                    let high = b ^ 0x5A;
                                    let tag = b ^ 0xA5;
                                    vm.bus_mut().ram_mut().write(p, b);
                                    vm.bus_mut().ram_mut().write(p + 1, high);
                                    vm.bus_mut().ram_mut().write(q, tag);
                                    vm.bus_mut().ram_mut().write(0x687, 0xD6);
                                    let mut args = p.to_le_bytes().to_vec();
                                    args.extend(q.to_le_bytes());
                                    if name == "ForwardByte" {
                                        args = vec![b];
                                    } else if name != "Forward" {
                                        args.push(b);
                                    }
                                    let (first, second, last) = match name {
                                        "Forward" => (p + u16::from(b) + 1, q + 1, tag),
                                        "ForwardByte" => (0, u16::from(b), b),
                                        "ForwardPrivate" => (u16::from_le_bytes([b, high]), q, b),
                                        _ => (p + 1, q, b),
                                    };
                                    let mut expected = first.to_le_bytes().to_vec();
                                    expected.extend(second.to_le_bytes());
                                    expected.push(last);
                                    let mut hooks = Observe {
                                        entry: entry("Observe"),
                                        byte_entry: entry("ObserveBytes"),
                                        targets: vec![p, p + 1, q],
                                        ..Observe::default()
                                    };
                                    vm.bus_mut().clear_events();
                                    let started = vm.cpu().cycles();
                                    vm = machine::call(vm, &mut hooks, entry(name), &args);
                                    assert_eq!(
                                        hooks.args,
                                        vec![expected.clone()],
                                        "{name}/{q:04X}/{b}/{opt}/{rt:?}"
                                    );
                                    assert_eq!(machine::bytes(&vm, 0x680, 5), expected);
                                    assert_eq!(vm.bus().ram().read(0x687), 0xD6);
                                    assert_eq!(
                                        hooks.reads,
                                        vec![match name {
                                            "Forward" => vec![p, q],
                                            "ForwardPrivate" => vec![p, p + 1],
                                            _ => vec![],
                                        }]
                                    );
                                    if name == "ForwardByte" {
                                        assert_eq!(hooks.byte_args, vec![[0, b]]);
                                        assert_eq!(machine::bytes(&vm, 0x685, 2), vec![0, b]);
                                    }
                                    let cycles = vm.cpu().cycles() - started;
                                    if !crlf {
                                        let key = (q, b, name);
                                        if !enabled {
                                            baseline.insert(
                                                key,
                                                (
                                                    cycles,
                                                    hooks.states.clone(),
                                                    hooks.writes.clone(),
                                                    hooks.reads.clone(),
                                                ),
                                            );
                                        } else {
                                            let old = &baseline[&key];
                                            assert_eq!(hooks.states, old.1);
                                            assert_eq!(hooks.writes, old.2);
                                            assert_eq!(hooks.reads, old.3);
                                            assert_eq!(
                                                cycles
                                                    + match name {
                                                        "ForwardByte" => 2,
                                                        "Plain" => 0,
                                                        _ => 1,
                                                    },
                                                old.0
                                            );
                                        }
                                    }
                                    if std::env::var_os("ACTIONC_ARGUMENT_X_METRICS").is_some()
                                        && !crlf
                                        && q == 0x7200
                                        && b == 1
                                    {
                                        eprintln!(
                                            "argument-x,{name},{rt:?},opt={opt},origin={origin:04X},enabled={enabled},cycles={}",
                                            vm.cpu().cycles() - started
                                        );
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
