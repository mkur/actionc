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

fn compile(
    text: &str,
    optimized: bool,
    runtime: Runtime,
    origin: u16,
    enabled: bool,
) -> CodegenOutput {
    compile_with_fusion(text, optimized, runtime, origin, enabled, true)
}
fn compile_with_fusion(
    text: &str,
    optimized: bool,
    runtime: Runtime,
    origin: u16,
    enabled: bool,
    fusion: bool,
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
            enable_pointer_load_sharing: enabled,
            enable_pointer_byte_plus_one_fusion: fusion,
            ..Mir6502Config::optimized()
        },
        runtime,
    )
    .unwrap()
}
#[derive(Default)]
struct Observe {
    targets: Vec<u16>,
    indirect: Vec<Vec<u16>>,
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
            self.indirect.push(
                events
                    .iter()
                    .filter(|e| e.access == BusAccess::Read && self.targets.contains(&e.address))
                    .map(|e| e.address)
                    .collect(),
            );
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
                let mut baseline = BTreeMap::new();
                for enabled in [false, true] {
                    let mut lf_image = None;
                    for crlf in [false, true] {
                        let text = if crlf {
                            lf.replace('\n', "\r\n")
                        } else {
                            lf.clone()
                        };
                        let output = compile(&text, optimized, runtime, origin, enabled);
                        let image = actionc::codegen::format_load_file(&output);
                        if crlf {
                            assert_eq!(lf_image.as_ref(), Some(&image));
                        } else {
                            lf_image = Some(image.clone());
                        }
                        if optimized && !crlf {
                            if let Some(dir) =
                                std::env::var_os("ACTIONC_POINTER_LOAD_MEASURED_IMAGES")
                            {
                                assert_eq!(
                                    image,
                                    std::fs::read(PathBuf::from(dir).join(format!(
                                        "neutral-{origin:04X}-{runtime:?}-{enabled}.xex"
                                    )))
                                    .unwrap()
                                );
                            }
                        }
                        if optimized && !crlf && !enabled {
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
                        for address in [0x7000, 0x7200, 0x7201, 0x72FF, 0x7300] {
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
                                        targets: vec![p, q, q + 1],
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
                                    assert_eq!(
                                        hooks.indirect,
                                        vec![if name == "Plain" {
                                            vec![q]
                                        } else if name == "Offset" {
                                            vec![p, q + 1]
                                        } else {
                                            vec![p, q]
                                        }]
                                    );
                                    assert_eq!(vm.bus().ram().read(0x685), 0xD6);
                                    assert_eq!(vm.bus().ram().read(p), b);
                                    assert_eq!(vm.bus().ram().read(q), tag);
                                    if name == "Forward" {
                                        let h = homes[name];
                                        assert_eq!(
                                            hooks.reads,
                                            vec![if enabled {
                                                vec![h, h + 1]
                                            } else {
                                                vec![h, h + 1, h, h + 1]
                                            }]
                                        );
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
                                                    hooks.abi.clone(),
                                                    hooks.pointer.clone(),
                                                    hooks.reads.clone(),
                                                ),
                                            );
                                        } else {
                                            let old = &baseline[&key];
                                            assert_eq!(hooks.states, old.1);
                                            assert_eq!(hooks.abi, old.2);
                                            assert_eq!(hooks.pointer, old.3);
                                            if name == "Forward" {
                                                assert_eq!(cycles + 8, old.0);
                                            } else {
                                                assert_eq!(cycles, old.0);
                                                assert_eq!(hooks.reads, old.4);
                                            }
                                        }
                                    }
                                    if std::env::var_os("ACTIONC_POINTER_LOAD_METRICS").is_some()
                                        && !crlf
                                        && q == 0x7200
                                        && b == 1
                                    {
                                        eprintln!(
                                            "pointer-load-sharing,{name},{runtime:?},optimized={optimized},origin={origin:04X},enabled={enabled},cycles={}",
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
fn pointer_load_sharing_preserves_indirect_aliases_of_abi_scratch_and_parameters() {
    let text =
        include_str!("../../../fixtures/runtime/pointer_load_sharing.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41C7] {
            let mut baseline: BTreeMap<
                usize,
                (Vec<(u8, u8)>, Vec<Vec<(u16, u8)>>, Vec<Vec<(u16, u8)>>),
            > = BTreeMap::new();
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
                                && s.scope == CodegenSymbolScope::Routine("Forward".into())
                        })
                        .unwrap()
                        .address
                };
                let ph = home("p");
                let qh = home("q");
                for (case, q) in [
                    0xA0u16,
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
                    let p = 0x7000u16;
                    let first = p + 4;
                    let second = q.wrapping_add(1);
                    let mut expected_memory = BTreeMap::from([
                        (q, 0xD6),
                        (ph, p as u8),
                        (ph + 1, (p >> 8) as u8),
                        (qh, q as u8),
                        (qh + 1, (q >> 8) as u8),
                        (0xAE, p as u8),
                        (0xAF, (p >> 8) as u8),
                        (0xA0, first as u8),
                        (0xA1, (first >> 8) as u8),
                        (0xA2, second as u8),
                        (0xA3, (second >> 8) as u8),
                        (0xAC, q as u8),
                        (0xAD, (q >> 8) as u8),
                    ]);
                    let tag = expected_memory.remove(&q).unwrap();
                    let mut expected = first.to_le_bytes().to_vec();
                    expected.extend(second.to_le_bytes());
                    expected.push(tag);
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(
                        profile,
                        &actionc::codegen::format_load_file(&output),
                    )
                    .unwrap();
                    for a in 0xA0..=0xAF {
                        vm.bus_mut().ram_mut().write(a, 0xD6);
                        vm.bus_mut().add_watchpoint(a);
                    }
                    vm.bus_mut().ram_mut().write(q, 0xD6);
                    vm.bus_mut().ram_mut().write(p, 3);
                    let mut args = p.to_le_bytes().to_vec();
                    args.extend(q.to_le_bytes());
                    let mut hooks = Observe {
                        entry: entry("Observe"),
                        source_home: qh,
                        ..Observe::default()
                    };
                    vm = machine::call(vm, &mut hooks, entry("Forward"), &args);
                    assert_eq!(
                        hooks.args,
                        vec![expected.clone()],
                        "{case}/{q:04X}/{enabled}/{runtime:?}"
                    );
                    assert_eq!(machine::bytes(&vm, 0x680, 5), expected);
                    if enabled {
                        let old = &baseline[&case];
                        assert_eq!(
                            (&hooks.states, &hooks.abi, &hooks.pointer),
                            (&old.0, &old.1, &old.2)
                        );
                    } else {
                        baseline.insert(case, (hooks.states, hooks.abi, hooks.pointer));
                    }
                }
            }
        }
    }
}

#[test]
fn pointer_load_sharing_composes_with_pointer_increment_fusion() {
    let text =
        include_str!("../../../fixtures/runtime/pointer_load_sharing.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let mut sizes = BTreeMap::new();
        for fusion in [false, true] {
            for sharing in [false, true] {
                let output = compile_with_fusion(&text, true, runtime, 0x3000, sharing, fusion);
                let image = actionc::codegen::format_load_file(&output);
                sizes.insert((fusion, sharing), image.len());
                let entry = |name: &str| {
                    output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                vm.bus_mut().ram_mut().write(0x7000, 1);
                vm.bus_mut().ram_mut().write(0x72FF, 255);
                let mut hooks = Observe {
                    entry: entry("Observe"),
                    ..Observe::default()
                };
                vm = machine::call(vm, &mut hooks, entry("Forward"), &[0, 0x70, 0xFF, 0x72]);
                assert_eq!(machine::bytes(&vm, 0x680, 5), vec![2, 0x70, 0, 0x73, 255]);
            }
            assert_eq!(sizes[&(fusion, false)] - sizes[&(fusion, true)], 6);
        }
    }
}
