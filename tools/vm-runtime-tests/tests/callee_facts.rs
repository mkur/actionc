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

#[test]
fn callee_facts_machine_memory_keeps_indexed_reads_and_offset_stores() {
    let source = include_str!("../../../fixtures/runtime/callee_facts_memory.act");
    let out = compile(source, &Mir6502Config::optimized(), Runtime::ActionCart);
    let mut vm = load(&out, Runtime::ActionCart);
    vm.bus_mut().ram_mut().write(0x7080, 0x55);
    vm.bus_mut().ram_mut().write(0x7082, 0xaa);
    for (name, result) in [("CheckOffset", 42), ("CheckIndexed", 90)] {
        vm = machine::call(vm, &mut Observe, entry(&out, name), &[]);
        assert_eq!(machine::bytes(&vm, 0xa0, 1), [result]);
    }
    assert_eq!(machine::bytes(&vm, 0x7080, 3), [0x55, 42, 0xaa]);
    vm = machine::call(vm, &mut Observe, entry(&out, "Boundary"), &[]);
    assert_eq!(machine::bytes(&vm, 0xa0, 2), [0, 0x71]);
}

#[test]
fn callee_facts_directional_reader_preserves_caller_and_pointee() {
    let optimized = Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() };
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for config in [Mir6502Config::default(), optimized.clone()] {
            let out = compile(include_str!("../../../fixtures/runtime/callee_facts_readers.act"), &config, runtime);
            let mut vm = load(&out, runtime);
            for base in [0x7000u16, 0x70ff] {
                vm.bus_mut().ram_mut().map(base, &[0x55, 0x81, 0xaa]).unwrap();
                for keep in [0u16, 0x1234, 0xffff] {
                    let args: Vec<_> = base.to_le_bytes().into_iter().chain(keep.to_le_bytes()).collect();
                    vm = machine::call(vm, &mut Observe, entry(&out, "KeepReader"), &args);
                    assert_eq!(machine::bytes(&vm, 0xa0, 2), keep.wrapping_add(0x81).to_le_bytes());
                    assert_eq!(machine::bytes(&vm, base, 3), [0x55, 0x81, 0xaa]);
                }
                vm = machine::call(vm, &mut Observe, entry(&out, "KeepWriter"), &[base as u8, (base >> 8) as u8, 0x34, 0x12]);
                assert_eq!(machine::bytes(&vm, 0xa0, 2), [0x34, 0x12]);
                assert_eq!(machine::bytes(&vm, base, 3), [0x55, 37, 0xaa]);
            }
            if config == optimized {
                for (name, resident) in [("KeepReader", true), ("KeepWriter", false)] {
                    let homes: Vec<_> = out.map.storage_symbols.iter().filter(|s|
                        s.scope == actionc::codegen::CodegenSymbolScope::Routine(name.into())
                        && s.kind == actionc::codegen::CodegenSymbolKind::Parameter).collect();
                    assert!(!homes.is_empty());
                    assert!(homes.iter().all(|s| (s.address < 256) == resident), "{name}: {homes:?}");
                }
            }
        }
    }
}
