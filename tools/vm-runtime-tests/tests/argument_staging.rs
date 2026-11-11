//! Independent observation of canonical argument bytes before the callee prologue.
use actionc::{
    codegen::CodegenOutput,
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
#[path = "support/argument_staging_effects.rs"]
mod effects;

fn compile(
    source: &str,
    optimized: bool,
    runtime: Runtime,
    origin: u16,
    enabled: bool,
) -> CodegenOutput {
    let source = support::Source::new(source);
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
        &Mir6502Config {
            enable_argument_staging_reduction: enabled,
            ..Mir6502Config::optimized()
        },
        runtime,
    )
    .unwrap()
}

struct Observe {
    callees: BTreeMap<u16, usize>,
    arguments: Vec<Vec<u8>>,
    states: Vec<(u8, u8)>,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if let Some(&bytes) = self.callees.get(&r.pc) {
            let mut args = vec![
                r.a,
                r.x,
                r.y,
                vm.bus().ram().read(0xA3),
                vm.bus().ram().read(0xA4),
            ];
            args.truncate(bytes);
            self.arguments.push(args);
            self.states.push((r.sp, r.status));
        }
        Ok(())
    }
}

#[test]
fn argument_staging_matches_entry_bytes_and_host_oracles_with_lf_and_crlf() {
    let text = include_str!("../../../fixtures/runtime/argument_staging.act").replace("\r\n", "\n");
    let words = [0u16, 1, 0xFF, 0x100, 0x7FFF, 0x8000, 0xFF00, 0xFFFF];
    for optimized in [false, true] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut baseline = BTreeMap::new();
                for enabled in [false, true] {
                    let mut lf = None;
                    for crlf in [false, true] {
                        let source = if crlf {
                            text.replace('\n', "\r\n")
                        } else {
                            text.clone()
                        };
                        let output = compile(&source, optimized, runtime, origin, enabled);
                        if let Some(expected) = &lf {
                            assert_eq!(&output.bytes, expected);
                        } else {
                            lf = Some(output.bytes.clone());
                        }
                        let entry = |name: &str| {
                            output
                                .routine_addresses
                                .iter()
                                .find(|r| r.name == name)
                                .unwrap()
                                .address
                        };
                        let mut hooks = Observe {
                            callees: [(entry("Capture"), 5), (entry("CapturePair"), 4)].into(),
                            arguments: Vec::new(),
                            states: Vec::new(),
                        };
                        let (mut vm, profile) = support::vm_for(runtime);
                        vm.load_atari_object_for_execution(
                            profile,
                            &actionc::codegen::format_load_file(&output),
                        )
                        .unwrap();
                        for address in (0xA0..=0xA7).chain(0xE0..=0xE1).chain(0x680..=0x684) {
                            vm.bus_mut().add_watchpoint(address);
                        }
                        for name in ["ForwardAdd", "ForwardSub", "FourByte", "FivePlain"] {
                            for (n, &first) in words.iter().enumerate() {
                                let second = words[(n + 3) % words.len()];
                                for tag in [0u8, 1, 127, 128, 254, 255] {
                                    for address in 0xA0..=0xA7 {
                                        vm.bus_mut().ram_mut().write(address, 0xD6);
                                    }
                                    let mut args = first.to_le_bytes().to_vec();
                                    args.extend(second.to_le_bytes());
                                    if name != "FourByte" {
                                        args.push(tag);
                                    }
                                    let expected_first = match name {
                                        "ForwardAdd" | "FourByte" => first.wrapping_add(1),
                                        "ForwardSub" => first.wrapping_sub(1),
                                        _ => first,
                                    };
                                    let mut expected = expected_first.to_le_bytes().to_vec();
                                    expected.extend(second.to_le_bytes());
                                    if name != "FourByte" {
                                        expected.push(tag);
                                    }
                                    hooks.arguments.clear();
                                    hooks.states.clear();
                                    vm.bus_mut().clear_events();
                                    let start = vm.cpu().cycles();
                                    vm = machine::call(vm, &mut hooks, entry(name), &args);
                                    let cycles = vm.cpu().cycles() - start;
                                    let key = (name, n, tag);
                                    let writes = vm
                                        .bus()
                                        .events()
                                        .iter()
                                        .filter(|e| e.access == BusAccess::Write)
                                        .map(|e| (e.address, e.value))
                                        .collect::<Vec<_>>();
                                    if !crlf {
                                        if !enabled {
                                            baseline.insert(
                                                key,
                                                (cycles, hooks.states.clone(), writes),
                                            );
                                        } else if name == "ForwardAdd" || name == "ForwardSub" {
                                            assert!(cycles < baseline[&key].0);
                                            assert_eq!(hooks.states, baseline[&key].1);
                                            assert_eq!(writes, baseline[&key].2);
                                        } else {
                                            assert_eq!(cycles, baseline[&key].0);
                                            assert_eq!(hooks.states, baseline[&key].1);
                                            assert_eq!(writes, baseline[&key].2);
                                        }
                                    }
                                    assert_eq!(
                                        hooks.arguments,
                                        vec![expected.clone()],
                                        "{name}/{runtime:?}/{optimized}/{first:04X}/{second:04X}/{tag}"
                                    );
                                    assert_eq!(
                                        machine::bytes(&vm, 0x680, expected.len()),
                                        expected
                                    );
                                    if std::env::var_os("ACTIONC_ARGUMENT_METRICS").is_some()
                                        && first == 0xFF
                                        && tag == 128
                                        && !crlf
                                    {
                                        eprintln!(
                                            "argument-staging,{name},{runtime:?},optimized={optimized},origin={origin:04X},enabled={enabled},cycles={cycles}"
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
