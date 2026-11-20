//! Source-home admission through existing helper and address contracts.
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
fn nir(text: &str) -> actionc::nir::NirProgram {
    let source = support::Source::new(text);
    let loaded = actionc::includes::load_compilation(&source.0, &Default::default()).unwrap();
    let model = actionc::semantic::analyze_compilation_with_options(&loaded, actionc::semantic::SemanticOptions::modern()).unwrap();
    actionc::nir::optimize_program(&actionc::nir::lower_program(&actionc::semantic::ir::lower_compilation(&loaded, &model))).unwrap()
}
fn config() -> Mir6502Config { Mir6502Config { enable_small_leaf_inlining:false,..Mir6502Config::optimized() } }
fn assert_zp(output: &actionc::codegen::CodegenOutput, routine: &str, name: &str) -> u16 {
    output.map.storage_symbols.iter().find(|s| matches!(&s.scope,
        actionc::codegen::CodegenSymbolScope::Routine(r) if r==routine) && s.name==name && s.address<256)
        .unwrap_or_else(|| panic!("missing {routine}.{name} ZP home: {:?}", output.map.storage_symbols)).address
}

#[test]
fn fixed_indexed_color_store_uses_disjoint_leaf_homes_in_both_runtimes() {
    let n = nir("PROC Paint(BYTE n,hue,lum)
        BYTE ARRAY colors(4)=$02C0
        n==&3 colors(n)=(hue LSH 4) % (lum & $0F) RETURN
        PROC Observe() [ $AD $A0 $00 $85 $A1 ] RETURN
        PROC Main() Paint(0,1,2) Observe() RETURN");
    // Exercise leaf selection on its own. The machine routine observes $A0 and $A1.
    let config = Mir6502Config { enable_cross_call_home_selection: false, ..config() };
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config, runtime).unwrap();
        for name in ["n", "hue", "lum"] {
            assert!(![0xa0, 0xa1].contains(&assert_zp(&output, "Paint", name)));
        }
        let entry = output.routine_addresses.iter().find(|r| r.name == "Paint").unwrap().address;
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
        vm.bus_mut().ram_mut().map(0x02bf, &[0x5a; 6]).unwrap();
        for (n, hue, lum) in [(0, 1, 0x12), (5, 5, 4), (0xfe, 9, 6), (0xff, 0xfd, 8)] {
            vm = machine::call(vm, &mut Observe, entry, &[n, hue, lum]);
        }
        assert_eq!(machine::bytes(&vm, 0x02bf, 6), [0x5a, 0x12, 0x54, 0x96, 0xd8, 0x5a]);
    }
}

#[test]
fn direct_word_arithmetic_keeps_a_zp_home_alongside_an_indexed_read() {
    let n = nir("CARD delta=$730,result=$740 BYTE POINTER p=$710 BYTE seen=$720
CARD FUNC Calc(CARD n,base) seen=p(1) n==+delta result=base-n RETURN(n)
PROC Main() Calc(0,0) RETURN");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config(), runtime).unwrap();
        assert_zp(&output, "Calc", "n");
        assert_zp(&output, "Calc", "base");
        let entry = output.routine_addresses.iter().find(|r| r.name == "Calc").unwrap().address;
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
        vm.bus_mut().ram_mut().map(0x710, &0x800u16.to_le_bytes()).unwrap();
        for (input, delta, base, seen) in [(0x00ffu16, 1u16, 0x0100u16, 9u8),
                                           (0xffff, 2, 0x0100, 17),
                                           (0x1200, 0x00ff, 3, 33)] {
            vm.bus_mut().ram_mut().map(0x801, &[seen]).unwrap();
            vm.bus_mut().ram_mut().map(0x730, &delta.to_le_bytes()).unwrap();
            let [n_lo, n_hi] = input.to_le_bytes();
            let [base_lo, base_hi] = base.to_le_bytes();
            vm = machine::call(vm, &mut Observe, entry, &[n_lo, n_hi, base_lo, base_hi]);
            let sum = input.wrapping_add(delta);
            assert_eq!(machine::bytes(&vm, 0xa0, 2), sum.to_le_bytes());
            assert_eq!(machine::bytes(&vm, 0x740, 2), base.wrapping_sub(sum).to_le_bytes());
            assert_eq!(machine::bytes(&vm, 0x720, 1), [seen]);
        }
    }
}

