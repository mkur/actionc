//! Decode the legacy packet into values; native placement comes from the CLI image.
mod common;
#[path = "common/compiler.rs"]
mod compiler;
#[path = "common/reference.rs"]
mod reference;
use actionc::mir68k::image::SymbolView;
use actionc_vm68k_tests::Machine;

const DRIVER: &str = include_str!("../../../fixtures/runtime/tacle/statemate.act");
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/statemate-kernel.inc");
const FIELDS: &str = include_str!("../../../fixtures/runtime/tacle/state.tsv");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/statemate-vectors.txt");

fn check(optimize: bool) {
    // Both real source/include loading and the vector decoder see LF and CRLF.
    let source = common::Source::new(&reference::text(DRIVER, optimize));
    let directory = source.0.parent().unwrap();
    std::fs::write(
        directory.join("statemate-kernel.inc"),
        reference::text(CORE, optimize),
    )
    .unwrap();
    let image = compiler::compile(
        &source.0,
        &directory.join("statemate.json"),
        if optimize { &[] } else { &["--no-opt"] },
    );
    let mut coverage = [false; 201];
    let fields: Vec<_> = FIELDS
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|line| {
            let f: Vec<_> = line.split_whitespace().collect();
            let offset = f[3].parse::<usize>().unwrap();
            let size = f[4].parse::<usize>().unwrap();
            for covered in &mut coverage[offset..offset + size] {
                assert!(!*covered);
                *covered = true;
            }
            let symbol = image.symbol(f[1]).unwrap();
            if size == 64 {
                let array = symbol.array.as_ref().unwrap();
                assert_eq!(
                    (array.count, array.stride, array.element_width),
                    (Some(64), 1, 1)
                );
            } else {
                assert_eq!(symbol.scalar_type(), Some((size as u32, f[2] == "INT")));
            }
            (symbol, offset, size)
        })
        .collect();
    assert!(coverage.into_iter().all(|c| c));
    let text = reference::text(VECTORS, optimize);
    let vectors: Vec<_> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(vectors.len(), 157);
    assert_eq!(reference::text(CORE, false).matches("CASE ").count(), 16);
    for line in vectors {
        let f: Vec<_> = line.split_whitespace().collect();
        assert_eq!(f.len(), 4);
        let input = reference::bytes(f[2]);
        let expected = reference::bytes(f[3]);
        assert_eq!((input.len(), expected.len()), (201, 201));
        let mut vm = Machine::from_image(&image).unwrap();
        vm.write_scalar(image.symbol("command").unwrap(), f[1].parse().unwrap())
            .unwrap();
        for &(symbol, offset, size) in &fields {
            let values = reference::words(
                &input[offset..offset + size],
                if size == 64 { 1 } else { size },
            );
            if size == 64 {
                vm.write_array(symbol, &values).unwrap();
            } else {
                vm.write_scalar(symbol, values[0]).unwrap();
            }
        }
        let result = vm.run(2_000_000);
        assert!(
            matches!(result.outcome, actionc_vm68k_tests::Outcome::Completed),
            "{optimize}/{}: {result:?}",
            f[0]
        );
        assert_eq!(
            vm.read_scalar(image.symbol("signature").unwrap()).unwrap(),
            0xa5
        );
        assert_eq!(
            vm.read_scalar(image.symbol("command").unwrap()).unwrap(),
            f[1].parse::<u32>().unwrap()
        );
        for &(symbol, offset, size) in &fields {
            let actual = if size == 64 {
                vm.read_array(symbol).unwrap()
            } else {
                vec![vm.read_scalar(symbol).unwrap()]
            };
            assert_eq!(
                actual,
                reference::words(
                    &expected[offset..offset + size],
                    if size == 64 { 1 } else { size }
                ),
                "{optimize}/{}: {}",
                f[0],
                symbol.name
            );
        }
    }
}
#[test]
fn statemate_raw_matches_every_c_state() {
    check(false);
}
#[test]
fn statemate_optimized_matches_every_c_state() {
    check(true);
}
