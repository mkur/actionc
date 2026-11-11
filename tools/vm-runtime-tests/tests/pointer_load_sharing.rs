//! Runtime source values, ordered reads and the complete callee-entry ABI.
use actionc::{
    codegen::{CodegenOutput, CodegenSymbolScope},
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

fn compile(text: &str, optimized: bool, runtime: Runtime, origin: u16) -> CodegenOutput {
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
    let nir = if optimized {
        actionc::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    mir6502::generate_output_with_config_and_runtime(
        &nir,
        origin,
        &Mir6502Config::optimized(),
        runtime,
    )
    .unwrap()
}
#[derive(Default)]
struct Observe {
    entry: u16,
    source_home: u16,
    args: Vec<Vec<u8>>,
    states: Vec<(u8, u8)>,
    reads: Vec<Vec<u16>>,
    abi: Vec<Vec<(u16, u8)>>,
    pointer: Vec<Vec<(u16, u8)>>,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.entry {
            self.args.push(vec![
                r.a,
                r.x,
                r.y,
                vm.bus().ram().read(0xA3),
                vm.bus().ram().read(0xA4),
            ]);
            self.states.push((r.sp, r.status));
            let events = vm.bus().events();
            self.reads.push(
                events
                    .iter()
                    .filter(|e| {
                        e.access == BusAccess::Read
                            && (self.source_home..self.source_home + 2).contains(&e.address)
                    })
                    .map(|e| e.address)
                    .collect(),
            );
            self.abi.push(
                events
                    .iter()
                    .filter(|e| e.access == BusAccess::Write && (0xA0..=0xA4).contains(&e.address))
                    .map(|e| (e.address, e.value))
                    .collect(),
            );
            self.pointer.push(
                events
                    .iter()
                    .filter(|e| e.access == BusAccess::Write && (0xAC..=0xAD).contains(&e.address))
                    .map(|e| (e.address, e.value))
                    .collect(),
            );
        }
        Ok(())
    }
}

#[test]
fn pointer_load_sharing_observes_runtime_arguments_and_reads_with_lf_and_crlf() {
    let lf =
        include_str!("../../../fixtures/runtime/pointer_load_sharing.act").replace("\r\n", "\n");
    for optimized in [false, true] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut lf_image = None;
                for crlf in [false, true] {
                    let text = if crlf {
                        lf.replace('\n', "\r\n")
                    } else {
                        lf.clone()
                    };
                    let output = compile(&text, optimized, runtime, origin);
                    let image = actionc::codegen::format_load_file(&output);
                    if crlf {
                        assert_eq!(lf_image.as_ref(), Some(&image));
                    } else {
                        lf_image = Some(image.clone());
                    }
                    if optimized && !crlf {
                        if let Some(dir) = std::env::var_os("ACTIONC_POINTER_LOAD_BASELINE") {
                            let rt = if runtime == Runtime::ActionCart {
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
                    let entry = |name: &str| {
                        output
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address
                    };
                    let homes = output
                        .map
                        .storage_symbols
                        .iter()
                        .filter_map(|s| match &s.scope {
                            CodegenSymbolScope::Routine(owner) if s.name == "q" => {
                                Some((owner.clone(), s.address))
                            }
                            _ => None,
                        })
                        .collect::<BTreeMap<_, _>>();
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    for address in homes
                        .values()
                        .flat_map(|&h| [h, h + 1])
                        .chain(0xA0..=0xA4)
                        .chain(0xAC..=0xAD)
                    {
                        vm.bus_mut().add_watchpoint(address);
                    }
                    for q in [0x7200u16, 0x72FF] {
                        for b in [0u8, 1, 127, 128, 254, 255] {
                            for name in [
                                "Forward", "PlusZero", "PlusTwo", "Subtract", "Offset", "Plain",
                            ] {
                                let p = 0x7000u16;
                                let tag = b ^ 0xA5;
                                let offset_tag = tag ^ 0x3C;
                                vm.bus_mut().ram_mut().write(p, b);
                                vm.bus_mut().ram_mut().write(q, tag);
                                vm.bus_mut().ram_mut().write(q + 1, offset_tag);
                                vm.bus_mut().ram_mut().write(0x685, 0xD6);
                                let mut args = p.to_le_bytes().to_vec();
                                args.extend(q.to_le_bytes());
                                let first = if name == "Plain" {
                                    p + 1
                                } else {
                                    p + u16::from(b) + 1
                                };
                                let second = match name {
                                    "PlusZero" => q,
                                    "PlusTwo" => q + 2,
                                    "Subtract" => q - 1,
                                    _ => q + 1,
                                };
                                let mut expected = first.to_le_bytes().to_vec();
                                expected.extend(second.to_le_bytes());
                                expected.push(if name == "Offset" { offset_tag } else { tag });
                                let mut hooks = Observe {
                                    entry: entry("Observe"),
                                    source_home: homes[name],
                                    ..Observe::default()
                                };
                                vm.bus_mut().clear_events();
                                let started = vm.cpu().cycles();
                                vm = machine::call(vm, &mut hooks, entry(name), &args);
                                assert_eq!(
                                    hooks.args,
                                    vec![expected.clone()],
                                    "{name}/{q:04X}/{b}"
                                );
                                assert_eq!(machine::bytes(&vm, 0x680, 5), expected);
                                assert_eq!(vm.bus().ram().read(0x685), 0xD6);
                                assert_eq!(vm.bus().ram().read(p), b);
                                assert_eq!(vm.bus().ram().read(q), tag);
                                if name == "Forward" {
                                    let h = homes[name];
                                    assert_eq!(hooks.reads, vec![vec![h, h + 1, h, h + 1]]);
                                }
                                if std::env::var_os("ACTIONC_POINTER_LOAD_METRICS").is_some()
                                    && !crlf
                                    && q == 0x7200
                                    && b == 1
                                {
                                    eprintln!(
                                        "pointer-load-sharing,{name},{runtime:?},optimized={optimized},origin={origin:04X},cycles={}",
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
