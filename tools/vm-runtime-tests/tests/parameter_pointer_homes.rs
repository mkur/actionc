//! Placement-only execution oracles for the real TN routine and an independent
//! two-pointer loop. Watchpoints compare pointee read order, not scratch traffic.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config},
};
use actionc_vm::{BusAccess, CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
struct Observe;
impl VmRunHooks for Observe {
    type Error = String;
}
fn compile(
    source: &str,
    runtime: Runtime,
    origin: u16,
    enabled: bool,
) -> actionc::codegen::CodegenOutput {
    compile_with_inlining(source, runtime, origin, enabled, true)
}
fn compile_with_inlining(
    source: &str,
    runtime: Runtime,
    origin: u16,
    enabled: bool,
    inline: bool,
) -> actionc::codegen::CodegenOutput {
    let file = support::Source::new(source);
    let loaded = actionc::includes::load_program_with_expanded_source(&file.0).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_program(&loaded.program, &model),
    ))
    .unwrap();
    mir6502::generate_output_with_config_and_runtime(
        &nir,
        origin,
        &Mir6502Config {
            enable_parameter_home_selection: enabled,
            enable_small_leaf_inlining: inline,
            ..Mir6502Config::optimized()
        },
        runtime,
    )
    .unwrap()
}
fn entry(output: &actionc::codegen::CodegenOutput, name: &str) -> u16 {
    output
        .routine_addresses
        .iter()
        .find(|r| r.name == name)
        .unwrap()
        .address
}
fn load(output: &actionc::codegen::CodegenOutput, _runtime: Runtime) -> CompilerVm {
    // These leaf/caller probes do not invoke ROM services. Load raw object
    // segments: the execution-profile loader installs OS ROM, which would
    // hide RAM writes at $FFFF in the 16-bit wraparound tests.
    let mut vm = CompilerVm::default();
    vm.load_atari_object(&actionc::codegen::format_load_file(output))
        .unwrap();
    vm
}

fn compare(
    mut vm: CompilerVm,
    address: u16,
    bases: (u16, u16),
    left: &[u8],
    right: &[u8],
) -> (CompilerVm, u8, Vec<(u16, u8)>) {
    for (base, data) in [(bases.0, left), (bases.1, right)] {
        for (offset, byte) in data.iter().enumerate() {
            let address = base.wrapping_add(offset as u16);
            vm.bus_mut().ram_mut().write(address, *byte);
            vm.bus_mut().add_watchpoint(address);
        }
    }
    vm.bus_mut().clear_events();
    let mut args = bases.0.to_le_bytes().to_vec();
    args.extend(bases.1.to_le_bytes());
    vm = machine::call(vm, &mut Observe, address, &args);
    let result = vm.bus().ram().read(0xa0);
    let reads = vm
        .bus()
        .events()
        .iter()
        .filter(|e| e.access == BusAccess::Read)
        .map(|e| (e.address, e.value))
        .collect();
    assert!(
        !vm.bus()
            .events()
            .iter()
            .any(|e| e.access == BusAccess::Write),
        "comparison wrote its input"
    );
    (vm, result, reads)
}

