//! Constructor storage elision preserves complete returned values and effects.
use actionc::{
    codegen,
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
struct NoHooks;
impl VmRunHooks for NoHooks {
    type Error = String;
    fn before_step(&mut self, _: &mut CompilerVm) -> Result<(), String> {
        Ok(())
    }
}
fn compile(
    source: &str,
    runtime: Runtime,
    optimized: bool,
    enabled: bool,
) -> codegen::CodegenOutput {
    let file = support::Source::new(source);
    let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
        &loaded.program,
        &model,
    ));
    let nir = if optimized {
        actionc::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    let mut config = if optimized {
        Mir6502Config::optimized()
    } else {
        Mir6502Config::default()
    };
    config.enable_constructor_store_elision = enabled;
    mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, &config, runtime).unwrap()
}
fn storage(output: &codegen::CodegenOutput, name: &str) -> u16 {
    output
        .map
        .storage_symbols
        .iter()
        .find(|s| s.name == name)
        .unwrap()
        .address
}
fn routine(output: &codegen::CodegenOutput, name: &str) -> u16 {
    output
        .routine_addresses
        .iter()
        .find(|r| r.name == name)
        .unwrap()
        .address
}
fn load(output: &codegen::CodegenOutput, runtime: Runtime) -> CompilerVm {
    let (mut vm, profile) = support::vm_for(runtime);
    vm.load_atari_object_for_execution(profile, &codegen::format_load_file(output))
        .unwrap();
    vm
}

#[test]
fn constructor_storage_preserves_complete_images_at_size_boundaries() {
    for (definition, constructed, payload) in [
        (
            "TYPE Choice=VARIANT [EMPTY FOUND]",
            "Choice.FOUND",
            Vec::<u8>::new(),
        ),
        (
            "TYPE Choice=VARIANT [EMPTY FOUND [CARD address]]",
            "Choice.FOUND(n)",
            vec![0x34, 0x12],
        ),
        (
            "TYPE Inner=VARIANT [NO YES [BYTE x]]\nTYPE Choice=VARIANT [EMPTY FOUND [Inner item CARD number]]",
            "Choice.FOUND(Inner.YES(42),n)",
            vec![2, 42, 0x34, 0x12],
        ),
        (
            "TYPE Choice=VARIANT [EMPTY FOUND [CARD a,b,c BYTE d]]",
            "Choice.FOUND(n,n,n,42)",
            vec![0x34, 0x12, 0x34, 0x12, 0x34, 0x12, 42],
        ),
        (
            "TYPE Choice=VARIANT [EMPTY FOUND [CARD a,b,c,d]]",
            "Choice.FOUND(n,n,n,n)",
            vec![0x34, 0x12, 0x34, 0x12, 0x34, 0x12, 0x34, 0x12],
        ),
    ] {
        let source = format!(
            "{definition}\nChoice result\nChoice FUNC Make(CARD n)\nIF n=0 THEN RETURN(Choice.EMPTY) FI\nRETURN({constructed})\nPROC Run(CARD n)\nresult=Make(n)\nRETURN\nPROC Main()\nRETURN\n"
        );
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for optimized in [false, true] {
                let off = compile(&source, runtime, optimized, false);
                let on = compile(&source, runtime, optimized, true);
                if payload.len() < 8 {
                    assert!(
                        on.bytes.len() < off.bytes.len(),
                        "{definition}: {runtime:?} {optimized}: {} -> {}",
                        off.bytes.len(),
                        on.bytes.len()
                    );
                } else {
                    assert_eq!(
                        codegen::format_load_file(&off),
                        codegen::format_load_file(&on),
                        "larger copies retain the fallback"
                    );
                }
                let crlf = compile(&source.replace('\n', "\r\n"), runtime, optimized, true);
                assert_eq!(
                    codegen::format_load_file(&on),
                    codegen::format_load_file(&crlf)
                );
                for output in [off, on] {
                    let mut vm = load(&output, runtime);
                    for populated in [true, false, true, false] {
                        let result = storage(&output, "result");
                        vm.bus_mut()
                            .ram_mut()
                            .map(result, &vec![0xCC; payload.len() + 1])
                            .unwrap();
                        vm = machine::call(
                            vm,
                            &mut NoHooks,
                            routine(&output, "Run"),
                            &if populated { 0x1234u16 } else { 0 }.to_le_bytes(),
                        );
                        let mut expected = vec![if populated { 2 } else { 1 }];
                        expected.extend(if populated {
                            payload.clone()
                        } else {
                            vec![0; payload.len()]
                        });
                        assert_eq!(machine::bytes(&vm, result, expected.len()), expected);
                    }
                }
            }
        }
    }
}

#[test]
fn constructor_storage_keeps_effectful_payload_order_and_reentry() {
    let source = "TYPE Choice=VARIANT [EMPTY FOUND [BYTE a,b]]\nChoice result\nBYTE value,calls,depth\nPROC POINTER callback()\nBYTE FUNC Change()\ncalls==+1\nvalue=99\nIF depth=0 THEN\ndepth=1\ncallback()\ndepth=0\nFI\nRETURN(7)\nChoice FUNC Make()\nRETURN(Choice.FOUND(value,Change()))\nPROC Run()\nresult=Make()\nRETURN\nPROC Main()\ncallback=Run\nvalue=42\nRun()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            let mut expected = None;
            for enabled in [false, true] {
                let output = compile(source, runtime, optimized, enabled);
                let mut vm = load(&output, runtime);
                vm = machine::call(vm, &mut NoHooks, routine(&output, "Main"), &[]);
                assert_eq!(vm.bus().ram().read(storage(&output, "calls")), 2);
                assert_eq!(vm.bus().ram().read(storage(&output, "depth")), 0);
                let observed = machine::bytes(&vm, storage(&output, "result"), 3);
                assert_eq!((observed[0], observed[2]), (2, 7));
                if let Some(expected) = &expected {
                    assert_eq!(&observed, expected);
                } else {
                    expected = Some(observed);
                }
            }
        }
    }
}
