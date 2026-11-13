//! Real LF/CRLF source and ABI oracle for a rejected allocation-pruning attempt.
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
fn pruning_rejection_keeps_argument_bytes_writes_and_full_return_state() {
    let text =
        include_str!("../../../fixtures/runtime/late_spill_pruning.act").replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for origin in [0x3000, 0x41c7] {
            let mut expected = None;
            let mut reference_image = None;
            for enabled in [false, true] {
                for crlf in [false, true] {
                    let source = support::Source::new(&if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.clone()
                    });
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
                        &Mir6502Config {
                            enable_late_spill_pruning: enabled,
                            ..Mir6502Config::optimized()
                        },
                        runtime,
                    )
                    .unwrap();
                    let image = actionc::codegen::format_load_file(&output);
                    if let Some(old) = &reference_image {
                        assert_eq!(old, &image);
                    } else {
                        reference_image = Some(image.clone());
                    }
                    let entry = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == "Forward")
                        .unwrap()
                        .address;
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    for a in 0x680..=0x684 {
                        vm.bus_mut().add_watchpoint(a);
                    }
                    let mut results = Vec::new();
                    for first in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                        let second = first ^ 0xa55a;
                        let tag = (first >> 8) as u8;
                        let mut args = first.to_le_bytes().to_vec();
                        args.extend(second.to_le_bytes());
                        args.push(tag);
                        vm.bus_mut().clear_events();
                        vm = machine::call(vm, &mut Observe, entry, &args);
                        let mut bytes = first.wrapping_add(1).to_le_bytes().to_vec();
                        bytes.extend(second.to_le_bytes());
                        bytes.push(tag);
                        assert_eq!(machine::bytes(&vm, 0x680, 5), bytes);
                        let writes: Vec<_> = vm
                            .bus()
                            .events()
                            .iter()
                            .filter(|e| e.access == BusAccess::Write)
                            .map(|e| (e.address, e.value))
                            .collect();
                        let r = vm.cpu().registers();
                        results.push((writes, r.a, r.x, r.y, r.sp, r.status));
                    }
                    if let Some(old) = &expected {
                        assert_eq!(old, &results);
                    } else {
                        expected = Some(results);
                    }
                }
            }
        }
    }
}
