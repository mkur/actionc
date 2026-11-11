//! Independent argument/result and exactly-once source-read observation.
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

fn compile(
    text: &str,
    optimized: bool,
    runtime: Runtime,
    origin: u16,
    enabled: bool,
) -> CodegenOutput {
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
        &Mir6502Config {
            enable_argument_x_forwarding: false,
            enable_pointer_load_sharing: false,
            enable_pointer_byte_plus_one_fusion: enabled,
            ..Mir6502Config::optimized()
        },
        runtime,
    )
    .unwrap()
}
struct Observe {
    entry: u16,
    args: Vec<Vec<u8>>,
    states: Vec<(u8, u8)>,
    carriers: Vec<Vec<u16>>,
    pairs: Vec<[u8; 2]>,
    scratch: Vec<[usize; 4]>,
    staging: Vec<Vec<(u16, u8)>>,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        if r.pc == self.entry {
            self.states.push((r.sp, r.status));
            let events = vm.bus().events();
            self.scratch.push([
                events
                    .iter()
                    .filter(|e| e.address == 0xAC && e.access == BusAccess::Read)
                    .count(),
                events
                    .iter()
                    .filter(|e| e.address == 0xAC && e.access == BusAccess::Write)
                    .count(),
                events
                    .iter()
                    .filter(|e| e.address == 0xAD && e.access == BusAccess::Read)
                    .count(),
                events
                    .iter()
                    .filter(|e| e.address == 0xAD && e.access == BusAccess::Write)
                    .count(),
            ]);
            self.staging.push(
                events
                    .iter()
                    .filter(|e| (0xA0..=0xA4).contains(&e.address) && e.access == BusAccess::Write)
                    .map(|e| (e.address, e.value))
                    .collect(),
            );
            self.pairs
                .push([vm.bus().ram().read(0xAE), vm.bus().ram().read(0xAF)]);
            self.carriers.push(
                vm.bus()
                    .events()
                    .iter()
                    .filter(|e| e.access == BusAccess::Read && (0xAE..=0xAF).contains(&e.address))
                    .map(|e| e.address)
                    .collect(),
            );
            self.args.push(vec![
                r.a,
                r.x,
                r.y,
                vm.bus().ram().read(0xA3),
                vm.bus().ram().read(0xA4),
            ]);
        }
        Ok(())
    }
}