#[test]
fn tn_and_independent_pointer_loops_preserve_results_and_read_order() {
    for (path, name, skip) in [
        ("samples/tn/modern/TN.ACT", "Fnamecmp", 2usize),
        ("fixtures/runtime/parameter_homes.act", "ComparePointers", 0),
    ] {
        // Resolve TN's real LIB.ACT include before exercising the complete
        // expanded source through temporary LF and CRLF input files.
        let lf = actionc::includes::load_program_with_expanded_source(root().join(path))
            .unwrap()
            .source
            .replace("\r\n", "\n");
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41c7] {
                let old = compile(&lf, runtime, origin, false);
                let new = compile(&lf, runtime, origin, true);
                let crlf = compile(&lf.replace('\n', "\r\n"), runtime, origin, true);
                assert_eq!(
                    actionc::codegen::format_load_file(&new),
                    actionc::codegen::format_load_file(&crlf)
                );
                assert_eq!(new.routine_addresses, crlf.routine_addresses);
                assert_eq!(new.map.storage_symbols, crlf.map.storage_symbols);
                let old_range = old
                    .map
                    .routine_ranges
                    .iter()
                    .find(|r| r.name == name)
                    .unwrap();
                let new_range = new
                    .map
                    .routine_ranges
                    .iter()
                    .find(|r| r.name == name)
                    .unwrap();
                assert!(
                    new_range.end - new_range.start < old_range.end - old_range.start,
                    "{path}: pointer placement was not accepted"
                );
                let mut old_vm = load(&old, runtime);
                let mut new_vm = load(&new, runtime);
                for bases in [(0x7000, 0x7500), (0x70ff, 0x75fe)] {
                    for offset in [0, 1, 254, 255, 256, 257, 511, 512, 700] {
                        for (a, b) in [(0, 255), (255, 0), (127, 128)] {
                            let mut left = vec![65; skip + offset + 1];
                            let mut right = left.clone();
                            left[skip + offset] = a;
                            right[skip + offset] = b;
                            let (v, actual, reads) =
                                compare(old_vm, entry(&old, name), bases, &left, &right);
                            old_vm = v;
                            let (v, result, new_reads) =
                                compare(new_vm, entry(&new, name), bases, &left, &right);
                            new_vm = v;
                            assert_eq!(actual, u8::from(a > b));
                            assert_eq!(result, actual);
                            assert_eq!(new_reads, reads);
                        }
                    }
                }
                // Wrap past $FFFF without reaching compiler-owned ZP bytes.
                let mut left = vec![65; skip + 8];
                let mut right = left.clone();
                left[skip + 7] = 255;
                right[skip + 7] = 0;
                let (v, actual, reads) =
                    compare(old_vm, entry(&old, name), (0xfffc, 0x7801), &left, &right);
                old_vm = v;
                let (v, result, new_reads) =
                    compare(new_vm, entry(&new, name), (0xfffc, 0x7801), &left, &right);
                new_vm = v;
                assert_eq!(actual, 1);
                assert_eq!(result, actual);
                assert_eq!(new_reads, reads);
                if skip == 2 {
                    for a in [0, 32, 58, 65, 255] {
                        for b in [0, 32, 58, 65, 255] {
                            let left = [0, a, 2];
                            let right = [0, b, 1];
                            let expected = if a != b && a == 58 { 0 } else { 1 };
                            let (v, result, _) =
                                compare(new_vm, entry(&new, name), (0x7000, 0x7500), &left, &right);
                            new_vm = v;
                            assert_eq!(result, expected, "colon prefix {a}/{b}");
                        }
                    }
                    sort_through_optimized_caller(new_vm, &new);
                }
                // Every unsigned byte comparison, once per independent body.
                if runtime == Runtime::ActionCart && origin == 0x3000 {
                    let mut vm = load(&new, runtime);
                    for a in 0..=255u8 {
                        for b in 0..=255u8 {
                            let mut left = vec![65; skip + 2];
                            let mut right = left.clone();
                            left[skip] = a;
                            right[skip] = b;
                            left[skip + 1] = 1;
                            right[skip + 1] = 0;
                            let (v, result, _) =
                                compare(vm, entry(&new, name), (0x7000, 0x7500), &left, &right);
                            vm = v;
                            assert_eq!(result, u8::from(a >= b), "byte pair {a}/{b}");
                        }
                    }
                }
                drop(old_vm);
            }
        }
    }
}
#[test]
fn tn_sort_preserves_order_with_cross_call_spill_placement() {
    let source = actionc::includes::load_program_with_expanded_source(root().join("samples/tn/modern/TN.ACT"))
        .unwrap().source.replace("\r\n", "\n");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = compile(&source, runtime, 0x3000, true);
        sort_through_optimized_caller(load(&output, runtime), &output);
    }
}

fn sort_through_optimized_caller(mut vm: CompilerVm, output: &actionc::codegen::CodegenOutput) {
    let global = |name| {
        output
            .map
            .storage_symbols
            .iter()
            .find(|s| s.name == name && s.scope == actionc::codegen::CodegenSymbolScope::Global)
            .unwrap()
            .address
    };
    let letters = [90, 65, 81, 66, 78];
    vm.bus_mut()
        .ram_mut()
        .write(global("active"), letters.len() as u8);
    vm.bus_mut().ram_mut().write_word(global("v"), 0x6c00);
    for (i, letter) in letters.iter().enumerate() {
        let address = 0x7000 + i as u16 * 18;
        vm.bus_mut()
            .ram_mut()
            .write_word(0x6c00 + i as u16 * 2, address);
        vm.bus_mut()
            .ram_mut()
            .map(address, &[0, 32, *letter])
            .unwrap();
    }
    vm = machine::call(vm, &mut Observe, entry(output, "Sort"), &[]);
    let actual: Vec<_> = (0..letters.len())
        .map(|i| {
            let address = vm.bus().ram().read_word(0x6c00 + i as u16 * 2);
            vm.bus().ram().read(address + 2)
        })
        .collect();
    assert_eq!(actual, vec![65, 66, 78, 81, 90]);
}

