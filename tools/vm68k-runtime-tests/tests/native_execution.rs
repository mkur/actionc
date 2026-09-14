mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

#[test]
fn source_sets_global_and_returns_at_two_origins_in_both_modes_and_line_endings() {
    for newline in ["\n", "\r\n"] {
        let source = common::Source::new(
            &"LONGINT result\nPROC Entry()\nresult=42\nRETURN\n".replace('\n', newline),
        );
        for origin in [0x2000, 0x10000] {
            for optimize in [false, true] {
                let compiled = compile_file(
                    &source.0,
                    &NativeCompileOptions {
                        origin,
                        optimize,
                        ..Default::default()
                    },
                )
                .unwrap();
                let result = compiled.image.symbol("result").unwrap();
                assert!(result.address().unwrap() >= origin);
                let mut vm = Machine::from_image(&compiled.image).unwrap();
                assert_eq!(vm.read_scalar(result).unwrap(), 0);
                vm.run(100).assert_completed();
                assert_eq!(vm.read_scalar(result).unwrap(), 42);
            }
        }
    }
}

#[test]
fn invalid_entries_and_unsupported_programs_produce_no_image() {
    for (source, expected) in [
        ("LONGINT value", "entry"),
        ("PROC Entry(BYTE argument) RETURN", "parameterless"),
        ("BYTE value value=1 PROC Entry() RETURN", "top-level"),
    ] {
        let source = common::Source::new(source);
        for optimize in [false, true] {
            let error = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#?}");
        }
    }
}

#[test]
fn unsupported_native_runtime_and_arithmetic_features_are_diagnosed() {
    for (source, expected) in [
        (
            "LONGINT a,b,result PROC Entry() result=a/b RETURN",
            "native",
        ),
        (
            "LONGINT a,b,result PROC Entry() result=a MOD b RETURN",
            "native",
        ),
        ("REAL a,b PROC Entry() a=a+b RETURN", "REAL"),
        ("PROC Entry() PrintE() RETURN", "adapter"),
        ("ORG $3000\nPROC Entry() RETURN", "origin"),
    ] {
        let source = common::Source::new(source);
        for optimize in [false, true] {
            let error = compile_file(
                &source.0,
                &NativeCompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "expected {expected}: {error:#?}"
            );
        }
    }
}
