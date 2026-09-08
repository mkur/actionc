//! Whole guarded-memory host oracle across public classic/MIR and raw NIR paths.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, RunRequest, StopReason, VmRunner,
};

fn execute(image: &[u8], runtime: Runtime) -> Vec<u8> {
    let mut vm = CompilerVm::default();
    let profile = if runtime == Runtime::ActionCart {
        let rom = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roms/action.rom");
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
            .all(|s| s.end < 0x600 || s.start > 0xAFF)
    );
    for address in 0x600..=0xAFF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    let result = VmRunner::new(vm).run(RunRequest {
        max_steps: 300_000,
        history_len: 16,
        ..RunRequest::default()
    });
    assert_eq!(
        result.stop_reason(),
        StopReason::StepLimit { max_steps: 300_000 },
        "{:?}",
        result.report
    );
    (0x600..=0xAFF).map(|a| result.memory().read(a)).collect()
}

fn check(name: &str, source: &str, expected: &[u8]) {
    let path = std::env::temp_dir().join(format!(
        "actionc-aggregate-values-{name}-{}.act",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        actionc::semantic::analyze_with_options(&ast, actionc::semantic::SemanticOptions::modern())
            .unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let artifact = compile_file(
                &path,
                &CompileOptions::for_mode(mode)
                    .with_runtime(runtime)
                    .with_origin(0x3000),
            )
            .unwrap_or_else(|e| panic!("{name}/{mode:?}/{runtime:?}: {e:?}"));
            compare(
                name,
                &format!("{mode:?}/{runtime:?}"),
                execute(artifact.object_bytes(), runtime),
                expected,
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
            let output = actionc::mir6502::generate_output_with_config_and_runtime(
                &nir,
                0x3000,
                &actionc::mir6502::Mir6502Config::default(),
                runtime,
            )
            .unwrap();
            compare(
                name,
                &format!("raw-MIR/{runtime:?}/optimized={optimized}"),
                execute(&actionc::codegen::format_load_file(&output), runtime),
                expected,
            );
        }
    }
    std::fs::remove_file(path).unwrap();
}

fn compare(name: &str, path: &str, actual: Vec<u8>, expected: &[u8]) {
    let differences: Vec<_> = actual
        .iter()
        .zip(expected)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| (format!("${:04X}", 0x600 + i), *a, *b))
        .collect();
    assert!(
        differences.is_empty(),
        "{name}/{path}: (address, actual, expected) {differences:?}"
    );
}

#[test]
fn record_snapshots_pointer_fields_and_nested_effectful_addresses() {
    let source = r#"
TYPE Pair=[INT value BYTE ARRAY bytes(3)]
TYPE Holder=[BYTE pre Pair POINTER child INT POINTER numbers BYTE post]
TYPE Wrap=[BYTE pre Holder inner BYTE post]
Pair ARRAY pool(3)=$681
Holder ARRAY links(2)=$691
Wrap wrapped=$6A1
INT ARRAY words(3)=$6B1
BYTE ARRAY output=$600
BYTE done=$63F,calls=$63E
BYTE FUNC Pick()
  calls==+1
RETURN(0)
PROC Main()
  calls=0
  pool(0).value=10 pool(0).bytes(0)=1 pool(0).bytes(1)=2 pool(0).bytes(2)=3
  pool(1).value=20
  words(0)=100 words(1)=200 words(2)=300
  links(0).pre=41 links(0).post=42
  links(0).child=@pool(0) links(0).numbers=words
  LET saved=links(Pick())
  LET captured=saved.child^
  links(0).pre=99 links(0).child=@pool(1)
  saved.child.value=-7 saved.numbers(1)=-300
  pool(0).bytes(1)=99
  links(1)=saved wrapped.inner=links(1)
  output(0)=saved.pre output(1)=saved.post
  output(2)=BYTE(saved.child.value) output(3)=BYTE(saved.child.value RSH 8)
  output(4)=captured.bytes(1)
  wrapped.inner.child.value=-9
  wrapped.inner.numbers(Pick())=-400
  output(5)=BYTE(wrapped.inner.child^.value)
  output(6)=BYTE(wrapped.inner.numbers(1) RSH 8)
  output(7)=BYTE(links(0).child.value)
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[..8].copy_from_slice(&[41, 42, 0xF9, 0xFF, 2, 0xF7, 0xFE, 20]);
    expected[0x3E] = 2;
    expected[0x3F] = 0xA5;
    expected[0x81..0x86].copy_from_slice(&[0xF7, 0xFF, 1, 99, 3]);
    expected[0x86..0x88].copy_from_slice(&[20, 0]);
    expected[0x91..0x97].copy_from_slice(&[99, 0x86, 6, 0xB1, 6, 42]);
    expected[0x97..0x9D].copy_from_slice(&[41, 0x81, 6, 0xB1, 6, 42]);
    expected[0xA2..0xA8].copy_from_slice(&[41, 0x81, 6, 0xB1, 6, 42]);
    expected[0xB1..0xB7].copy_from_slice(&[0x70, 0xFE, 0xD4, 0xFE, 0x2C, 1]);
    check("pointer-fields", source, &expected);
}

#[test]
fn large_runtime_snapshots_reinitialize_and_copies_handle_both_overlap_directions() {
    let source = r#"
TYPE Chunk=[BYTE ARRAY bytes(257)]
Chunk POINTER first,second
BYTE ARRAY raw=$700
BYTE done=$63F,calls=$63E,iteration
CARD index
BYTE FUNC Pick()
  calls==+1
RETURN(0)
PROC Main()
  calls=0
  FOR index=0 TO 299 DO raw(index)=BYTE(index) OD
  first=$701 second=$702
  FOR iteration=0 TO 1 DO
    BEGIN
      LET saved=first(Pick())
      second^=saved
      first^=first^
      first^=second^
    END
  OD
  done=$A5
  DO OD
RETURN
"#;
    let mut expected = vec![0xCC; 0x500];
    expected[0x3E] = 2;
    expected[0x3F] = 0xA5;
    for i in 0..300 {
        expected[0x100 + i] = i as u8;
    }
    for _ in 0..2 {
        let captured = expected[0x101..0x202].to_vec();
        expected[0x102..0x203].copy_from_slice(&captured);
        expected.copy_within(0x102..0x203, 0x101);
    }
    check("overlap", source, &expected);
}
