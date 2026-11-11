//! Host-oracle execution of captured-byte comparisons through pointer operands.
use actionc::codegen::CodegenOutput;
use actionc::compiler::Runtime;
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{BusAccess, CompilerVm, VmRunHooks};
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

fn compile(text: &str, optimized_nir: bool, runtime: Runtime, enabled: bool) -> CodegenOutput {
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
    let config = Mir6502Config {
        enable_captured_byte_comparisons: enabled,
        ..Mir6502Config::optimized()
    };
    mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, &config, runtime).unwrap()
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
            for enabled in [false, true] {
                let lf = compile(&text, optimized_nir, runtime, enabled);
                let crlf = compile(&text.replace('\n', "\r\n"), optimized_nir, runtime, enabled);
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
                                    vm = machine::call(
                                        vm,
                                        &mut Hooks,
                                        entry,
                                        &address.to_le_bytes(),
                                    );
                                    let truth = if name == "Greater" {
                                        left > right
                                    } else {
                                        left <= right
                                    };
                                    assert_eq!(
                                        vm.bus().ram().read(0x680),
                                        if truth { 17 } else { 29 },
                                        "{name}/{runtime:?}/optimized_nir={optimized_nir}/enabled={enabled}/{left}/{right}"
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
                                            "direct-comparison,{name},{runtime:?},optimized_nir={optimized_nir},enabled={enabled},cycles={}",
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
}

struct IndirectReads {
    address: u16,
    values: Vec<u8>,
}

fn indirect_address(vm: &CompilerVm) -> Option<u16> {
    let registers = vm.cpu().registers();
    let ram = vm.bus().ram();
    if !matches!(ram.read(registers.pc), 0xB1 | 0xD1) {
        return None;
    }
    let pair = ram.read(registers.pc.wrapping_add(1));
    let base = u16::from_le_bytes([
        ram.read(u16::from(pair)),
        ram.read(u16::from(pair.wrapping_add(1))),
    ]);
    Some(base.wrapping_add(u16::from(registers.y)))
}

impl VmRunHooks for IndirectReads {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        if indirect_address(vm) == Some(self.address) {
            self.values.push(
                vm.bus()
                    .io()
                    .read(self.address)
                    .unwrap_or_else(|| vm.bus().ram().read(self.address)),
            );
        }
        Ok(())
    }
}

struct ChangingInput(IndirectReads);

impl VmRunHooks for ChangingInput {
    type Error = String;

    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        if indirect_address(vm) == Some(self.0.address) {
            // Change the input after its capture, immediately before the RHS
            // read. Rereading the input for CMP would reverse the answer.
            vm.bus_mut().write(0xD20F, 255);
        }
        self.0.before_step(vm)
    }
}

#[test]
fn captured_comparisons_keep_the_original_hardware_snapshot() {
    let source = "MODULE\nBYTE hardware=$D20F\nBYTE result=$0680\n\
        PROC Snapshot(BYTE POINTER base)\n\
          IF hardware <= base^ THEN result=17 ELSE result=29 FI\n\
        RETURN\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized_nir in [false, true] {
            for enabled in [false, true] {
                for crlf in [false, true] {
                    let source = if crlf {
                        source.replace('\n', "\r\n")
                    } else {
                        source.to_owned()
                    };
                    let output = compile(&source, optimized_nir, runtime, enabled);
                    let entry = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name.eq_ignore_ascii_case("Snapshot"))
                        .unwrap()
                        .address;
                    let mut vm = load(&output, runtime);
                    vm.bus_mut().write(0xD20F, 1);
                    vm.bus_mut().ram_mut().write(0x7000, 127);
                    vm.bus_mut().add_watchpoint(0xD20F);
                    vm.bus_mut().clear_events();
                    let mut hooks = ChangingInput(IndirectReads {
                        address: 0x7000,
                        values: Vec::new(),
                    });
                    vm = machine::call(vm, &mut hooks, entry, &0x7000u16.to_le_bytes());
                    assert_eq!(hooks.0.values, vec![127]);
                    assert_eq!(
                        vm.bus().ram().read(0x680),
                        17,
                        "{runtime:?}/{optimized_nir}/{enabled}"
                    );
                    let reads: Vec<_> = vm
                        .bus()
                        .events()
                        .iter()
                        .filter(|e| e.access == BusAccess::Read && e.address == 0xD20F)
                        .collect();
                    assert_eq!(reads.len(), 1, "{reads:?}");
                    assert_eq!(reads[0].value, 1);
                    assert_eq!(vm.bus().io().read(0xD20F), Some(255));
                }
            }
        }
    }
}

#[test]
fn captured_comparisons_preserve_parameter_aliases_scratch_and_hardware_reads() {
    let source = "MODULE\nBYTE comparisonIndex=$E0\nBYTE comparisonResult=$0680\n\
        CARD parameterHome=$0684\n\
        PROC Exposed(BYTE POINTER base)\n\
          parameterHome=@base\n\
          IF comparisonIndex <= base^ THEN comparisonResult=17 ELSE comparisonResult=29 FI\n\
        RETURN\nPROC Main()\nRETURN\n";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for optimized_nir in [false, true] {
            for crlf in [false, true] {
                let source = if crlf {
                    source.replace('\n', "\r\n")
                } else {
                    source.to_owned()
                };
                let mut reference = None;
                for enabled in [false, true] {
                    let output = compile(&source, optimized_nir, runtime, enabled);
                    let parameter = output
                        .map
                        .storage_symbols
                        .iter()
                        .find(|s| s.name.eq_ignore_ascii_case("base"))
                        .unwrap()
                        .address;
                    let entry = output
                        .routine_addresses
                        .iter()
                        .find(|r| r.name.eq_ignore_ascii_case("Exposed"))
                        .unwrap()
                        .address;
                    let mut observations = Vec::new();
                    let mut vm = load(&output, runtime);
                    for address in [
                        0xE0,
                        0xE1,
                        0xAC,
                        0xAD,
                        parameter,
                        parameter + 1,
                        0x7000,
                        0x70FF,
                        0xD20F,
                    ] {
                        for left in [0, 128, 255] {
                            let ram = vm.bus_mut().ram_mut();
                            ram.write(0xE0, left);
                            if address != 0xE0 {
                                ram.write(address, 127);
                            }
                            vm.bus_mut().write(0xD20F, 127);
                            let mut reads = IndirectReads {
                                address,
                                values: Vec::new(),
                            };
                            vm = machine::call(vm, &mut reads, entry, &address.to_le_bytes());
                            assert_eq!(
                                reads.values.len(),
                                1,
                                "{runtime:?}/{enabled}/{address:04X}"
                            );
                            let answer = vm.bus().ram().read(0x680);
                            assert_eq!(answer, if left <= reads.values[0] { 17 } else { 29 });
                            assert_eq!(vm.bus().ram().read(0xE0), left);
                            assert_eq!(vm.bus().ram().read_word(parameter), address);
                            assert_eq!(vm.bus().ram().read_word(0x684), parameter);
                            // Parameter addresses can relocate with emitted code. Other
                            // accesses have the same physical address in both images.
                            if address != parameter && address != parameter + 1 {
                                observations.push((address, left, reads.values[0], answer));
                            }
                        }
                    }
                    if let Some(expected) = &reference {
                        assert_eq!(
                            &observations, expected,
                            "{runtime:?}/optimized_nir={optimized_nir}"
                        );
                    } else {
                        reference = Some(observations);
                    }
                }
            }
        }
    }
}
