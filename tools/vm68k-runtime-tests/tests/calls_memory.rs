mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

#[test]
fn nested_direct_and_indirect_mixed_width_calls_preserve_values_and_stack() {
    let source = common::Source::new(
        r#"
BYTE seed,storage
LONGINT directResult,indirectResult,nestedResult
LONGINT FUNC Sum6(BYTE a INT b CARD c LONGINT d LONGCARD e BYTE f)
  a==+1 b==-2
RETURN(LONGINT(a)+b+LONGINT(c)+d+LONGINT(e)+f)
LONGINT FUNC POINTER callback(BYTE a INT b CARD c LONGINT d LONGCARD e BYTE f)
BYTE POINTER FUNC Address()
RETURN(@storage)
PROC Entry()
  BYTE POINTER p
  callback=@Sum6
  directResult=Sum6(seed,-300,60000,-100000,70000,7)
  indirectResult=callback(seed,-300,60000,-100000,70000,7)
  nestedResult=Sum6(seed,-300,60000,callback(seed,-300,60000,-100000,70000,7),70000,7)
  p=Address() p^=seed
RETURN
"#,
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
        vm.write_scalar(image.symbol("seed").unwrap(), 13).unwrap();
        vm.run(10000).assert_completed();
        for (name, expected) in [
            ("directResult", 29719),
            ("indirectResult", 29719),
            ("nestedResult", 159438),
            ("storage", 13),
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
fn recursive_native_fixtures_execute_with_invocation_local_homes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (file, symbol, expected, budget) in [
        ("recursive_scalar.act", "result", 21, 10000),
        ("recursive_permutation.act", "permutations", 720, 500000),
        ("recursive_queens.act", "solutions", 92, 10000000),
    ] {
        for optimize in [false, true] {
            let image = compile_file(
                root.join("fixtures/native").join(file),
                &NativeCompileOptions {
                    optimize,
                    ..Default::default()
                },
            )
            .unwrap()
            .image;
            let mut vm = Machine::from_image(&image).unwrap();
            vm.run(budget).assert_completed();
            assert_eq!(
                vm.read_scalar(image.symbol(symbol).unwrap()).unwrap(),
                expected,
                "{file}/{optimize}"
            );
        }
    }
}

#[test]
fn local_arrays_initializers_record_stride_and_odd_pointers_execute() {
    let source = common::Source::new(
        r#"
TYPE Trio=[BYTE a,b,c]
LONGCARD result,oddResult
BYTE untouchedBefore,untouchedAfter
VOLATILE LONGCARD odd=$2001
LONGCARD FUNC Local(LONGCARD seed)
  LONGCARD ARRAY values(3)=[1 2 3]
  Trio ARRAY triples(3)
  BYTE ARRAY raw(8)
  LONGCARD POINTER p
  values(1)=seed
  triples(2).c=9
  raw(0)=$A5 raw(5)=$5A
  p=@odd
  p^=$FEDCBA98
  oddResult=odd
  untouchedBefore=raw(0) untouchedAfter=raw(5)
RETURN(values(0)+values(1)+values(2)+triples(2).c)
PROC Entry()
  result=Local(100)+Local(200)
RETURN
"#,
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
        assert_eq!(
            image.symbol("odd").unwrap().address().unwrap(),
            0x2001,
            "odd symbol: {:?}",
            image.symbol("odd").unwrap()
        );
        vm.cpu.mem.map(0x2000, &[0xa5; 8], true, false).unwrap();
        vm.run(10000).assert_completed();
        assert_eq!(
            vm.cpu.mem.bytes(0x2000, 8).unwrap(),
            &[0xa5, 0xfe, 0xdc, 0xba, 0x98, 0xa5, 0xa5, 0xa5]
        );
        for (name, expected) in [
            ("result", 326),
            ("oddResult", 0xfedcba98),
            ("untouchedBefore", 0xa5),
            ("untouchedAfter", 0x5a),
        ] {
            assert_eq!(
                vm.read_scalar(image.symbol(name).unwrap()).unwrap(),
                expected,
                "{optimize}/{name}"
            );
        }
        assert!(image.symbols.iter().any(|s| matches!(
            s.location,
            actionc::mir68k::image::SymbolLocation::Frame { .. }
        )));
    }
}

#[test]
fn overlapping_aggregate_copies_work_in_both_directions() {
    let source = common::Source::new(
        r#"
TYPE Blob=[BYTE a,b,c,d,e]
Blob first=$2000,second=$2001
BYTE direction
PROC Entry()
  Blob POINTER p,q
  p=@first q=@second
  IF direction=0 THEN q^=p^ ELSE p^=q^ FI
RETURN
"#,
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
        for direction in [0, 1] {
            let mut vm = Machine::from_image(&image).unwrap();
            let data = 0x2000;
            vm.cpu
                .mem
                .map(data, &[1, 2, 3, 4, 5, 6, 7, 8], true, false)
                .unwrap();
            vm.write_scalar(image.symbol("direction").unwrap(), direction)
                .unwrap();
            vm.run(10000).assert_completed();
            assert_eq!(
                vm.cpu.mem.bytes(data, 8).unwrap(),
                if direction == 0 {
                    &[1, 1, 2, 3, 4, 5, 7, 8]
                } else {
                    &[2, 3, 4, 5, 6, 6, 7, 8]
                }
            );
        }
    }
}

#[test]
fn oversized_frames_are_rejected_without_truncation() {
    let source = common::Source::new(
        "BYTE result PROC Entry() BYTE ARRAY big(32768) big(0)=1 result=big(0) RETURN",
    );
    let error = compile_file(&source.0, &NativeCompileOptions::default()).unwrap_err();
    assert!(error.to_string().contains("signed-16"), "{error:#?}");
}

#[test]
fn volatile_odd_word_accesses_touch_each_byte_once() {
    let source = common::Source::new(
        "INT input,result VOLATILE INT odd=$2001 PROC Entry() odd=input result=odd RETURN",
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
        vm.cpu.mem.map(0x2000, &[0xa5; 4], true, false).unwrap();
        vm.write_scalar(image.symbol("input").unwrap(), (-2001i16) as u16 as u32)
            .unwrap();
        vm.cpu.mem.trace_range(0x2000..0x2004);
        vm.run(1000).assert_completed();
        assert_eq!(
            vm.cpu.mem.take_trace(),
            [
                (0x2001, true),
                (0x2002, true),
                (0x2001, false),
                (0x2002, false)
            ]
        );
        assert_eq!(
            vm.cpu.mem.bytes(0x2000, 4).unwrap(),
            [0xa5, 0xf8, 0x2f, 0xa5]
        );
        assert_eq!(
            vm.read_scalar(image.symbol("result").unwrap()).unwrap(),
            (-2001i16) as u16 as u32
        );
    }
}

#[test]
fn portable_signed_shift_library_executes_as_native_calls() {
    let source = common::Source::new(
        "MODULE TEST USE MATH.INTEGER AS BITS INT value,result LONGINT wide,wideResult BYTE count\nPROC Main() result=BITS.AsrI(value,count) wideResult=BITS.AsrLI(wide,count) RETURN ENDMODULE",
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
        let symbol = |name| {
            image
                .symbol(name)
                .unwrap_or_else(|e| panic!("{e}: {:?}", image.symbols))
        };
        for value in [i32::MIN, -65537, -32768, -3, -1, 0, 1, 32767, i32::MAX] {
            for count in [0, 1, 15, 16, 31, 32, 64, 255] {
                let mut vm = Machine::from_image(&image).unwrap();
                vm.write_scalar(symbol("TEST.value"), value as u16 as u32)
                    .unwrap();
                vm.write_scalar(symbol("TEST.wide"), value as u32).unwrap();
                vm.write_scalar(symbol("TEST.count"), count).unwrap();
                vm.run(5000).assert_completed();
                assert_eq!(
                    vm.read_scalar(symbol("TEST.result")).unwrap(),
                    ((value as i16) >> count.min(15)) as u16 as u32
                );
                assert_eq!(
                    vm.read_scalar(symbol("TEST.wideResult")).unwrap(),
                    (value >> count.min(31)) as u32
                );
            }
        }
    }
}

#[test]
fn array_indices_keep_all_32_bits_above_64k() {
    let source = common::Source::new(
        "BYTE ARRAY values(65538) LONGCARD index BYTE result PROC Entry() values(index)=77 result=values(index) RETURN",
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
        vm.write_scalar(image.symbol("index").unwrap(), 65537)
            .unwrap();
        vm.run(1000).assert_completed();
        let base = image
            .symbol("values")
            .unwrap()
            .array
            .as_ref()
            .unwrap()
            .backing_address
            .unwrap();
        assert_eq!(vm.cpu.mem.bytes(base + 65537, 1).unwrap(), [77]);
        assert_eq!(vm.cpu.mem.bytes(base + 1, 1).unwrap(), [0]);
        assert_eq!(vm.read_scalar(image.symbol("result").unwrap()).unwrap(), 77);
    }
}
