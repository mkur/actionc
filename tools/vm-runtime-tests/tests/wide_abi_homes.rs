//! Public wide ABI metadata and execution across a live narrow caller home.
use actionc::{
    compiler::Runtime,
    mir6502::{self, Mir6502Config, MirDirectParamAbi, MirDirectResultAbi},
};
use actionc_vm::{CompilerVm, VmRunHooks};
fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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
    fn before_step(&mut self, _: &mut CompilerVm) -> Result<(), String> {
        Ok(())
    }
}
fn nir(source: &str) -> actionc::nir::NirProgram {
    let source = support::Source::new(source);
    let loaded = actionc::includes::load_compilation(
        &source.0,
        &actionc::includes::ModuleLoadOptions::default(),
    )
    .unwrap();
    let model = actionc::semantic::analyze_compilation_with_options(
        &loaded,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    actionc::nir::optimize_program(&actionc::nir::lower_program(
        &actionc::semantic::ir::lower_compilation(&loaded, &model),
    ))
    .unwrap()
}
fn config() -> Mir6502Config {
    Mir6502Config {
        enable_small_leaf_inlining: false,
        ..Mir6502Config::optimized()
    }
}

#[test]
fn public_library_signatures_keep_wide_values_and_pointer_arguments() {
    let n = nir(
        "MODULE CLIENT USE SYS USE MATH.INTEGER AS I USE MATH.Q4_12 AS Q
LONGCARD u LONGINT v SIZE length BYTE ARRAY buffer(32) CHAR ARRAY text=\"0\"
SIZE FUNC WidePointer(LONGCARD value BYTE POINTER destination SIZE capacity) RETURN(capacity)
PROC Main()
u=SYS.ValLC(text) v=SYS.ValLI(text)
u=SYS.InputLC() u=SYS.InputLCD(0) v=SYS.InputLI() v=SYS.InputLID(0)
SYS.StrLC(u,text) SYS.StrLI(v,text) SYS.PrintLC(u) SYS.PrintLI(v)
u=Q.SqrWide(7) v=I.AsrLI(v,2) length=WidePointer(u,buffer,32)
RETURN ENDMODULE",
    );
    let p = mir6502::lower_program(&n).unwrap();
    for (stem, parameter_bytes, result_bytes) in [
        ("VALLC", vec![2], 4),
        ("VALLI", vec![2], 4),
        ("INPUTLC", vec![], 4),
        ("INPUTLCD", vec![1], 4),
        ("INPUTLI", vec![], 4),
        ("INPUTLID", vec![1], 4),
        ("STRLC", vec![4, 2], 0),
        ("STRLI", vec![4, 2], 0),
        ("PRINTLC", vec![4], 0),
        ("PRINTLI", vec![4], 0),
        ("SQRWIDE", vec![2], 4),
        ("ASRLI", vec![4, 1], 4),
        ("WIDEPOINTER", vec![4, 2, 2], 2),
    ] {
        let r = p
            .routines
            .iter()
            .find(|r| {
                let name = r.name.to_ascii_uppercase();
                name.ends_with(&format!(".{stem}")) || name.contains(&format!("_{stem}_"))
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing {stem}: {:?}",
                    p.routines.iter().map(|r| &r.name).collect::<Vec<_>>()
                )
            });
        let actual: Vec<_> = r
            .frame
            .direct_parameter_abi
            .as_ref()
            .expect("known parameters")
            .iter()
            .map(|a| a.width().bytes())
            .collect();
        assert_eq!(actual, parameter_bytes, "{}", r.name);
        let pointer_index = match stem {
            "VALLC" | "VALLI" => Some(0),
            "STRLC" | "STRLI" | "WIDEPOINTER" => Some(1),
            _ => None,
        };
        if let Some(index) = pointer_index {
            assert_eq!(
                r.frame.direct_parameter_abi.as_ref().unwrap()[index],
                MirDirectParamAbi::Pointer
            );
        }
        let result = r.direct_result_abi.expect("known result");
        assert_eq!(
            result.width().map_or(0, |w| w.bytes()),
            result_bytes,
            "{}",
            r.name
        );
        if result_bytes == 0 {
            assert_eq!(result, MirDirectResultAbi::Void);
        }
    }
    // Exercise selected providers as well as the public declarations. These
    // calls compile here; OS input/output is covered by the existing I/O target.
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config(), runtime).unwrap();
    }
}

#[test]
fn wide_calls_and_sargs_preserve_live_caller_storage_and_all_result_bytes() {
    let n = nir("LONGCARD input=$700,first=$900,second=$904
LONGCARD FUNC Echo(LONGCARD value) RETURN(value)
LONGINT FUNC Shifted(BYTE tag LONGINT value BYTE tail) RETURN(value+LONGINT(tag)+tail)
PROC Work() first=Echo(input) second=LONGCARD(Shifted(3,LONGINT(input),5)) Echo(input) RETURN
BYTE FUNC Hold(BYTE keep) Work() RETURN(keep+1)
PROC Main() RETURN");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output =
            mir6502::generate_output_with_config_and_runtime(&n, 0x3000, &config(), runtime)
                .unwrap();
        let hold = output
            .routine_addresses
            .iter()
            .find(|r| r.name == "Hold")
            .unwrap()
            .address;
        let homes: Vec<_> = output
            .map
            .storage_symbols
            .iter()
            .filter(|s| {
                matches!(&s.scope,
            actionc::codegen::CodegenSymbolScope::Routine(name) if name == "Hold")
                    && s.address < 256
            })
            .collect();
        assert!(
            !homes.is_empty(),
            "{runtime:?}: live caller home should be placed in ZP"
        );
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, &actionc::codegen::format_load_file(&output))
            .unwrap();
        for value in [0u32, 1, 0x12345678, 0x7fffffff, 0x80000000, 0xffffffff] {
            vm.bus_mut()
                .ram_mut()
                .map(0x700, &value.to_le_bytes())
                .unwrap();
            vm.bus_mut().ram_mut().map(0x8ff, &[0x5a; 10]).unwrap();
            vm = machine::call(vm, &mut Observe, hold, &[0x83]);
            assert_eq!(machine::bytes(&vm, 0xa0, 1), [0x84]);
            assert_eq!(machine::bytes(&vm, 0x900, 4), value.to_le_bytes());
            assert_eq!(
                machine::bytes(&vm, 0x904, 4),
                value.wrapping_add(8).to_le_bytes()
            );
            assert_eq!(machine::bytes(&vm, 0x8ff, 1), [0x5a]);
            assert_eq!(machine::bytes(&vm, 0x908, 1), [0x5a]);
        }
    }
}
