//! Execution controls for selected SYS call contracts and live caller storage.
use actionc::{compiler::Runtime, mir6502::{self, Mir6502Config}};
use actionc_vm::{CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf { std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..") }
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
struct Observe;
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, _: &mut CompilerVm) -> Result<(), String> { Ok(()) }
}
fn compile(source: &str, runtime: Runtime) -> actionc::codegen::CodegenOutput {
    let source = support::Source::new(source);
    let loaded = actionc::includes::load_compilation(&source.0, &actionc::includes::ModuleLoadOptions::default()).unwrap();
    let model = actionc::semantic::analyze_compilation_with_options(&loaded, actionc::semantic::SemanticOptions::modern()).unwrap();
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(&actionc::semantic::ir::lower_compilation(&loaded, &model))).unwrap();
    mir6502::generate_output_with_config_and_runtime(&nir, 0x3000,
        &Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() }, runtime).unwrap()
}

#[test]
fn scompare_keeps_signed_results_string_reads_and_a_live_caller_home() {
    let source = "MODULE CLIENT USE SYS
BYTE ARRAY left(8)=$9FE,right(8)=$BFE INT outcome=$900
INT FUNC Reader() RETURN(SYS.SCompare(left,right))
BYTE FUNC Hold(BYTE keep) outcome=Reader() RETURN(keep)
PROC Main() RETURN ENDMODULE";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = compile(source, runtime);
        let hold = output.routine_addresses.iter().find(|r| r.name.starts_with("M_CLIENT_HOLD_")).unwrap().address;
        assert!(output.map.storage_symbols.iter().any(|s| match &s.scope {
            actionc::codegen::CodegenSymbolScope::Routine(name) => name.starts_with("M_CLIENT_HOLD_") && s.address < 256,
            _ => false,
        }), "{runtime:?}: caller home should survive reader");
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
        for (left, right, sign) in [("", "", 0), ("ABCD", "ABCD", 0), ("ABC", "ABD", -1), ("ABD", "ABC", 1), ("A", "AB", -1)] {
            for (address, text) in [(0x9fe, left), (0xbfe, right)] {
                vm.bus_mut().ram_mut().write(address - 1, 0x5a);
                vm.bus_mut().ram_mut().write(address + 8, 0xa5);
                vm.bus_mut().ram_mut().write(address, text.len() as u8);
                if !text.is_empty() { vm.bus_mut().ram_mut().map(address + 1, text.as_bytes()).unwrap(); }
            }
            vm = machine::call(vm, &mut Observe, hold, &[0x83]);
            assert_eq!(machine::bytes(&vm, 0xa0, 1), [0x83]);
            let result = machine::bytes(&vm, 0x900, 2);
            assert_eq!(i16::from_le_bytes([result[0], result[1]]).signum(), sign, "{runtime:?}: {left}/{right}");
            for (address, text) in [(0x9fe, left), (0xbfe, right)] {
                assert_eq!(machine::bytes(&vm, address, 1), [text.len() as u8]);
                assert_eq!(machine::bytes(&vm, address + 1, text.len()), text.as_bytes());
                assert_eq!(machine::bytes(&vm, address - 1, 1), [0x5a]);
                assert_eq!(machine::bytes(&vm, address + 8, 1), [0xa5]);
            }
        }
    }
}
