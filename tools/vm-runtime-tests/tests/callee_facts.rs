//! Independent result oracles for shared callee facts.
use actionc::{compiler::Runtime, mir6502::{self, Mir6502Config}};
use actionc_vm::{CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..") }
#[allow(dead_code)]
#[path = "support/tn_machine.rs"] mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"] mod support;
struct Observe;
impl VmRunHooks for Observe { type Error = String; }

fn compile(source: &str, config: &Mir6502Config, runtime: Runtime) -> actionc::codegen::CodegenOutput {
    let file = support::Source::new(source);
    let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
    let model = actionc::semantic::analyze_with_options(&loaded.program, actionc::semantic::SemanticOptions::modern()).unwrap();
    let n = actionc::nir::optimize_program(&actionc::nir::lower_program(&actionc::semantic::ir::lower_program(&loaded.program, &model))).unwrap();
    mir6502::generate_output_with_config_and_runtime(&n, 0x3000, config, runtime).unwrap()
}
fn load(out: &actionc::codegen::CodegenOutput, runtime: Runtime) -> CompilerVm {
    let (mut vm, profile) = support::vm_for(runtime);
    vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(out)).unwrap();
    vm
}
fn entry(out: &actionc::codegen::CodegenOutput, name: &str) -> u16 {
    out.routine_addresses.iter().find(|r| r.name == name).unwrap().address
}

#[test]
fn callee_facts_flags_store_only_entry_and_produced_result() {
    let lf = include_str!("../../../fixtures/runtime/callee_facts_flags.act").replace("\r\n", "\n");
    let mut optimized = None;
    for (config, source) in [(Mir6502Config::default(), lf.clone()),
        (Mir6502Config::optimized(), lf.clone()), (Mir6502Config::optimized(), lf.replace('\n', "\r\n"))] {
        let out = compile(&source, &config, Runtime::ActionCart);
        if config == Mir6502Config::optimized() {
            if let Some(old) = &optimized { assert_eq!(&out.bytes, old); } else { optimized = Some(out.bytes.clone()); }
        }
        let mut vm = load(&out, Runtime::ActionCart);
        vm = machine::call(vm, &mut Observe, entry(&out, "Check"), &[]);
        assert_eq!(machine::bytes(&vm, 0xa0, 1), [1]);
        assert_eq!(machine::bytes(&vm, 0xfe, 2), [128, 0]);
        for v in [0u8, 128, 1, 255] {
            vm = machine::call(vm, &mut Observe, entry(&out, "CheckProduced"), &[v]);
            assert_eq!(machine::bytes(&vm, 0xa0, 1), [u8::from(v & 127 == 0)]);
        }
    }
}
