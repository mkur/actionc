mod common;
use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc::mir68k::materialize::Options;
use actionc_vm68k_tests::{Machine, Outcome};

#[test]
fn static_byte_word_volatile_and_copy_operations_do_not_acquire_guards() {
    for source in [
        "LONGCARD ARRAY data(4) LONGCARD result PROC Main() LONGCARD POINTER p p=data p^=42 result=p^ RETURN",
        "BYTE ARRAY bytes(2,2) CARD ARRAY words(2,2) BYTE b CARD w PROC Main() bytes(0,0)=7 words(0,0)=42 b=bytes(0,0) w=words(0,0) RETURN",
        "VOLATILE LONGCARD ARRAY grid(2,2) LONGCARD result PROC Main() grid(0,0)=$12345678 result=grid(0,0) RETURN",
        "VOLATILE LONGCARD even=$4000,odd=$4005 LONGCARD result PROC Main() even=$12345678 odd=even result=odd RETURN",
        "TYPE Pair=[LONGCARD a,b] Pair first,second PROC Main() first.a=1 first.b=2 second=first RETURN",
    ] {
        let source = common::Source::new(source);
        for optimize in [false, true] {
            let programs = [false, true].map(|guarded_memory| {
                compile_file(
                    &source.0,
                    &NativeCompileOptions {
                        optimize,
                        codegen: Options {
                            guarded_memory,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
                .unwrap()
            });
            assert_eq!(
                format!("{:?}", programs[0].machine),
                format!("{:?}", programs[1].machine)
            );
        }
    }
}

#[test]
fn guards_preserve_longword_values_calls_traces_and_scratch_register_contracts() {
    let source = common::Source::new(
        "LONGCARD ARRAY values(4,4)
LONGCARD seed,total,calls
BYTE count
LONGCARD FUNC Bump(LONGCARD value)
  calls==+1
RETURN(value XOR LONGCARD($A55AA55A))
PROC Main()
  LONGCARD POINTER p
  CARD i
  p=values total=0
  IF count>0 THEN
    FOR i=0 TO CARD(count)-1 DO
      p^=Bump(seed+LONGCARD(i)) total==+p^ p==+4
    OD
  FI
RETURN
",
    );
    for optimize in [false, true] {
        for (register_allocation, forward_temporaries) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let programs = [false, true].map(|guarded_memory| {
                compile_file(
                    &source.0,
                    &NativeCompileOptions {
                        optimize,
                        codegen: Options {
                            guarded_memory,
                            register_allocation,
                            forward_temporaries,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
                .unwrap()
            });
            assert_eq!(
                programs[0]
                    .machine
                    .routines
                    .iter()
                    .map(|r| &r.frame)
                    .collect::<Vec<_>>(),
                programs[1]
                    .machine
                    .routines
                    .iter()
                    .map(|r| &r.frame)
                    .collect::<Vec<_>>()
            );
            for address in [0x4000u32, 0x4001] {
                for count in [0, 8] {
                    let mut baseline = None;
                    for program in &programs {
                        let mut vm = Machine::from_image(&program.image).unwrap();
                        vm.cpu.mem.map(0x4000, &[0xcc; 68], true, false).unwrap();
                        vm.cpu
                            .mem
                            .write(
                                program.image.symbol("values").unwrap().address().unwrap(),
                                &address.to_be_bytes(),
                            )
                            .unwrap();
                        vm.write_scalar(program.image.symbol("count").unwrap(), count)
                            .unwrap();
                        let seed = 0xfffffffcu32;
                        vm.write_scalar(program.image.symbol("seed").unwrap(), seed)
                            .unwrap();
                        vm.cpu.mem.trace_range(0x4000..0x4044);
                        let run = vm.run(100_000);
                        run.assert_completed();
                        let expected: Vec<u32> = (0..count)
                            .map(|i| seed.wrapping_add(i) ^ 0xA55AA55A)
                            .collect();
                        assert_eq!(
                            vm.read_scalar(program.image.symbol("total").unwrap())
                                .unwrap(),
                            expected.iter().fold(0u32, |sum, v| sum.wrapping_add(*v))
                        );
                        assert_eq!(
                            vm.read_scalar(program.image.symbol("calls").unwrap())
                                .unwrap(),
                            count
                        );
                        let bytes = vm.cpu.mem.bytes(0x4000, 68).unwrap().to_vec();
                        let trace = vm.cpu.mem.take_trace();
                        let mut expected_bytes = vec![0xcc; 68];
                        for (i, value) in expected.iter().enumerate() {
                            let at = (address - 0x4000) as usize + 4 * i;
                            expected_bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
                        }
                        assert_eq!(bytes, expected_bytes);
                        let expected_trace: Vec<_> = (0..count)
                            .flat_map(|i| {
                                [true, false].into_iter().flat_map(move |write| {
                                    (0..4).map(move |b| (address + 4 * i + b, write))
                                })
                            })
                            .collect();
                        assert_eq!(trace, expected_trace);
                        if let Some((old_bytes, old_trace, steps)) = &baseline {
                            assert_eq!((&bytes, &trace), (old_bytes, old_trace));
                            if count != 0 && address & 1 == 0 {
                                assert!(
                                    run.steps < *steps,
                                    "aligned unknown pointers must improve"
                                );
                            }
                            if optimize && register_allocation && forward_temporaries && count == 8
                            {
                                eprintln!(
                                    "guarded loop address={address:x}: {steps} -> {} instructions",
                                    run.steps
                                );
                            }
                        } else {
                            baseline = Some((bytes, trace, run.steps));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn guarded_and_bytewise_faults_stop_at_the_same_byte_without_resuming_or_later_writes() {
    for statement in ["answer=p^", "p^=$12345678"] {
        let source = common::Source::new(&format!(
            "LONGCARD POINTER p\nLONGCARD answer\nBYTE before,after\nPROC Main() before=1 answer=0 {statement} after=2 RETURN\n"
        ));
        let programs = [false, true].map(|guarded_memory| {
            compile_file(
                &source.0,
                &NativeCompileOptions {
                    codegen: Options {
                        guarded_memory,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap()
        });
        for address in [0x4000u32, 0x4001] {
            for missing in 0..4u32 {
                let mut baseline = None;
                for program in &programs {
                    let mut vm = Machine::from_image(&program.image).unwrap();
                    for byte in 0..4u32 {
                        if byte != missing {
                            vm.cpu
                                .mem
                                .map(address + byte, &[0xaa], true, false)
                                .unwrap();
                        }
                    }
                    vm.write_scalar(program.image.symbol("p").unwrap(), address)
                        .unwrap();
                    vm.cpu.mem.trace_range(address..address + 4);
                    let run = vm.run(10_000);
                    assert!(
                        matches!(run.outcome, Outcome::MemoryViolation(ref v) if v.address == address + missing && v.write == statement.starts_with("p^")),
                        "{run:?}"
                    );
                    assert_eq!(
                        vm.read_scalar(program.image.symbol("before").unwrap())
                            .unwrap(),
                        1
                    );
                    assert_eq!(
                        vm.read_scalar(program.image.symbol("after").unwrap())
                            .unwrap(),
                        0
                    );
                    assert_eq!(
                        vm.read_scalar(program.image.symbol("answer").unwrap())
                            .unwrap(),
                        0
                    );
                    let bytes: Vec<_> = (0..4)
                        .filter(|b| *b != missing)
                        .map(|b| vm.cpu.mem.bytes(address + b, 1).unwrap()[0])
                        .collect();
                    let trace = vm.cpu.mem.take_trace();
                    if let Some(expected) = &baseline {
                        assert_eq!(&(bytes, trace), expected);
                    } else {
                        baseline = Some((bytes, trace));
                    }
                    assert!(matches!(
                        vm.run(10_000).outcome,
                        Outcome::MemoryViolation(_)
                    ));
                }
            }
        }
    }
}
