//! Reused constructor values must retain complete snapshots, including overlap.
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
    config.enable_copy_value_forwarding = enabled;
    mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, &config, runtime).unwrap()
}

#[test]
fn forwarded_returns_preserve_exposed_sources_and_both_overlap_directions() {
    // Exposure is deliberate. Forwarding keeps the original stores and uses
    // captured values, so it needs no assumption that the source is private.
    let source = "TYPE Value=[BYTE code CARD number]\nTYPE Pad=[BYTE ARRAY bytes(4)]\nBYTE POINTER exposed=$900\nValue FUNC Make()\nPad front\nValue piece\nPad back\nexposed=BYTE POINTER(@front)\nexposed=BYTE POINTER(@back)\nexposed=BYTE POINTER(@piece)\npiece.code=42\npiece.number=$1234\nRETURN(piece)\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            let mut off_size = None;
            for enabled in [false, true] {
                let mut lf_image = None;
                for crlf in [false, true] {
                    let output = compile(
                        &if crlf {
                            source.replace('\n', "\r\n")
                        } else {
                            source.into()
                        },
                        runtime,
                        optimized,
                        enabled,
                    );
                    if !crlf {
                        if enabled {
                            assert!(
                                output.bytes.len() < off_size.unwrap(),
                                "forwarding must be selected: {runtime:?}, optimized={optimized}, {} -> {}",
                                off_size.unwrap(),
                                output.bytes.len()
                            );
                        } else {
                            off_size = Some(output.bytes.len());
                        }
                    }
                    let image = codegen::format_load_file(&output);
                    if let Some(lf) = &lf_image {
                        assert_eq!(&image, lf);
                    } else {
                        lf_image = Some(image.clone());
                    }
                    let guard = output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| s.name == "front")
                        .unwrap()
                        .address;
                    assert_eq!(
                        output
                            .map
                            .storage_symbols
                            .iter()
                            .find(|s| s.name == "piece")
                            .unwrap()
                            .address,
                        guard + 4
                    );
                    assert_eq!(
                        output
                            .map
                            .storage_symbols
                            .iter()
                            .find(|s| s.name == "back")
                            .unwrap()
                            .address,
                        guard + 7
                    );
                    let make = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == "Make")
                        .unwrap()
                        .address;
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    for displacement in [-2i16, -1, 0, 1, 2] {
                        let destination = guard.wrapping_add((4 + displacement) as u16);
                        vm.bus_mut().ram_mut().map(guard, &[0xCC; 11]).unwrap();
                        vm = machine::call(
                            vm,
                            &mut NoHooks,
                            make,
                            &[destination as u8, (destination >> 8) as u8],
                        );
                        assert_eq!(vm.bus().ram().read_word(0x900), guard + 4);
                        let bytes = [42, 0x34, 0x12];
                        let mut expected = [0xCC; 11];
                        expected[4..7].copy_from_slice(&bytes);
                        let offset = (4 + displacement) as usize;
                        expected[offset..offset + 3].copy_from_slice(&bytes);
                        assert_eq!(
                            machine::bytes(&vm, guard, 11),
                            expected,
                            "{runtime:?}, optimized={optimized}, enabled={enabled}, crlf={crlf}, displacement={displacement}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn forwarded_variant_returns_keep_changing_payloads_and_zero_empty_arms() {
    let source = "TYPE Choice=VARIANT [EMPTY FOUND [CARD address]]\nChoice FUNC Make(CARD n)\nIF n=0 THEN RETURN(Choice.EMPTY) FI\nRETURN(Choice.FOUND(n))\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized in [false, true] {
            let before = compile(source, runtime, optimized, false);
            let after = compile(source, runtime, optimized, true);
            assert!(
                after.bytes.len() < before.bytes.len(),
                "{runtime:?}, optimized={optimized}"
            );
            for output in [before, after] {
                let make = output
                    .routine_addresses
                    .iter()
                    .find(|r| r.name == "Make")
                    .unwrap()
                    .address;
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &codegen::format_load_file(&output))
                    .unwrap();
                for n in [0x1234u16, 0, 0xFFFF, 1, 0, 0xABCD] {
                    vm.bus_mut().ram_mut().map(0x900, &[0xCC; 5]).unwrap();
                    vm = machine::call(vm, &mut NoHooks, make, &[1, 9, n as u8, (n >> 8) as u8]);
                    let expected = [
                        0xCC,
                        if n == 0 { 1 } else { 2 },
                        n as u8,
                        (n >> 8) as u8,
                        0xCC,
                    ];
                    assert_eq!(machine::bytes(&vm, 0x900, 5), expected);
                }
                // A result write can overwrite its original argument cells.
                // Payload lanes must remain captured values, not late rereads.
                let argument = output
                    .map
                    .storage_symbols
                    .iter()
                    .find(|s| s.name == "n")
                    .unwrap()
                    .address;
                for destination in [argument - 1, argument, argument + 1] {
                    for n in [0x1234u16, 0xFFFF, 0, 1] {
                        vm = machine::call(
                            vm,
                            &mut NoHooks,
                            make,
                            &[
                                destination as u8,
                                (destination >> 8) as u8,
                                n as u8,
                                (n >> 8) as u8,
                            ],
                        );
                        assert_eq!(
                            machine::bytes(&vm, destination, 3),
                            [if n == 0 { 1 } else { 2 }, n as u8, (n >> 8) as u8]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn forwarded_copy_limit_keeps_all_bytes_and_the_larger_fallback() {
    for size in [8, 9] {
        let writes = (0..size)
            .map(|i| format!("piece.bytes({i})={}\n", 30 + i))
            .collect::<String>();
        let source = format!(
            "TYPE Value=[BYTE ARRAY bytes({size})]\nValue FUNC Make()\nValue piece\n{writes}RETURN(piece)\nPROC Main()\nRETURN\n"
        );
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for optimized in [false, true] {
                let before = compile(&source, runtime, optimized, false);
                let after = compile(&source, runtime, optimized, true);
                if size == 8 {
                    assert!(after.bytes.len() < before.bytes.len());
                } else {
                    assert_eq!(after.bytes, before.bytes);
                }
                for output in [before, after] {
                    let make = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name == "Make")
                        .unwrap()
                        .address;
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(
                        profile,
                        &codegen::format_load_file(&output),
                    )
                    .unwrap();
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x900, &vec![0xCC; size + 2])
                        .unwrap();
                    vm = machine::call(vm, &mut NoHooks, make, &[1, 9]);
                    let mut expected = vec![0xCC];
                    expected.extend((0..size).map(|i| (30 + i) as u8));
                    expected.push(0xCC);
                    assert_eq!(machine::bytes(&vm, 0x900, size + 2), expected);
                }
            }
        }
    }
}
