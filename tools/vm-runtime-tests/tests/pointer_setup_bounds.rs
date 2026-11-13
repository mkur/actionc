use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, RunRequest, StopReason, VmRunner};

#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

#[test]
fn pointer_setup_preserves_both_indirect_reads_across_origins_runtimes_and_line_endings() {
    let lf =
        include_str!("../../../fixtures/runtime/pointer_setup_bounds.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut reference = None;
            for text in [lf.clone(), lf.replace('\n', "\r\n")] {
                let source = support::Source::new(&text);
                let loaded =
                    actionc::includes::load_program_with_expanded_source(&source.0).unwrap();
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
                    &Mir6502Config::optimized(),
                    runtime,
                )
                .unwrap();
                let image = actionc::codegen::format_load_file(&output);
                if let Some(old) = &reference {
                    assert_eq!(old, &image);
                } else {
                    reference = Some(image.clone());
                }
                let entry = output
                    .routine_addresses
                    .iter()
                    .find(|r| r.name == "ReadPair")
                    .unwrap()
                    .address;
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                for address in [0x7200u16, 0x72ff] {
                    vm.bus_mut().add_watchpoint(address);
                }
                for address in [0x7200u16, 0x72ff] {
                    vm.bus_mut()
                        .ram_mut()
                        .map(
                            0x700,
                            &[
                                0xa9,
                                address as u8,
                                0xa2,
                                (address >> 8) as u8,
                                0xa0,
                                0,
                                0x20,
                                entry as u8,
                                (entry >> 8) as u8,
                                0x4c,
                                0x10,
                                7,
                            ],
                        )
                        .unwrap();
                    for value in [0u8, 1, 0x7f, 0x80, 0xff] {
                        vm.bus_mut().ram_mut().write(address, value);
                        vm.bus_mut().ram_mut().write(0x600, value ^ 0xff);
                        vm.bus_mut().ram_mut().write(0x601, value ^ 0xff);
                        vm.bus_mut().clear_events();
                        vm.set_pc(0x700);
                        let outcome = VmRunner::new(vm).run(RunRequest {
                            max_steps: 10_000,
                            stop_after_pc: Some(0x710),
                            history_len: 8,
                        });
                        assert_eq!(
                            outcome.stop_reason(),
                            StopReason::PcReached { pc: 0x710 },
                            "{:?}",
                            outcome.report
                        );
                        assert_eq!(outcome.memory().read(0x600), value);
                        assert_eq!(outcome.memory().read(0x601), value);
                        vm = outcome.into_vm();
                        assert_eq!(
                            vm.bus()
                                .events()
                                .iter()
                                .filter(|e| e.access == BusAccess::Read && e.address == address)
                                .count(),
                            2
                        );
                    }
                }
            }
        }
    }
}
