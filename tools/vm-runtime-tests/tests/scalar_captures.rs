//! Scalar value captures retain evaluation order, CASE snapshots and ABI state.
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
    arguments: bool,
    bindings: bool,
) -> codegen::CodegenOutput {
    compile_options(
        source, runtime, optimized, arguments, bindings, true, 0x3000,
    )
}
fn compile_options(
    source: &str,
    runtime: Runtime,
    optimized: bool,
    arguments: bool,
    bindings: bool,
    across_calls: bool,
    origin: u16,
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
    config.enable_argument_capture_forwarding = arguments;
    config.enable_pattern_binding_forwarding = bindings;
    config.enable_scalar_capture_call_elision = across_calls;
    mir6502::generate_output_with_config_and_runtime(&nir, origin, &config, runtime).unwrap()
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
fn pattern_binding_storage_is_elided_across_exact_callees() {
    let source = "TYPE Choice=VARIANT [NONE SOME [CARD value]]\nChoice input\nCARD answer\nPROC Save(CARD n)\nanswer=n\nRETURN\nPROC Relay(CARD n)\nSave(n)\nRETURN\nBYTE FUNC ReadPointer()\nBYTE POINTER p=$9000\nRETURN(p^)\nPROC Run()\nCASE input OF\nWHEN Choice.SOME(value) THEN\nRelay(value)\nWHEN Choice.NONE THEN\nanswer=ReadPointer()\nESAC\nRETURN\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            let before = compile_options(source, runtime, optimized, true, true, false, 0x3000);
            let after = compile_options(source, runtime, optimized, true, true, true, 0x3000);
            assert!(after.bytes.len() <= before.bytes.len());
            if !optimized {
                assert!(after.bytes.len() < before.bytes.len());
            }
            let crlf = compile_options(
                &source.replace('\n', "\r\n"),
                runtime,
                optimized,
                true,
                true,
                true,
                0x3000,
            );
            assert_eq!(
                codegen::format_load_file(&after),
                codegen::format_load_file(&crlf)
            );
            for output in [before, after] {
                let mut vm = load(&output, runtime);
                vm.bus_mut().ram_mut().map(0x9000, &[0xA7]).unwrap();
                for n in [0u16, 1, 0x12FF, 0xFF00, 0xFFFF] {
                    for tag in [2, 1] {
                        let value = [tag, n as u8, (n >> 8) as u8];
                        vm.bus_mut()
                            .ram_mut()
                            .map(storage(&output, "input"), &value)
                            .unwrap();
                        vm = machine::call(vm, &mut NoHooks, routine(&output, "Run"), &[]);
                        assert_eq!(
                            vm.bus().ram().read_word(storage(&output, "answer")),
                            if tag == 2 { n } else { 0xA7 }
                        );
                        assert_eq!(machine::bytes(&vm, storage(&output, "input"), 3), value);
                    }
                }
            }
        }
    }
}

#[test]
fn argument_address_captures_keep_relocations_after_forwarding() {
    let source = "TYPE Data=[CARD value]\nData result\nBYTE ARRAY buffer(4)\nPROC Marker()\nRETURN\nData FUNC Make(CARD n)\nData item\nitem.value=n\nRETURN(item)\nPROC CodeAddress()\nresult=Make(CARD(@Marker))\nRETURN\nPROC DataAddress()\nresult=Make(CARD(buffer))\nRETURN\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        // Raw NIR needs capture forwarding; optimized NIR may already fold it.
        for (optimized, origin) in [(false, 0x3000), (true, 0x3200)] {
            let before = compile_options(source, runtime, optimized, false, false, false, origin);
            let after = compile_options(source, runtime, optimized, true, false, true, origin);
            assert!(after.bytes.len() <= before.bytes.len());
            if !optimized {
                assert!(after.bytes.len() < before.bytes.len());
            }
            for output in [before, after] {
                let mut vm = load(&output, runtime);
                for (name, expected) in [
                    ("CodeAddress", routine(&output, "Marker")),
                    ("DataAddress", storage(&output, "buffer")),
                ] {
                    vm = machine::call(vm, &mut NoHooks, routine(&output, name), &[]);
                    assert_eq!(
                        vm.bus().ram().read_word(storage(&output, "result")),
                        expected
                    );
                }
            }
        }
    }
}