#[test]
fn pointer_byte_plus_one_matches_host_oracles_and_ordered_reads_with_lf_and_crlf() {
    let text =
        include_str!("../../../fixtures/runtime/pointer_byte_plus_one.act").replace("\r\n", "\n");
    for optimized in [false, true] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut baseline = BTreeMap::new();
                for enabled in [false, true] {
                    let mut lf = None;
                    for crlf in [false, true] {
                        let text = if crlf {
                            text.replace('\n', "\r\n")
                        } else {
                            text.clone()
                        };
                        let output = compile(&text, optimized, runtime, origin, enabled);
                        let image = actionc::codegen::format_load_file(&output);
                        if optimized && !crlf {
                            if let Some(dir) = std::env::var_os("ACTIONC_POINTER_MEASURED_IMAGES") {
                                assert_eq!(
                                    image,
                                    std::fs::read(PathBuf::from(dir).join(format!(
                                        "neutral-{origin:04X}-{runtime:?}-{enabled}.xex"
                                    )))
                                    .unwrap()
                                );
                            }
                        }
                        if let Some(expected) = &lf {
                            assert_eq!(&image, expected);
                        } else {
                            lf = Some(image.clone());
                        }
                        if optimized && !crlf && !enabled {
                            if let Some(dir) =
                                std::env::var_os("ACTIONC_POINTER_INCREMENT_BASELINE")
                            {
                                let runtime = if runtime == Runtime::ActionCart {
                                    "cart"
                                } else {
                                    "standalone"
                                };
                                assert_eq!(
                                    image,
                                    std::fs::read(
                                        PathBuf::from(dir)
                                            .join(format!("neutral-{runtime}-{origin:04X}.xex"))
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
                        for p in [0x7000u16, 0x70FF] {
                            let q = 0x7200u16;
                            let (mut vm, profile) = support::vm_for(runtime);
                            vm.load_atari_object_for_execution(profile, &image).unwrap();
                            vm.bus_mut().add_watchpoint(p);
                            vm.bus_mut().add_watchpoint(q);
                            vm.bus_mut().add_watchpoint(0xAE);
                            vm.bus_mut().add_watchpoint(0xAF);
                            for address in (0xA0..=0xA4).chain(0xAC..=0xAD) {
                                vm.bus_mut().add_watchpoint(address);
                            }
                            let mut hooks = Observe {
                                entry: entry("Observe"),
                                args: Vec::new(),
                                states: Vec::new(),
                                carriers: Vec::new(),
                                pairs: Vec::new(),
                                scratch: Vec::new(),
                                staging: Vec::new(),
                            };
                            for b in [0u8, 1, 127, 128, 254, 255] {
                                for name in ["Forward", "PlusZero", "PlusTwo", "Subtract", "Plain"]
                                {
                                    let tag = b ^ 0xA5;
                                    vm.bus_mut().ram_mut().write(p, b);
                                    vm.bus_mut().ram_mut().write(q, tag);
                                    for a in 0xA0..=0xAF {
                                        vm.bus_mut().ram_mut().write(a, 0xD6);
                                    }
                                    let mut args = p.to_le_bytes().to_vec();
                                    args.extend(q.to_le_bytes());
                                    let result = match name {
                                        "Forward" => p.wrapping_add(u16::from(b)).wrapping_add(1),
                                        "PlusZero" => p.wrapping_add(u16::from(b)),
                                        "PlusTwo" => p.wrapping_add(u16::from(b)).wrapping_add(2),
                                        "Subtract" => p.wrapping_sub(u16::from(b)).wrapping_add(1),
                                        _ => p.wrapping_add(1),
                                    };
                                    let mut expected = result.to_le_bytes().to_vec();
                                    expected.extend(q.wrapping_add(1).to_le_bytes());
                                    expected.push(tag);
                                    hooks.args.clear();
                                    hooks.states.clear();
                                    hooks.carriers.clear();
                                    hooks.pairs.clear();
                                    hooks.scratch.clear();
                                    hooks.staging.clear();
                                    vm.bus_mut().clear_events();
                                    let started = vm.cpu().cycles();
                                    vm = machine::call(vm, &mut hooks, entry(name), &args);
                                    let cycles = vm.cpu().cycles() - started;
                                    if !crlf {
                                        let key = (p, b, name);
                                        if !enabled {
                                            baseline.insert(
                                                key,
                                                (
                                                    cycles,
                                                    hooks.states.clone(),
                                                    hooks.carriers.clone(),
                                                    hooks.scratch.clone(),
                                                    hooks.staging.clone(),
                                                ),
                                            );
                                        } else {
                                            assert_eq!(hooks.states, baseline[&key].1);
                                            assert_eq!(hooks.carriers, baseline[&key].2);
                                            assert_eq!(hooks.staging, baseline[&key].4);
                                            if name == "Forward" {
                                                for index in [0usize, 1] {
                                                    assert_eq!(
                                                        hooks.scratch[0][index] + 1,
                                                        baseline[&key].3[0][index]
                                                    );
                                                }
                                                for index in [2usize, 3] {
                                                    assert_eq!(
                                                        hooks.scratch[0][index],
                                                        baseline[&key].3[0][index]
                                                    );
                                                }
                                            } else {
                                                assert_eq!(hooks.scratch, baseline[&key].3);
                                            }
                                            if name == "Forward" {
                                                assert!(cycles < baseline[&key].0);
                                            } else {
                                                assert_eq!(cycles, baseline[&key].0);
                                            }
                                        }
                                    }
                                    assert_eq!(
                                        hooks.args,
                                        vec![expected.clone()],
                                        "{name}/{p:04X}/{b}/{optimized}/{runtime:?}"
                                    );
                                    assert_eq!(machine::bytes(&vm, 0x680, 5), expected);
                                    let reads = vm
                                        .bus()
                                        .events()
                                        .iter()
                                        .filter(|e| {
                                            e.access == BusAccess::Read
                                                && (e.address == p || e.address == q)
                                        })
                                        .map(|e| e.address)
                                        .collect::<Vec<_>>();
                                    assert_eq!(
                                        reads,
                                        if name == "Plain" { vec![q] } else { vec![p, q] },
                                        "{name}: original source reads"
                                    );
                                    if name == "Forward" {
                                        assert_eq!(
                                            hooks.carriers,
                                            vec![vec![0xAE, 0xAE, 0xAF, 0xAF]]
                                        );
                                        assert_eq!(hooks.pairs, vec![p.to_le_bytes()]);
                                    }
                                    assert_eq!(vm.bus().ram().read(p), b);
                                    assert_eq!(vm.bus().ram().read(q), tag);
                                    if std::env::var_os("ACTIONC_POINTER_METRICS").is_some()
                                        && !crlf
                                        && p == 0x7000
                                        && b == 1
                                    {
                                        eprintln!(
                                            "pointer-byte-plus-one,{name},{runtime:?},optimized={optimized},origin={origin:04X},enabled={enabled},cycles={}",
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

#[test]
fn pointer_byte_plus_one_preserves_incoming_abi_scratch_and_parameter_aliases() {
    let text =
        include_str!("../../../fixtures/runtime/pointer_byte_plus_one.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41C7] {
            let mut traces = BTreeMap::new();
            for enabled in [false, true] {
                let output = compile(&text, true, runtime, origin, enabled);
                let entry = |name: &str| {
                    output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                let home = |name: &str| {
                    output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| {
                            s.name == name
                                && s.scope
                                    == actionc::codegen::CodegenSymbolScope::Routine(
                                        "Forward".into(),
                                    )
                        })
                        .unwrap()
                        .address
                };
                let ph = home("p");
                let qh = home("q");
                for (case, p) in [
                    0xA0,
                    0xA1,
                    0xA2,
                    0xA3,
                    0xA4,
                    0xAC,
                    0xAD,
                    0xAE,
                    0xAF,
                    ph,
                    ph + 1,
                    qh,
                    qh + 1,
                ]
                .into_iter()
                .enumerate()
                {
                    let q = 0x7200u16;
                    let b = if p == ph || p == 0xAE {
                        p as u8
                    } else if p == ph + 1 || p == 0xAF {
                        (p >> 8) as u8
                    } else if p == qh {
                        q as u8
                    } else if p == qh + 1 || p == 0xA3 {
                        (q >> 8) as u8
                    } else {
                        0xD6
                    };
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(
                        profile,
                        &actionc::codegen::format_load_file(&output),
                    )
                    .unwrap();
                    for address in 0xA0..=0xAF {
                        vm.bus_mut().ram_mut().write(address, 0xD6);
                    }
                    vm.bus_mut().ram_mut().write(p, 0xD6);
                    vm.bus_mut().ram_mut().write(q, 8);
                    vm.bus_mut().add_watchpoint(0xAE);
                    vm.bus_mut().add_watchpoint(0xAF);
                    let mut args = p.to_le_bytes().to_vec();
                    args.extend(q.to_le_bytes());
                    let mut hooks = Observe {
                        entry: entry("Observe"),
                        args: vec![],
                        states: vec![],
                        carriers: vec![],
                        pairs: vec![],
                        scratch: vec![],
                        staging: vec![],
                    };
                    vm = machine::call(vm, &mut hooks, entry("Forward"), &args);
                    let mut expected = p
                        .wrapping_add(u16::from(b))
                        .wrapping_add(1)
                        .to_le_bytes()
                        .to_vec();
                    expected.extend(q.wrapping_add(1).to_le_bytes());
                    expected.push(8);
                    assert_eq!(
                        hooks.args,
                        vec![expected.clone()],
                        "{p:04X}/{runtime:?}/{enabled}"
                    );
                    assert_eq!(machine::bytes(&vm, 0x680, 5), expected);
                    assert_eq!(hooks.pairs, vec![p.to_le_bytes()]);
                    if enabled {
                        assert_eq!(hooks.carriers, traces[&case]);
                    } else {
                        traces.insert(case, hooks.carriers);
                    }
                }
            }
        }
    }
}
