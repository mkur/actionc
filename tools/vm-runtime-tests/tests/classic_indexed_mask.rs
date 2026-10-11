//! Indexed loads must leave A's loaded value distinct from X's index.
use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile};

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
impl actionc_vm::VmRunHooks for Hooks {
    type Error = String;
    fn before_step(&mut self, _: &mut CompilerVm) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn indexed_mask_capture_survives_indirect_set_clear_and_branches() {
    let source = std::fs::read_to_string(root().join("surveys/tn/reproducers/indexed-mask.act"))
        .unwrap()
        .replace("\r\n", "\n");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let file = Source(std::env::temp_dir().join(format!(
                    "actionc-indexed-mask-{}-{mode:?}-{runtime:?}-{crlf}.act",
                    std::process::id()
                )));
                std::fs::write(
                    &file.0,
                    if crlf {
                        source.replace('\n', "\r\n")
                    } else {
                        source.clone()
                    },
                )
                .unwrap();
                let compiled = compile_file(
                    &file.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap();
                let listing = compiled.source_listing();
                let apply = symbols::routine_address(&listing, "Apply");
                let table = symbols::global_address(&listing, "masks");
                let mut vm = if runtime == Runtime::ActionCart {
                    machine::load(&compiled)
                } else {
                    let mut vm = CompilerVm::default();
                    vm.load_atari_object_for_execution(
                        ExecutionProfile::StandaloneObject,
                        compiled.object_bytes(),
                    )
                    .unwrap();
                    vm
                };
                for ordinal in [0u16, 1, 2, 3, 4, 5, 6, 7, 8, 63, 64, 255, 256, 4095] {
                    for initial in [0u8, 0x55, 0xAA, 0xFF] {
                        for value in [0u8, 1] {
                            let mut expected = vec![initial; 514];
                            expected[0] = 0xA5;
                            expected[513] = 0x5A;
                            vm.bus_mut().ram_mut().map(0x6000, &expected).unwrap();
                            let index = 1 + usize::from(ordinal / 8);
                            let mask = 1 << (ordinal & 7);
                            if value != 0 {
                                expected[index] |= mask;
                            } else {
                                expected[index] &= !mask;
                            }
                            let mut args = [0x6001u16, ordinal]
                                .into_iter()
                                .flat_map(u16::to_le_bytes)
                                .collect::<Vec<_>>();
                            args.push(value);
                            vm = machine::call(vm, &mut Hooks, apply, &args);
                            assert_eq!(
                                machine::bytes(&vm, 0x6000, 514),
                                expected,
                                "{mode:?}/{runtime:?}/crlf={crlf}/ordinal={ordinal}/initial={initial:02X}/value={value}"
                            );
                            assert_eq!(
                                machine::bytes(&vm, table, 8),
                                [1, 2, 4, 8, 16, 32, 64, 128]
                            );
                        }
                    }
                }
            }
        }
    }
}
