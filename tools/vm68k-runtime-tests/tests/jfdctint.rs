//! Compare both DCT passes and sign-preserving rounding with the C oracle.
mod common;
#[path = "common/reference.rs"]
mod reference;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
use reference::{bytes, replace_once, text, words};

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/jfdctint/jfdctint.act");
const SHAPED_SOURCE: &str =
    include_str!("../../../fixtures/runtime/tacle/jfdctint/multidimensional.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/jfdctint/vectors.txt");

fn instrument(source: &str, shaped: bool) -> String {
    let mut source = text(source, false);
    replace_once(
        &mut source,
        "INT result\n",
        "INT result\n\
        BYTE testCommand,testShift\n\
        LONGINT ARRAY testInput(64),testInitial(64),testRows(64)\n\
        PROC CaptureRows()\n\
          BYTE i\n\
          FOR i=0 TO 63 DO testRows(i)=block(i) OD\n\
        RETURN\n",
    );
    replace_once(
        &mut source,
        "  ; Pass 2: eight values per column, eight elements apart.",
        "  CaptureRows()\n  ; Pass 2: eight values per column, eight elements apart.",
    );
    replace_once(
        &mut source,
        "PROC Main()\n  Init()\n  Dct()\n  result=CheckResult()\nRETURN\n",
        "PROC Main()\n\
          BYTE i\n\
          IF testCommand=0 THEN Init()\n\
          ELSE FOR i=0 TO 63 DO block(i)=testInput(i) OD FI\n\
          FOR i=0 TO 63 DO testInitial(i)=block(i) OD\n\
          IF testCommand=2 THEN\n\
            FOR i=0 TO 63 DO block(i)=Descale(block(i),testShift) OD\n\
          ELSE Dct() FI\n\
          result=CheckResult()\n\
        RETURN\n",
    );
    if shaped {
        // Only generated capture/driver accesses use this spelling in the
        // shaped source. Keep the algorithm itself in two coordinates.
        assert_eq!(source.matches("block(i)").count(), 5);
        source = source.replace("block(i)", "testFlat(i)");
        replace_once(
            &mut source,
            "INT result\n",
            "INT result\nLONGINT ARRAY testFlat\n",
        );
        replace_once(
            &mut source,
            "IF testCommand=0 THEN Init()",
            "testFlat=block\nIF testCommand=0 THEN Init()",
        );
    }
    source
}

fn execute(optimize: bool, shaped: bool) {
    let source = common::Source::new(&text(
        &instrument(
            &text(if shaped { SHAPED_SOURCE } else { SOURCE }, optimize),
            shaped,
        ),
        optimize,
    ));
    let image = compile_file(
        &source.0,
        &NativeCompileOptions {
            optimize,
            ..Default::default()
        },
    )
    .unwrap()
    .image;
    let vectors = text(VECTORS, optimize);
    let mut count = 0;
    for line in vectors
        .lines()
        .filter(|s| !s.starts_with('#') && !s.is_empty())
    {
        let f: Vec<_> = line.split_whitespace().collect();
        assert_eq!(f.len(), 9);
        let label = format!("{optimize}/{}", f[0]);
        let mut vm = Machine::from_image(&image).unwrap();
        for (name, field) in [("testCommand", 1), ("testShift", 2)] {
            vm.write_scalar(image.symbol(name).unwrap(), f[field].parse().unwrap())
                .unwrap();
        }
        let input = words(&bytes(f[3]), 4);
        vm.write_array(image.symbol("testInput").unwrap(), &input)
            .unwrap();
        vm.write_array(image.symbol("testRows").unwrap(), &[0xcccccccc; 64])
            .unwrap();
        let run = vm.run(2_000_000);
        assert!(
            matches!(run.outcome, actionc_vm68k_tests::Outcome::Completed),
            "{label}: {run:#?}"
        );
        for (name, field) in [
            ("testInput", 3),
            ("testInitial", 4),
            ("testRows", 5),
            ("block", 6),
        ] {
            assert_eq!(
                vm.read_array(image.symbol(name).unwrap()).unwrap(),
                words(&bytes(f[field]), 4),
                "{label}/{name}"
            );
        }
        for (name, field, width) in [("checksum", 7, 4), ("result", 8, 2)] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                words(&bytes(f[field]), width)[0],
                "{label}/{name}"
            );
        }
        for (name, field) in [("testCommand", 1), ("testShift", 2)] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                f[field].parse::<u32>().unwrap(),
                "{label}/{name}"
            );
        }
        count += 1;
    }
    assert_eq!(count, 181);
}

#[test]
fn jfdctint_raw_matches_both_passes_and_rounding() {
    execute(false, false);
}
#[test]
fn jfdctint_optimized_matches_both_passes_and_rounding() {
    execute(true, false);
}

#[test]
fn jfdctint_multidimensional_raw_matches_both_passes_and_rounding() {
    execute(false, true);
}
#[test]
fn jfdctint_multidimensional_optimized_matches_both_passes_and_rounding() {
    execute(true, true);
}
