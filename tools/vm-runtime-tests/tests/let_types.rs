use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::mir6502::{self, Mir6502Config};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Source(PathBuf);
impl Source {
    fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-let-types-{}-{}.act",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, text).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn execute(image: &[u8], runtime: Runtime) -> Vec<u8> {
    let mut vm = CompilerVm::default();
    // Native REAL uses the Atari OS floating-point package on either runtime.
    vm.load_image_bytes(
        ImageKind::Rom,
        "altirraos-xl.rom",
        OS_ROM_BASE,
        std::fs::read(root().join("roms/altirraos-xl.rom")).unwrap(),
    )
    .unwrap();
    let profile = if runtime == Runtime::ActionCart {
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            DEFAULT_CART_BASE,
            std::fs::read(root().join("roms/action.rom")).unwrap(),
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
            .all(|s| s.end < 0x600 || s.start > 0x6FF)
    );
    for address in 0x600..=0x6FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    for (address, bytes) in [
        (0x680, 30_000u32.to_le_bytes()),
        (0x684, (-70_000i32).to_le_bytes()),
    ] {
        vm.bus_mut().ram_mut().map(address, &bytes).unwrap();
    }
    vm.bus_mut().ram_mut().write_word(0x682, 3);
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
    (0x600..0x640)
        .map(|address| result.memory().read(address))
        .collect()
}

fn compile(text: &str, mode: CompileMode, runtime: Runtime) -> Vec<u8> {
    let source = Source::new(text);
    compile_file(
        &source.0,
        &CompileOptions::for_mode(mode).with_runtime(runtime),
    )
    .unwrap_or_else(|e| panic!("{mode:?}/{runtime:?}: {e:?}"))
    .object_bytes()
    .to_vec()
}

fn compile_mir(text: &str, optimized: bool, config: &Mir6502Config, runtime: Runtime) -> Vec<u8> {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    let nir = actionc::nir::lower_program(&semir);
    actionc::nir::verify_program(&nir).unwrap();
    let nir = if optimized {
        actionc::nir::optimize_program(&nir).unwrap()
    } else {
        nir
    };
    let output =
        mir6502::generate_output_with_config_and_runtime(&nir, 0x3000, config, runtime).unwrap();
    actionc::codegen::format_load_file(&output)
}

const TYPES: &str = r#"
TYPE Mode=ENUM [TITLE PLAYING PAUSED]
TYPE Box=[BYTE value BYTE ARRAY items(3)]
Box object
BYTE ARRAY output=$600
BYTE done=$63F
Mode FUNC State() RETURN(Mode.PLAYING)
BYTE FUNC Adjust(BYTE value)
  LET value=value+2
RETURN(value)
BYTE FUNC ReadTwelve() RETURN(Adjust(10))
PROC Main()
  LET r=1.25
  LET r=r+2.0
  output(0)=BYTE(r)
  output(1)=r=3.25
  LET state=State()
  CASE state OF
  WHEN Mode.PLAYING THEN
    BEGIN
      LET selected=11
      output(2)=selected
    END
  ELSE
    output(2)=0
  ESAC
  LET Box POINTER ptr=object
  ptr.value=3
  ptr.items(1)=4
  LET element=@ptr.items(1)
  element^=9
  output(3)=object.value
  output(4)=object.items(1)
  LET callback=@ReadTwelve
  output(5)=callback()
  LET BYTE FUNC POINTER typed()=callback
  output(6)=typed()
  LET BYTE small=$1FF
  output(7)=small
  LET CHAR character='A
  output(8)=character
  done=$A5
  DO OD
RETURN
"#;

const WIDE: &str = r#"
LONGCARD ARRAY output=$600
CARD a=$680,b=$682
LONGINT input=$684
BYTE done=$63F
LONGCARD FUNC Product(CARD left,right)
  LET result=LONGCARD(left)*right
RETURN(result)
LONGINT FUNC Negate(LONGINT arg)
  LET arg=-arg
RETURN(arg)
PROC Main()
  LET LONGCARD narrow=a*b
  LET promoted=LONGCARD(a)*b
  output(0)=narrow
  output(1)=promoted
  LET value=Negate(input)
  LET value=value/LONGINT(3)
  output(2)=LONGCARD(value)
  LET callback=@Product
  LET calculated=callback(a,b)
  output(3)=calculated
  LET bits=LONGCARD($FEDCBA98)
  LET bits=bits RSH 12
  output(4)=bits
  output(5)=LONGCARD(input MOD LONGINT(3))
  CASE calculated OF
  WHEN 90000 THEN
    BEGIN
      LET selected=LONGCARD(1)
      output(6)=selected
    END
  ELSE
    output(6)=0
  ESAC
  done=$A5
  DO OD
RETURN
"#;

#[test]
fn let_real_enum_record_pointer_and_typed_callback_execute_in_all_modes() {
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let bytes = execute(&compile(TYPES, mode, runtime), runtime);
            assert_eq!(
                &bytes[..9],
                &[3, 1, 11, 3, 9, 12, 12, 255, 65],
                "{mode:?}/{runtime:?}"
            );
            assert_eq!(bytes[63], 0xA5);
        }
    }
}

#[test]
fn let_wide_operations_preserve_narrow_intermediates_and_shadowed_results() {
    for (mode, runtime) in [CompileMode::Optimized, CompileMode::Mir6502].into_iter().flat_map(|mode| [Runtime::ActionCart, Runtime::Standalone].into_iter().map(move |runtime| (mode, runtime))) {
        let bytes = execute(&compile(WIDE, mode, runtime), runtime);
        let words = bytes[..28]
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            words,
            [
                90_000 % 65_536,
                90_000,
                70_000 / 3,
                90_000,
                0xFEDCBA98 >> 12,
                u32::MAX,
                1
            ]
        );
        assert_eq!(bytes[63], 0xA5);

    }
}

#[test]
fn let_baseline_and_optimized_nir_mir_preserve_effectful_initialization() {
    let text = std::fs::read_to_string(root().join("fixtures/runtime/let_bindings.act")).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for (optimized, config) in [
            (false, Mir6502Config::default()),
            (true, Mir6502Config::optimized()),
            (
                false,
                Mir6502Config {
                    enable_peepholes: false,
                    select_runtime_helpers: false,
                    ..Mir6502Config::default()
                },
            ),
        ] {
            let bytes = execute(&compile_mir(&text, optimized, &config, runtime), runtime);
            assert_eq!(
                &bytes[..14],
                &[10, 11, 101, 10, 7, 8, 9, 11, 6, 42, 42, 7, 8, 0xA5]
            );
        }
    }
}

#[test]
fn let_wide_legalization_does_not_depend_on_optional_optimizations() {
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        // Narrow multiplication needs the ordinary helper selector. Pure wide
        // arithmetic has mandatory legalization even when that selector is off.
        for (source, select_runtime_helpers) in [
            (WIDE.to_string(), true),
            (
                WIDE.replace("LET LONGCARD narrow=a*b", "LET LONGCARD narrow=24464"),
                false,
            ),
        ] {
            let bytes = execute(
                &compile_mir(
                    &source,
                    false,
                    &Mir6502Config {
                        enable_peepholes: false,
                        select_runtime_helpers,
                        ..Mir6502Config::default()
                    },
                    runtime,
                ),
                runtime,
            );
            let words = bytes[..28]
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(
                words,
                [24_464, 90_000, 23_333, 90_000, 0xFEDCB, u32::MAX, 1]
            );
            assert_eq!(bytes[63], 0xA5);
        }
    }
}
