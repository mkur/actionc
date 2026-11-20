//! Independent scalar/pointer placement oracles through real LF/CRLF sources.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{CompilerVm, VmRunHooks};
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
fn scalar_pointer_and_exposed_parameters_follow_the_action_abi() {
    let lf = include_str!("../../../fixtures/runtime/parameter_homes.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut reference = None;
            for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                let file = support::Source::new(&source);
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
                let config = Mir6502Config {
                    enable_parameter_home_selection: true,
                    enable_small_leaf_inlining: false,
                    ..Mir6502Config::optimized()
                };
                let output = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                let image = actionc::codegen::format_load_file(&output);
                if let Some(old) = &reference {
                    assert_eq!(old, &image);
                } else {
                    reference = Some(image.clone());
                }
                let entry = |name| {
                    output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                for value in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                    vm = machine::call(vm, &mut Observe, entry("KeepByte"), &[value as u8]);
                    assert_eq!(vm.cpu().registers().a, value as u8);
                    assert_eq!(machine::bytes(&vm, 0xa0, 1), vec![value as u8]);
                    vm = machine::call(vm, &mut Observe, entry("StepWord"), &value.to_le_bytes());
                    assert_eq!(
                        machine::bytes(&vm, 0xa0, 2),
                        value.wrapping_add(1).to_le_bytes()
                    );
                    vm = machine::call(vm, &mut Observe, entry("ExposeWord"), &value.to_le_bytes());
                    let address =
                        u16::from_le_bytes(machine::bytes(&vm, 0xa0, 2).try_into().unwrap());
                    assert_eq!(machine::bytes(&vm, address, 2), value.to_le_bytes());
                    let mut args = value.to_le_bytes().to_vec();
                    args.extend(3u16.to_le_bytes());
                    vm = machine::call(vm, &mut Observe, entry("MultiplyWords"), &args);
                    assert_eq!(
                        machine::bytes(&vm, 0xa0, 2),
                        value.wrapping_mul(3).to_le_bytes()
                    );
                }
                for (s, t) in [(0x7000u16, 0x7500u16), (0x70ff, 0x75fe)] {
                    for offset in [0, 1, 254, 255, 256, 257, 511, 512] {
                        for greater in [false, true] {
                            vm.bus_mut()
                                .ram_mut()
                                .map(s, &vec![65; offset + 1])
                                .unwrap();
                            vm.bus_mut()
                                .ram_mut()
                                .map(t, &vec![65; offset + 1])
                                .unwrap();
                            vm.bus_mut()
                                .ram_mut()
                                .write(s + offset as u16, if greater { 255 } else { 0 });
                            vm.bus_mut().ram_mut().write(t + offset as u16, 128);
                            let mut args = s.to_le_bytes().to_vec();
                            args.extend(t.to_le_bytes());
                            vm = machine::call(vm, &mut Observe, entry("ComparePointers"), &args);
                            assert_eq!(machine::bytes(&vm, 0xa0, 1), vec![u8::from(greater)]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn register_bound_parameters_preserve_each_abi_lane() {
    let lf =
        include_str!("../../../fixtures/runtime/parameter_registers.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut reference = None;
            for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                let file = support::Source::new(&source);
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
                let output = mir6502::generate_output_with_config_and_runtime(
                    &nir,
                    origin,
                    &Mir6502Config {
                        enable_parameter_home_selection: true,
                        enable_small_leaf_inlining: false,
                        ..Mir6502Config::optimized()
                    },
                    runtime,
                )
                .unwrap();
                let image = actionc::codegen::format_load_file(&output);
                if let Some(old) = &reference {
                    assert_eq!(old, &image);
                } else {
                    reference = Some(image.clone());
                }
                let entry = |name| {
                    output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                for value in 0..=255u8 {
                    let args = [value, value.wrapping_add(73), value.wrapping_add(149)];
                    for (lane, name) in ["First", "Second", "Third"].into_iter().enumerate() {
                        vm = machine::call(vm, &mut Observe, entry(name), &args);
                        // Public Action results live at $A0. Equality with A is
                        // an inferred callee fact, not a universal ABI guarantee.
                        assert_eq!(machine::bytes(&vm, 0xa0, 1), vec![args[lane]]);
                    }
                    vm = machine::call(vm, &mut Observe, entry("WordXY"), &args);
                    assert_eq!(machine::bytes(&vm, 0xa0, 2), args[1..]);
                }
            }
        }
    }
}

#[test]
fn scalar_pairs_preserve_carries_loops_and_uncaptured_a3() {
    let lf = include_str!("../../../fixtures/runtime/parameter_scalars.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut reference = None;
            for source in [lf.clone(), lf.replace('\n', "\r\n")] {
                let file = support::Source::new(&source);
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
                let output = mir6502::generate_output_with_config_and_runtime(
                    &nir,
                    origin,
                    &Mir6502Config {
                        enable_parameter_home_selection: true,
                        enable_small_leaf_inlining: false,
                        ..Mir6502Config::optimized()
                    },
                    runtime,
                )
                .unwrap();
                let image = actionc::codegen::format_load_file(&output);
                if let Some(old) = &reference {
                    assert_eq!(old, &image);
                } else {
                    reference = Some(image.clone());
                }
                let entry = |name| {
                    output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                for value in [0u16, 1, 5, 6, 255, 256, 511, 512, 0x7fff, 0x8000, 0xffff] {
                    for other in [0u16, 1, 255, 256, 0x7fff, 0xffff] {
                        let mut args = value.to_le_bytes().to_vec();
                        args.extend(other.to_le_bytes());
                        vm = machine::call(vm, &mut Observe, entry("AdvanceBoth"), &args);
                        assert_eq!(
                            machine::bytes(&vm, 0xa0, 2),
                            value.wrapping_add(other).wrapping_add(2).to_le_bytes()
                        );
                    }
                    vm = machine::call(vm, &mut Observe, entry("ReadWord"), &value.to_le_bytes());
                    assert_eq!(machine::bytes(&vm, 0xa0, 2), value.to_le_bytes());
                    vm = machine::call(vm, &mut Observe, entry("CountDown"), &value.to_le_bytes());
                    assert_eq!(machine::bytes(&vm, 0xa0, 2), value.min(5).to_le_bytes());
                    vm = machine::call(
                        vm,
                        &mut Observe,
                        entry("Edges"),
                        &[value as u8, 73, 149, 199],
                    );
                    assert_eq!(
                        machine::bytes(&vm, 0xa0, 1),
                        vec![(value as u8).wrapping_add(199)]
                    );
                }
            }
        }
    }
}

#[test]
fn explicit_and_inferred_own_values_survive_repeated_calls() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    #[allow(dead_code)]
    #[path = "support/tn_symbols.rs"]
    mod symbols;
    let source = include_str!("../../../fixtures/native/own_locals.act");
    for source in [source.to_owned(), source.replace("OWN ", "")] {
        let file = support::Source::new(&source);
        for (mode, runtime) in [
            (CompileMode::Compatibility, Runtime::ActionCart),
            (CompileMode::Optimized, Runtime::Standalone),
            (CompileMode::Mir6502, Runtime::ActionCart),
            (CompileMode::Mir6502, Runtime::Standalone),
        ] {
            let compiled = compile_file(
                &file.0,
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap();
            let (mut vm, profile) = support::vm_for(runtime);
            vm.load_atari_object_for_execution(profile, compiled.object_bytes())
                .unwrap();
            let entry = symbols::routine_address(&compiled.source_listing(), "AdvanceCount");
            for (argument, expected) in [(1, 5), (0, 7), (0, 9)] {
                vm = machine::call(vm, &mut Observe, entry, &[argument]);
                assert_eq!(vm.cpu().registers().a, expected, "{mode:?} {runtime:?}");
            }
        }
    }
}
