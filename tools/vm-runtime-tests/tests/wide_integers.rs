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
    assert_eq!(bytes[255], 0xA5, "completion {runtime:?} {a:08x} {b:08x}");
    bytes
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
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
