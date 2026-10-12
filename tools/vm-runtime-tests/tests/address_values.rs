//! Address values, retained source pointer stores, and public result lanes.
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
struct Source(PathBuf);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn named_address(listing: &str, label: &str) -> u16 {
    if let Some(address) = listing
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{label} = $")))
    {
        return u16::from_str_radix(address, 16).unwrap();
    }
    listing
        .lines()
        .skip_while(|line| *line != format!("{label}:"))
        .find_map(|line| {
            line.split_once("; $")
                .and_then(|(_, rest)| rest.split_once(':'))
                .and_then(|(address, _)| u16::from_str_radix(address, 16).ok())
        })
        .unwrap_or_else(|| panic!("missing {label}"))
}

#[test]
fn known_record_addresses_preserve_locals_fields_and_neighbors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text =
        std::fs::read_to_string(root.join("surveys/tn/reproducers/known-record-address.act"))
            .unwrap()
            .replace("\r\n", "\n");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let source = Source(std::env::temp_dir().join(format!(
                    "actionc-known-address-{}-{mode:?}-{runtime:?}-{crlf}.act",
                    std::process::id()
                )));
                std::fs::write(
                    &source.0,
                    if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.clone()
                    },
                )
                .unwrap();
                let compiled = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
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
                let ordinary = symbols::global_address(&listing, "ordinary");
                for value in [0u16, 1, 0x00FF, 0x0100, 0xFEDC] {
                    let mut guarded = vec![0xA5; 35];
                    guarded[17..19].copy_from_slice(&value.to_le_bytes());
                    vm.bus_mut().ram_mut().map(0x5FF0, &guarded).unwrap();
                    vm.bus_mut()
                        .ram_mut()
                        .map(ordinary, &[0xA5, value as u8, (value >> 8) as u8])
                        .unwrap();
                    for (name, expected) in [
                        ("ReadFixed", 0xA5),
                        ("ReadFixedWord", value),
                        ("ReadLocal", value),
                        ("ReadOrdinary", value),
                        ("FieldAddress", 0x6001),
                    ] {
                        vm = machine::call(
                            vm,
                            &mut Hooks,
                            symbols::routine_address(&listing, name),
                            &[],
                        );
                        if name == "ReadFixed" {
                            assert_eq!(vm.bus().ram().read(0xA0), expected as u8);
                        } else {
                            assert_eq!(
                                vm.bus().ram().read_word(0xA0),
                                expected,
                                "{mode:?}/{runtime:?}/{crlf}/{name}"
                            );
                        }
                        if matches!(name, "ReadLocal" | "ReadOrdinary") {
                            let p = named_address(&listing, &format!("local_{name}_p"));
                            assert_eq!(
                                vm.bus().ram().read_word(p),
                                if name == "ReadLocal" {
                                    0x6000
                                } else {
                                    ordinary
                                }
                            );
                        }
                        assert_eq!(machine::bytes(&vm, 0x5FF0, 35), guarded);
                    }
                    let written = value ^ 0x55AA;
                    guarded[17..19].copy_from_slice(&written.to_le_bytes());
                    vm = machine::call(
                        vm,
                        &mut Hooks,
                        symbols::routine_address(&listing, "WriteLocal"),
                        &written.to_le_bytes(),
                    );
                    assert_eq!(
                        vm.bus()
                            .ram()
                            .read_word(named_address(&listing, "local_WriteLocal_p")),
                        0x6000
                    );
                    assert_eq!(machine::bytes(&vm, 0x5FF0, 35), guarded);
                }
            }
        }
    }
}
