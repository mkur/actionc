//! Exercise tiny fills through real aggregate returns, with poisoned storage.
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

#[test]
fn tiny_fills_preserve_repeated_aggregate_returns_in_both_runtimes() {
    for bytes in [2, 8, 9] {
        let payload = (0..bytes)
            .map(|i| format!("payload.bytes({i})={}\n", 20 + i))
            .collect::<String>();
        let source = format!(
            "TYPE Payload=[BYTE ARRAY bytes({bytes})]\nTYPE Value=VARIANT [EMPTY FULL [Payload payload] SHORT [BYTE value]]\nTYPE Guarded=[BYTE before Value item BYTE after]\nGuarded result\nBYTE mode=$800\nValue FUNC Empty()\nRETURN(Value.EMPTY)\nValue FUNC Short()\nRETURN(Value.SHORT(42))\nValue FUNC Build()\nPayload payload\nIF mode=0 THEN RETURN(Empty()) FI\nIF mode=1 THEN RETURN(Short()) FI\n{payload}RETURN(Value.FULL(payload))\nPROC Run()\nresult.item=Build()\nRETURN\nPROC Main()\nRETURN\n"
        );
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for optimized_nir in [false, true] {
                for optimized_mir in [false, true] {
                    let mut lf_image = None;
                    for crlf in [false, true] {
                        let file = support::Source::new(&if crlf {
                            source.replace('\n', "\r\n")
                        } else {
                            source.clone()
                        });
                        let loaded =
                            actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
                        let model = actionc::semantic::analyze_with_options(
                            &loaded.program,
                            actionc::semantic::SemanticOptions::modern(),
                        )
                        .unwrap();
                        let sem = actionc::semantic::ir::lower_program(&loaded.program, &model);
                        let raw = actionc::nir::lower_program(&sem);
                        let nir = if optimized_nir {
                            actionc::nir::optimize_program(&raw).unwrap()
                        } else {
                            raw
                        };
                        let config = if optimized_mir {
                            Mir6502Config::optimized()
                        } else {
                            Mir6502Config {
                                enable_tiny_constant_fills: false,
                                ..Default::default()
                            }
                        };
                        let output = mir6502::generate_output_with_config_and_runtime(
                            &nir, 0x3000, &config, runtime,
                        )
                        .unwrap();
                        let image = codegen::format_load_file(&output);
                        if let Some(lf) = &lf_image {
                            assert_eq!(&image, lf);
                        } else {
                            lf_image = Some(image.clone());
                        }
                        let result = output
                            .map
                            .storage_symbols
                            .iter()
                            .find(|s| s.name == "result")
                            .unwrap();
                        let run = output
                            .routine_addresses
                            .iter()
                            .find(|r| r.name == "Run")
                            .unwrap()
                            .address;
                        let (mut vm, profile) = support::vm_for(runtime);
                        vm.load_atari_object_for_execution(profile, &image).unwrap();
                        // The same routine-static constructor temporaries are
                        // reused across all six calls. EMPTY/SHORT must clear
                        // every inactive byte left behind by FULL.
                        for mode in [2, 0, 2, 1, 0, 1] {
                            vm.bus_mut()
                                .ram_mut()
                                .map(result.address, &vec![0xCC; bytes + 3])
                                .unwrap();
                            vm.bus_mut().ram_mut().write(0x800, mode);
                            vm = machine::call(vm, &mut NoHooks, run, &[]);
                            let mut expected = vec![0; bytes + 3];
                            expected[0] = 0xCC;
                            expected[bytes + 2] = 0xCC;
                            expected[1] = match mode {
                                0 => 1,
                                1 => 3,
                                _ => 2,
                            };
                            if mode == 1 {
                                expected[2] = 42;
                            }
                            if mode == 2 {
                                for i in 0..bytes {
                                    expected[i + 2] = (20 + i) as u8;
                                }
                            }
                            assert_eq!(
                                machine::bytes(&vm, result.address, expected.len()),
                                expected,
                                "bytes={bytes}, mode={mode}, {runtime:?}, nir={optimized_nir}, mir={optimized_mir}, crlf={crlf}"
                            );
                        }
                    }
                }
            }
        }
    }
}
