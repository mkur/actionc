//! Independent execution and measurements of the current TN audit's generic forms.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, VmRunHooks};
use std::path::{Path, PathBuf};
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[path = "support/tn_symbols.rs"]
mod symbols;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
struct Source(PathBuf);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

#[test]
fn escaped_and_direct_record_addresses_preserve_words_and_guards() {
    let source =
        std::fs::read_to_string(root().join("surveys/tn/reproducers/escaping-record-address.act"))
            .unwrap()
            .replace("\r\n", "\n");
    println!("audit-reproducer,backend,runtime,crlf,routine,index,cycles_including_wrapper");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let path = Source(std::env::temp_dir().join(format!(
                    "actionc-address-audit-{}-{mode:?}-{runtime:?}-{crlf}.act",
                    std::process::id()
                )));
                std::fs::write(
                    &path.0,
                    if crlf {
                        source.replace('\n', "\r\n")
                    } else {
                        source.clone()
                    },
                )
                .unwrap();
                let compiled = compile_file(
                    &path.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap();
                let listing = compiled.source_listing();
                let mut vm = if runtime == Runtime::ActionCart {
                    machine::load(&compiled)
                } else {
                    let mut vm = CompilerVm::default();
                    let loaded = vm
                        .load_atari_object_for_execution(
                            ExecutionProfile::StandaloneObject,
                            compiled.object_bytes(),
                        )
                        .unwrap();
                    assert!(
                        loaded
                            .segments
                            .iter()
                            .all(|s| s.end < 0x4500 || s.start >= 0x8000)
                    );
                    vm
                };
                for base in [0x5000u16, 0x50F1] {
                    for name in [
                        "Address",
                        "AddressExpanded",
                        "ReadViaLocal",
                        "ReadDirect",
                        "AddressZero",
                        "AddressField",
                    ] {
                        let entry = symbols::routine_address(&listing, name);
                        let indices: &[u16] = if matches!(name, "AddressZero" | "AddressField") {
                            &[0]
                        } else if name.starts_with("Address") {
                            &[0, 1, 63, 127, 255]
                        } else {
                            &[0, 1, 63, 127, 255, 257, 65535]
                        };
                        for &index in indices {
                            let field = base.wrapping_add(38).wrapping_add(index.wrapping_mul(20));
                            assert!((0x4500..0x7FFF).contains(&field));
                            let mut expected = vec![0xA5; 0x3B00];
                            expected[usize::from(field - 0x4500)..usize::from(field - 0x4500) + 2]
                                .copy_from_slice(&0x73D9u16.to_le_bytes());
                            vm.bus_mut().ram_mut().map(0x4500, &expected).unwrap();
                            let mut args = base.to_le_bytes().to_vec();
                            args.push(index as u8);
                            if !name.starts_with("Address") {
                                args.push((index >> 8) as u8);
                            }
                            let start = vm.cpu().cycles();
                            vm = machine::call(vm, &mut Hooks, entry, &args);
                            let result = vm.bus().ram().read_word(0xA0);
                            assert_eq!(
                                result,
                                if name.starts_with("Address") {
                                    field
                                } else {
                                    0x73D9
                                },
                                "{mode:?}/{runtime:?}/crlf={crlf}/{name}/{base:04X}/{index}"
                            );
                            assert_eq!(
                                machine::bytes(&vm, 0x4500, 0x3B00),
                                expected,
                                "{name}: full data and neighboring guards"
                            );
                            if base == 0x5000 {
                                println!(
                                    "audit-reproducer,{mode:?},{runtime:?},{crlf},{name},{index},{}",
                                    vm.cpu().cycles() - start
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

fn load(compiled: &actionc::compiler::CompiledProgram, runtime: Runtime) -> CompilerVm {
    if runtime == Runtime::ActionCart {
        machine::load(compiled)
    } else {
        let mut vm = CompilerVm::default();
        vm.load_atari_object_for_execution(
            ExecutionProfile::StandaloneObject,
            compiled.object_bytes(),
        )
        .unwrap();
        vm
    }
}

#[test]
fn pure_call_predicate_and_bounded_masks_preserve_results() {
    for fixture in ["captured-call-arguments", "bounded-bit-mask"] {
        let source =
            std::fs::read_to_string(root().join(format!("surveys/tn/reproducers/{fixture}.act")))
                .unwrap()
                .replace("\r\n", "\n");
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                for crlf in [false, true] {
                    let path = Source(std::env::temp_dir().join(format!(
                        "actionc-{fixture}-audit-{}-{mode:?}-{runtime:?}-{crlf}.act",
                        std::process::id()
                    )));
                    std::fs::write(
                        &path.0,
                        if crlf {
                            source.replace('\n', "\r\n")
                        } else {
                            source.clone()
                        },
                    )
                    .unwrap();
                    let compiled = compile_file(
                        &path.0,
                        &CompileOptions::for_mode(mode).with_runtime(runtime),
                    )
                    .unwrap();
                    let listing = compiled.source_listing();
                    let mut vm = load(&compiled, runtime);
                    if fixture == "bounded-bit-mask" {
                        let table = symbols::global_address(&listing, "powers");
                        for name in ["ShiftMask", "TableMask"] {
                            let entry = symbols::routine_address(&listing, name);
                            for ordinal in [0u16, 1, 2, 3, 4, 5, 6, 7, 8, 63, 255, 256, 4095, 65535]
                            {
                                let start = vm.cpu().cycles();
                                vm = machine::call(vm, &mut Hooks, entry, &ordinal.to_le_bytes());
                                assert_eq!(
                                    vm.bus().ram().read(0xA0),
                                    1u8 << (ordinal & 7),
                                    "{mode:?}/{runtime:?}/crlf={crlf}/{name}/{ordinal}"
                                );
                                assert_eq!(
                                    machine::bytes(&vm, table, 8),
                                    [1, 2, 4, 8, 16, 32, 64, 128]
                                );
                                println!(
                                    "audit-reproducer,{mode:?},{runtime:?},{crlf},{name},{ordinal},{}",
                                    vm.cpu().cycles() - start
                                );
                            }
                        }
                    } else {
                        for name in ["Forward", "PredicateFirst"] {
                            let entry = symbols::routine_address(&listing, name);
                            for value in [0u16, 1, 0xFF, 0x100, 0xFEDC] {
                                for setting in [0u8, 1, 2, 255] {
                                    let mut expected = vec![0xA5; 34];
                                    expected[1..3].copy_from_slice(
                                        &(if setting == 0 { 0 } else { value }).to_le_bytes(),
                                    );
                                    vm.bus_mut().ram_mut().map(0x6000, &vec![0xA5; 34]).unwrap();
                                    let mut args = 0x6001u16.to_le_bytes().to_vec();
                                    args.extend(value.to_le_bytes());
                                    args.push(setting);
                                    let start = vm.cpu().cycles();
                                    vm = machine::call(vm, &mut Hooks, entry, &args);
                                    assert_eq!(
                                        vm.bus().ram().read(0xA0),
                                        u8::from(setting != 0),
                                        "{mode:?}/{runtime:?}/crlf={crlf}/{name}/{value}/{setting}"
                                    );
                                    assert_eq!(machine::bytes(&vm, 0x6000, 34), expected);
                                    if value == 0xFEDC {
                                        println!(
                                            "audit-reproducer,{mode:?},{runtime:?},{crlf},{name},{setting},{}",
                                            vm.cpu().cycles() - start
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
}
