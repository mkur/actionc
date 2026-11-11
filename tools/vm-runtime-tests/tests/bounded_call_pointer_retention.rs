//! Bounded writes preserve pointer homes; unknown writes can change them.
use actionc::{
    codegen::{self, CodegenOutput, CodegenSymbolScope},
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

fn global(o: &CodegenOutput, name: &str) -> u16 {
    o.map
        .storage_symbols
        .iter()
        .find(|s| s.name == name && s.scope == CodegenSymbolScope::Global)
        .unwrap()
        .address
}
fn param(o: &CodegenOutput, routine: &str, name: &str) -> u16 {
    o.map
        .storage_symbols
        .iter()
        .find(|s| s.name == name && s.scope == CodegenSymbolScope::Routine(routine.into()))
        .unwrap()
        .address
}
struct Observe {
    pair_stores: usize,
    reads: Vec<u16>,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let ram = vm.bus().ram();
        if ram.read(r.pc) == 0xB1 {
            self.reads.push(
                ram.read_word(u16::from(ram.read(r.pc + 1)))
                    .wrapping_add(u16::from(r.y)),
            );
        }
        if matches!(ram.read(r.pc), 0x85 | 0x86 | 0x84)
            && (0xAC..=0xAF).contains(&ram.read(r.pc + 1))
        {
            self.pair_stores += 1;
        }
        Ok(())
    }
}

