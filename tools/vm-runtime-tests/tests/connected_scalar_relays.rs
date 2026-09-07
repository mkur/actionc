use actionc::mir6502::{self, Mir6502Config};
use actionc::nir;
use actionc::runtime::Runtime;
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::Path;

fn table(value: u8) -> u8 {
    value.wrapping_mul(37).wrapping_add(17)
}

fn source(form: usize, parameter: bool) -> String {
    let values = (0..=255)
        .map(|i| table(i).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let (locals, body) = match form {
        0 => (
            "BYTE value",
            "value=target(index)\nvalue=table(value)\nvalue=table(value)\ntarget(index)=value",
        ),
        1 => (
            "BYTE first,second,third",
            "first=target(index)\nsecond=table(first)\nthird=table(second)\ntarget(index)=third",
        ),
        2 => (
            "",
            "LET value=target(index)\nLET value=table(value)\nLET value=table(value)\ntarget(index)=value",
        ),
        _ => unreachable!(),
    };
    let prefix = format!(
        "BYTE ARRAY table(256)=[{values}]\nBYTE ARRAY data=$6F3\nBYTE index=$6F0,done=$6F2\n"
    );
    if parameter {
        format!(
            "{prefix}PROC Map(BYTE ARRAY target,BYTE index)\n{locals}\n{body}\nRETURN\nPROC Main()\nMap(data,index)\ndone=$A5\nDO OD\nRETURN"
        )
    } else {
        format!(
            "{prefix}BYTE POINTER target=[$6F3]\nPROC Main()\n{locals}\n{body}\ndone=$A5\nDO OD\nRETURN"
        )
    }
}

fn lower(source: &str) -> nir::NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let program = nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
    nir::verify_program(&program).unwrap();
    program
}

fn run(image: &[u8], runtime: Runtime, index: u8, value: u8) {
    let mut vm = CompilerVm::default();
    let profile = if runtime == Runtime::ActionCart {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (kind, name, address) in [
            (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
            (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
        ] {
            vm.load_image_bytes(
                kind,
                name,
                address,
                std::fs::read(root.join("roms").join(name)).unwrap(),
            )
            .unwrap();
        }
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0x6F0 || s.start > 0x900)
    );
    let mut expected = vec![0xCC; 0x211];
    expected[0] = index;
    expected[2] = 0;
    expected[usize::from(index) + 3] = value;
    vm.bus_mut().ram_mut().map(0x6F0, &expected).unwrap();
    expected[2] = 0xA5;
    expected[usize::from(index) + 3] = table(table(value));
    let mut done = false;
    for _ in 0..10_000 {
        vm.step_cpu().unwrap();
        if vm.bus().ram().read(0x6F2) == 0xA5 {
            done = true;
            break;
        }
    }
    assert!(done, "relay did not finish");
    let actual = (0x6F0..=0x900)
        .map(|addr| vm.bus().ram().read(addr))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "index={index}, value={value}");
}

#[test]
fn connected_scalar_relays_preserve_all_byte_values_and_page_crossing_stores() {
    for parameter in [false, true] {
        for form in 0..3 {
            let program = lower(&source(form, parameter));
            let optimized = nir::optimize_program(&program).unwrap();
            assert!(optimized.routines.iter().all(|r| r.locals.is_empty()));
            for origin in [0x3000, 0x30F3] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    for (program, config) in [
                        (&program, Mir6502Config::default()),
                        (&optimized, Mir6502Config::optimized()),
                    ] {
                        let output = mir6502::generate_output_with_config_and_runtime(
                            program, origin, &config, runtime,
                        )
                        .unwrap();
                        let image = actionc::codegen::format_load_file(&output);
                        for value in 0..=255 {
                            for index in [0, 255] {
                                run(&image, runtime, index, value);
                            }
                        }
                    }
                }
            }
        }
    }
}
