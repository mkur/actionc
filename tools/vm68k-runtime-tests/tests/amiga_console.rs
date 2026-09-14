mod common;
use actionc::compiler::native::amiga::{self, Options};
use actionc_vm68k_tests::{
    amiga::{Call, Os},
    hunk::File,
};

#[test]
fn console_outputs_exact_byte_word_signed_decimal_strings_and_raw_bytes() {
    let cards = [
        0, 1, 9, 10, 99, 100, 255, 256, 999, 1000, 9999, 10000, 32767, 32768, 65535,
    ];
    let ints = [-32768, -9999, -1000, -100, -10, -9, -1, 0, 1, 32767];
    let mut source = String::from(
        "BYTE ARRAY raw=[3 65 0 255]\nPROC Main()\nBYTE value\nPrint(\"\") PrintE(\"hello\") Put(0) Put(255) PutE() Print(raw)\nFOR value=0 TO 255 DO PrintB(value) Put(32) OD\nPutE()\n",
    );
    let mut expected = b"hello\n\0\xff\nA\0\xff".to_vec();
    for value in 0..=255 {
        expected.extend(format!("{value} ").bytes());
    }
    expected.push(10);
    for value in cards {
        source.push_str(&format!("PrintCE({value})\n"));
        expected.extend(format!("{value}\n").bytes());
    }
    for value in ints {
        source.push_str(&format!("PrintIE({value})\n"));
        expected.extend(format!("{value}\n").bytes());
    }
    source.push_str("PrintBE(255) PrintC(65535) Put(32) PrintI(-32768) PutE()\n");
    expected.extend(b"255\n65535 -32768\n");
    source.push_str(&format!("PrintE(\"{}\") RETURN\n", "x".repeat(255)));
    expected.extend(vec![b'x'; 255]);
    expected.push(10);
    let source = common::Source::new(&source);
    for (optimize, codegen) in [
        (false, Default::default()),
        (true, Default::default()),
        (true, actionc::mir68k::materialize::Options::conservative()),
    ] {
        let compiled = amiga::compile_file(
            &source.0,
            &Options {
                optimize,
                codegen,
                ..Default::default()
            },
        )
        .unwrap();
        let file = File::parse(&compiled.executable.bytes).unwrap();
        let mut vm = file.load(&[0x10000, 0x40000, 0x60000]).unwrap();
        let mut os = Os::default();
        os.install(&mut vm).unwrap();
        let run = os.run(&mut vm, 2_000_000);
        run.assert_completed();
        assert_eq!(run.registers[0], 0);
        assert_eq!(os.output, expected);
        assert_eq!((os.open_count, os.close_count), (1, 1));
    }
}

