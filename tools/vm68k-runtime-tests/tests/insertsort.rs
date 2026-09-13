//! One shared Action algorithm; expected state comes from pinned TACLeBench C.
mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/insertsort/kernel.inc");
const DRIVER: &str = include_str!("../../../fixtures/runtime/tacle/insertsort/insertsort.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/insertsort/vectors.txt");
const STATS: [&str; 6] = ["itersI", "minI", "maxI", "itersA", "minA", "maxA"];

#[derive(Debug, PartialEq, Eq)]
struct Vector {
    label: String,
    command: u32,
    input_stats: Vec<u32>,
    input_values: Vec<u32>,
    expected_stats: Vec<u32>,
    expected_values: Vec<u32>,
    result: u32,
}
fn words(hex: &str) -> Vec<u32> {
    assert_eq!(hex.len() % 8, 0);
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    // The reference format is LE. Decode numbers here; VM writes encode them BE.
    bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn parse(text: &str) -> Vec<Vector> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let f: Vec<_> = line.split_whitespace().collect();
            assert_eq!(f.len(), 7);
            let v = Vector {
                label: f[0].into(),
                command: f[1].parse().unwrap(),
                input_stats: words(f[2]),
                input_values: words(f[3]),
                expected_stats: words(f[4]),
                expected_values: words(f[5]),
                result: f[6].parse().unwrap(),
            };
            assert_eq!(v.input_stats.len(), 6);
            assert_eq!(v.expected_stats.len(), 6);
            assert_eq!(v.input_values.len(), 11);
            assert_eq!(v.expected_values.len(), 11);
            assert!(v.command <= 1 && v.result <= 1);
            v
        })
        .collect()
}
#[test]
fn reference_vectors_accept_both_host_line_endings() {
    let lf = VECTORS.replace("\r\n", "\n");
    let vectors = parse(&lf);
    assert_eq!(vectors, parse(&lf.replace('\n', "\r\n")));
    assert_eq!(vectors.len(), 209);
    assert_eq!(vectors[0].label, "upstream");
    assert_eq!(vectors[0].expected_stats, [9, 9, 9, 9, 1, 9]);
    assert_eq!(
        vectors[0].expected_values,
        [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
}
fn execute(optimize: bool) {
    let text = |s: &str| {
        let lf = s.replace("\r\n", "\n");
        if optimize {
            lf.replace('\n', "\r\n")
        } else {
            lf
        }
    };
    let source = common::Source::new(&text(DRIVER));
    std::fs::write(source.0.parent().unwrap().join("kernel.inc"), text(CORE)).unwrap();
    let compiled = compile_file(
        &source.0,
        &NativeCompileOptions {
            optimize,
            ..Default::default()
        },
    )
    .unwrap();
    let image = &compiled.image;
    let symbol = |name| image.symbol(name).unwrap();
    let vectors = parse(&text(VECTORS));
    assert_eq!(vectors.len(), 209);
    let mut total_steps = 0;
    for v in vectors {
        let mut vm = Machine::from_image(image).unwrap();
        vm.write_scalar(symbol("command"), v.command).unwrap();
        for (name, value) in STATS.iter().zip(&v.input_stats) {
            vm.write_scalar(symbol(name), *value).unwrap();
        }
        vm.write_array(symbol("input"), &v.input_values).unwrap();
        vm.write_array(symbol("values"), &[0xa5a5a5a5; 11]).unwrap();
        let result = vm.run(1_000_000);
        assert!(
            matches!(result.outcome, actionc_vm68k_tests::Outcome::Completed),
            "{} / optimize={optimize}: {result:#?}",
            v.label
        );
        total_steps += result.steps;
        assert_eq!(
            vm.read_array(symbol("values")).unwrap(),
            v.expected_values,
            "{} output",
            v.label
        );
        assert_eq!(
            vm.read_array(symbol("input")).unwrap(),
            v.input_values,
            "{} input modified",
            v.label
        );
        for (name, expected) in STATS.iter().zip(&v.expected_stats) {
            assert_eq!(
                vm.read_scalar(symbol(name)).unwrap(),
                *expected,
                "{} {name}",
                v.label
            );
        }
        assert_eq!(
            vm.read_scalar(symbol("result")).unwrap(),
            v.result,
            "{} checksum",
            v.label
        );
        assert_eq!(
            vm.read_scalar(symbol("command")).unwrap(),
            v.command,
            "{} command modified",
            v.label
        );
    }
    eprintln!("MC68000 insertsort optimize={optimize}: 209 vectors, {total_steps} instructions");
}
#[test]
fn insertsort_raw_matches_complete_c_state() {
    execute(false);
}
#[test]
fn insertsort_optimized_matches_complete_c_state() {
    execute(true);
}
