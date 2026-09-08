//! Independent byte/guard oracle for homonymous aggregate layouts.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, RunRequest, StopReason, VmRunner,
};
use std::path::Path;

struct Source(std::path::PathBuf);
impl Source {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-aggregate-identity-{}.act",
            std::process::id()
        ));
        std::fs::write(&path, SOURCE).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const SOURCE: &str = r#"
BYTE ARRAY output=$600
BYTE done=$63F
PROC First()
  TYPE Pair=[BYTE value]
  TYPE Wrap=[BYTE tag Pair inner]
  Pair ARRAY rows(3)=$681
  Wrap box
  rows(0).value=$11 rows(1).value=$22 rows(2).value=$33
  box.tag=$A1 box.inner=rows(1)
  output(0)=BYTE(SIZEOF(Pair))
  output(1)=BYTE(SIZEOF(Wrap))
  output(2)=box.inner.value
RETURN
PROC Second()
  TYPE Pair=[CARD value]
  TYPE Wrap=[BYTE tag Pair inner]
  Pair ARRAY rows(3)=$691
  Wrap box
  rows(0).value=$1234 rows(1).value=$5678 rows(2).value=$9ABC
  box.tag=$A2 box.inner=rows(1)
  output(3)=BYTE(SIZEOF(Pair))
  output(4)=BYTE(SIZEOF(Wrap))
  output(5)=BYTE(box.inner.value)
  output(6)=BYTE(box.inner.value RSH 8)
RETURN
PROC Main()
  First() Second()
  done=$A5
  DO OD
RETURN
"#;

fn execute(image: &[u8], runtime: Runtime) -> Vec<u8> {
    let mut vm = CompilerVm::default();
    let profile = if runtime == Runtime::ActionCart {
        let rom = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roms/action.rom");
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            DEFAULT_CART_BASE,
            std::fs::read(rom).unwrap(),
        )
        .unwrap();
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|segment| segment.end < 0x600 || segment.start > 0x6FF)
    );
    for address in 0x600..=0x6FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    let result = VmRunner::new(vm).run(RunRequest {
        max_steps: 100_000,
        history_len: 16,
        ..RunRequest::default()
    });
    assert_eq!(
        result.stop_reason(),
        StopReason::StepLimit { max_steps: 100_000 },
        "{:?}",
        result.report
    );
    (0x600..=0x6FF)
        .map(|address| result.memory().read(address))
        .collect()
}

#[test]
fn homonymous_records_preserve_layouts_copies_and_every_guard_byte() {
    let mut expected = vec![0xCC; 256];
    expected[..7].copy_from_slice(&[1, 2, 0x22, 2, 3, 0x78, 0x56]);
    expected[0x3F] = 0xA5;
    expected[0x81..0x84].copy_from_slice(&[0x11, 0x22, 0x33]);
    expected[0x91..0x97].copy_from_slice(&[0x34, 0x12, 0x78, 0x56, 0xBC, 0x9A]);
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(SOURCE).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    let source = Source::new();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let artifact = compile_file(
                &source.0,
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap();
            assert_eq!(
                execute(artifact.object_bytes(), runtime),
                expected,
                "{mode:?}/{runtime:?}"
            );
        }
        for optimized in [false, true] {
            let nir = actionc::nir::lower_program(&semir);
            actionc::nir::verify_program(&nir).unwrap();
            let nir = if optimized {
                actionc::nir::optimize_program(&nir).unwrap()
            } else {
                nir
            };
            let output = mir6502::generate_output_with_config_and_runtime(
                &nir,
                0x3000,
                &Mir6502Config::default(),
                runtime,
            )
            .unwrap();
            let image = actionc::codegen::format_load_file(&output);
            assert_eq!(
                execute(&image, runtime),
                expected,
                "MIR6502/{runtime:?}/optimized={optimized}"
            );
        }
    }
}
