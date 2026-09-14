mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

#[test]
fn runtime_products_match_wrapping_host_oracle() {
    for (kind, bits) in [
        ("BYTE", 8),
        ("CARD", 16),
        ("INT", 16),
        ("LONGCARD", 32),
        ("LONGINT", 32),
    ] {
        let source = common::Source::new(&format!(
            "{kind} a,b,product,square\nPROC Entry() product=a*b square=a*a RETURN\n"
        ));
        let mask = u32::MAX >> (32 - bits);
        let sign = 1u32 << (bits - 1);
        let values = [
            0,
            1,
            2,
            3,
            sign - 1,
            sign,
            mask - 1,
            mask,
            0xffff & mask,
            0x10000 & mask,
            0x10001 & mask,
            0x80010001 & mask,
            0x12345678 & mask,
        ];
        let mut pairs: Vec<_> = values
            .iter()
            .flat_map(|&a| values.iter().map(move |&b| (a, b)))
            .collect();
        let mut seed = 0x918a4207u32;
        for _ in 0..128 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let a = seed & mask;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            pairs.push((a, seed & mask));
        }
        for optimize in [false, true] {
            let image = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap()
            .image;
            for &(a, b) in &pairs {
                let mut vm = Machine::from_image(&image).unwrap();
                for (name, value) in [("a", a), ("b", b)] {
                    vm.write_scalar(image.symbol(name).unwrap(), value).unwrap();
                }
                vm.run(1000).assert_completed();
                for (name, expected) in [
                    ("product", a.wrapping_mul(b) & mask),
                    ("square", a.wrapping_mul(a) & mask),
                    ("a", a),
                    ("b", b),
                ] {
                    assert_eq!(
                        vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                        expected,
                        "{kind}/{optimize}/{a:x}/{b:x}/{name}"
                    );
                }
            }
        }
    }
}

#[test]
fn products_preserve_captured_operands_calls_and_result_widths() {
    let source = common::Source::new(
        "LONGINT a,b,result\nBYTE calls\nINT x,y\nLONGINT narrow,wide\nLONGINT FUNC Factor() calls==+1 RETURN(b)\nLONGINT FUNC Product(LONGINT p,q) RETURN(p*q)\nPROC Entry() result=Product(a,Factor())+Product(Factor(),a) narrow=LONGINT(x*y) wide=LONGINT(x)*LONGINT(y) RETURN",
    );
    for optimize in [false, true] {
        let image = compile_file(
            &source.0,
            &NativeCompileOptions {
                optimize,
                ..Default::default()
            },
        )
        .unwrap()
        .image;
        let mut vm = Machine::from_image(&image).unwrap();
        for (name, value) in [
            ("a", 0x81234567),
            ("b", 0x12348009),
            ("x", 0xffff),
            ("y", 0x8000),
        ] {
            vm.write_scalar(image.symbol(name).unwrap(), value).unwrap();
        }
        vm.run(5000).assert_completed();
        for (name, expected) in [
            (
                "result",
                0x81234567u32.wrapping_mul(0x12348009).wrapping_mul(2),
            ),
            ("calls", 2),
            ("narrow", 0xffff8000),
            ("wide", 0x8000),
        ] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                expected,
                "{optimize}/{name}"
            );
        }
    }
}

#[test]
fn constant_products_match_oracle_with_selection_on_and_off() {
    use actionc::mir68k::materialize::Options;
    for (kind, bits) in [
        ("BYTE", 8),
        ("INT", 16),
        ("CARD", 16),
        ("LONGINT", 32),
        ("LONGCARD", 32),
    ] {
        let mask = u32::MAX >> (32 - bits);
        let mut factors = vec![
            0,
            1,
            3,
            5,
            7,
            9,
            10,
            12,
            17,
            31,
            63,
            85,
            127,
            255,
            257,
            65535,
            65537,
            0x12345678,
            u32::MAX,
        ];
        for shift in 0..bits {
            factors.push(1 << shift);
        }
        let negatives: Vec<_> = factors
            .iter()
            .map(|v: &u32| v.wrapping_neg() & mask)
            .collect();
        factors.extend(negatives);
        let mut text = format!(
            "{kind} input\n{kind} ARRAY results({})\nPROC Main()\n",
            factors.len() * 2
        );
        for (i, factor) in factors.iter().enumerate() {
            // Unsigned word literals must enter CARD before widening: bare
            // decimal 32768..65535 otherwise has Action's signed INT meaning.
            let unsigned = if *factor <= 65535 { "CARD" } else { "LONGCARD" };
            let literal = format!("{kind}({unsigned}({factor}))");
            text.push_str(&format!(
                "results({})=input*{literal}\nresults({})={literal}*input\n",
                i * 2,
                i * 2 + 1
            ));
        }
        text.push_str("RETURN\n");
        let source = common::Source::new(&text);
        let mut seed = 0x918a4207u32;
        let mut values = vec![0, 1, 127, 128, 65535, 65536, 0x80000000, u32::MAX];
        for _ in 0..24 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            values.push(seed);
        }
        for select_instructions in [false, true] {
            let image = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize: true,
                    codegen: Options {
                        select_instructions,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap()
            .image;
            for &input in &values {
                let mut vm = Machine::from_image(&image).unwrap();
                vm.write_scalar(image.symbol("input").unwrap(), input & mask)
                    .unwrap();
                vm.run(100_000).assert_completed();
                let expected: Vec<_> = factors
                    .iter()
                    .flat_map(|v| [input.wrapping_mul(*v) & mask; 2])
                    .collect();
                assert_eq!(
                    vm.read_array(image.symbol("results").unwrap()).unwrap(),
                    expected,
                    "{kind}/{select_instructions}/{input:x}"
                );
            }
        }
    }
}

#[test]
fn constant_stride_selection_reduces_long_multiply_instructions() {
    use actionc::mir68k::materialize::Options;
    let source =
        common::Source::new("LONGCARD input,result\nPROC Main() result=input*LONGCARD(10) RETURN");
    let steps: Vec<_> = [false, true]
        .into_iter()
        .map(|select_instructions| {
            let image = compile_file(
                &source.0,
                &NativeCompileOptions {
                    codegen: Options {
                        select_instructions,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap()
            .image;
            let mut vm = Machine::from_image(&image).unwrap();
            vm.write_scalar(image.symbol("input").unwrap(), 0x81234567)
                .unwrap();
            let run = vm.run(1000);
            run.assert_completed();
            assert_eq!(
                vm.read_scalar(image.symbol("result").unwrap()).unwrap(),
                0x81234567u32.wrapping_mul(10)
            );
            run.steps
        })
        .collect();
    assert!(steps[1] < steps[0], "{steps:?}");
}
