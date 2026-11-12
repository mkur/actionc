use actionc::{
    codegen::{self, CodegenOutput, CodegenSymbolScope},
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::VmRunHooks;
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

#[test]
fn home_value_invalidation_preserves_countdown_final_zero() {
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};
    for initial in [0, 1, 3, 127, 128, 255] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let text = format!(
                    "BYTE counter,total\nPROC Main()\n FOR counter={initial} TO 0 STEP -1 DO\n total==+1\n OD\nRETURN\n"
                );
                let source = support::Source::new(&if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text
                });
                let out = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
                )
                .unwrap();
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, out.object_bytes())
                    .unwrap();
                let listing = out.source_listing();
                let counter = symbols::global_address(&listing, "counter");
                let total = symbols::global_address(&listing, "total");
                vm.bus_mut().ram_mut().write(total, 0);
                let entry = out.run_address();
                vm = machine::call(vm, &mut Hooks, entry, &[]);
                assert_eq!(
                    vm.bus().ram().read(counter),
                    0,
                    "{initial}/{runtime:?}/{crlf}"
                );
                assert_eq!(vm.bus().ram().read(total), (initial as u8).wrapping_add(1));
            }
        }
    }
}
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

fn global(out: &CodegenOutput, name: &str) -> u16 {
    out.map
        .storage_symbols
        .iter()
        .find(|s| s.name == name && s.scope == CodegenSymbolScope::Global)
        .unwrap()
        .address
}
#[test]
fn home_value_invalidation_keeps_word_snapshots_and_partial_source_mutations() {
    let lf = std::fs::read_to_string(root().join("fixtures/runtime/home_value_invalidation.act"))
        .unwrap()
        .replace("\r\n", "\n");
    let mut cycles = String::new();
    let mut total = 0;
    for opt in [false, true] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut baseline = None;
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
                    let out = mir6502::generate_output_with_config_and_runtime(
                        &nir,
                        origin,
                        &Mir6502Config::optimized(),
                        rt,
                    )
                    .unwrap();
                    let image = codegen::format_load_file(&out);
                    if let Some(bytes) = &baseline {
                        assert_eq!(bytes, &image);
                    } else {
                        baseline = Some(image.clone());
                    }
                    let (mut vm, profile) = support::vm_for(rt);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    let values = if opt && rt == Runtime::ActionCart && origin == 0x3000 && !crlf {
                        (0..=255).map(|i| (i << 8) | i).collect::<Vec<u16>>()
                    } else {
                        vec![0, 1, 0xFF, 0x100, 0x101, 0x7FFF, 0x8000, 0xFFFF]
                    };
                    for &amount in &values {
                        for cursor in [0x2103u16, 0x21FF, 0x270B] {
                            for name in ["CheckRoom", "ChangedSource", "ChangedHigh"] {
                                vm.bus_mut().ram_mut().write_word(0xE8, cursor);
                                let entry = out
                                    .map
                                    .routine_addresses
                                    .iter()
                                    .find(|r| r.name == name)
                                    .unwrap()
                                    .address;
                                let started = vm.cpu().cycles();
                                vm = machine::call(vm, &mut Hooks, entry, &amount.to_le_bytes());
                                let expected = match name {
                                    "CheckRoom" => amount,
                                    "ChangedSource" => 0xA55A,
                                    _ => (amount & 255) | 0xA500,
                                };
                                assert_eq!(
                                    vm.bus().ram().read_word(0xA0),
                                    expected,
                                    "{name}/{amount:04X}"
                                );
                                assert_eq!(
                                    vm.bus().ram().read(global(&out, "verdict")),
                                    u8::from(
                                        amount
                                            > if name == "CheckRoom" {
                                                0x270B - cursor
                                            } else {
                                                0x100
                                            }
                                    )
                                );
                                assert_eq!(vm.bus().ram().read_word(0xE8), cursor);
                                if name == "CheckRoom" {
                                    assert_eq!(
                                        vm.bus().ram().read_word(global(&out, "sum")),
                                        amount.wrapping_add(cursor)
                                    );
                                }
                                if !crlf && amount == 0x101 && cursor == 0x2103 {
                                    cycles.push_str(&format!(
                                        "{opt}\t{rt:?}\t{origin:04X}\t{name}\t{}\n",
                                        vm.cpu().cycles() - started
                                    ));
                                }
                                total += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("ACTIONC_HOME_VALUE_VM_RESULTS") {
        std::fs::write(path, cycles).unwrap();
    }
    eprintln!(
        "home value VM: {total} calls, raw/optimized NIR, both runtimes/origins and actual LF/CRLF"
    );
}

#[allow(dead_code)]
#[path = "support/tn_symbols.rs"]
mod symbols;

/// Optional before/after image oracle, also usable with the frozen compiler's
/// images after the linked compiler has changed.
#[test]
fn home_value_invalidation_real_alloc_image_controls() {
    use actionc_vm::{BusAccess, RunRequest, StopReason, VmRunner};
    let Some(directory) = std::env::var_os("ACTIONC_HOME_VALUE_TN_IMAGES") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let mut cycles = String::new();
    let mut total = 0;
    for name in ["TN", "TNDBG"] {
        for rt in [Runtime::ActionCart, Runtime::Standalone] {
            let stem = format!(
                "{name}-{}",
                if rt == Runtime::ActionCart {
                    "cart"
                } else {
                    "standalone"
                }
            );
            let image = std::fs::read(directory.join(format!("{stem}.xex"))).unwrap();
            let listing = std::fs::read_to_string(directory.join(format!("{stem}.asm")))
                .unwrap()
                .replace("\r\n", "\n");
            let alloc = symbols::routine_address(&listing, "Alloc");
            let fault = symbols::global_address(&listing, "ArenaFault");
            let stop = symbols::routine_address(&listing, "ArenaStop");
            let end = listing
                .lines()
                .find(|l| l.starts_with("; ===== PROC ArenaStop "))
                .unwrap()
                .split("..$")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap();
            let end = u16::from_str_radix(end, 16).unwrap();
            for (cursor, amount, reason) in [
                (0x2000u16, 0u16, 0u8),
                (0x2000, 1, 0),
                (0x20FF, 0x100, 0),
                (0x27FF, 1, 0),
                (0x2800, 0, 0),
                (0x2000, 0x800, 0),
                (0x2000, 0xFFFF, 2),
                (0x2800, 1, 2),
                (0x27FF, 2, 2),
                (0x1FFF, 0, 1),
                (0x2801, 0, 1),
            ] {
                let (mut vm, profile) = support::vm_for(rt);
                vm.load_atari_object_for_execution(profile, &image).unwrap();
                vm.bus_mut()
                    .ram_mut()
                    .map(0x1FFF, &vec![0x5A; 0x802])
                    .unwrap();
                vm.bus_mut().ram_mut().write_word(0xE8, cursor);
                let started = vm.cpu().cycles();
                if reason == 0 {
                    vm = machine::call(vm, &mut Hooks, alloc, &amount.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(0xA0), cursor);
                    assert_eq!(vm.bus().ram().read_word(0xE8), cursor + amount);
                    assert_eq!(vm.bus().ram().read(fault), 0);
                    cycles.push_str(&format!(
                        "{stem}\t{cursor}\t{amount}\t{}\n",
                        vm.cpu().cycles() - started
                    ));
                } else {
                    let [lo, hi] = amount.to_le_bytes();
                    vm.bus_mut()
                        .ram_mut()
                        .map(
                            machine::ENTRY,
                            &[
                                0xA9,
                                lo,
                                0xA2,
                                hi,
                                0x20,
                                alloc as u8,
                                (alloc >> 8) as u8,
                                0x4C,
                                machine::STOP as u8,
                                (machine::STOP >> 8) as u8,
                            ],
                        )
                        .unwrap();
                    vm.set_pc(machine::ENTRY);
                    vm.bus_mut().clear_events();
                    let result = VmRunner::new(vm).run(RunRequest {
                        max_steps: 1500,
                        stop_after_pc: Some(machine::STOP),
                        history_len: 8,
                    });
                    assert_eq!(
                        result.stop_reason(),
                        StopReason::StepLimit { max_steps: 1500 }
                    );
                    assert!((stop..end).contains(&result.report.registers.pc));
                    vm = result.into_vm();
                    assert_eq!(vm.bus().ram().read(fault), reason);
                    assert_eq!(vm.bus().ram().read_word(0xE8), cursor);
                    assert!(
                        !vm.bus()
                            .events()
                            .iter()
                            .any(|e| e.access == BusAccess::Write
                                && ((0xE8..=0xE9).contains(&e.address)
                                    || (0x1FFF..=0x2800).contains(&e.address)))
                    );
                }
                assert_eq!(machine::bytes(&vm, 0x1FFF, 0x802), vec![0x5A; 0x802]);
                total += 1;
            }
        }
    }
    if let Some(path) = std::env::var_os("ACTIONC_HOME_VALUE_ALLOC_CYCLES") {
        std::fs::write(path, cycles).unwrap();
    }
    eprintln!("real Alloc: {total} successful/fault controls over four fresh images");
}
