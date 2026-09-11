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

fn modes_and_runtimes() -> impl Iterator<Item = (CompileMode, Runtime)> {
    [CompileMode::Compatibility, CompileMode::Optimized, CompileMode::Mir6502]
        .into_iter().flat_map(|mode| [Runtime::ActionCart, Runtime::Standalone]
            .into_iter().map(move |runtime| (mode, runtime)))
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
            [101, 0, 101, 1]
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
fn constant_wide_shifts_preserve_all_lanes_narrow_results_and_calls() {
    for count in [0u32, 1, 4, 7, 8, 12, 15, 16, 17, 24, 31, 32, 33, 256, 65536, u32::MAX] {
        let source = Source::new(&format!(
            "LONGCARD a=$6E0,l=$600,r=$604\nLONGINT sa=$6E0,sl=$608,sr=$60C\n\
             CARD narrow=$610 BYTE small=$612,touches=$613,done=$6FF\n\
             LONGCARD FUNC ReadValue() touches==+1 RETURN(a)\n\
             PROC Main() touches=0 l=ReadValue() l=l LSH ${count:X} r=ReadValue() r=r RSH ${count:X}\n\
             sl=sa LSH ${count:X} sr=sa RSH ${count:X}\n\
             narrow=CARD(a RSH ${count:X}) small=BYTE(a RSH ${count:X}) done=$A5 DO OD RETURN"
        ));
        for (mode, runtime) in modes_and_runtimes() {
            let compiled = compile_file(&source.0, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
            if mode == CompileMode::Mir6502 {
                assert!(!compiled.source_listing().contains("::LShift32"));
                assert!(!compiled.source_listing().contains("::RShift32"));
            }
            for a in [0u32, 1, 0x80, 0x100, 0xFFFF, 0x10000, 0x12345678, 0x7FFFFFFF, 0x80000000, 0xFEDCBA98, u32::MAX] {
                let actual = run(compiled.object_bytes(), runtime, a, 0);
                let (left, right) = (a.checked_shl(count).unwrap_or(0), a.checked_shr(count).unwrap_or(0));
                let mut expected = vec![0xCC; 256];
                for (offset, value) in [(0, left), (4, right), (8, left), (12, right), (0xE0, a), (0xE4, 0)] {
                    expected[offset..offset+4].copy_from_slice(&value.to_le_bytes());
                }
                expected[16..18].copy_from_slice(&(right as u16).to_le_bytes());
                expected[18] = right as u8;
                expected[19] = 2;
                expected[255] = 0xA5;
                assert_eq!(actual, expected, "{mode:?}/{runtime:?} input={a:08X}, count={count}");
            }
        }
    }
}

#[test]
fn aligned_wide_comparisons_preserve_all_predicates_and_operand_orders() {
    let operators = ["=", "<>", "<", "<=", ">", ">="];
    for constant in [0u32, 0xFFFF, 0x10000, 0x1FFFF, 0x03FFFFFF, 0x04000000,
        0x04010000, 0x0400FFFF, 0x7FFFFFFF, 0x80000000, 0x8000FFFF, 0xFFFF0000,
        u32::MAX, 0x04000001, 0x03FFFFFE] {
        let mut text = String::from("LONGCARD a=$6E0 LONGINT sa=$6E0 BYTE ARRAY flags(24)=$600 BYTE done=$6FF\n");
        // Keep each routine small: this is a value matrix, not a large-CFG
        // stress test. Every run still checks all 24 results and their guards.
        for (index, op) in operators.into_iter().enumerate() {
            text.push_str(&format!("PROC Check{index}()\n"));
            for (signed, name) in ["a", "sa"].into_iter().enumerate() {
                let literal = format!("{}(${constant:X})", if signed == 1 { "LONGINT" } else { "LONGCARD" });
                for reversed in 0..2 {
                    let (left, right) = if reversed == 0 { (name, literal.as_str()) } else { (literal.as_str(), name) };
                    let slot = signed * 12 + reversed * 6 + index;
                    text.push_str(&format!("IF {left} {op} {right} THEN flags({slot})=1 ELSE flags({slot})=0 FI\n"));
                }
            }
            text.push_str("RETURN\n");
        }
        text.push_str("PROC Main() Check0() Check1() Check2() Check3() Check4() Check5() done=$A5 DO OD RETURN");
        let source = Source::new(&text);
        let mut values = vec![0, 1, 0xFFFF, 0x10000, 0x7FFFFFFF, 0x80000000, u32::MAX, constant];
        for offset in [1u32, 255, 256, 65535, 65536] {
            values.extend([constant.wrapping_sub(offset), constant.wrapping_add(offset)]);
        }
        values.sort_unstable();
        values.dedup();
        for (mode, runtime) in modes_and_runtimes() {
            let compiled = compile_file(&source.0, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
            for &a in &values {
                let actual = run(compiled.object_bytes(), runtime, a, 0);
                let mut expected = vec![0xCC; 256];
                for signed in 0..2 {
                    let (a, b) = if signed == 1 { (a as i32 as i64, constant as i32 as i64) } else { (a as i64, constant as i64) };
                    for reversed in 0..2 {
                        let (a, b) = if reversed == 0 { (a, b) } else { (b, a) };
                        for (index, result) in [a==b, a!=b, a<b, a<=b, a>b, a>=b].into_iter().enumerate() {
                            expected[signed*12 + reversed*6 + index] = u8::from(result);
                        }
                    }
                }
                expected[0xE0..0xE4].copy_from_slice(&a.to_le_bytes());
                expected[0xE4..0xE8].fill(0);
                expected[255] = 0xA5;
                assert_eq!(actual, expected, "{mode:?}/{runtime:?} {a:08X} compared with {constant:08X}");
            }
        }
    }
}

#[test]
fn aligned_wide_comparison_preserves_every_volatile_input_byte() {
    use actionc_vm::{AddressRange, BusAccess};
    let source = Source::new(
        "VOLATILE LONGCARD input=$6E0 BYTE flag=$600,done=$6FF\n\
         PROC Main() IF input >= $04000000 THEN flag=1 ELSE flag=0 FI done=$A5 DO OD RETURN",
    );
    let compiled = compile_file(&source.0,
        &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(Runtime::Standalone)).unwrap();
    for value in [0x03FFFFFFu32, 0x04000000, 0x80000000] {
        let mut vm = CompilerVm::default();
        vm.load_atari_object_for_execution(ExecutionProfile::StandaloneObject, compiled.object_bytes()).unwrap();
        vm.bus_mut().ram_mut().map(0x6E0, &value.to_le_bytes()).unwrap();
        vm.bus_mut().add_watch_range(AddressRange { start: 0x6E0, end: 0x6E3 });
        vm.bus_mut().clear_events();
        let outcome = VmRunner::new(vm).run(RunRequest { max_steps: 25_000, history_len: 8, ..Default::default() });
        assert_eq!(outcome.memory().read(0x6FF), 0xA5, "{:?}", outcome.report);
        assert_eq!(outcome.memory().read(0x600), u8::from(value >= 0x04000000));
        let reads: Vec<_> = outcome.vm.bus().events().iter().filter(|e| e.access == BusAccess::Read)
            .map(|e| e.address).collect();
        assert_eq!(reads, [0x6E0, 0x6E1, 0x6E2, 0x6E3]);
    }
}

fn multiplication_edges() -> [u32; 22] {
    [0, 1, 255, 256, 32767, 32768, 65535, 65536, 0xFFFFFF, 0x1000000,
     0x7FFFFFFF, 0x80000000, 0xFF000000, 0xFFFF0000, 0xFFFF8000, 0xFFFFFFFF,
     0x00010001, 0x00FF0001, 0xFF00FFFF, 0xFFFEFFFF, 0xFFFF0001, 0xFFFFFF00]
}

#[test]
fn wide_multiplication_preserves_products_across_operand_widths() {
    let source = Source::new(
        "LONGCARD a=$6E0,b=$6E4,product=$600 LONGINT sa=$6E0,sb=$6E4,signedProduct=$604\n\
         BYTE done=$6FF PROC Main() product=a*b signedProduct=sa*sb done=$A5 DO OD RETURN",
    );
    for (mode, runtime) in modes_and_runtimes() {
        let compiled = compile_file(&source.0, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        for a in multiplication_edges() {
            for b in multiplication_edges() {
                let actual = run(compiled.object_bytes(), runtime, a, b);
                let mut expected = vec![0xCC; 256];
                for (offset, value) in [(0, a.wrapping_mul(b)), (4, a.wrapping_mul(b)), (0xE0, a), (0xE4, b)] {
                    expected[offset..offset+4].copy_from_slice(&value.to_le_bytes());
                }
                expected[255] = 0xA5;
                assert_eq!(actual, expected, "{mode:?}/{runtime:?} {a:08X} * {b:08X}");
            }
        }
    }
}

fn linked_multiply_probe() -> CompilerVm {
    let source = Source::new(
        "LONGCARD a=$6E0,b=$6E4,result=$600 PROC Main() result=a*b RETURN",
    );
    let compiled = compile_file(&source.0,
        &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(Runtime::Standalone)).unwrap();
    // Invoke the actual linked helper directly so caller setup cannot mask a
    // decimal-mode or scratch-clobber violation. Do not duplicate its assembly.
    let listing = compiled.source_listing();
    let header = listing.lines().find(|line|
        line.starts_with("; ===== PROC ACTION.RUNTIME.ACTIONC::Mult32 ")).unwrap();
    let address = header.split_whitespace().nth(4).unwrap().split("..").next().unwrap();
    let target = u16::from_str_radix(address.trim_start_matches('$'), 16).unwrap();
    let mut vm = CompilerVm::default();
    let load = vm.load_atari_object_for_execution(ExecutionProfile::StandaloneObject, compiled.object_bytes()).unwrap();
    assert!(load.segments.iter().all(|s| s.end < 0x700 || s.start > 0x703));
    vm.bus_mut().ram_mut().map(0x700, &[0xF8, 0x20, target as u8, (target >> 8) as u8]).unwrap(); // SED; JSR
    vm
}

fn probe_product(vm: &mut CompilerVm, a: u32, b: u32) -> u64 {
    vm.bus_mut().ram_mut().map(0, &[0xCC; 256]).unwrap();
    vm.bus_mut().ram_mut().map(0x82, &a.to_le_bytes()).unwrap();
    vm.bus_mut().ram_mut().map(0xC0, &b.to_le_bytes()).unwrap();
    vm.set_pc(0x700);
    let sp = vm.cpu().registers().sp;
    let cycles = vm.cpu().cycles();
    for _ in 0..4000 {
        if vm.cpu().registers().pc == 0x704 { break; }
        vm.step_cpu().unwrap();
    }
    assert_eq!(vm.cpu().registers().pc, 0x704, "helper did not return: {a:08X} * {b:08X}");
    let bytes = std::array::from_fn(|i| vm.bus().ram().read(0xC4 + i as u16));
    assert_eq!(u32::from_le_bytes(bytes), a.wrapping_mul(b), "{a:08X} * {b:08X}");
    assert_eq!(vm.cpu().registers().sp, sp);
    assert_eq!(vm.cpu().registers().status & 8, 0);
    for address in (0..0x82).chain(0x88..0xC0).chain(0xC8..0x100) {
        assert_eq!(vm.bus().ram().read(address), 0xCC, "scratch at ${address:02X}");
    }
    vm.cpu().cycles() - cycles
}

#[test]
fn linked_mult32_preserves_scratch_stack_and_decimal_contract() {
    let mut vm = linked_multiply_probe();
    let mut pairs: Vec<_> = multiplication_edges().into_iter().flat_map(|a|
        multiplication_edges().into_iter().map(move |b| (a, b))).collect();
    let mut seed = 412u32;
    for _ in 0..1024 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = seed;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.push((a, seed));
        pairs.push((a as i16 as i32 as u32, seed as i16 as i32 as u32));
        // Width-specialized helper paths must retain a fully general left
        // operand, including carries from each of its upper bytes.
        pairs.push((a, seed & 0xFF));
        pairs.push((a, seed & 0xFFFF));
        pairs.push((a, seed & 0xFFFFFF));
    }
    for (a, b) in pairs {
        probe_product(&mut vm, a, b);
    }
}

#[test]
fn linked_mult32_narrow_products_match_exhaustive_word_sweeps() {
    let mut vm = linked_multiply_probe();
    let mut maximum = [0; 3];
    // All byte pairs, and every 16-bit pattern as an unsigned and signed
    // operand. Dense and pseudorandom partners exercise carry chains; these
    // sweeps are not a claim to exhaust all 2^32 word-pair combinations.
    for a in 0..=255 {
        for b in 0..=255 {
            maximum[0] = maximum[0].max(probe_product(&mut vm, a, b));
        }
    }
    let mut seed = 0x16_32_6502u32;
    for a in 0..=65535 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let b = seed >> 16;
        for b in [a, b, 65535] {
            maximum[1] = maximum[1].max(probe_product(&mut vm, a, b));
            maximum[2] = maximum[2].max(probe_product(
                &mut vm, a as i16 as i32 as u32, b as i16 as i32 as u32,
            ));
        }
    }
    eprintln!("Mult32 maximum cycles including SED/JSR/RTS: byte={}, word={}, signed={}",
        maximum[0], maximum[1], maximum[2]);
    // Allow branch page crossings while requiring the rotating narrow path
    // to improve on the former four-byte shift/add loop.
    assert!(maximum[0] <= 450, "byte cycles: {}", maximum[0]);
    assert!(maximum[1] <= 850, "word cycles: {}", maximum[1]);
    assert!(maximum[2] <= 950, "signed cycles: {}", maximum[2]);
}

#[test]
fn wide_zero_divisors_fault_before_stores_and_do_not_resume() {
    for ty in ["LONGINT", "LONGCARD"] {
        for operation in ["/", "MOD"] {
            let source = Source::new(&format!(
                "{ty} a=$6E0,b=$6E4,result=$600 BYTE state=$604,done=$6FF\nPROC Main() state=41 result=a {operation} b state=42 done=$A5 RETURN"
            ));
            for (mode, runtime) in modes_and_runtimes() {
                let compiled = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    // This fixture also uses modern source features (value comparisons,
    // CASE, or inline record arrays). Compatibility's LONG path is covered
    // independently by classic_long_integers and the common-source tests.
    for (mode, runtime) in modes_and_runtimes().filter(|(mode, _)| *mode != CompileMode::Compatibility) {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    for (mode, runtime) in modes_and_runtimes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    for (mode, runtime) in modes_and_runtimes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    // This fixture also uses modern source features (value comparisons,
    // CASE, or inline record arrays). Compatibility's LONG path is covered
    // independently by classic_long_integers and the common-source tests.
    for (mode, runtime) in modes_and_runtimes().filter(|(mode, _)| *mode != CompileMode::Compatibility) {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    // This fixture also uses modern source features (value comparisons,
    // CASE, or inline record arrays). Compatibility's LONG path is covered
    // independently by classic_long_integers and the common-source tests.
    for (mode, runtime) in modes_and_runtimes().filter(|(mode, _)| *mode != CompileMode::Compatibility) {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    for (mode, runtime) in modes_and_runtimes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    for (mode, runtime) in modes_and_runtimes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    // This fixture also uses modern source features (value comparisons,
    // CASE, or inline record arrays). Compatibility's LONG path is covered
    // independently by classic_long_integers and the common-source tests.
    for (mode, runtime) in modes_and_runtimes().filter(|(mode, _)| *mode != CompileMode::Compatibility) {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    for (mode, runtime) in modes_and_runtimes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
    // This fixture also uses modern source features (value comparisons,
    // CASE, or inline record arrays). Compatibility's LONG path is covered
    // independently by classic_long_integers and the common-source tests.
    for (mode, runtime) in modes_and_runtimes().filter(|(mode, _)| *mode != CompileMode::Compatibility) {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
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
fn narrow_absolute_loop_bounds_are_values_not_constant_addresses() {
    let source = Source::new(
        "CARD index,limit=$6E4 BYTE count=$600,done=$6FF\nPROC Main() count=0 FOR index=$FFFD TO limit DO count==+1 OD done=$A5 RETURN",
    );
    for (mode, runtime) in modes_and_runtimes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
        )
        .unwrap();
        let bytes = run(compiled.object_bytes(), runtime, 0, 65535);
        assert_eq!(bytes[0], 3);
    }
}

#[test]
fn signed_widening_preserves_every_sign_byte_and_captured_call() {
    let source = Source::new(
        "INT input=$6E0 LONGINT result=$600,again=$604 BYTE calls=$608,done=$6FF\n\
         INT FUNC Capture() calls==+1 RETURN(input)\n\
         LONGINT FUNC Widen(INT value) LONGINT wide wide=LONGINT(value) RETURN(wide)\n\
         PROC Main() calls=0 result=LONGINT(Capture()) again=Widen(input) done=$A5 DO OD RETURN",
    );
    for (mode, runtime) in modes_and_runtimes() {
        let compiled = compile_file(&source.0, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        for high in 0u32..=255 {
            for low in [0, 255] {
                let input = (high << 8) | low;
                let actual = run(compiled.object_bytes(), runtime, input, 0);
                let value = input as i16 as i32 as u32;
                let mut expected = vec![0xCC; 256];
                for (offset, value) in [(0, value), (4, value), (0xE0, input), (0xE4, 0)] {
                    expected[offset..offset+4].copy_from_slice(&value.to_le_bytes());
                }
                expected[8] = 1;
                expected[255] = 0xA5;
                assert_eq!(actual, expected, "{mode:?}/{runtime:?}: {input:04X}");
            }
        }
    }
}

#[test]
fn small_word_shifts_preserve_carries_overlap_and_calls() {
    for count in [3u32, 4] {
        let source = Source::new(&format!(
            "CARD input=$6E0,left=$600,right=$602,overlap=$604\n\
             BYTE calls=$606,done=$6FF\n\
             CARD FUNC Capture() calls==+1 RETURN(input)\n\
             PROC Main() calls=0 left=Capture() LSH {count} right=Capture() RSH {count}\n\
             overlap=input overlap==LSH {count} done=$A5 DO OD RETURN"
        ));
        for (mode, runtime) in modes_and_runtimes() {
            let compiled = compile_file(&source.0, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
            for high in 0u32..=255 {
                for low in [0, 255] {
                    let input = (high << 8) | low;
                    let actual = run(compiled.object_bytes(), runtime, input, 0);
                    let mut expected = vec![0xCC; 256];
                    for (offset, value) in [(0, (input << count) as u16), (2, (input >> count) as u16), (4, (input << count) as u16)] {
                        expected[offset..offset+2].copy_from_slice(&value.to_le_bytes());
                    }
                    expected[6] = 2;
                    expected[0xE0..0xE4].copy_from_slice(&input.to_le_bytes());
                    expected[0xE4..0xE8].fill(0);
                    expected[255] = 0xA5;
                    assert_eq!(actual, expected, "{mode:?}/{runtime:?}: {input:04X}, count={count}");
                }
            }
        }
    }
}

#[test]
fn wide_carry_chains_preserve_high_only_results_negation_and_captures() {
    let source = Source::new(
        "LONGCARD a=$6E0,b=$6E4,sum=$600,difference=$604,inplace=$610\n\
         CARD upper=$608,borrow=$60A,narrow=$60E LONGINT negative=$614\n\
         BYTE top=$60C,calls=$60D,done=$6FF\n\
         LONGCARD FUNC Left() calls==+1 RETURN(a)\n\
         LONGCARD FUNC Right() calls==+1 RETURN(b)\n\
         PROC Main() calls=0 sum=Left()+Right() difference=a-b\n\
         upper=CARD((a+b) RSH 16) borrow=CARD((a-b) RSH 16) top=BYTE((a+b) RSH 24)\n\
         narrow=CARD(a+b) inplace=a inplace==+b inplace==-a negative=-LONGINT(a)\n\
         done=$A5 DO OD RETURN",
    );
    let boundaries = [0u32, 1, 0x7F, 0x80, 0xFF, 0x100, 0xFFFE, 0xFFFF, 0x10000, 0xFFFFFF, 0x7FFFFFFF, 0x80000000, u32::MAX];
    let mut pairs: Vec<_> = boundaries.into_iter().flat_map(|a| boundaries.into_iter().map(move |b| (a,b))).collect();
    let mut seed = 0x1937AB21u32;
    for _ in 0..256 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = seed;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.push((a, seed));
    }
    for (mode, runtime) in modes_and_runtimes() {
        // Compatibility requires explicit staging for calls in arithmetic.
        let staged = (mode == CompileMode::Compatibility).then(|| Source::new(
            &std::fs::read_to_string(&source.0).unwrap().replace(
                "sum=Left()+Right()", "sum=Left() difference=Right() sum=sum+difference",
            ),
        ));
        let source = staged.as_ref().unwrap_or(&source);
        let compiled = compile_file(&source.0, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        for &(a, b) in &pairs {
            let actual = run(compiled.object_bytes(), runtime, a, b);
            let sum = a.wrapping_add(b);
            let difference = a.wrapping_sub(b);
            let mut expected = vec![0xCC; 256];
            for (offset, value) in [(0, sum), (4, difference), (16, b), (20, a.wrapping_neg()), (0xE0, a), (0xE4, b)] {
                expected[offset..offset+4].copy_from_slice(&value.to_le_bytes());
            }
            for (offset, value) in [(8, (sum >> 16) as u16), (10, (difference >> 16) as u16), (14, sum as u16)] {
                expected[offset..offset+2].copy_from_slice(&value.to_le_bytes());
            }
            expected[12] = (sum >> 24) as u8;
            expected[13] = 2;
            expected[255] = 0xA5;
            assert_eq!(actual, expected, "{mode:?}/{runtime:?}: {a:08X}, {b:08X}");
        }
    }
}