#[test]
fn long_decimal_matches_host_formatting_at_boundaries_and_across_full_width_values() {
    use std::collections::BTreeSet;
    let mut values = BTreeSet::from([
        0,
        1,
        32767,
        32768,
        65535,
        65536,
        0x7fffffff,
        0x80000000,
        0x80000001,
        u32::MAX,
    ]);
    for exponent in 0..=9 {
        let power = 10u32.pow(exponent);
        for value in [power - 1, power, power + 1] {
            values.insert(value);
            values.insert(value.wrapping_neg());
        }
    }
    let mut seed = 0xabcdef01u32;
    for _ in 0..64 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        values.insert(seed);
    }
    let data = values
        .iter()
        .map(|v| format!("${v:08X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        "MODULE TEST\nUSE SYS\nLONGCARD ARRAY values=[{data}]\n\
         PROC Show(LONGCARD unsigned LONGINT signed)\n\
         SYS.PrintLC(unsigned) SYS.Put(32) SYS.PrintLCE(unsigned)\n\
         SYS.PrintLI(signed) SYS.Put(32) SYS.PrintLIE(signed)\nRETURN\n\
         PROC Main()\nCARD index\n\
         FOR index=0 TO {} DO Show(values(index),LONGINT(values(index))) OD\n\
         SYS.PrintIE(-32768) SYS.PrintCE(65535)\nRETURN\nENDMODULE\n",
        values.len() - 1,
    );
    let mut expected = Vec::new();
    for &unsigned in &values {
        let signed = unsigned as i32;
        expected.extend(format!("{unsigned} {unsigned}\n{signed} {signed}\n").bytes());
    }
    expected.extend(b"-32768\n65535\n");
    for newline in ["\n", "\r\n"] {
        let source = common::Source::new(&source.replace('\n', newline));
        for (optimize, codegen) in [
            (false, Default::default()),
            (true, Default::default()),
            (true, actionc::mir68k::materialize::Options::conservative()),
        ] {
            let compiled = amiga::compile_file(
                &source.0,
                &Options {
                    optimize,
                    codegen,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut vm = File::parse(&compiled.executable.bytes)
                .unwrap()
                .load(&[0x10000, 0x40000, 0x60000])
                .unwrap();
            let mut os = Os::default();
            os.install(&mut vm).unwrap();
            let run = os.run(&mut vm, 5_000_000);
            run.assert_completed();
            assert_eq!(run.registers[0], 0);
            assert_eq!(
                os.output, expected,
                "optimize={optimize}, newline={newline:?}"
            );
            assert_eq!((os.open_count, os.close_count), (1, 1));
        }
    }
}

#[test]
fn long_decimal_partial_writes_and_failure_preserve_cleanup_and_stop_execution() {
    let source = common::Source::new(
        "MODULE TEST\nUSE SYS\nBYTE after\nPROC Main()\n\
         SYS.PrintLIE(-2147483648) SYS.PrintLCE($FFFFFFFF) after=9\nRETURN\nENDMODULE\n",
    );
    let expected = b"-2147483648\n4294967295\n";
    for optimize in [false, true] {
        let compiled = amiga::compile_file(
            &source.0,
            &Options {
                optimize,
                ..Default::default()
            },
        )
        .unwrap();
        let file = File::parse(&compiled.executable.bytes).unwrap();
        let bases = [0x30000, 0x70000, 0x50000];
        let after = compiled
            .executable
            .object
            .symbols
            .iter()
            .find(|s| s.name == "TEST.after")
            .unwrap()
            .location
            .resolve(&bases)
            .unwrap();
        for (results, bytes, status) in [
            (vec![1; expected.len()], &expected[..], 0),
            (vec![2, 1, 0], &expected[..3], 20),
            (vec![12, 2, -1], &expected[..14], 20),
        ] {
            let mut vm = file.load(&bases).unwrap();
            let mut os = Os::default();
            os.write_results.extend(results);
            os.install(&mut vm).unwrap();
            let run = os.run(&mut vm, 100_000);
            run.assert_completed();
            assert_eq!(run.registers[0], status);
            assert_eq!(os.output, bytes);
            assert_eq!((os.open_count, os.close_count), (1, 1));
            assert_eq!(
                vm.cpu.mem.bytes(after, 1).unwrap(),
                [if status == 0 { 9 } else { 0 }]
            );
        }
    }
}

#[test]
fn partial_writes_advance_the_span_and_errors_cannot_resume_the_program() {
    let source =
        common::Source::new("BYTE after PROC Main() Print(\"Hello\") PrintE(\"!\") after=9 RETURN");
    for optimize in [false, true] {
        let compiled = amiga::compile_file(
            &source.0,
            &Options {
                optimize,
                ..Default::default()
            },
        )
        .unwrap();
        let file = File::parse(&compiled.executable.bytes).unwrap();
        let after = compiled
            .executable
            .object
            .symbols
            .iter()
            .find(|s| s.name == "after")
            .unwrap()
            .location
            .resolve(&[0x10000, 0x40000, 0x60000])
            .unwrap();
        for (results, expected, status) in [
            (&[2, 1][..], &b"Hello!\n"[..], 0),
            (&[2, 1, -1][..], &b"Hel"[..], 20),
            (&[0][..], &b""[..], 20),
            (&[-1][..], &b""[..], 20),
        ] {
            let mut vm = file.load(&[0x10000, 0x40000, 0x60000]).unwrap();
            let mut os = Os::default();
            os.write_results.extend(results);
            os.install(&mut vm).unwrap();
            let run = os.run(&mut vm, 100000);
            run.assert_completed();
            assert_eq!(run.registers[0], status);
            assert_eq!(os.output, expected);
            assert_eq!((os.open_count, os.close_count), (1, 1));
            assert_eq!(
                vm.cpu.mem.bytes(after, 1).unwrap(),
                [if status == 0 { 9 } else { 0 }]
            );
            if results.len() >= 2 {
                let writes: Vec<_> = os.events.iter().filter(|e| e.call == Call::Write).collect();
                assert_eq!(writes[0].args[2], 5);
                assert_eq!(writes[1].args[2], 3);
                assert_eq!(writes[1].args[1], writes[0].args[1] + 2);
                assert_eq!(writes[2].args[2], 2);
                assert_eq!(writes[2].args[1], writes[0].args[1] + 3);
            }
        }
    }
    let source = common::Source::new("PROC Main() Print(\"\") RETURN");
    let compiled = amiga::compile_file(&source.0, &Default::default()).unwrap();
    let mut vm = File::parse(&compiled.executable.bytes)
        .unwrap()
        .load(&[0x10000, 0x40000, 0x60000])
        .unwrap();
    let mut os = Os::default();
    os.install(&mut vm).unwrap();
    os.run(&mut vm, 10000).assert_completed();
    assert!(!os.events.iter().any(|e| e.call == Call::Write));
}
