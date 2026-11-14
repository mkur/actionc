//! Host arithmetic and ordered indirect-read oracles for terminal register reuse.
use actionc::{
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
struct Observe;
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, _: &mut CompilerVm) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn exposed_pointer_arithmetic_preserves_carry_borrow_and_indirect_reads() {
    let lf =
        include_str!("../../../fixtures/runtime/late_register_reuse.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            for enabled in [false, true] {
                let mut reference = None;
                for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                    let file = support::Source::new(&source);
                    let loaded =
                        actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
                    let model = actionc::semantic::analyze_with_options(
                        &loaded.program,
                        actionc::semantic::SemanticOptions::modern(),
                    )
                    .unwrap();
                    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(
                        &actionc::semantic::ir::lower_program(&loaded.program, &model),
                    ))
                    .unwrap();
                    let out = mir6502::generate_output_with_config_and_runtime(
                        &nir,
                        origin,
                        &Mir6502Config {
                            enable_small_leaf_inlining: false,
                            enable_late_register_reuse: enabled,
                            ..Mir6502Config::optimized()
                        },
                        runtime,
                    )
                    .unwrap();
                    let image = actionc::codegen::format_load_file(&out);
                    if let Some(old) = &reference {
                        assert_eq!(old, &image);
                    } else {
                        reference = Some(image.clone());
                    }
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    // Standalone also exercises modulo-65536 overflow, without ROM.
                    let mut pointers = vec![0x7000u16, 0x70ff, 0x7fff, 0x00ac, 0x00ae, 0x00af];
                    if runtime == Runtime::Standalone {
                        pointers.extend([0xff00, 0xffff]);
                    }
                    for p in pointers {
                        vm.bus_mut().add_watchpoint(p);
                        for byte in [0u8, 1, 127, 128, 254, 255] {
                            for name in ["Advance", "Retreat"] {
                                vm.bus_mut().ram_mut().write(p, byte);
                                let value = match p {
                                    0xAE => 0xAE,
                                    0xAF => 0,
                                    _ => byte,
                                };
                                let expected = if name == "Advance" {
                                    p.wrapping_add(value as u16).wrapping_add(5)
                                } else {
                                    p.wrapping_sub(value as u16).wrapping_sub(5)
                                };
                                let entry = out
                                    .routine_addresses
                                    .iter()
                                    .find(|r| r.name == name)
                                    .unwrap()
                                    .address;
                                let start = vm.bus().events().len();
                                vm = machine::call(vm, &mut Observe, entry, &p.to_le_bytes());
                                assert_eq!(
                                    machine::bytes(&vm, 0xA0, 2),
                                    expected.to_le_bytes(),
                                    "{name} {runtime:?} p={p:04x} byte={byte} reuse={enabled}"
                                );
                                // The pointer-byte RAM data read must remain exactly once.
                                // Scratch aliases also contain pointer fetches and stores.
                                if p >= 0x400 {
                                    assert_eq!(
                                        vm.bus().events()[start..]
                                            .iter()
                                            .filter(
                                                |e| e.address == p && e.access == BusAccess::Read
                                            )
                                            .count(),
                                        1
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

#[test]
fn actual_tn_key_and_next_follow_host_oracles() {
    let loaded = actionc::includes::load_program_with_expanded_source(
        root().join("samples/tn/modern/TN.ACT"),
    )
    .unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&loaded.program, &model),
    ))
    .unwrap();
    for (runtime, origin) in [(Runtime::ActionCart, 0x3000), (Runtime::Standalone, 0x41c7)] {
        let out = mir6502::generate_output_with_config_and_runtime(
            &nir,
            origin,
            &Mir6502Config::optimized(),
            runtime,
        )
        .unwrap();
        let image = actionc::codegen::format_load_file(&out);
        if let Some(directory) = std::env::var_os("ACTIONC_LATE_REUSE_IMAGES") {
            assert_eq!(
                image,
                std::fs::read(
                    std::path::PathBuf::from(directory)
                        .join(format!("tn-{runtime:?}-{origin:04x}-true.xex"))
                )
                .unwrap()
            );
        }
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &image).unwrap();
        for pointer in [0x7000u16, 0x70ff] {
            vm.bus_mut().add_watchpoint(pointer);
            for length in [0u8, 1, 250, 255] {
                for (name, expected, width) in [
                    ("Next", pointer + u16::from(length) + 5, 2),
                    ("Key", 0x5a, 1),
                ] {
                    let key = pointer + u16::from(length) + 3;
                    vm.bus_mut().ram_mut().write(pointer, length);
                    vm.bus_mut().ram_mut().write(key, 0x5a);
                    let start = vm.bus().events().len();
                    let entry = out
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address;
                    vm = machine::call(vm, &mut Observe, entry, &pointer.to_le_bytes());
                    assert_eq!(
                        machine::bytes(&vm, 0xA0, width),
                        expected.to_le_bytes()[..width],
                        "{name} {pointer:04x} {length}"
                    );
                    assert_eq!(
                        vm.bus().events()[start..]
                            .iter()
                            .filter(|e| e.address == pointer && e.access == BusAccess::Read)
                            .count(),
                        1
                    );
                    assert_eq!(vm.bus().ram().read(pointer), length);
                }
            }
        }
    }
}

#[test]
fn combined_pointer_fusion_and_reuse_preserve_call_arguments_and_source_reads() {
    let text =
        include_str!("../../../fixtures/runtime/pointer_byte_plus_one.act").replace("\r\n", "\n");
    let file = support::Source::new(&text);
    let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&loaded.program, &model),
    ))
    .unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for fusion in [false, true] {
            let out = mir6502::generate_output_with_config_and_runtime(
                &nir,
                0x41c7,
                &Mir6502Config {
                    enable_late_register_reuse: true,
                    enable_pointer_byte_plus_one_fusion: fusion,
                    ..Mir6502Config::optimized()
                },
                runtime,
            )
            .unwrap();
            let (mut vm, profile) = support::vm_for(runtime);
            vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&out))
                .unwrap();
            let q = 0x7400u16;
            vm.bus_mut().add_watchpoint(q);
            for p in [0x7000u16, 0x70ff] {
                vm.bus_mut().add_watchpoint(p);
                for byte in [0u8, 1, 254, 255] {
                    for (name, result) in [
                        ("Forward", p + u16::from(byte) + 1),
                        ("PlusZero", p + u16::from(byte)),
                        ("PlusTwo", p + u16::from(byte) + 2),
                        ("Subtract", p - u16::from(byte) + 1),
                        ("Plain", p + 1),
                    ] {
                        vm.bus_mut().ram_mut().write(p, byte);
                        vm.bus_mut().ram_mut().write(q, 9);
                        let args = p
                            .to_le_bytes()
                            .into_iter()
                            .chain(q.to_le_bytes())
                            .collect::<Vec<_>>();
                        let expected = result
                            .to_le_bytes()
                            .into_iter()
                            .chain((q + 1).to_le_bytes())
                            .chain([9])
                            .collect::<Vec<_>>();
                        let entry = out
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address;
                        let start = vm.bus().events().len();
                        vm = machine::call(vm, &mut Observe, entry, &args);
                        assert_eq!(
                            machine::bytes(&vm, 0x680, 5),
                            expected,
                            "{name} fusion={fusion} {runtime:?}"
                        );
                        let reads = vm.bus().events()[start..]
                            .iter()
                            .filter(|e| {
                                e.access == BusAccess::Read && (e.address == p || e.address == q)
                            })
                            .map(|e| e.address)
                            .collect::<Vec<_>>();
                        assert_eq!(
                            reads,
                            if name == "Plain" { vec![q] } else { vec![p, q] },
                            "{name} ordered public reads"
                        );
                        assert_eq!(vm.bus().ram().read(p), byte);
                        assert_eq!(vm.bus().ram().read(q), 9);
                    }
                }
            }
        }
    }
}
