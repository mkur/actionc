use actionc::{
    codegen,
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::VmRunHooks;
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
struct Hooks;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
impl VmRunHooks for Hooks {
    type Error = String;
}

#[test]
fn word_move_store_preserves_carries_borrows_overlaps_and_call_snapshots() {
    let lf = include_str!("../../../fixtures/runtime/word_move_store.act").replace("\r\n", "\n");
    let mut calls = 0;
    for opt in [false, true] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut first = None;
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
                    let nir = if opt {
                        actionc::nir::optimize_program(&raw).unwrap()
                    } else {
                        raw
                    };
                    let config = Mir6502Config {
                        enable_small_leaf_inlining: false,
                        ..Mir6502Config::optimized()
                    };
                    let out =
                        mir6502::generate_output_with_config_and_runtime(&nir, origin, &config, rt)
                            .unwrap();
                    let image = codegen::format_load_file(&out);
                    if let Some(first) = &first {
                        assert_eq!(first, &image);
                    } else {
                        first = Some(image.clone());
                    }
                    let (mut vm, profile) = support::vm_for(rt);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    let mut values = vec![
                        (0u16, 0u16),
                        (0, 1),
                        (1, 1),
                        (255, 1),
                        (256, 1),
                        (0x7FFF, 1),
                        (0xFFFF, 1),
                        (0x0101, 0x00FF),
                        (0xFFFF, 0xFFFF),
                    ];
                    if opt && rt == Runtime::ActionCart && origin == 0x3000 && !crlf {
                        values
                            .extend((0u16..=255).flat_map(|hi| [(hi << 8 | 255, 1), (hi << 8, 1)]));
                    }
                    for (left, right) in values {
                        for (name, address) in [
                            ("Add", 0xD8),
                            ("Subtract", 0xD8),
                            ("Chained", 0xD8),
                            ("OverlapLeft", 0xD1),
                            ("OverlapRight", 0xD3),
                            ("InPlace", 0xD0),
                            ("AcrossCall", 0xD8),
                        ] {
                            vm.bus_mut().ram_mut().write_word(0xD0, left);
                            vm.bus_mut().ram_mut().write_word(0xD2, right);
                            let entry = out
                                .map
                                .routine_addresses
                                .iter()
                                .find(|r| r.name == name)
                                .unwrap()
                                .address;
                            vm = machine::call(vm, &mut Hooks, entry, &[]);
                            let expected = if name == "Subtract" {
                                left.wrapping_sub(right)
                            } else {
                                left.wrapping_add(right)
                            };
                            assert_eq!(
                                vm.bus().ram().read_word(address),
                                expected,
                                "{name}: {left:04X}/{right:04X}, {opt}/{rt:?}/{origin:04X}/{crlf}"
                            );
                            if name == "AcrossCall" {
                                assert_eq!(vm.bus().ram().read_word(0xD0), 0xA55A);
                                assert_eq!(vm.bus().ram().read_word(0xD2), 0x1234);
                            }
                            calls += 1;
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "word move store: {calls} calls, raw/optimized NIR, both runtimes/origins and LF/CRLF"
    );
}
