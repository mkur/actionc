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
fn assert_zp(output: &actionc::codegen::CodegenOutput, routine: &str, name: &str) {
    assert!(output.map.storage_symbols.iter().any(|s| matches!(&s.scope,
        actionc::codegen::CodegenSymbolScope::Routine(r) if r==routine) && s.name==name && s.address<256),
        "missing {routine}.{name} ZP home: {:?}",output.map.storage_symbols);
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
