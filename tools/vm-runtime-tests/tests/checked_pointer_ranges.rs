//! The emitted bounded concatenations, compared with the feature disabled.
use actionc::{
    codegen::{self, CodegenOutput},
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{CompilerVm, VmRunHooks};
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
struct Observe {
    reads: Vec<u16>,
    stores: usize,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let ram = vm.bus().ram();
        if matches!(ram.read(r.pc), 0xB1 | 0x71) {
            self.reads.push(
                ram.read_word(u16::from(ram.read(r.pc + 1)))
                    .wrapping_add(u16::from(r.y)),
            );
        }
        if ram.read(r.pc) == 0x85 && (0xAC..=0xAF).contains(&ram.read(r.pc + 1)) {
            self.stores += 1;
        }
        Ok(())
    }
}
fn compile(source: &str, opt: bool, rt: Runtime, origin: u16, enabled: bool) -> CodegenOutput {
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
    let nir = if opt {
        actionc::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    mir6502::generate_output_with_config_and_runtime(
        &nir,
        origin,
        &Mir6502Config {
            enable_checked_range_pointer_retention: enabled,
            ..Mir6502Config::optimized()
        },
        rt,
    )
    .unwrap()
}
#[test]
fn checked_copy_pairs_preserve_order_overlap_boundaries_and_exit_state() {
    let lf = std::fs::read_to_string(root().join("fixtures/runtime/checked_pointer_ranges.act"))
        .unwrap()
        .replace("\r\n", "\n");
    let mut total = 0;
    let mut measurements = String::new();
    for opt in [false, true] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut images = BTreeMap::new();
                let mut baseline = BTreeMap::new();
                for crlf in [false, true] {
                    for enabled in [false, true] {
                        let text = if crlf {
                            lf.replace('\n', "\r\n")
                        } else {
                            lf.clone()
                        };
                        let out = compile(&text, opt, rt, origin, enabled);
                        let image = codegen::format_load_file(&out);
                        if crlf {
                            assert_eq!(images[&enabled], image);
                        } else {
                            images.insert(enabled, image.clone());
                        }
                        let (mut vm, profile) = support::vm_for(rt);
                        vm.load_atari_object_for_execution(profile, &image).unwrap();
                        let counts: Vec<u8> =
                            if opt && rt == Runtime::ActionCart && origin == 0x3000 && !crlf {
                                (0..=255).collect()
                            } else {
                                vec![0, 1, 2, 127, 128, 254, 255]
                            };
                        for name in ["Guarded", "FromReturn", "Unrestricted"] {
                            for &count in &counts {
                                for kind in 0..4 {
                                    let (dst, src) = if name == "FromReturn" {
                                        (0x2103, 0x2123)
                                    } else {
                                        match kind {
                                            0 => (0x2103, 0x2407),
                                            1 => (0x22FE, 0x2300),
                                            2 => (0x2300, 0x22FE),
                                            _ => (0x2300, 0x2300),
                                        }
                                    };
                                    let data = (0..0xB01)
                                        .map(|i| (i as u8).wrapping_mul(37).wrapping_add(11))
                                        .collect::<Vec<_>>();
                                    vm.bus_mut().ram_mut().map(0x2000, &data).unwrap();
                                    vm.bus_mut().ram_mut().write_word(0xE8, 0x2103);
                                    let dlen = if kind == 1 { 250 } else { 3 };
                                    vm.bus_mut().ram_mut().write(dst, dlen);
                                    vm.bus_mut().ram_mut().write(src, count);
                                    let mut expected = machine::bytes(&vm, 0x2000, 0xB01);
                                    let at = |a: u16| usize::from(a - 0x2000);
                                    let start = dst + u16::from(expected[at(dst)]) + 1;
                                    let n = if count == 0 { 256 } else { u16::from(count) };
                                    for i in (0..n).rev() {
                                        expected[at(start + i)] = expected[at(src + 1 + i)];
                                    }
                                    expected[at(dst)] =
                                        expected[at(dst)].wrapping_add(expected[at(src)]);
                                    let address = out
                                        .map
                                        .routine_addresses
                                        .iter()
                                        .find(|r| r.name == name)
                                        .unwrap()
                                        .address;
                                    let args = if name == "FromReturn" {
                                        vec![]
                                    } else {
                                        [dst.to_le_bytes(), src.to_le_bytes()].concat()
                                    };
                                    let mut hook = Observe {
                                        reads: Vec::new(),
                                        stores: 0,
                                    };
                                    let started = vm.cpu().cycles();
                                    vm = machine::call(vm, &mut hook, address, &args);
                                    let cycles = vm.cpu().cycles() - started;
                                    assert_eq!(
                                        machine::bytes(&vm, 0x2000, 0xB01),
                                        expected,
                                        "{name}/{count}/{kind}"
                                    );
                                    assert_eq!(&hook.reads[hook.reads.len() - 2..], &[dst, src]);
                                    assert_eq!(hook.reads.len(), usize::from(n) + 4);
                                    let r = vm.cpu().registers();
                                    let state = (hook.reads, [r.a, r.x, r.y, r.status, r.sp]);
                                    let key = (crlf, name, count, kind);
                                    if enabled {
                                        let (old, old_cycles, old_stores) = &baseline[&key];
                                        assert_eq!(&state, old, "{name}/{count}/{kind}");
                                        let saving = if name == "Unrestricted" { 0 } else { 28 };
                                        assert_eq!(old_cycles - cycles, saving);
                                        assert_eq!(
                                            old_stores - hook.stores,
                                            if saving == 0 { 0 } else { 4 }
                                        );
                                    } else {
                                        baseline.insert(key, (state, cycles, hook.stores));
                                    }
                                    if name == "Guarded" && count == 2 && kind == 0 && !crlf {
                                        measurements.push_str(&format!("{opt}\t{rt:?}\t{origin:04X}\t{enabled}\t{}\t{cycles}\n",out.bytes.len()));
                                    }
                                    total += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("ACTIONC_CHECKED_RANGE_VM_RESULTS") {
        std::fs::write(path, measurements).unwrap();
    }
    eprintln!(
        "checked range VM: {total} calls, all counts 0–255, overlap, LF/CRLF, both runtimes/origins/NIR modes"
    );
}