#[test]
fn argument_captures_keep_order_when_later_arguments_change_the_source() {
    let source = "TYPE Pair=[CARD first BYTE second]\nPair result\nCARD number\nCARD FUNC Change()\nnumber=999\nRETURN(7)\nPair FUNC Make(CARD n BYTE b)\nPair item\nitem.first=n\nitem.second=b\nRETURN(item)\nPROC Plain()\nresult=Make(number,42)\nRETURN\nPROC Nested()\nresult=Make(number,Change())\nRETURN\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            let off = compile(source, runtime, optimized, false, false);
            let on = compile(source, runtime, optimized, true, false);
            assert!(
                on.bytes.len() < off.bytes.len(),
                "argument optimization must save bytes: {runtime:?} {optimized}: {} -> {}",
                off.bytes.len(),
                on.bytes.len()
            );
            // Exercise CRLF through the actual source/include parser.
            let crlf = compile(
                &source.replace('\n', "\r\n"),
                runtime,
                optimized,
                true,
                false,
            );
            assert_eq!(
                codegen::format_load_file(&on),
                codegen::format_load_file(&crlf)
            );
            for output in [off, on] {
                let mut vm = load(&output, runtime);
                let result = storage(&output, "result");
                let number = storage(&output, "number");
                for n in [0u16, 1, 0x1234, 0xFFFF] {
                    for (name, byte) in [("Plain", 42), ("Nested", 7)] {
                        vm.bus_mut()
                            .ram_mut()
                            .map(number, &n.to_le_bytes())
                            .unwrap();
                        vm.bus_mut().ram_mut().map(result, &[0xCC; 3]).unwrap();
                        vm = machine::call(vm, &mut NoHooks, routine(&output, name), &[]);
                        assert_eq!(
                            machine::bytes(&vm, result, 3),
                            [n as u8, (n >> 8) as u8, byte]
                        );
                        assert_eq!(
                            vm.bus().ram().read_word(number),
                            if name == "Nested" { 999 } else { n }
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn pattern_bindings_keep_payloads_and_remove_unobserved_storage() {
    let source = "TYPE Maybe=VARIANT [NONE SOME [CARD value]]\nMaybe sample\nCARD answer\nPROC Match()\nCASE sample OF\nWHEN Maybe.SOME(value) THEN\nanswer=value\nWHEN Maybe.NONE THEN\nanswer=0\nESAC\nRETURN\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            let off = compile(source, runtime, optimized, false, false);
            let on = compile(source, runtime, optimized, false, true);
            assert!(
                if optimized {
                    on.bytes.len() <= off.bytes.len()
                } else {
                    on.bytes.len() < off.bytes.len()
                },
                "binding optimization must save bytes: {runtime:?} {optimized}: {} -> {}",
                off.bytes.len(),
                on.bytes.len()
            );
            let crlf = compile(
                &source.replace('\n', "\r\n"),
                runtime,
                optimized,
                false,
                true,
            );
            assert_eq!(
                codegen::format_load_file(&on),
                codegen::format_load_file(&crlf)
            );
            for output in [off, on] {
                let mut vm = load(&output, runtime);
                for n in [1u16, 0xFFFF, 0, 0x1234] {
                    for tag in [2, 1] {
                        let bytes = [tag, n as u8, (n >> 8) as u8];
                        vm.bus_mut()
                            .ram_mut()
                            .map(storage(&output, "sample"), &bytes)
                            .unwrap();
                        vm = machine::call(vm, &mut NoHooks, routine(&output, "Match"), &[]);
                        assert_eq!(
                            vm.bus().ram().read_word(storage(&output, "answer")),
                            if tag == 2 { n } else { 0 }
                        );
                        assert_eq!(machine::bytes(&vm, storage(&output, "sample"), 3), bytes);
                    }
                }
            }
        }
    }
}

#[test]
fn guard_calls_keep_case_snapshots_and_reinitialize_bindings() {
    let source = "TYPE Event=VARIANT [NONE KEY [CARD code]]\nEvent current\nCARD answer\nBYTE calls\nBYTE FUNC Reject(CARD old)\ncalls==+1\ncurrent=Event.NONE\nRETURN(0)\nPROC Match(CARD n)\ncurrent=Event.KEY(n)\nCASE current OF\nWHEN Event.KEY(code) IF Reject(code) THEN\nanswer=65535\nWHEN Event.KEY(code) IF code>3 THEN\nanswer=code\nELSE\nanswer=0\nESAC\nRETURN\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            for (arguments, bindings) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                let output = compile(source, runtime, optimized, arguments, bindings);
                let mut vm = load(&output, runtime);
                for (iteration, n) in [42u16, 1, 0x1234, 0xFFFF, 0].into_iter().enumerate() {
                    vm = machine::call(
                        vm,
                        &mut NoHooks,
                        routine(&output, "Match"),
                        &n.to_le_bytes(),
                    );
                    assert_eq!(
                        vm.bus().ram().read_word(storage(&output, "answer")),
                        if n > 3 { n } else { 0 }
                    );
                    assert_eq!(
                        vm.bus().ram().read(storage(&output, "calls")),
                        (iteration + 1) as u8
                    );
                    assert_eq!(vm.bus().ram().read(storage(&output, "current")), 1);
                }
            }
        }
    }
}

#[test]
fn argument_captures_preserve_static_frame_reentry_behavior() {
    let source = "TYPE Pair=[CARD first BYTE second]\nPair result\nCARD number\nBYTE depth,calls\nPROC POINTER again()\nBYTE FUNC NextValue()\ncalls==+1\nIF depth=0 THEN\ndepth=1\nnumber=23\nagain()\ndepth=0\nFI\nRETURN(5)\nPair FUNC Make(CARD n BYTE b)\nPair item\nitem.first=n\nitem.second=b\nRETURN(item)\nPROC Gather()\nresult=Make(number,NextValue())\nRETURN\nPROC Main()\nagain=Gather\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            let mut expected = None;
            for enabled in [false, true] {
                let output = compile(source, runtime, optimized, enabled, enabled);
                let mut vm = load(&output, runtime);
                vm = machine::call(vm, &mut NoHooks, routine(&output, "Main"), &[]);
                vm.bus_mut()
                    .ram_mut()
                    .map(storage(&output, "number"), &7u16.to_le_bytes())
                    .unwrap();
                vm = machine::call(vm, &mut NoHooks, routine(&output, "Gather"), &[]);
                let observed = (
                    machine::bytes(&vm, storage(&output, "result"), 3),
                    vm.bus().ram().read(storage(&output, "depth")),
                    vm.bus().ram().read(storage(&output, "calls")),
                );
                assert_eq!((observed.1, observed.2), (0, 2));
                if let Some(expected) = &expected {
                    assert_eq!(&observed, expected);
                } else {
                    expected = Some(observed);
                }
            }
        }
    }
}
