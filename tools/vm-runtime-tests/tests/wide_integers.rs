use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Source(PathBuf);
impl Source {
    fn new(source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-wide-{}-{}.act",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, source).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run(image: &[u8], runtime: Runtime, a: u32, b: u32) -> Vec<u8> {
    execute(image, runtime, a, b, false)
}

fn execute(image: &[u8], runtime: Runtime, a: u32, b: u32, fault: bool) -> Vec<u8> {
    let mut vm = CompilerVm::default();
    let profile = match runtime {
        Runtime::Standalone => ExecutionProfile::StandaloneObject,
        Runtime::ActionCart => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            for (kind, name, base) in [
                (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
                (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
            ] {
                vm.load_image_bytes(
                    kind,
                    name,
                    base,
                    std::fs::read(root.join("roms").join(name)).unwrap(),
                )
                .unwrap();
            }
            ExecutionProfile::CartridgeObject
        }
    };
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|segment| segment.end < 0x600 || segment.start > 0x6FF)
    );
    for address in 0x600..=0x6FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    for (address, value) in [(0x6E0, a), (0x6E4, b)] {
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            vm.bus_mut().ram_mut().write(address + offset as u16, byte);
        }
    }
    if fault {
        assert!(
            load.segments
                .iter()
                .all(|segment| segment.end < 0x700 || segment.start > 0x7FF)
        );
        vm.bus_mut()
            .ram_mut()
            .map(
                0x780,
                &[
                    0x8D, 0x20, 0x07, 0x8E, 0x21, 0x07, 0x8C, 0x22, 0x07, 0xEE, 0x23, 0x07, 0xF8,
                    0x60,
                ],
            )
            .unwrap(); // Capture A/X/Y and count; SED; RTS exercises the defensive guard.
        vm.bus_mut().ram_mut().write(0x723, 0);
        match runtime {
            Runtime::ActionCart => vm
                .bus_mut()
                .ram_mut()
                .map(0x04CB, &[0x4C, 0x80, 7])
                .unwrap(),
            Runtime::Standalone => vm.bus_mut().ram_mut().write_word(0x000A, 0x0780),
        }
    }
    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps: 25_000,
        history_len: 8,
        ..Default::default()
    });
    assert_eq!(
        outcome.stop_reason(),
        StopReason::StepLimit { max_steps: 25_000 },
        "{runtime:?} {a:08x} {b:08x}: {:?}",
        outcome.report
    );
    let bytes: Vec<_> = (0x600..=0x6FF)
        .map(|address| outcome.memory().read(address))
        .collect();
    assert_eq!(
        bytes[255],
        if fault { 0xCC } else { 0xA5 },
        "completion {runtime:?} {a:08x} {b:08x}; prefix={:?}; {:?}",
        &bytes[..8],
        outcome.report
    );
    if fault {
        assert_eq!(
            (0x720..=0x723)
                .map(|a| outcome.memory().read(a))
                .collect::<Vec<_>>(),
            [100, 0, 100, 1]
        );
        assert_eq!(outcome.report.registers.status & 8, 0);
        let pc = outcome.report.registers.pc;
        assert_eq!(
            [outcome.memory().read(pc), outcome.memory().read(pc + 1)],
            [0xB0, 0xFE]
        );
    }
    bytes
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[test]
fn wide_zero_divisors_fault_before_stores_and_do_not_resume() {
    for ty in ["LONGINT", "LONGCARD"] {
        for operation in ["/", "MOD"] {
            let source = Source::new(&format!(
                "{ty} a=$6E0,b=$6E4,result=$600 BYTE state=$604,done=$6FF\nPROC Main() state=41 result=a {operation} b state=42 done=$A5 RETURN"
            ));
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let compiled = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
                )
                .unwrap();
                let bytes = execute(compiled.object_bytes(), runtime, 0x80000000, 0, true);
                assert_eq!(&bytes[..4], &[0xCC; 4]);
                assert_eq!(bytes[4], 41);
            }
        }
    }
}