#[test]
fn helper_only_routine_keeps_narrow_home_across_wide_shift_in_both_runtimes() {
    let n = nir("CARD reserve=$A0,reserve2=$A2 LONGCARD input=$700,result=$900
CARD FUNC Hold(CARD keep BYTE count) result=input RSH count RETURN(keep+1)
PROC Main() RETURN");
    for runtime in [Runtime::ActionCart,Runtime::Standalone] {
        let output = mir6502::generate_output_with_config_and_runtime(&n,0x3000,&config(),runtime).unwrap();
        assert_zp(&output,"Hold","keep");
        let off = mir6502::generate_output_with_config_and_runtime(&n,0x3000,
            &Mir6502Config { enable_cross_call_home_selection:false,..config() },runtime).unwrap();
        assert!(output.bytes.len()<off.bytes.len());
        let entry=output.routine_addresses.iter().find(|r|r.name=="Hold").unwrap().address;
        let (mut vm,profile)=support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile,&actionc::codegen::format_load_file(&output)).unwrap();
        for value in [0u32,1,0x12345678,0x80000000,0xffffffff] {
            for count in [0u8,1,7,16,31] {
                vm.bus_mut().ram_mut().map(0x700,&value.to_le_bytes()).unwrap();
                vm.bus_mut().ram_mut().map(0x8ff,&[0x5a;6]).unwrap();
                vm=machine::call(vm,&mut Observe,entry,&[0xff,0x7f,count]);
                assert_eq!(machine::bytes(&vm,0xa0,2),[0,0x80]);
                assert_eq!(machine::bytes(&vm,0x900,4),(value>>count).to_le_bytes());
                assert_eq!(machine::bytes(&vm,0x8ff,1),[0x5a]);
                assert_eq!(machine::bytes(&vm,0x904,1),[0x5a]);
            }
        }
        eprintln!("{runtime:?} helper control: {} -> {} bytes",off.bytes.len(),output.bytes.len());
    }
}

