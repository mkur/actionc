//! Constant WORD shift differential checks through the public Action! ABI.
use actionc::compiler::Runtime;
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{CompilerVm, VmRunHooks};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[derive(Default)]
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

#[test]
fn private_memory_word_shifts_preserve_carries_snapshots_and_both_result_lanes() {
    let counts = [0u8, 1, 2, 3, 4, 8, 15, 16];
    let mut text = String::new();
    for count in counts {
        text.push_str(&format!(
            "CARD FUNC Shift{count}(CARD value)\nRETURN(value RSH {count})\n"
        ));
    }
    text.push_str("CARD FUNC Snapshot(CARD POINTER cell)\n  CARD captured\n  captured=cell^ cell^=$ABCD\nRETURN(captured RSH 3)\nPROC Main()\nRETURN\n");
    let mut values = vec![
        0u16, 1, 2, 127, 128, 255, 256, 257, 0x7FFF, 0x8000, 0xFF00, 0xFF01, 0xFFFE, 0xFFFF,
    ];
    values.extend((0..256u16).map(|n| n.wrapping_mul(251).wrapping_add(0x713D)));
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for crlf in [false, true] {
            let source = if crlf {
                text.replace('\n', "\r\n")
            } else {
                text.clone()
            };
            for enabled in [false, true] {
                let file = support::Source::new(&source);
                let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
                let model = actionc::semantic::analyze_with_options(
                    &loaded.program,
                    actionc::semantic::SemanticOptions::modern(),
                )
                .unwrap();
                let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
                let nir =
                    actionc::nir::optimize_program(&actionc::nir::lower_program(&sem)).unwrap();
                let mut config = Mir6502Config::optimized();
                config.enable_private_memory_shifts = enabled;
                let raw = mir6502::lower_program(&nir).unwrap();
                let selected = mir6502::materialize_program_with_origin_and_runtime(
                    raw, &config, 0x3000, runtime,
                )
                .unwrap();
                let native = selected
                    .routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .filter(|op| {
                        matches!(
                            op,
                            mir6502::MirOp::UpdateMem {
                                op: mir6502::MirUpdateOp::Lsr | mir6502::MirUpdateOp::Ror,
                                ..
                            }
                        )
                    })
                    .count();
                assert_eq!(native > 0, enabled, "native shift selection");
                let output = mir6502::generate_output_with_config_and_runtime(
                    &nir, 0x3000, &config, runtime,
                )
                .unwrap();
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(
                    profile,
                    &actionc::codegen::format_load_file(&output),
                )
                .unwrap();
                let entry = |name: &str| {
                    output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap()
                        .address
                };
                for count in counts {
                    for &value in &values {
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x9F, &[0xA5, 0x55, 0xAA, 0x5A])
                            .unwrap();
                        let cycles = vm.cpu().cycles();
                        vm = machine::call(
                            vm,
                            &mut Hooks,
                            entry(&format!("Shift{count}")),
                            &value.to_le_bytes(),
                        );
                        assert_eq!(
                            vm.bus().ram().read_word(0xA0),
                            value.checked_shr(u32::from(count)).unwrap_or(0),
                            "{runtime:?}/{crlf}/{enabled}/{count}/{value:04X}"
                        );
                        assert_eq!(vm.bus().ram().read(0x9F), 0xA5);
                        assert_eq!(vm.bus().ram().read(0xA2), 0x5A);
                        if !crlf && value == 0xFF01 {
                            println!(
                                "private-shift,{runtime:?},{enabled},{count},{}",
                                vm.cpu().cycles() - cycles
                            );
                        }
                    }
                }
                for &value in &values {
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x50FF, &[0xA5, value as u8, (value >> 8) as u8, 0x5A])
                        .unwrap();
                    vm = machine::call(vm, &mut Hooks, entry("Snapshot"), &0x5100u16.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(0xA0), value >> 3);
                    assert_eq!(machine::bytes(&vm, 0x50FF, 4), [0xA5, 0xCD, 0xAB, 0x5A]);
                }
            }
        }
    }
}
