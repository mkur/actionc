//! Source-level oracle for direct byte stores and counted pointer forwarding.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, VmRunHooks};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
#[path = "support/tn_symbols.rs"]
mod symbols;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

#[test]
fn direct_bytes_and_counted_copies_preserve_values_and_guards() {
    let source =
        std::fs::read_to_string(root().join("fixtures/runtime/register_copy_forwarding.act"))
            .unwrap()
            .replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for crlf in [false, true] {
            let file = support::Source::new(&if crlf {
                source.replace('\n', "\r\n")
            } else {
                source.clone()
            });
            let compiled = compile_file(
                &file.0,
                &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
            )
            .unwrap();
            let listing = compiled.source_listing();
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
            let observed = symbols::global_address(&listing, "observed");
            let save = symbols::routine_address(&listing, "SaveBytes");
            for value in [0, 1, 127, 128, 254, 255] {
                vm = machine::call(vm, &mut Hooks, save, &[value, !value, value ^ 0x55]);
                assert_eq!(
                    machine::bytes(&vm, observed, 3),
                    [value ^ 0x55, value, !value]
                );
            }
            for name in ["CopyCounted", "SendString"] {
                let entry = symbols::routine_address(&listing, name);
                for (input, output) in [(0x5000u16, 0x5300u16), (0x50FE, 0x53FE)] {
                    for length in [0u8, 1, 127, 128, 254, 255] {
                        let mut data: Vec<_> =
                            (0..256).map(|i| (i as u8).wrapping_mul(37)).collect();
                        data[0] = length;
                        let count = usize::from(length) + 1;
                        vm.bus_mut().ram_mut().map(input, &data).unwrap();
                        vm.bus_mut()
                            .ram_mut()
                            .map(output - 1, &vec![0xCC; 258])
                            .unwrap();
                        let mut args = output.to_le_bytes().to_vec();
                        args.extend(input.to_le_bytes());
                        vm = machine::call(vm, &mut Hooks, entry, &args);
                        assert_eq!(
                            machine::bytes(&vm, output, count),
                            data[..count],
                            "{name}/{runtime:?}/crlf={crlf}/{length}"
                        );
                        assert_eq!(vm.bus().ram().read(output - 1), 0xCC);
                        assert_eq!(vm.bus().ram().read(output + count as u16), 0xCC);
                        assert_eq!(machine::bytes(&vm, input, 256), data);
                    }
                }
            }
        }
    }
}