#[test]
fn wide_storage_arithmetic_comparisons_and_casts_execute_in_both_runtimes() {
    let source = Source::new(
        "LONGCARD a=$6E0,b=$6E4,sum=$600,difference=$604,both=$608,either=$60C,different=$610\nLONGINT sa=$6E0,sb=$6E4,negative=$614,extended=$628\nCARD low=$62C\nBYTE ueq=$618,une=$619,ult=$61A,ule=$61B,ugt=$61C,uge=$61D,slt=$61E,sle=$61F,sgt=$620,sge=$621,done=$6FF\nPROC Main()\nsum=a+b difference=a-b both=a & b either=a % b different=a XOR b negative=-sa\nueq=a=b une=a<>b ult=a<b ule=a<=b ugt=a>b uge=a>=b\nslt=sa<sb sle=sa<=sb sgt=sa>sb sge=sa>=sb\nextended=LONGINT(INT(a)) low=CARD(a) done=$A5 RETURN",
    );
    let values = [
        0u32, 1, 0xFF, 0x100, 0xFFFF, 0x10000, 0x7FFFFFFF, 0x80000000, 0xFFFF0000, 0xFFFFFFFF,
        0x89ABCDEF,
    ];
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for a in values {
            for b in values {
                let bytes = run(compiled.object_bytes(), runtime, a, b);
                let expected = [
                    a.wrapping_add(b),
                    a.wrapping_sub(b),
                    a & b,
                    a | b,
                    a ^ b,
                    a.wrapping_neg(),
                ];
                for (index, expected) in expected.into_iter().enumerate() {
                    assert_eq!(
                        word(&bytes, index * 4),
                        expected,
                        "operation {index} {runtime:?} {a:08x} {b:08x}"
                    );
                }
                let (sa, sb) = (a as i32, b as i32);
                let comparisons = [
                    a == b,
                    a != b,
                    a < b,
                    a <= b,
                    a > b,
                    a >= b,
                    sa < sb,
                    sa <= sb,
                    sa > sb,
                    sa >= sb,
                ]
                .map(u8::from);
                assert_eq!(
                    &bytes[24..34],
                    &comparisons,
                    "comparison {runtime:?} {a:08x} {b:08x}"
                );
                assert_eq!(word(&bytes, 40), a as u16 as i16 as i32 as u32);
                assert_eq!(&bytes[44..46], &(a as u16).to_le_bytes());
                assert_eq!(&bytes[46..48], &[0xCC; 2]);
            }
        }
    }
}

#[test]
fn wide_nested_direct_and_typed_indirect_calls_preserve_all_result_bytes() {
    let source = Source::new(
        "LONGCARD a=$6E0,b=$6E4,result=$600\nBYTE count=$604,done=$6FF\nLONGCARD FUNC Echo(LONGCARD value) count==+1 RETURN(value)\nLONGCARD FUNC Combine(LONGCARD left,right BYTE bias) RETURN(left+right+LONGCARD(bias))\nLONGCARD FUNC POINTER callback(LONGCARD value)\nPROC Main() count=0 callback=@Echo result=Combine(callback(a),Echo(b),7) done=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for (a, b) in [
            (0, 0),
            (0x12345678, 0x87654321),
            (0xFFFFFFFF, 1),
            (0x7FFFFFFF, 0x10002),
        ] {
            let bytes = run(compiled.object_bytes(), runtime, a, b);
            assert_eq!(
                word(&bytes, 0),
                a.wrapping_add(b).wrapping_add(7),
                "{runtime:?} {a:08x} {b:08x}\n{}",
                compiled.source_listing()
            );
            assert_eq!(bytes[4], 2);
            assert_eq!(&bytes[5..8], &[0xCC; 3]);
        }
    }
}

#[test]
fn typed_byte_callbacks_are_calls_and_preserve_the_argument_in_a() {
    let source = Source::new(
        "CARD input=$6E0 BYTE result=$600,count=$601,done=$6FF\nBYTE FUNC Echo(BYTE value) count==+1 RETURN(value)\nBYTE FUNC POINTER callback(BYTE value)\nPROC Main() count=0 callback=@Echo result=callback(BYTE(input)) done=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for a in [0, 1, 7, 127, 128, 255] {
            let bytes = run(compiled.object_bytes(), runtime, a, 0);
            assert_eq!(
                &bytes[..4],
                &[a as u8, 1, 0xCC, 0xCC],
                "{}",
                compiled.source_listing()
            );
        }
    }
}

#[test]
fn wide_destination_does_not_reassociate_narrow_arithmetic() {
    let source = Source::new(
        "CARD a=$6E0,b=$6E4 LONGCARD narrow=$600,wide=$604 BYTE smaller=$608,done=$6FF\nPROC Main() narrow=a+b wide=LONGCARD(a)+b smaller=(a+b)<LONGCARD($10000) done=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for (a, b) in [(65535, 1), (32768, 32768), (50000, 50000), (0, 0)] {
            let bytes = run(compiled.object_bytes(), runtime, a, b);
            assert_eq!(
                word(&bytes, 0),
                u32::from((a as u16).wrapping_add(b as u16))
            );
            assert_eq!(word(&bytes, 4), a + b);
            assert_eq!(bytes[8], 1);
        }
    }
}

