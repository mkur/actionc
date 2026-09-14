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
