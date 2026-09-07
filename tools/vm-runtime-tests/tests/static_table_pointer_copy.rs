use actionc::mir6502::{self, Mir6502Config};
use actionc::nir;
use actionc::runtime::Runtime;
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::Path;

fn source(wide_index: bool) -> String {
    let ty = if wide_index { "CARD" } else { "BYTE" };
    let size = if wide_index { 512 } else { 256 };
    // Use byte literals in the image. The runtime index expression still
    // wraps at its declared type, independently of image construction.
    let values = (0..size)
        .map(|i| ((i * 37 + i / 256 * 11 + 17) % 256).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "BYTE ARRAY table({size})=[{values}]\n{ty} sourceIndex=$500\nCARD targetIndex=$502\nBYTE POINTER destination=$504\nBYTE done=$506\nPROC Main()\ndestination(targetIndex)=table(sourceIndex+1)\ndone=$A5\nDO OD\nRETURN"
    )
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

fn run(
    image: &[u8],
    runtime: Runtime,
    source_index: u16,
    wide_index: bool,
    base: u16,
    offset: u16,
) {
    let mut vm = CompilerVm::default();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let profile = if runtime == Runtime::ActionCart {
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
            .all(|s| s.end < 0x500 || s.start > 0xAFF)
    );
    let mut expected = vec![0xCC; 0x600];
    expected[0..2].copy_from_slice(&source_index.to_le_bytes());
    expected[2..4].copy_from_slice(&offset.to_le_bytes());
    expected[4..6].copy_from_slice(&base.to_le_bytes());
    expected[6] = 0;
    vm.bus_mut().ram_mut().map(0x500, &expected).unwrap();
    let index = if wide_index {
        source_index + 1
    } else {
        u16::from((source_index as u8).wrapping_add(1))
    };
    expected[usize::from(base + offset - 0x500)] =
        ((index * 37 + index / 256 * 11 + 17) % 256) as u8;
    expected[6] = 0xA5;
    let mut done = false;
    for _ in 0..10_000 {
        vm.step_cpu().unwrap();
        if vm.bus().ram().read(0x506) == 0xA5 {
            done = true;
            break;
        }
    }
    assert!(done, "copy did not finish");
    let actual = (0x500..=0xAFF)
        .map(|address| vm.bus().ram().read(address))
        .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "index={source_index} wide={wide_index} destination={base:04X}+{offset}"
    );
}

#[test]
fn static_table_pointer_copy_preserves_wrapping_pages_and_captured_destination_inputs() {
    for wide_index in [false, true] {
        let program = lower(&source(wide_index));
        let optimized = nir::optimize_program(&program).unwrap();
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
                    let seeds: &[u16] = if wide_index {
                        &[0, 1, 127, 254, 255, 256, 510]
                    } else {
                        &[0, 1, 127, 128, 253, 254, 255]
                    };
                    for &seed in seeds {
                        // The first five targets overwrite the captured index,
                        // destination index or pointer cell itself. Other cases
                        // cross pages and require word destination arithmetic.
                        for (base, offset) in [
                            (0x500, 0),
                            (0x500, 2),
                            (0x500, 3),
                            (0x500, 4),
                            (0x500, 5),
                            (0x6F0, 15),
                            (0x6F0, 255),
                            (0x700, 256),
                            (0x800, 511),
                        ] {
                            run(&image, runtime, seed, wide_index, base, offset);
                        }
                    }
                }
            }
        }
    }
}