#[test]
fn wide_record_arrays_and_pointer_views_preserve_guards_and_capture_addresses() {
    let source = Source::new(
        "LONGCARD a=$6E0,b=$6E4,result=$600\nBYTE guards=$604,done=$6FF\nTYPE BufferType=[BYTE before LONGCARD ARRAY data(260) BYTE after]\nBufferType buffer\nLONGCARD POINTER ptr\nPROC Main() buffer.before=$12 buffer.after=$34 ptr=@buffer.data(0) ptr(129)=a buffer.data(259)=b result=ptr(129)+buffer.data(259) guards=buffer.before XOR buffer.after done=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for (a, b) in [(0x12345678, 0x87654321), (0xFFFFFFFF, 1)] {
            let bytes = run(compiled.object_bytes(), runtime, a, b);
            assert_eq!(word(&bytes, 0), a.wrapping_add(b));
            assert_eq!(bytes[4], 0x26);
            assert_eq!(&bytes[5..8], &[0xCC; 3]);
        }
    }
}

#[test]
fn wide_multiply_divide_remainder_and_shifts_match_host_oracles() {
    let source = Source::new(
        "LONGCARD a=$6E0,b=$6E4,product=$600,quotient=$604,remainder=$608,left=$614,right=$618\nLONGINT sa=$6E0,sb=$6E4,sq=$60C,sr=$610 BYTE done=$6FF\nPROC Main() product=a*b quotient=a/b remainder=a MOD b sq=sa/sb sr=sa MOD sb left=a LSH b right=a RSH b done=$A5 RETURN",
    );
    let values = [
        0u32, 1, 2, 7, 8, 15, 16, 17, 31, 32, 33, 255, 256, 65535, 65536, 0x7FFFFFFF, 0x80000000,
        0x80000001, 0xFFFF0001, 0xFFFFFFFF, 0x89ABCDEF,
    ];
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for a in values {
            for b in values.into_iter().filter(|b| *b != 0) {
                let bytes = run(compiled.object_bytes(), runtime, a, b);
                let expected = [
                    a.wrapping_mul(b),
                    a / b,
                    a % b,
                    (a as i32).wrapping_div(b as i32) as u32,
                    (a as i32).wrapping_rem(b as i32) as u32,
                    a.checked_shl(b).unwrap_or(0),
                    a.checked_shr(b).unwrap_or(0),
                ];
                for (index, expected) in expected.into_iter().enumerate() {
                    assert_eq!(
                        word(&bytes, index * 4),
                        expected,
                        "operation {index} {runtime:?} {a:08x} {b:08x}\n{}",
                        compiled.source_listing()
                    );
                }
                assert_eq!(&bytes[28..32], &[0xCC; 4]);
            }
        }
    }
}

#[test]
fn wide_composition_compound_assignments_and_mixed_operands_keep_typed_widths() {
    let source = Source::new(
        "LONGCARD a=$6E0,b=$6E4,result=$600,left=$604,right=$608,wide=$60C,narrow=$610 LONGINT signed=$614,sa=$6E0 INT small=$6E4 CARD na=$6E0,nb=$6E4 BYTE done=$6FF\nPROC Main() result=(a*b)+(b*a) result==XOR a left=a LSH LONGCARD(0) right=a RSH LONGCARD(0) wide=LONGCARD(na)*nb narrow=LONGCARD(CARD(na*nb)) signed=sa*small signed==+small done=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for (a, b) in [
            (65535, 2),
            (0x80000000, 0xFFFF),
            (0x12345678, 0xFEDCBA98),
            (0xFFFFFFFF, 0),
            (0x80008000, 0x8000),
        ] {
            let bytes = run(compiled.object_bytes(), runtime, a, b);
            let small = b as u16 as i16 as i32;
            let expected = [
                a.wrapping_mul(b).wrapping_mul(2) ^ a,
                a,
                a,
                u32::from(a as u16) * u32::from(b as u16),
                u32::from((a as u16).wrapping_mul(b as u16)),
                (a as i32).wrapping_mul(small).wrapping_add(small) as u32,
            ];
            for (i, value) in expected.into_iter().enumerate() {
                assert_eq!(
                    word(&bytes, i * 4),
                    value,
                    "{i} {runtime:?} {a:08x} {b:08x}"
                );
            }
        }
    }
}

