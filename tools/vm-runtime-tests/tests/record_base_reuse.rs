//! General record sequences and source-cell aliasing through emitted code.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, VmRunHooks};
use std::path::{Path, PathBuf};
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
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
            let loaded = actionc::includes::load_program_with_expanded_source(&path).unwrap();
            std::fs::remove_file(path).unwrap();
            let model = actionc::semantic::analyze_with_options(
                &loaded.program,
                actionc::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
            let raw = actionc::nir::lower_program(&sem);
            for raw_nir in [false, true] {
                let normalized = if raw_nir {
                    raw.clone()
                } else {
                    actionc::nir::optimize_program(&raw).unwrap()
                };
                for enabled in [false, true] {
                    let mut config = actionc::mir6502::Mir6502Config::optimized();
                    config.enable_record_base_reuse = enabled;
                    config.enable_record_base_writes = false;
                    config.enable_record_base_parameter_origins = false;
                    let output = actionc::mir6502::generate_output_with_config_and_runtime(
                        &normalized,
                        0x2C00,
                        &config,
                        runtime,
                    )
                    .unwrap();
                    let routine = |name: &str| {
                        output
                            .map
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap()
                            .address
                    };
                    let global = |name: &str| {
                        output
                            .map
                            .storage_symbols
                            .iter()
                            .find(|s| {
                                s.name == name
                                    && s.scope == actionc::codegen::CodegenSymbolScope::Global
                            })
                            .unwrap()
                            .address
                    };
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
                    vm.bus_mut()
                        .ram_mut()
                        .map(output.origin, &output.bytes)
                        .unwrap();
                    for base in [0x6000u16, 0x60FF] {
                        vm.bus_mut()
                            .ram_mut()
                            .map(base - 1, &[0xA5, 0x34, 0x12, 8, 9, 10, 0xCD, 0xAB, 0xA5])
                            .unwrap();
                        vm.bus_mut()
                            .ram_mut()
                            .map(global("current"), &base.to_le_bytes())
                            .unwrap();
                        vm = machine::call(vm, &mut Hooks, routine("ReadCurrent"), &[]);
                        assert_eq!(vm.bus().ram().read_word(0xA0), 0xBE01);
                        assert_eq!(vm.bus().ram().read_word(global("observed")), 0x1234);
                        vm = machine::call(
                            vm,
                            &mut Hooks,
                            routine("ReadFields"),
                            &base.to_le_bytes(),
                        );
                        assert_eq!(vm.bus().ram().read_word(0xA0), 0xBE01);
                        let mut args = base.to_le_bytes().to_vec();
                        args.extend(0xFEDCu16.to_le_bytes());
                        vm = machine::call(vm, &mut Hooks, routine("WriteFields"), &args);
                        assert_eq!(
                            machine::bytes(&vm, base - 1, 9),
                            [0xA5, 0xDC, 0xFE, 0, 0, 1, 0xCD, 0xAB, 0xA5]
                        );
                        vm = machine::call(vm, &mut Hooks, routine("MixedFields"), &args);
                        assert_eq!(vm.bus().ram().read_word(global("observed")), 0xFDB8);
                        let mut args = base.to_le_bytes().to_vec();
                        args.extend(base.to_le_bytes());
                        vm = machine::call(vm, &mut Hooks, routine("NestedFields"), &args);
                        assert_eq!(vm.bus().ram().read_word(base), 0xFEDC);
                        assert_eq!(vm.bus().ram().read(base + 2), 0);
                        assert_eq!(vm.bus().ram().read(base + 4), 0);
                        assert_eq!(vm.bus().ram().read(base - 1), 0xA5);
                        assert_eq!(vm.bus().ram().read(base + 7), 0xA5);
                    }
                    let cell = output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| {
                            s.name == "p"
                                && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                                && s.scope
                                    == actionc::codegen::CodegenSymbolScope::Routine(
                                        "MutateSource".into(),
                                    )
                        })
                        .unwrap()
                        .address;
                    vm.bus_mut().ram_mut().write(0x6002, 0xA5);
                    vm =
                        machine::call(vm, &mut Hooks, routine("MutateSource"), &cell.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(cell), 0x6000);
                    assert_eq!(
                        vm.bus().ram().read(0x6002),
                        0x12,
                        "{runtime:?}/{crlf}: must reread the changed source pointer"
                    );
                }
            }
        }
    }
}