#[test]
fn offset_copies_preserve_the_resident_pointer_and_fourth_input() {
    let source = "BYTE observed=$0690,result=$0691
BYTE FUNC Offset(CHAR POINTER p)
 CHAR POINTER q
 p==+1 q=p+256 observed=q^ RETURN(p^)
BYTE FUNC Fourth(BYTE a,b,c,d) RETURN(d)
PROC Main() result=Offset($7000) result=Fourth(1,2,3,4) DO OD RETURN";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let old = compile_with_inlining(source, runtime, 0x3000, false, false);
        let new = compile_with_inlining(source, runtime, 0x3000, true, false);
        for (routine, expected) in [("Offset", 0xa0), ("Fourth", 0xa3)] {
            assert!(
                new.map.storage_symbols.iter().any(|s| {
                    s.scope == actionc::codegen::CodegenSymbolScope::Routine(routine.into())
                        && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
                        && s.address == expected
                }),
                "{routine} was not bound"
            );
        }
        let mut vm = load(&new, runtime);
        for base in [0x7000u16, 0x70ff, 0x7fff, 0xfffd] {
            for output in [&old, &new] {
                let mut probe = load(output, runtime);
                probe.bus_mut().ram_mut().write(base.wrapping_add(1), 47);
                probe.bus_mut().ram_mut().write(base.wrapping_add(257), 193);
                probe = machine::call(
                    probe,
                    &mut Observe,
                    entry(output, "Offset"),
                    &base.to_le_bytes(),
                );
                assert_eq!(
                    probe.bus().ram().read(0xa0),
                    47,
                    "base={base:04x}, new={}",
                    std::ptr::eq(output, &new)
                );
                assert_eq!(
                    probe.bus().ram().read(0x0690),
                    193,
                    "base={base:04x}, new={}",
                    std::ptr::eq(output, &new)
                );
            }
        }
        for value in 0..=255u8 {
            vm = machine::call(
                vm,
                &mut Observe,
                entry(&new, "Fourth"),
                &[19, 61, 127, value],
            );
            assert_eq!(vm.bus().ram().read(0xa0), value);
        }
    }
}

#[test]
fn competing_source_storage_leaves_only_verified_pointer_scratch_pairs() {
    let source = "BYTE ARRAY window(4)=$A0,pool(16)=$E0
BYTE result=$0680
BYTE FUNC Pair(CHAR POINTER left,right)
 WHILE left^=right^ DO left==+1 right==+1 OD
 RETURN(left^>right^)
PROC Main() result=Pair($7000,$7500) DO OD RETURN";
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let old = compile(source, runtime, 0x41c7, false);
        let new = compile(source, runtime, 0x41c7, true);
        let homes: Vec<_> = new
            .map
            .storage_symbols
            .iter()
            .filter(|s| {
                s.scope == actionc::codegen::CodegenSymbolScope::Routine("Pair".into())
                    && s.kind == actionc::codegen::CodegenSymbolKind::Parameter
            })
            .map(|s| s.address)
            .collect();
        assert_eq!(homes, vec![0xaa, 0xae]);
        for offset in [0, 255, 256, 512] {
            let mut left = vec![65; offset + 1];
            let mut right = left.clone();
            left[offset] = 255;
            right[offset] = 1;
            let (_, expected, reads) = compare(
                load(&old, runtime),
                entry(&old, "Pair"),
                (0x70ff, 0x75fe),
                &left,
                &right,
            );
            let (_, actual, new_reads) = compare(
                load(&new, runtime),
                entry(&new, "Pair"),
                (0x70ff, 0x75fe),
                &left,
                &right,
            );
            assert_eq!(expected, 1);
            assert_eq!(actual, expected);
            assert_eq!(new_reads, reads);
        }
    }
}
