//! Independent values, retained writes and full state at the callee observer.
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
fn compile(text: &str, opt: bool, rt: Runtime, origin: u16, enabled: bool) -> CodegenOutput {
    compile_flags(text, opt, rt, origin, enabled, [true, true, true])
}
fn compile_flags(
    text: &str,
    opt: bool,
    rt: Runtime,
    origin: u16,
    enabled: bool,
    predecessors: [bool; 3],
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
    let nir = if opt {
        actionc::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    mir6502::generate_output_with_config_and_runtime(
        &nir,
        origin,
        &Mir6502Config {
            enable_precise_callee_entry_flags: false,
            enable_argument_x_forwarding: enabled,
            enable_argument_staging_reduction: predecessors[0],
            enable_pointer_byte_plus_one_fusion: predecessors[1],
            enable_pointer_load_sharing: predecessors[2],
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
                        if opt && !crlf {
                            if let Some(dir) =
                                std::env::var_os("ACTIONC_ARGUMENT_X_MEASURED_IMAGES")
                            {
                                assert_eq!(
                                    image,
                                    std::fs::read(PathBuf::from(dir).join(format!(
                                        "neutral-{origin:04X}-{rt:?}-{enabled}.xex"
                                    )))
                                    .unwrap()
                                );
                            }
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
                        for a in 0xE0..=0xE2 {
                            vm.bus_mut().add_watchpoint(a);
                        }
                        let tag_home = out
                            .map
                            .storage_symbols
                            .iter()
                            .find(|s| {
                                s.name == "tag"
                                    && s.scope
                                        == actionc::codegen::CodegenSymbolScope::Routine(
                                            "ForwardByte".into(),
                                        )
                            })
                            .unwrap()
                            .address;
                        vm.bus_mut().add_watchpoint(tag_home);
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

#[test]
fn argument_x_forwarding_preserves_reads_of_captures_abi_and_parameter_aliases() {
    let text =
        include_str!("../../../fixtures/runtime/argument_x_forwarding.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41C7] {
            let mut baseline: std::collections::BTreeMap<
                usize,
                (Vec<(u8, u8)>, Vec<Vec<(u16, u8)>>),
            > = std::collections::BTreeMap::new();
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
                        byte_entry: entry("ObserveBytes"),
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
                        assert_eq!((&hooks.states, &hooks.writes), (&old.0, &old.1));
                    } else {
                        baseline.insert(case, (hooks.states, hooks.writes));
                    }
                }
            }
        }
    }
}

#[test]
fn argument_x_forwarding_composes_with_preceding_argument_and_pointer_selectors() {
    let text =
        include_str!("../../../fixtures/runtime/argument_x_forwarding.act").replace("\r\n", "\n");
    for rt in [Runtime::ActionCart, Runtime::Standalone] {
        for bits in 0..8 {
            let predecessors = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
            let mut sizes = Vec::new();
            for enabled in [false, true] {
                let out = compile_flags(&text, true, rt, 0x3000, enabled, predecessors);
                let image = actionc::codegen::format_load_file(&out);
                sizes.push(image.len());
                let entry = |name: &str| {
                    out.routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                let (mut vm, profile) = support::vm_for(rt);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                vm.bus_mut().ram_mut().write(0x7000, 1);
                vm.bus_mut().ram_mut().write(0x72FF, 255);
                let mut hooks = Observe {
                    entry: entry("Observe"),
                    byte_entry: entry("ObserveBytes"),
                    ..Observe::default()
                };
                vm = machine::call(vm, &mut hooks, entry("Forward"), &[0, 0x70, 0xFF, 0x72]);
                assert_eq!(machine::bytes(&vm, 0x680, 5), vec![2, 0x70, 0, 0x73, 255]);
                let mut hooks = Observe {
                    entry: entry("Observe"),
                    byte_entry: entry("ObserveBytes"),
                    ..Observe::default()
                };
                vm = machine::call(vm, &mut hooks, entry("ForwardByte"), &[255]);
                assert_eq!(machine::bytes(&vm, 0x680, 5), vec![0, 0, 255, 0, 255]);
            }
            assert!(sizes[1] < sizes[0]);
        }
    }
}