#[test]
fn wide_case_labels_preserve_high_bits_and_single_evaluation() {
    let source = Source::new(
        "LONGCARD input=$6E0 BYTE unsigned=$600,signed=$601,calls=$602,done=$6FF\nLONGCARD FUNC Capture() calls==+1 RETURN(input)\nPROC Main() calls=0\nCASE Capture() OF\nWHEN $10001 THEN\nunsigned=1\nWHEN $20001 THEN\nunsigned=2\nWHEN $FFFFFFF0 TO $FFFFFFFF THEN\nunsigned=3\nELSE\nunsigned=4\nESAC\nCASE LONGINT(input) OF\nWHEN -2147483648 TO -2147483646 THEN\nsigned=1\nWHEN -1 THEN\nsigned=2\nWHEN 65537 THEN\nsigned=3\nELSE\nsigned=4\nESAC\ndone=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for a in [
            0u32, 1, 0x10001, 0x20001, 0x80000000, 0x80000001, 0x80000002, 0x80000003, 0xFFFFFFF0,
            0xFFFFFFFF,
        ] {
            let bytes = run(compiled.object_bytes(), runtime, a, 0);
            let unsigned = if a == 0x10001 {
                1
            } else if a == 0x20001 {
                2
            } else if a >= 0xFFFFFFF0 {
                3
            } else {
                4
            };
            let signed = if (i32::MIN..=i32::MIN + 2).contains(&(a as i32)) {
                1
            } else if a == 0xFFFFFFFF {
                2
            } else if a == 65537 {
                3
            } else {
                4
            };
            assert_eq!(
                &bytes[..4],
                &[unsigned, signed, 1, 0xCC],
                "{runtime:?} {a:08x}"
            );
        }
    }
}

#[test]
fn wide_for_loops_terminate_at_signed_and_unsigned_limits() {
    let source = Source::new(
        "LONGCARD index,start=$6E0,finish=$6E4 LONGINT signedIndex,ss=$6E0,se=$6E4 BYTE up=$600,down=$601,sup=$602,sdown=$603,done=$6FF\nPROC Main() up=0 down=0 sup=0 sdown=0\nFOR index=start TO finish DO up==+1 OD\nFOR index=finish TO start STEP -1 DO down==+1 OD\nFOR signedIndex=ss TO se DO sup==+1 OD\nFOR signedIndex=se TO ss STEP -1 DO sdown==+1 OD\ndone=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for (a, b) in [
            (0, 2),
            (65535, 65537),
            (0x7FFFFFFD, 0x7FFFFFFF),
            (0x80000000, 0x80000002),
            (0xFFFFFFFD, 0xFFFFFFFF),
            (7, 5),
        ] {
            let bytes = run(compiled.object_bytes(), runtime, a, b);
            assert_eq!(
                &bytes[..4],
                &[if a <= b { 3 } else { 0 }; 4],
                "{runtime:?} {a:08x} {b:08x}"
            );
        }
    }
}

#[test]
fn wide_initializers_dynamic_indexes_large_steps_and_overlapping_pointer_cells() {
    let source = Source::new(
        "LONGCARD a=$6E0,result=$600,initial=[$12345678] LONGINT negative=[-70000] LONGCARD POINTER ptr=$640 TYPE Container=[BYTE before LONGCARD ARRAY data(260) BYTE after] Container buffer BYTE guards=$604,up=$605,down=$606,done=$6FF\nPROC Main() CARD offset LONGCARD i,local=[$87654321]\noffset=CARD(a) & 255 buffer.before=$12 buffer.after=$34 ptr=@buffer.data(0) ptr(offset)=a buffer.data(259)=initial result=buffer.data(offset)+buffer.data(259)+local+LONGCARD(negative) guards=buffer.before XOR buffer.after\nptr=$640 ptr(0)=a\nup=0 down=0 FOR i=0 TO $30000 STEP $10000 DO up==+1 OD FOR i=$30000 TO 0 STEP -LONGINT($10000) DO down==+1 OD done=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        for a in [0u32, 0x12345681, 0x800000FF, 0xFFFF0001] {
            let bytes = run(compiled.object_bytes(), runtime, a, 0);
            assert_eq!(
                word(&bytes, 0),
                a.wrapping_add(0x12345678)
                    .wrapping_add(0x87654321)
                    .wrapping_sub(70000)
            );
            assert_eq!(&bytes[4..8], &[0x26, 4, 4, 0xCC]);
            assert_eq!(
                word(&bytes, 64),
                a,
                "a store must capture its address before overwriting the pointer cell"
            );
            assert_eq!(&bytes[68..72], &[0xCC; 4]);
        }
    }
}

#[test]
fn classic_diagnoses_wide_types_instead_of_truncating() {
    for text in [
        "LONGCARD value PROC Main() value=$12345678 RETURN",
        "CARD value PROC Main() value=CARD(LONGINT(70000)) RETURN",
    ] {
        let source = Source::new(text);
        for mode in [CompileMode::Compatibility, CompileMode::Optimized] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let error = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap_err();
                assert!(
                    format!("{error:?}").contains("requires the MIR6502 backend"),
                    "{error:?}"
                );
            }
        }
    }
}

#[test]
fn narrow_absolute_loop_bounds_are_values_not_constant_addresses() {
    let source = Source::new(
        "CARD index,limit=$6E4 BYTE count=$600,done=$6FF\nPROC Main() count=0 FOR index=$FFFD TO limit DO count==+1 OD done=$A5 RETURN",
    );
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap();
        let bytes = run(compiled.object_bytes(), runtime, 0, 65535);
        assert_eq!(bytes[0], 3);
    }
}
