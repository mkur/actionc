//! Argument snapshots, source ordering and the real TN public normalization wrapper.
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
        if self.0.is_dir() {
            let _ = std::fs::remove_dir_all(&self.0);
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
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
fn words(values: &[u16], tail: &[u8]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .chain(tail.iter().copied())
        .collect()
}

#[test]
fn argument_snapshots_and_effect_order_survive_later_predicates() {
    snapshot_cases(false);
}

#[test]
#[ignore = "Stage 0: MIR6502 loses the word snapshot across a call that aliases its parameter"]
fn aliased_parameter_snapshot_survives_later_argument_call() {
    snapshot_cases(true);
}

fn snapshot_cases(known_failure_only: bool) {
    let text =
        std::fs::read_to_string(root().join("surveys/tn/reproducers/call-predicate-snapshots.act"))
            .unwrap()
            .replace("\r\n", "\n");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for crlf in [false, true] {
                let path = Source(std::env::temp_dir().join(format!(
                    "actionc-call-snapshots-{}-{mode:?}-{runtime:?}-{crlf}.act",
                    std::process::id()
                )));
                std::fs::write(
                    &path.0,
                    if crlf {
                        text.replace('\n', "\r\n")
                    } else {
                        text.clone()
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
                for setting in [0u8, 1, 2, 255] {
                    for name in ["AliasWord", "AliasHigh", "Ordered", "Absolute"] {
                        let known_failure = mode == CompileMode::Mir6502 && name == "AliasWord";
                        if known_failure != known_failure_only {
                            continue;
                        }
                        vm.bus_mut().ram_mut().map(0x0800, &[0; 10]).unwrap();
                        vm.bus_mut()
                            .ram_mut()
                            .map(0x0808, &0xFEDCu16.to_le_bytes())
                            .unwrap();
                        vm.bus_mut().ram_mut().write(0x0804, setting);
                        let args = match name {
                            "Absolute" => Vec::new(),
                            "Ordered" => words(&[0x6001], &[setting]),
                            _ => words(&[0x6001, 0xFEDC], &[setting]),
                        };
                        let cycles = vm.cpu().cycles();
                        vm = machine::call(
                            vm,
                            &mut Hooks,
                            symbols::routine_address(&listing, name),
                            &args,
                        );
                        let label = format!("{mode:?}/{runtime:?}/crlf={crlf}/{name}/{setting}");
                        assert_eq!(vm.bus().ram().read_word(0x0800), 0x6001, "{label}: pointer");
                        assert_eq!(
                            vm.bus().ram().read_word(0x0802),
                            if name == "AliasHigh" { 0x12DC } else { 0xFEDC },
                            "{label}: earlier word snapshot"
                        );
                        assert_eq!(
                            vm.bus().ram().read(0x0804),
                            u8::from(setting != 0),
                            "{label}: normalized byte"
                        );
                        assert_eq!(
                            vm.bus().ram().read(0xA0),
                            u8::from(setting != 0),
                            "{label}: result ABI"
                        );
                        assert_eq!(
                            vm.bus().ram().read(0x0805),
                            u8::from(matches!(name, "AliasWord" | "Ordered")),
                            "{label}: effect multiplicity"
                        );
                        assert_eq!(
                            vm.bus().ram().read(0x0806),
                            if name == "Ordered" { 12 } else { 0 },
                            "{label}: first before last"
                        );
                        println!(
                            "call-predicate,{mode:?},{runtime:?},{crlf},{name},{setting},{}",
                            vm.cpu().cycles() - cycles
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn tn_roots_set_tagged_normalizes_and_preserves_bounds_and_counts() {
    for program in ["TN", "TNDBG"] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            for crlf in [false, true] {
                let dir = Source(std::env::temp_dir().join(format!(
                    "actionc-call-tags-{}-{program}-{mode:?}-{crlf}",
                    std::process::id()
                )));
                std::fs::create_dir_all(&dir.0).unwrap();
                for file in std::fs::read_dir(root().join("samples/tn/modern")).unwrap() {
                    let file = file.unwrap();
                    if file.path().extension().is_some_and(|e| e == "ACT") {
                        let text = std::fs::read_to_string(file.path())
                            .unwrap()
                            .replace("\r\n", "\n");
                        std::fs::write(
                            dir.0.join(file.file_name()),
                            if crlf {
                                text.replace('\n', "\r\n")
                            } else {
                                text
                            },
                        )
                        .unwrap();
                    }
                }
                let compiled = compile_file(
                    dir.0.join(format!("{program}.ACT")),
                    &CompileOptions::for_mode(mode).with_runtime(Runtime::ActionCart),
                )
                .unwrap();
                let listing = compiled.source_listing();
                let mut vm = machine::load(&compiled);
                let selection = 0x7000;
                let bitmap = 0x7101;
                for ordinal in [0u16, 7, 8, 19, 20, 64, 65535] {
                    for setting in [0u8, 1, 2, 255] {
                        let mut state = words(&[bitmap, 64, 20, 0, 0], &[0, 0, 0, 0]);
                        let mut bits = vec![0u8; 8];
                        vm.bus_mut().ram_mut().map(selection, &state).unwrap();
                        vm.bus_mut().ram_mut().map(bitmap - 1, &[0xA5; 10]).unwrap();
                        vm.bus_mut().ram_mut().map(bitmap, &bits).unwrap();
                        let cycles = vm.cpu().cycles();
                        vm = machine::call(
                            vm,
                            &mut Hooks,
                            symbols::routine_address(&listing, "SetTagged"),
                            &words(&[selection, ordinal], &[setting]),
                        );
                        let valid = ordinal < 20;
                        if valid && setting != 0 {
                            bits[usize::from(ordinal / 8)] |= 1 << (ordinal & 7);
                            state[6..8].copy_from_slice(&1u16.to_le_bytes());
                        }
                        assert_eq!(
                            vm.bus().ram().read(0xA0),
                            if valid { 1 } else { 2 },
                            "{program}/{mode:?}/{ordinal}/{setting}"
                        );
                        assert_eq!(machine::bytes(&vm, selection, 14), state, "selection state");
                        assert_eq!(
                            machine::bytes(&vm, bitmap, 8),
                            bits,
                            "bitmap and complete count"
                        );
                        assert_eq!(vm.bus().ram().read(bitmap - 1), 0xA5);
                        assert_eq!(vm.bus().ram().read(bitmap + 8), 0xA5);
                        if !crlf && matches!(ordinal, 0 | 20) {
                            println!(
                                "call-tags,{program},{mode:?},{ordinal},{setting},{}",
                                vm.cpu().cycles() - cycles
                            );
                        }
                    }
                }
            }
        }
    }
}
