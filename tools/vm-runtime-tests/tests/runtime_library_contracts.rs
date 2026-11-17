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
    let config = Mir6502Config { enable_small_leaf_inlining: false, ..Mir6502Config::optimized() };
    if std::env::var_os("ACTIONC_LIBRARY_TRACE").is_some() {
        let mut trace = mir6502::trace::Mir6502Trace::new(mir6502::trace::TraceOptions::default());
        let output = mir6502::generate_output_with_trace(&nir, 0x3000, &config, runtime, &mut trace).unwrap();
        for event in trace.events().iter().filter(|e| e.kind.starts_with("cross-call") || e.kind.contains("parameter-home")) {
            eprintln!("{:?}: {} {}", runtime, event.kind, event.data);
        }
        output
    } else { mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, &config, runtime).unwrap() }
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

#[test]
fn scopy_preserves_private_caller_storage_and_copies_length_and_payload() {
    let source = "MODULE CLIENT USE SYS
BYTE ARRAY source(258)=$9FE,destination(258)=$BFE
PROC Copy() SYS.SCopy(destination,source) RETURN
PROC Same() SYS.SCopy(destination,destination) RETURN
BYTE FUNC Hold(BYTE keep) Copy() RETURN(keep)
PROC Main() RETURN ENDMODULE";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = compile(source, runtime);
        let address = |stem| output.routine_addresses.iter().find(|r| r.name.starts_with(stem)).unwrap().address;
        let hold = address("M_CLIENT_HOLD_");
        let same = address("M_CLIENT_SAME_");
        assert!(output.map.storage_symbols.iter().any(|s| matches!(&s.scope,
            actionc::codegen::CodegenSymbolScope::Routine(name) if name.starts_with("M_CLIENT_HOLD_") && s.address < 256)), "{runtime:?}: live private caller home");
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
        for length in [0u8, 1, 4, 255] {
            let mut bytes = vec![length];
            bytes.extend((0..length).map(|i| i.wrapping_mul(13).wrapping_add(3)));
            vm.bus_mut().ram_mut().map(0x9fe, &bytes).unwrap();
            vm.bus_mut().ram_mut().map(0xbfe, &vec![0xcc; bytes.len()]).unwrap();
            vm.bus_mut().ram_mut().write(0xbfd, 0x5a);
            vm.bus_mut().ram_mut().write(0xbfe + bytes.len() as u16, 0xa5);
            vm = machine::call(vm, &mut Observe, hold, &[0x83]);
            assert_eq!(machine::bytes(&vm, 0xa0, 1), [0x83]);
            assert_eq!(machine::bytes(&vm, 0xbfe, bytes.len()), bytes);
            assert_eq!(machine::bytes(&vm, 0x9fe, bytes.len()), bytes);
            assert_eq!(machine::bytes(&vm, 0xbfd, 1), [0x5a]);
            assert_eq!(machine::bytes(&vm, 0xbfe + bytes.len() as u16, 1), [0xa5]);
            vm = machine::call(vm, &mut Observe, same, &[]);
            assert_eq!(machine::bytes(&vm, 0xbfe, bytes.len()), bytes);
        }
    }
}

#[test]
fn memory_library_handles_dynamic_counts_pages_overlap_and_live_callers() {
    let source = "MODULE CLIENT USE SYS
BYTE POINTER dst=$700,src=$702 CARD count=$704
PROC Clear() SYS.Zero(dst,count) RETURN
PROC Fill() SYS.SetBlock(dst,count,$5A) RETURN
PROC Copy() SYS.MoveBlock(dst,src,count) RETURN
BYTE FUNC HoldClear(BYTE keep) Clear() RETURN(keep)
BYTE FUNC HoldFill(BYTE keep) Fill() RETURN(keep)
BYTE FUNC HoldCopy(BYTE keep) Copy() RETURN(keep)
PROC Main() RETURN ENDMODULE";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = compile(source, runtime);
        let address = |stem| output.routine_addresses.iter().find(|r| r.name.starts_with(stem)).unwrap().address;
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
        for (stem, fill) in [("M_CLIENT_HOLDCLEAR_", Some(0u8)), ("M_CLIENT_HOLDFILL_", Some(0x5a)), ("M_CLIENT_HOLDCOPY_", None)] {
            let entry = address(stem);
            assert!(output.map.storage_symbols.iter().any(|s| matches!(&s.scope,
                actionc::codegen::CodegenSymbolScope::Routine(name) if name.starts_with(stem) && s.address < 256)), "{runtime:?}: {stem} private home");
            for count in [0u16, 1, 3, 255, 256, 257] {
                vm.bus_mut().ram_mut().map(0x700, &[0xfe,0x09,0xfe,0x0d,count as u8,(count>>8) as u8]).unwrap();
                let source: Vec<_> = (0..usize::from(count).max(1)).map(|i| (i as u8).wrapping_mul(17).wrapping_add(3)).collect();
                vm.bus_mut().ram_mut().map(0xdfe, &source).unwrap();
                vm.bus_mut().ram_mut().map(0x9fd, &vec![0xcc; usize::from(count)+2]).unwrap();
                vm = machine::call(vm, &mut Observe, entry, &[0x83]);
                assert_eq!(machine::bytes(&vm, 0xa0, 1), [0x83]);
                let expected = fill.map_or_else(|| source[..usize::from(count)].to_vec(), |v| vec![v; usize::from(count)]);
                assert_eq!(machine::bytes(&vm, 0x9fe, usize::from(count)), expected, "{runtime:?}: {stem}, {count}");
                assert_eq!(machine::bytes(&vm, 0x9fd, 1), [0xcc]);
                assert_eq!(machine::bytes(&vm, 0x9fe + count, 1), [0xcc]);
                assert_eq!(machine::bytes(&vm, 0xdfe, source.len()), source);
            }
        }
        // Forward copy supports a lower overlapping destination; this is not
        // an arbitrary-overlap memmove promise.
        vm.bus_mut().ram_mut().map(0x700, &[0xfe,0x09,0xff,0x09,4,0]).unwrap();
        vm.bus_mut().ram_mut().map(0x9fd, &[0xcc,9,1,2,3,4,0xcc]).unwrap();
        vm = machine::call(vm, &mut Observe, address("M_CLIENT_COPY_"), &[]);
        assert_eq!(machine::bytes(&vm, 0x9fd, 7), [0xcc,1,2,3,4,4,0xcc]);
    }
}