#[test]
fn global_and_static_addresses_preserve_live_homes_and_relocate_in_both_runtimes() {
    for table in ["BYTE ARRAY table=[5 9 17 33]", "BYTE ARRAY table=$800"] {
        let n=nir(&format!("CARD reserve=$A0,reserve2=$A2 BYTE guard=$E0 {table}
BYTE FUNC Fetch(BYTE POINTER p) RETURN(p(1))
CARD FUNC Hold(CARD keep) BYTE b b=Fetch(table) RETURN(keep+b)
CARD FUNC Literal(CARD keep) BYTE b b=Fetch(\"Hi\") RETURN(keep+b)
PROC Main() RETURN"));
        for runtime in [Runtime::ActionCart,Runtime::Standalone] {
            for origin in [0x3000,0x5000] {
                let output=mir6502::generate_output_with_config_and_runtime(&n,origin,&config(),runtime).unwrap();
                let off=mir6502::generate_output_with_config_and_runtime(&n,origin,
                    &Mir6502Config{enable_cross_call_home_selection:false,..config()},runtime).unwrap();
                assert_zp(&output,"Hold","keep"); assert_zp(&output,"Literal","keep");
                assert!(output.bytes.len()<off.bytes.len());
                let (mut vm,profile)=support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile,&actionc::codegen::format_load_file(&output)).unwrap();
                vm.bus_mut().ram_mut().map(0x800,&[5,9,17,33]).unwrap();
                vm.bus_mut().ram_mut().map(0xe0,&[0x5a]).unwrap();
                for (name,delta) in [("Hold",9u16),("Literal",u16::from(b'H'))] {
                    let entry=output.routine_addresses.iter().find(|r|r.name==name).unwrap().address;
                    for keep in [0u16,255,0x7fff,0xffff] {
                        vm=machine::call(vm,&mut Observe,entry,&keep.to_le_bytes());
                        assert_eq!(machine::bytes(&vm,0xa0,2),keep.wrapping_add(delta).to_le_bytes());
                        assert_eq!(machine::bytes(&vm,0xe0,1),[0x5a]);
                    }
                }
                eprintln!("{runtime:?} address control at {origin:04x} ({table}): {} -> {} bytes",off.bytes.len(),output.bytes.len());
            }
        }
    }
}

#[test]
fn bounded_word_shifts_use_resident_homes_with_overlap_and_live_siblings() {
    for (direction, counts) in [("LSH", &[0u32, 1, 4][..]), ("RSH", &[0, 1, 4, 8][..])] {
        for &count in counts {
            let n=nir(&format!("CARD FUNC Shift(CARD x,keep) x=x {direction} {count} RETURN(x+keep) PROC Main() RETURN"));
            for runtime in [Runtime::ActionCart,Runtime::Standalone] {
                let output=mir6502::generate_output_with_config_and_runtime(&n,0x3000,&config(),runtime).unwrap();
                let off=mir6502::generate_output_with_config_and_runtime(&n,0x3000,
                    &Mir6502Config { enable_parameter_home_selection:false,..config() },runtime).unwrap();
                assert_zp(&output,"Shift","x"); assert_zp(&output,"Shift","keep");
                assert!(output.bytes.len()<off.bytes.len());
                let entry=output.routine_addresses.iter().find(|r|r.name=="Shift").unwrap().address;
                let (mut vm,profile)=support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile,&actionc::codegen::format_load_file(&output)).unwrap();
                for x in [0u16,1,0xff,0x0100,0x8001,0xffff] {
                    for keep in [0x50ffu16,0xffff] {
                        let mut args=x.to_le_bytes().to_vec(); args.extend(keep.to_le_bytes());
                        vm=machine::call(vm,&mut Observe,entry,&args);
                        let shifted=if direction=="LSH" {x<<count} else {x>>count};
                        assert_eq!(machine::bytes(&vm,0xa0,2),shifted.wrapping_add(keep).to_le_bytes(),
                            "{runtime:?}: {x:04x} {direction} {count}, keep={keep:04x}");
                    }
                }
                eprintln!("{runtime:?} {direction} {count}: {} -> {} bytes",off.bytes.len(),output.bytes.len());
            }
        }
    }
}

#[test]
fn helper_backed_word_shifts_preserve_live_words_and_truncated_results() {
    for (direction, count) in [("RSH", 5u32), ("RSH", 6), ("LSH", 5), ("LSH", 6)] {
        let n = nir(&format!("CARD full=$800 BYTE ARRAY bytes=$810
CARD FUNC Hold(CARD x,keep)
  full=x {direction} {count}
  bytes(1)=x {direction} {count}
  RETURN(x+keep)
PROC Main() RETURN"));
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let output = mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config(), runtime).unwrap();
            assert_zp(&output, "Hold", "x");
            assert_zp(&output, "Hold", "keep");
            let entry = output.routine_addresses.iter().find(|r| r.name == "Hold").unwrap().address;
            let (mut vm, profile) = support::vm_for(runtime);
            vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
            for x in [0u16, 0xff, 0x0100, 0x807f, 0xffff] {
                let keep = 0xa53bu16;
                let [lo, hi] = x.to_le_bytes();
                let [klo, khi] = keep.to_le_bytes();
                vm.bus_mut().ram_mut().map(0x810, &[0xcc; 3]).unwrap();
                vm = machine::call(vm, &mut Observe, entry, &[lo, hi, klo, khi]);
                let result = if direction == "RSH" { x >> count } else { x << count };
                assert_eq!(machine::bytes(&vm, 0x800, 2), result.to_le_bytes());
                assert_eq!(machine::bytes(&vm, 0x810, 3), [0xcc, result as u8, 0xcc]);
                assert_eq!(machine::bytes(&vm, 0xa0, 2), x.wrapping_add(keep).to_le_bytes());
            }
        }
    }
}

#[test]
fn caller_keeps_a_word_in_zp_while_a_native_leaf_polls_a_volatile_byte() {
    struct Keyboard { entry: u16, polls: usize }
    impl VmRunHooks for Keyboard {
        type Error = String;
        fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
            if vm.cpu().registers().pc == self.entry {
                let value = [0xff, 0x04, 0x80].get(self.polls).ok_or("unexpected extra poll")?;
                vm.bus_mut().write(0xd20f, *value);
                self.polls += 1;
            }
            Ok(())
        }
    }

    let n = nir("PROC Poll=*() VOLATILE BYTE status=$D20F WHILE status&4 DO OD RETURN
CARD FUNC Hold(CARD keep) Poll() RETURN(keep+1)
PROC Main() Hold(0) RETURN");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config(), runtime).unwrap();
        assert_zp(&output, "Hold", "keep");
        let entry = |name| output.routine_addresses.iter().find(|r| r.name == name).unwrap().address;
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output)).unwrap();
        vm.bus_mut().add_watchpoint(0xd20f);
        vm.bus_mut().clear_events();
        let mut keyboard = Keyboard { entry: entry("Poll"), polls: 0 };
        vm = machine::call(vm, &mut keyboard, entry("Hold"), &0x12ffu16.to_le_bytes());
        let reads = vm.bus().events().iter()
            .filter(|e| e.address == 0xd20f && e.access == actionc_vm::BusAccess::Read)
            .map(|e| e.value).collect::<Vec<_>>();
        assert_eq!(reads, [0xff, 0x04, 0x80]);
        assert_eq!(machine::bytes(&vm, 0xa0, 2), 0x1300u16.to_le_bytes());
    }
}
