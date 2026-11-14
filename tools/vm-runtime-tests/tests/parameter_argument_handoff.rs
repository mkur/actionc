//! Independent entry and ordered-memory oracles, through real LF/CRLF inputs.
use actionc::{
    codegen::CodegenOutput,
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

struct Observe {
    entry: u16,
    args: Vec<Vec<u8>>,
    extra: Vec<u8>,
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
                vm.bus().ram().read(0xa3),
                vm.bus().ram().read(0xa4),
            ]);
            self.extra.push(vm.bus().ram().read(0xac));
        }
        Ok(())
    }
}
fn compile(source: &str, runtime: Runtime, origin: u16) -> CodegenOutput {
    let file = support::Source::new(source);
    let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let n = actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&loaded.program, &model),
    ))
    .unwrap();
    mir6502::generate_output_with_config_and_runtime(
        &n,
        origin,
        &Mir6502Config {
            enable_small_leaf_inlining: false,
            ..Mir6502Config::optimized()
        },
        runtime,
    )
    .unwrap()
}
fn entry(out: &CodegenOutput, name: &str) -> u16 {
    out.routine_addresses
        .iter()
        .find(|r| r.name == name)
        .unwrap()
        .address
}
fn words(a: u16, b: u16) -> Vec<u8> {
    a.to_le_bytes().into_iter().chain(b.to_le_bytes()).collect()
}

#[test]
fn forwarding_and_copy_wrappers_follow_independent_oracles() {
    let lf = include_str!("../../../fixtures/runtime/parameter_argument_handoff.act")
        .replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut reference = None;
            for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                let out = compile(&source, runtime, origin);
                let image = actionc::codegen::format_load_file(&out);
                if let Some(old) = &reference {
                    assert_eq!(old, &image);
                } else {
                    reference = Some(image.clone());
                }
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                for a in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                    let b = a ^ 0x5aa5;
                    for name in [
                        "ForwardWords",
                        "Mutated",
                        "AfterCall",
                        "Twice",
                        "Exposed",
                        "ExtraEntry",
                    ] {
                        let callee = if name == "ExtraEntry" {
                            "ObserveExtra"
                        } else {
                            "Observe"
                        };
                        let mut hook = Observe {
                            entry: entry(&out, callee),
                            args: vec![],
                            extra: vec![],
                        };
                        vm = machine::call(vm, &mut hook, entry(&out, name), &words(a, b));
                        let first = if name == "Mutated" {
                            a.wrapping_add(1)
                        } else if name == "Exposed" {
                            out.map
                                .storage_symbols
                                .iter()
                                .find(|s| {
                                    s.name == "first"
                                        && s.scope
                                            == actionc::codegen::CodegenSymbolScope::Routine(
                                                name.into(),
                                            )
                                })
                                .unwrap()
                                .address
                        } else {
                            a
                        };
                        let mut expected = words(first, b);
                        expected.push(7);
                        let mut calls = vec![expected.clone()];
                        if name == "Twice" {
                            expected[4] = 8;
                            calls.push(expected.clone());
                        }
                        assert_eq!(hook.args, calls, "{name}");
                        assert_eq!(machine::bytes(&vm, 0x680, 5), expected);
                        if name == "AfterCall" {
                            assert_eq!(machine::bytes(&vm, 0x688, 2), a.to_le_bytes());
                        }
                        if name == "Exposed" {
                            assert_eq!(machine::bytes(&vm, first, 2), a.to_le_bytes());
                        }
                        if name == "ExtraEntry" {
                            assert_eq!(machine::bytes(&vm, 0x685, 1), hook.extra);
                        }
                    }
                }
                for a in 0..=255u8 {
                    let b = a.wrapping_add(73);
                    let tag = a ^ 149;
                    let mut hook = Observe {
                        entry: entry(&out, "Observe"),
                        args: vec![],
                        extra: vec![],
                    };
                    vm = machine::call(vm, &mut hook, entry(&out, "ForwardBytes"), &[a, b, tag]);
                    assert_eq!(hook.args, vec![vec![a, 0, b, 0, tag]]);
                    assert_eq!(machine::bytes(&vm, 0x680, 5), vec![a, 0, b, 0, tag]);
                }
                for address in 0x7000..0x7800 {
                    vm.bus_mut().add_watchpoint(address);
                }
                for (dst, src) in [
                    (0x7000u16, 0x7500u16),
                    (0x70ff, 0x75fe),
                    (0x7000, 0x7001),
                    (0x7001, 0x7000),
                    (0x7000, 0x7000),
                ] {
                    for length in 0..=255u8 {
                        let mut model: Vec<_> = (0..0x800usize)
                            .map(|a| (a as u8).wrapping_mul(37).wrapping_add(11))
                            .collect();
                        model[usize::from(src - 0x7000)] = length;
                        vm.bus_mut().ram_mut().map(0x7000, &model).unwrap();
                        let mut expected_accesses = vec![(BusAccess::Read, src, length)];
                        for i in (0..=u16::from(length)).rev() {
                            let value = model[usize::from(src + i - 0x7000)];
                            expected_accesses.push((BusAccess::Read, src + i, value));
                            expected_accesses.push((BusAccess::Write, dst + i, value));
                            model[usize::from(dst + i - 0x7000)] = value;
                        }
                        vm.bus_mut().clear_events();
                        let mut hook = Observe {
                            entry: entry(&out, "ReverseCopy"),
                            args: vec![],
                            extra: vec![],
                        };
                        vm = machine::call(
                            vm,
                            &mut hook,
                            entry(&out, "CopyCounted"),
                            &words(dst, src),
                        );
                        let mut args = words(dst, src);
                        args.push(length.wrapping_add(1));
                        assert_eq!(hook.args, vec![args]);
                        assert_eq!(machine::bytes(&vm, 0x7000, model.len()), model);
                        let accesses: Vec<_> = vm
                            .bus()
                            .events()
                            .iter()
                            .map(|e| (e.access, e.address, e.value))
                            .collect();
                        assert_eq!(
                            accesses, expected_accesses,
                            "dst={dst:x} src={src:x} length={length}"
                        );
                    }
                }
            }
        }
    }
}
