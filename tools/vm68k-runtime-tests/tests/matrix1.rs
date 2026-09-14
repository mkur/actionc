mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;
const CORE: &str = include_str!("../../../fixtures/runtime/tacle/matrix1/kernel.inc");
const DRIVER: &str = include_str!("../../../fixtures/runtime/tacle/matrix1/matrix1.act");
const VECTORS: &str = include_str!("../../../fixtures/runtime/tacle/matrix1/vectors.txt");
const SHAPES: [&str; 3] = ["10x10x10", "3x7x5", "2x129x1"];
const KINDS: [(&str, usize); 4] = [("LONGINT", 4), ("BYTE", 1), ("INT", 2), ("CARD", 2)];
fn numbers(hex: &str, width: usize) -> Vec<u32> {
    assert_eq!(hex.len() % (2 * width), 0);
    hex.as_bytes()
        .chunks_exact(2 * width)
        .map(|chunk| {
            let text = std::str::from_utf8(chunk).unwrap();
            (0..width)
                .map(|i| u32::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap() << (8 * i))
                .sum()
        })
        .collect()
}
fn text(source: &str, optimize: bool) -> String {
    let lf = source.replace("\r\n", "\n");
    if optimize {
        lf.replace('\n', "\r\n")
    } else {
        lf
    }
}
fn replace_once(source: &mut String, old: &str, new: &str) {
    assert_eq!(source.matches(old).count(), 1, "{old}");
    *source = source.replace(old, new);
}
fn execute(optimize: bool) {
    let vector_text = text(VECTORS, optimize);
    let vectors: Vec<Vec<_>> = vector_text
        .lines()
        .filter(|s| !s.starts_with('#') && !s.is_empty())
        .map(|s| s.split_whitespace().collect())
        .collect();
    assert_eq!(vectors.len(), 252);
    let mut executed = 0;
    for (kind, width) in KINDS {
        for shape in SHAPES {
            let dimensions: Vec<_> = shape.split('x').collect();
            let mut driver = DRIVER.replace("\r\n", "\n");
            let mut core = CORE.replace("\r\n", "\n");
            replace_once(
                &mut driver,
                "CONST Rows=10,Inner=10,Columns=10,ElementBytes=4",
                &format!(
                    "CONST Rows={},Inner={},Columns={},ElementBytes={width}",
                    dimensions[0], dimensions[1], dimensions[2]
                ),
            );
            for old in [
                "LONGINT ARRAY matrixA(",
                "LONGINT ARRAY matrixB(",
                "LONGINT ARRAY matrixC(",
            ] {
                replace_once(&mut driver, old, &old.replace("LONGINT", kind));
            }
            for old in [
                "PROC PinDown(LONGINT ARRAY a,b,c)",
                "VOLATILE LONGINT one=[1]",
                "LONGINT POINTER pa,pb,pc",
            ] {
                replace_once(&mut core, old, &old.replace("LONGINT", kind));
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
            .unwrap_or_else(|e| panic!("{kind}/{shape}/{optimize}: {e}"))
            .image;
            for f in vectors.iter().filter(|f| f[0] == kind && f[1] == shape) {
                assert_eq!(f.len(), 12);
                let mut vm = Machine::from_image(&image).unwrap();
                let command = f[3].parse().unwrap();
                vm.write_scalar(image.symbol("command").unwrap(), command)
                    .unwrap();
                for (i, name) in ["matrixA", "matrixB", "matrixC"].iter().enumerate() {
                    vm.write_array(image.symbol(name).unwrap(), &numbers(f[4 + i], width))
                        .unwrap();
                }
                let run = vm.run(2_000_000);
                assert!(
                    matches!(run.outcome, actionc_vm68k_tests::Outcome::Completed),
                    "{kind}/{shape}/{optimize}/{}: {run:#?}",
                    f[2]
                );
                for (i, name) in ["matrixA", "matrixB", "matrixC"].iter().enumerate() {
                    assert_eq!(
                        vm.read_array(image.symbol(name).unwrap()).unwrap(),
                        numbers(f[7 + i], width),
                        "{kind}/{shape}/{optimize}/{}/{name}",
                        f[2]
                    );
                }
                for (name, expected) in [
                    ("checksum", numbers(f[10], 4)[0]),
                    ("result", numbers(f[11], 2)[0]),
                    ("command", command),
                ] {
                    assert_eq!(
                        vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                        expected,
                        "{kind}/{shape}/{optimize}/{}/{name}",
                        f[2]
                    );
                }
                executed += 1;
            }
        }
    }
    assert_eq!(executed, 252);
}
#[test]
fn matrix1_raw_matches_all_types_shapes_and_c_state() {
    execute(false);
}
#[test]
fn matrix1_optimized_matches_all_types_shapes_and_c_state() {
    execute(true);
}
