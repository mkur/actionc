mod common;
#[path = "common/reference.rs"]
mod reference;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
use reference::{bytes, replace_once, text, words};
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/binarysearch/kernel.inc");
const DRIVER: &str = include_str!("../../../fixtures/runtime/tacle/binarysearch/binarysearch.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/binarysearch/vectors.txt");
fn execute(optimize: bool) {
    let vectors = text(VECTORS, optimize);
    let rows: Vec<Vec<_>> = vectors
        .lines()
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(|s| s.split_whitespace().collect())
        .collect();
    assert_eq!(rows.len(), 1153);
    let mut executed = 0;
    for (kind, width, result_kind, result_width) in [
        ("LONGINT", 4, "LONGINT", 4),
        ("BYTE", 1, "INT", 2),
        ("INT", 2, "INT", 2),
        ("CARD", 2, "LONGINT", 4),
    ] {
        let mut driver = text(DRIVER, false);
        let mut core = text(CORE, false);
        replace_once(
            &mut driver,
            "TYPE Entry=[LONGINT key,value]",
            &format!("TYPE Entry=[{kind} key,value]"),
        );
        replace_once(&mut driver, "LONGINT query", &format!("{kind} query"));
        replace_once(
            &mut driver,
            "LONGINT result",
            &format!("{result_kind} result"),
        );
        replace_once(
            &mut core,
            "LONGINT FUNC Search(LONGINT x)",
            &format!("{result_kind} FUNC Search({kind} x)"),
        );
        replace_once(
            &mut core,
            "  LONGINT fvalue\n",
            &format!("  {result_kind} fvalue\n"),
        );
        if width != 4 {
            for field in ["key", "value"] {
                replace_once(
                    &mut core,
                    &format!("data(i).{field}=RandomInteger()"),
                    &format!("data(i).{field}={kind}(RandomInteger())"),
                );
            }
        }
        let source = common::Source::new(&text(&driver, optimize));
        std::fs::write(
            source.0.parent().unwrap().join("kernel.inc"),
            text(&core, optimize),
        )
        .unwrap();
        let image = compile_file(
            &source.0,
            &NativeCompileOptions {
                optimize,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{kind}/{optimize}: {e}"))
        .image;
        let table = image.symbol("data").unwrap();
        let layout = table.array.as_ref().unwrap();
        // Entry has two consecutive fields of the same primitive type. Check
        // that schema against emitted layout; never import the 6502 address.
        assert_eq!(layout.count, Some(15));
        assert_eq!(layout.element_width, 2 * width as u32);
        assert_eq!(layout.stride, 2 * width as u32);
        let base = layout.backing_address.unwrap();
        for f in rows.iter().filter(|f| f[0] == kind) {
            assert_eq!(f.len(), 9);
            let mut vm = Machine::from_image(&image).unwrap();
            let command = f[2].parse().unwrap();
            let query = words(&bytes(f[3]), width)[0];
            for (name, value) in [
                ("command", command),
                ("query", query),
                ("seed", words(&bytes(f[4]), 4)[0]),
            ] {
                vm.write_scalar(image.symbol(name).unwrap(), value).unwrap();
            }
            let fields = words(&bytes(f[5]), width);
            assert_eq!(fields.len(), 30);
            let encoded: Vec<_> = fields
                .iter()
                .flat_map(|n| n.to_be_bytes()[4 - width..].to_vec())
                .collect();
            vm.cpu.mem.write(base, &encoded).unwrap();
            let run = vm.run(200000);
            assert!(
                matches!(run.outcome, actionc_vm68k_tests::Outcome::Completed),
                "{kind}/{optimize}/{}: {run:#?}",
                f[1]
            );
            for (name, value) in [
                ("result", words(&bytes(f[8]), result_width)[0]),
                ("seed", words(&bytes(f[6]), 4)[0]),
                ("command", command),
                ("query", query),
            ] {
                assert_eq!(
                    vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                    value,
                    "{kind}/{optimize}/{}/{name}",
                    f[1]
                );
            }
            let expected: Vec<_> = words(&bytes(f[7]), width)
                .iter()
                .flat_map(|n| n.to_be_bytes()[4 - width..].to_vec())
                .collect();
            assert_eq!(
                vm.cpu.mem.bytes(base, expected.len()).unwrap(),
                expected,
                "{kind}/{optimize}/{}/records",
                f[1]
            );
            executed += 1;
        }
    }
    assert_eq!(executed, 1153);
}
#[test]
fn binarysearch_raw_matches_all_c_record_variants() {
    execute(false);
}
#[test]
fn binarysearch_optimized_matches_all_c_record_variants() {
    execute(true);
}
