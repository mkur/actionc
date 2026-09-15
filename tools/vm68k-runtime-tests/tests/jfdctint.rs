//! Compare both DCT passes and sign-preserving rounding with the C oracle.
mod common;
#[path = "common/reference.rs"]
mod reference;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
use reference::{bytes, text, words};
#[path = "common/dct.rs"]
mod dct;
use dct::instrument;

const SOURCE: &str = include_str!("../../../fixtures/runtime/tacle/jfdctint/jfdctint.act");
const SHAPED_SOURCE: &str =
    include_str!("../../../fixtures/runtime/tacle/jfdctint/multidimensional.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/jfdctint/vectors.txt");

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
