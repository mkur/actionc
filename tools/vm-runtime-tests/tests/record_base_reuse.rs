//! General record sequences and source-cell aliasing through emitted code.
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
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

fn label_address(listing: &str, label: &str) -> u16 {
    listing
        .lines()
        .skip_while(|line| *line != format!("{label}:"))
        .find_map(|line| {
            line.split_once("; $")
                .and_then(|(_, rest)| rest.split_once(':'))
                .map(|(address, _)| u16::from_str_radix(address, 16).unwrap())
        })
        .unwrap()
}

#[test]
fn record_sequences_keep_fields_snapshots_and_mutable_source_cells() {
    let original =
        std::fs::read_to_string(root().join("surveys/tn/reproducers/record-base-reuse.act"))
            .unwrap()
            .replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for crlf in [false, true] {
            let path = std::env::temp_dir().join(format!(
                "record-bases-{}-{runtime:?}-{crlf}.act",
                std::process::id()
            ));
            std::fs::write(
                &path,
                if crlf {
                    original.replace('\n', "\r\n")
                } else {
                    original.clone()
                },
            )
            .unwrap();
            let compiled = compile_file(
                &path,
                &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
            )
            .unwrap();
            std::fs::remove_file(path).unwrap();
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
            for base in [0x6000u16, 0x60FF] {
                vm.bus_mut()
                    .ram_mut()
                    .map(base - 1, &[0xA5, 0x34, 0x12, 8, 9, 10, 0xCD, 0xAB, 0xA5])
                    .unwrap();
                vm = machine::call(
                    vm,
                    &mut Hooks,
                    symbols::routine_address(&listing, "ReadFields"),
                    &base.to_le_bytes(),
                );
                assert_eq!(vm.bus().ram().read_word(0xA0), 0xBE01);
                let mut args = base.to_le_bytes().to_vec();
                args.extend(0xFEDCu16.to_le_bytes());
                vm = machine::call(
                    vm,
                    &mut Hooks,
                    symbols::routine_address(&listing, "WriteFields"),
                    &args,
                );
                assert_eq!(
                    machine::bytes(&vm, base - 1, 9),
                    [0xA5, 0xDC, 0xFE, 0, 0, 1, 0xCD, 0xAB, 0xA5]
                );
                vm = machine::call(
                    vm,
                    &mut Hooks,
                    symbols::routine_address(&listing, "MixedFields"),
                    &args,
                );
                assert_eq!(
                    vm.bus()
                        .ram()
                        .read_word(symbols::global_address(&listing, "observed")),
                    0xFDB8
                );
                let mut args = base.to_le_bytes().to_vec();
                args.extend(base.to_le_bytes());
                vm = machine::call(
                    vm,
                    &mut Hooks,
                    symbols::routine_address(&listing, "NestedFields"),
                    &args,
                );
                assert_eq!(vm.bus().ram().read_word(base), 0xFEDC);
                assert_eq!(vm.bus().ram().read(base + 2), 0);
                assert_eq!(vm.bus().ram().read(base + 4), 0);
                assert_eq!(vm.bus().ram().read(base - 1), 0xA5);
                assert_eq!(vm.bus().ram().read(base + 7), 0xA5);
            }
            let cell = label_address(&listing, "param_MutateSource_p");
            vm.bus_mut().ram_mut().write(0x6002, 0xA5);
            vm = machine::call(
                vm,
                &mut Hooks,
                symbols::routine_address(&listing, "MutateSource"),
                &cell.to_le_bytes(),
            );
            assert_eq!(vm.bus().ram().read_word(cell), 0x6000);
            assert_eq!(
                vm.bus().ram().read(0x6002),
                0x12,
                "{runtime:?}/{crlf}: must reread the changed source pointer"
            );
        }
    }
}
