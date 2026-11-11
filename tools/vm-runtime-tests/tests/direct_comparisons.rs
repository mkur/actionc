//! Host-oracle execution of captured-byte comparisons through pointer operands.
use actionc::codegen::CodegenOutput;
use actionc::compiler::Runtime;
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{CompilerVm, VmRunHooks};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile(text: &str, optimized_nir: bool, runtime: Runtime) -> CodegenOutput {
    let source = support::Source::new(text);
    let loaded = actionc::includes::load_program_with_expanded_source(&source.0).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let semir = actionc::semantic::ir::lower_program(&loaded.program, &model);
    let raw = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&raw).unwrap();
    let nir = if optimized_nir {
        actionc::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    };
    mir6502::generate_output_with_config_and_runtime(
        &nir,
        0x3000,
        &Mir6502Config::optimized(),
        runtime,
    )
    .unwrap()
}

struct Hooks;
impl VmRunHooks for Hooks {
    type Error = String;
}

fn load(output: &CodegenOutput, runtime: Runtime) -> CompilerVm {
    let (mut vm, profile) = support::vm_for(runtime);
    vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(output))
        .unwrap();
    vm
}

#[test]
fn captured_byte_comparisons_match_unsigned_oracle_with_lf_and_crlf() {
    let text =
        include_str!("../../../fixtures/runtime/direct_comparisons.act").replace("\r\n", "\n");
    let values = [0, 1, 127, 128, 254, 255];
    for optimized_nir in [false, true] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let lf = compile(&text, optimized_nir, runtime);
            let crlf = compile(&text.replace('\n', "\r\n"), optimized_nir, runtime);
            assert_eq!(
                lf.bytes, crlf.bytes,
                "{runtime:?}/optimized_nir={optimized_nir}"
            );
            for output in [&lf, &crlf] {
                let ordinary = output
                    .map
                    .storage_symbols
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case("ordinaryIndex"))
                    .unwrap()
                    .address;
                let mut vm = load(output, runtime);
                for name in ["AtMost", "Greater", "NamedAtMost"] {
                    let entry = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name.eq_ignore_ascii_case(name))
                        .unwrap()
                        .address;
                    for address in [0x7000u16, 0x70FF] {
                        for left in values {
                            for right in values {
                                let ram = vm.bus_mut().ram_mut();
                                ram.write(0xE0, left);
                                ram.write(ordinary, left);
                                ram.write(0x680, 0xA5);
                                ram.write(address, right);
                                let start = vm.cpu().cycles();
                                vm = machine::call(vm, &mut Hooks, entry, &address.to_le_bytes());
                                let truth = if name == "Greater" {
                                    left > right
                                } else {
                                    left <= right
                                };
                                assert_eq!(
                                    vm.bus().ram().read(0x680),
                                    if truth { 17 } else { 29 },
                                    "{name}/{runtime:?}/optimized_nir={optimized_nir}/{left}/{right}"
                                );
                                assert_eq!(vm.bus().ram().read(address), right);
                                assert_eq!(vm.bus().ram().read(0xE0), left);
                                assert_eq!(vm.bus().ram().read(ordinary), left);
                                if std::env::var_os("ACTIONC_COMPARE_METRICS").is_some()
                                    && address == 0x7000
                                    && left == 128
                                    && right == 254
                                {
                                    eprintln!(
                                        "direct-comparison,{name},{runtime:?},optimized_nir={optimized_nir},cycles={}",
                                        vm.cpu().cycles() - start
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
