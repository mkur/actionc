use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, RunRequest, StopReason, VmRunner};

#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

#[test]
fn ordinary_word_return_preserves_values_and_stores_high_lane_first() {
    let lf = include_str!("../../../fixtures/runtime/core_layout_census.act").replace("\r\n", "\n");
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
                let entry = output
                    .routine_addresses
                    .iter()
                    .find(|r| r.name == "Main")
                    .unwrap()
                    .address;
                let saved = output
                    .map
                    .storage_symbols
                    .iter()
                    .find(|s| s.name == "saved")
                    .unwrap()
                    .address;
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                vm.bus_mut().add_watchpoint(saved);
                vm.bus_mut().add_watchpoint(saved + 1);
                vm.bus_mut()
                    .ram_mut()
                    .map(
                        0x700,
                        &[0x20, entry as u8, (entry >> 8) as u8, 0x4c, 0x10, 7],
                    )
                    .unwrap();
                for input in [0u16, 0xff, 0x7fff, 0x8000, 0xfffe, 0xffff] {
                    vm.bus_mut().ram_mut().write_word(0x600, input);
                    vm.bus_mut().ram_mut().write_word(0x602, 0xdead);
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
                    let expected = input.wrapping_add(1);
                    assert_eq!(outcome.memory().read(saved), expected as u8);
                    assert_eq!(outcome.memory().read(saved + 1), (expected >> 8) as u8);
                    assert_eq!(outcome.memory().read(0x602), 0);
                    assert_eq!(outcome.memory().read(0x603), 0);
                    vm = outcome.into_vm();
                    let writes: Vec<_> = vm
                        .bus()
                        .events()
                        .iter()
                        .filter(|e| e.access == BusAccess::Write)
                        .map(|e| (e.address, e.value))
                        .collect();
                    assert_eq!(
                        writes,
                        vec![(saved + 1, (expected >> 8) as u8), (saved, expected as u8)]
                    );
                }
            }
        }
    }
}