#[test]
fn bounded_calls_preserve_values_aliases_and_order_with_lf_and_crlf() {
    let lf =
        std::fs::read_to_string(root().join("fixtures/runtime/bounded_call_pointer_retention.act"))
            .unwrap()
            .replace("\r\n", "\n");
    let mut measurements = String::new();
    let mut calls = 0;
    for opt in [false, true] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut images = BTreeMap::new();
                let mut baseline = BTreeMap::new();
                for crlf in [false, true] {
                    let source = support::Source::new(&if crlf {
                        lf.replace('\n', "\r\n")
                    } else {
                        lf.clone()
                    });
                    let loaded =
                        actionc::includes::load_program_with_expanded_source(&source.0).unwrap();
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
                    for enabled in [false, true] {
                        let config = Mir6502Config {
                            enable_bounded_call_pointer_retention: enabled,
                            ..Mir6502Config::optimized()
                        };
                        let out = mir6502::generate_output_with_config_and_runtime(
                            &nir, origin, &config, rt,
                        )
                        .unwrap();
                        let image = codegen::format_load_file(&out);
                        if crlf {
                            assert_eq!(images[&enabled], image);
                        } else {
                            images.insert(enabled, image.clone());
                        }
                        let (mut vm, profile) = support::vm_for(rt);
                        vm.load_atari_object_for_execution(profile, &image).unwrap();
                        let dst = global(&out, "dstBuf");
                        let src = global(&out, "srcBuf");
                        let mut cases = Vec::new();
                        let counts: Vec<u8> =
                            if opt && rt == Runtime::ActionCart && origin == 0x3000 && !crlf {
                                (0..=255).collect()
                            } else {
                                vec![0, 1, 2, 127, 128, 254, 255]
                            };
                        for count in counts {
                            for kind in 0..4 {
                                cases.push(("ReadAcrossCopy", count, kind));
                            }
                        }
                        cases.extend([("UnknownCopy", 1, 0), ("UnknownCopy", 1, 4)]);
                        for (name, count, kind) in cases {
                            let home = param(&out, name, "p");
                            let p = match kind {
                                0 => dst,
                                1 => src,
                                2 | 4 => 0x7000,
                                _ => home,
                            };
                            for (base, seed) in [(dst, 11u8), (src, 0x73), (0x7000, 0x39)] {
                                let data = (0..512)
                                    .map(|i| (i as u8).wrapping_mul(37).wrapping_add(seed))
                                    .collect::<Vec<_>>();
                                vm.bus_mut().ram_mut().map(base, &data).unwrap();
                            }
                            vm.bus_mut().ram_mut().write(0x6FFF, 0xA5);
                            vm.bus_mut().ram_mut().write(0x7200, 0x5A);
                            let mut expected = (0..=65535)
                                .map(|a| vm.bus().ram().read(a))
                                .collect::<Vec<_>>();
                            expected[home as usize..home as usize + 2]
                                .copy_from_slice(&p.to_le_bytes());
                            let destination = if kind == 4 { home } else { dst };
                            let mut args = p.to_le_bytes().to_vec();
                            if name == "UnknownCopy" {
                                args.extend(destination.to_le_bytes());
                                let d = param(&out, name, "dst");
                                expected[d as usize..d as usize + 2]
                                    .copy_from_slice(&destination.to_le_bytes());
                            } else {
                                args.push(count);
                                expected[param(&out, name, "count") as usize] = count;
                            }
                            let before = expected[p as usize];
                            expected[global(&out, "before") as usize] = before;
                            let n = if count == 0 { 256u16 } else { u16::from(count) };
                            for i in (0..n).rev() {
                                expected[(destination + i) as usize] = expected[(src + i) as usize];
                            }
                            let updated = u16::from_le_bytes([
                                expected[home as usize],
                                expected[home as usize + 1],
                            ]);
                            let after = expected[updated as usize];
                            expected[global(&out, "after") as usize] = after;
                            let entry = out
                                .map
                                .routine_addresses
                                .iter()
                                .find(|r| r.name == name)
                                .unwrap()
                                .address;
                            let mut hooks = Observe {
                                pair_stores: 0,
                                reads: Vec::new(),
                            };
                            vm.bus_mut().clear_events();
                            let started = vm.cpu().cycles();
                            vm = machine::call(vm, &mut hooks, entry, &args);
                            let cycles = vm.cpu().cycles() - started;
                            for (base, size) in [(dst, 512), (src, 512), (0x7000, 512), (home, 2)] {
                                assert_eq!(
                                    machine::bytes(&vm, base, size),
                                    expected[base as usize..base as usize + size],
                                    "{name}/{kind}/{count}"
                                );
                            }
                            assert_eq!(vm.bus().ram().read(global(&out, "before")), before);
                            assert_eq!(vm.bus().ram().read(global(&out, "after")), after);
                            assert_eq!(vm.bus().ram().read(0x6FFF), 0xA5);
                            assert_eq!(vm.bus().ram().read(0x7200), 0x5A);
                            assert_eq!(vm.bus().ram().read_word(0xAC), updated);
                            let r = vm.cpu().registers();
                            assert_eq!(r.a, after);
                            assert_eq!(r.x, destination.to_le_bytes()[1]);
                            assert_eq!(r.y, 0);
                            assert_eq!(
                                r.status & 0x83,
                                1 | (after & 128) | u8::from(after == 0) * 2
                            );
                            let normalize = |a: u16| {
                                for (role, start, size) in [
                                    (0u8, dst, 512),
                                    (1, src, 512),
                                    (2, 0x7000, 512),
                                    (3, home, 2),
                                ] {
                                    if a.wrapping_sub(start) < size {
                                        return (role, a - start);
                                    }
                                }
                                panic!("unexpected indirect address {a:04X}")
                            };
                            let reads = hooks
                                .reads
                                .iter()
                                .map(|&a| normalize(a))
                                .collect::<Vec<_>>();
                            assert_eq!(reads.len(), usize::from(n) + 2);
                            let writes = vm
                                .bus()
                                .events()
                                .iter()
                                .filter(|e| {
                                    e.access == BusAccess::Write
                                        && (home..home + 2).contains(&e.address)
                                })
                                .map(|e| (e.address - home, e.value))
                                .collect::<Vec<_>>();
                            let state = (before, after, reads, writes, [r.a, r.y, r.status, r.sp]);
                            let key = (crlf, name, count, kind);
                            if enabled {
                                let (old, old_cycles, old_stores) = &baseline[&key];
                                assert_eq!(&state, old);
                                if name == "ReadAcrossCopy" {
                                    assert_eq!(old_cycles - cycles, 14);
                                    assert_eq!(old_stores - hooks.pair_stores, 2);
                                } else {
                                    assert_eq!(*old_cycles, cycles);
                                    assert_eq!(*old_stores, hooks.pair_stores);
                                }
                            } else {
                                baseline.insert(key, (state, cycles, hooks.pair_stores));
                            }
                            if name == "ReadAcrossCopy" && kind == 0 && count == 2 && !crlf {
                                measurements.push_str(&format!(
                                    "{opt}\t{rt:?}\t{origin:04X}\t{enabled}\t{}\t{cycles}\t{}\n",
                                    out.bytes.len(),
                                    hooks.pair_stores
                                ));
                            }
                            calls += 1;
                        }
                    }
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("ACTIONC_BOUNDED_CALL_VM_RESULTS") {
        std::fs::write(path, &measurements).unwrap();
    }
    eprintln!(
        "bounded-call VM: {calls} calls; 14 cycles and two setup stores saved per proven call"
    );
}
