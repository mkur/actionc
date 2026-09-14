mod common;
use actionc::{
    compiler::native::{NativeCompileOptions, compile_file},
    mir68k::materialize::Options,
};
use actionc_vm68k_tests::Machine;

#[test]
fn immediate_arithmetic_and_full_constant_shift_counts_match_host_oracle() {
    for (kind, bits) in [
        ("BYTE", 8),
        ("INT", 16),
        ("CARD", 16),
        ("LONGINT", 32),
        ("LONGCARD", 32),
    ] {
        let mask = u32::MAX >> (32 - bits);
        let mut operations = Vec::new();
        for count in [0, 1, 7, 8, 9, 15, 16, 17, 31, 32, 64, 255, u32::MAX] {
            for op in ["LSH", "RSH"] {
                operations.push((op, count));
            }
        }
        for constant in [0, 1, 8, 9, 127, 128, 65535, 0x80000000, u32::MAX] {
            for op in ["+", "-", "AND", "OR", "XOR"] {
                operations.push((op, constant));
            }
        }
        let mut source = format!(
            "{kind} input\n{kind} ARRAY results({})\nPROC Main()\n",
            operations.len()
        );
        for (i, (op, value)) in operations.iter().enumerate() {
            let right_type = if matches!(*op, "LSH" | "RSH") {
                // BYTE counts keep narrow operations narrow. The full-width
                // outlier still checks that CPU modulo-64 decoding cannot win.
                if *value <= 255 { "BYTE" } else { "LONGCARD" }
            } else {
                kind
            };
            let right = if matches!(*op, "LSH" | "RSH") {
                format!("{right_type}({value})")
            } else {
                // Bare decimal 65535 is INT -1. Give the literal an unsigned
                // domain before converting it to the tested operand type.
                let unsigned = if *value <= 65535 { "CARD" } else { "LONGCARD" };
                format!("{right_type}({unsigned}({value}))")
            };
            source.push_str(&format!("results({i})=input {op} {right}\n"));
        }
        source.push_str("RETURN\n");
        let source = common::Source::new(&source);
        for optimize in [false, true] {
            for select_instructions in [false, true] {
                let image = compile_file(
                    &source.0,
                    &NativeCompileOptions {
                        optimize,
                        codegen: Options {
                            forward_temporaries: true,
                            select_instructions,
                        },
                        ..Default::default()
                    },
                )
                .unwrap()
                .image;
                for input in [0, 1, 127, 128, 65535, 65536, 0x80000000, u32::MAX] {
                    let input = input & mask;
                    let mut vm = Machine::from_image(&image).unwrap();
                    vm.write_scalar(image.symbol("input").unwrap(), input)
                        .unwrap();
                    vm.run(100_000).assert_completed();
                    let actual = vm.read_array(image.symbol("results").unwrap()).unwrap();
                    let expected: Vec<_> = operations
                        .iter()
                        .map(|(op, value)| {
                            let rhs = value & mask;
                            let value = match *op {
                                "LSH" => {
                                    if *value >= bits {
                                        0
                                    } else {
                                        input << value
                                    }
                                }
                                "RSH" => {
                                    if *value >= bits {
                                        0
                                    } else {
                                        input >> value
                                    }
                                }
                                "+" => input.wrapping_add(rhs),
                                "-" => input.wrapping_sub(rhs),
                                "AND" => input & rhs,
                                "OR" => input | rhs,
                                "XOR" => input ^ rhs,
                                _ => unreachable!(),
                            };
                            value & mask
                        })
                        .collect();
                    assert_eq!(
                        actual, expected,
                        "{kind}/{optimize}/{select_instructions}/{input:x}"
                    );
                }
            }
        }
    }
}
