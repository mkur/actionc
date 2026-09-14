mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc::runtime_fault::RuntimeFault;
use actionc_vm68k_tests::{Machine, Outcome};
const KINDS: [(&str, u32, bool); 5] = [
    ("BYTE", 8, false),
    ("CARD", 16, false),
    ("INT", 16, true),
    ("LONGCARD", 32, false),
    ("LONGINT", 32, true),
];
#[test]
fn division_and_remainder_match_full_width_host_oracles() {
    for (kind, bits, signed) in KINDS {
        let source = common::Source::new(&format!(
            "{kind} a,b,q,r PROC Entry() q=a/b r=a MOD b RETURN"
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
        ];
        let mut pairs: Vec<_> = values
            .iter()
            .flat_map(|&a| values.iter().filter(|&&b| b != 0).map(move |&b| (a, b)))
            .collect();
        let mut seed = 0x71429803u32;
        for _ in 0..64 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let a = seed & mask;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            pairs.push((a, (seed & mask).max(1)));
        }
        let number = |v: u32| {
            if signed && v & sign != 0 {
                i64::from(v) - (1i64 << bits)
            } else {
                i64::from(v)
            }
        };
        for optimize in [false, true] {
            let image = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|error| panic!("{kind}/{optimize}: {error}"))
            .image;
            for &(a, b) in &pairs {
                let mut vm = Machine::from_image(&image).unwrap();
                for (name, value) in [("a", a), ("b", b)] {
                    vm.write_scalar(image.symbol(name).unwrap(), value).unwrap();
                }
                vm.run(5000).assert_completed();
                for (name, expected) in [
                    ("q", (number(a) / number(b)) as u32 & mask),
                    ("r", (number(a) % number(b)) as u32 & mask),
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
fn zero_division_faults_before_stores_and_cannot_resume() {
    for (kind, bits, _) in KINDS {
        for operation in ["/", "MOD"] {
            let source = common::Source::new(&format!(
                "{kind} a,b,result BYTE before,after PROC Entry() before=7 result=a {operation} b after=9 RETURN"
            ));
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
                let sentinel = 0xa5a5a5a5 & (u32::MAX >> (32 - bits));
                vm.write_scalar(image.symbol("a").unwrap(), 123).unwrap();
                vm.write_scalar(image.symbol("result").unwrap(), sentinel)
                    .unwrap();
                let run = vm.run(1000);
                assert!(
                    matches!(
                        run.outcome,
                        Outcome::RuntimeFault(RuntimeFault::DivisionByZero)
                    ),
                    "{kind}/{operation}/{optimize}: {run:#?}"
                );
                for (name, expected) in [("before", 7), ("after", 0), ("result", sentinel)] {
                    assert_eq!(
                        vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                        expected
                    );
                }
                let repeated = vm.run(1000);
                assert!(matches!(
                    repeated.outcome,
                    Outcome::RuntimeFault(RuntimeFault::DivisionByZero)
                ));
                assert_eq!(repeated.steps, 0);
                assert_eq!(repeated.pc, run.pc);
            }
        }
    }
}
#[test]
fn division_preserves_calls_captures_compounds_and_converted_zero() {
    for optimize in [false, true] {
        for (body, expected, calls) in [
            (
                "result=Quot(a,Divisor())+Rem(a,Divisor())",
                Some((-11i32) as u32),
                2,
            ),
            ("result=a result==/Divisor()", Some((-10i32) as u32), 1),
            ("result=a result==MOD Divisor()", Some((-1i32) as u32), 1),
            (
                "IF b#LONGINT(0) THEN result=a/b ELSE result=42 FI",
                Some(42),
                0,
            ),
            ("result=a/BYTE(denominator)", None, 0),
            ("LET unused=a/b", None, 0),
        ] {
            let source = common::Source::new(&format!(
                "LONGINT a,b,result CARD denominator BYTE calls,after LONGINT FUNC Divisor() calls==+1 RETURN(10) LONGINT FUNC Quot(LONGINT x,y) RETURN(x/y) LONGINT FUNC Rem(LONGINT x,y) RETURN(x MOD y) PROC Entry() {body} after=1 RETURN"
            ));
            let image = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|error| panic!("{optimize}/{body}: {error}"))
            .image;
            let mut vm = Machine::from_image(&image).unwrap();
            for (name, value) in [
                ("a", (-101i32) as u32),
                ("denominator", 256),
                ("result", 0x12345678),
            ] {
                // Reachability may remove unused test inputs in this case.
                if let Ok(symbol) = image.symbol(name) {
                    vm.write_scalar(symbol, value).unwrap();
                }
            }
            let run = vm.run(20000);
            if let Some(expected) = expected {
                run.assert_completed();
                assert_eq!(
                    vm.read_scalar(image.symbol("result").unwrap()).unwrap(),
                    expected,
                    "{optimize}/{body}"
                );
                assert_eq!(vm.read_scalar(image.symbol("after").unwrap()).unwrap(), 1);
            } else {
                assert!(
                    matches!(
                        run.outcome,
                        Outcome::RuntimeFault(RuntimeFault::DivisionByZero)
                    ),
                    "{optimize}/{body}: {run:#?}"
                );
                if let Ok(symbol) = image.symbol("result") {
                    assert_eq!(vm.read_scalar(symbol).unwrap(), 0x12345678);
                }
                assert_eq!(vm.read_scalar(image.symbol("after").unwrap()).unwrap(), 0);
            }
            let actual_calls = image
                .symbol("calls")
                .map(|s| vm.read_scalar(s).unwrap())
                .unwrap_or(0);
            assert_eq!(actual_calls, calls);
        }
    }
}
