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
    fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-q8-vm-{}-{}.act",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, text).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const BOUNDARIES: &[i16] = &[
    -32768, -32767, -1024, -513, -512, -257, -256, -129, -128, -1, 0, 1, 127, 128, 255, 256, 257,
    512, 1024, 32766, 32767,
];

fn lanes() -> impl Iterator<Item = (CompileMode, Runtime)> {
    [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ]
    .into_iter()
    .flat_map(|mode| {
        [Runtime::ActionCart, Runtime::Standalone]
            .into_iter()
            .map(move |runtime| (mode, runtime))
    })
}

fn vm_for(runtime: Runtime) -> (CompilerVm, ExecutionProfile) {
    let mut vm = CompilerVm::default();
    let profile = if runtime == Runtime::ActionCart {
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
    } else {
        // Numeric standalone calls must work without either ROM.
        ExecutionProfile::StandaloneObject
    };
    (vm, profile)
}

fn put(page: &mut [u8], offset: usize, value: i64) {
    // Explicit modulo-65536 conversion, independent of compiler evaluators.
    let bits = value.rem_euclid(65536) as u16;
    page[offset..offset + 2].copy_from_slice(&bits.to_le_bytes());
}

fn check(
    image: &[u8],
    mode: CompileMode,
    runtime: Runtime,
    a: i16,
    b: i16,
    expected: impl FnOnce(&mut [u8]),
) {
    execute(image, mode, runtime, (a, b), false, expected);
}

fn execute(
    image: &[u8],
    mode: CompileMode,
    runtime: Runtime,
    (a, b): (i16, i16),
    fault: bool,
    expected: impl FnOnce(&mut [u8]),
) {
    let (mut vm, profile) = vm_for(runtime);
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x7FF)
    );
    let mut page = vec![0xCC; 0x120];
    put(&mut page, 0xE0, i64::from(a));
    put(&mut page, 0xE2, i64::from(b));
    for (offset, &value) in page.iter().enumerate() {
        vm.bus_mut().ram_mut().write(0x600 + offset as u16, value);
    }
    if fault {
        // Capture Error's registers/count, set decimal mode, and deliberately
        // return. The arithmetic helper must clear D and never resume its caller.
        vm.bus_mut()
            .ram_mut()
            .map(
                0x780,
                &[
                    0x8D, 0x20, 7, 0x8E, 0x21, 7, 0x8C, 0x22, 7, 0xEE, 0x23, 7, 0xF8, 0x60,
                ],
            )
            .unwrap();
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
    expected(&mut page);
    if !fault {
        page[0xDF] = 0xA5;
    }
    let max_steps = 60_000;
    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps,
        history_len: 8,
        ..Default::default()
    });
    assert_eq!(
        outcome.stop_reason(),
        StopReason::StepLimit { max_steps },
        "{mode:?}/{runtime:?} ({a},{b}): {:?}",
        outcome.report
    );
    let actual: Vec<_> = (0x600..=0x71F)
        .map(|address| outcome.memory().read(address))
        .collect();
    assert_eq!(
        actual, page,
        "{mode:?}/{runtime:?} ({a},{b}): {:?}",
        outcome.report
    );
    if fault {
        assert_eq!(
            (0x720..=0x723)
                .map(|a| outcome.memory().read(a))
                .collect::<Vec<_>>(),
            [101, 0, 101, 1],
            "{mode:?}/{runtime:?} Error register/count capture"
        );
        assert_eq!(
            outcome.report.registers.status & 8,
            0,
            "decimal mode after returning Error"
        );
        let pc = outcome.report.registers.pc;
        assert_eq!(
            [outcome.memory().read(pc), outcome.memory().read(pc + 1)],
            [0xB0, 0xFE],
            "fault must stop at the non-returning guard"
        );
    }
}

fn numeric_expected(page: &mut [u8], a: i16, b: i16) {
    let (a, b) = (i64::from(a), i64::from(b));
    put(page, 1, a * 256);
    put(page, 3, a / 256);
    put(page, 5, a * b / 256);
    if b != 0 {
        put(page, 7, a * 256 / b);
        put(page, 9, a * 256 / b);
    }
    for (index, raw) in [256, 128, 1, -32768, 32767].into_iter().enumerate() {
        put(page, 0x11 + index * 2, raw);
    }
}

#[test]
fn fixed_q8_8_conversions_and_constants_match_host_oracle() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/runtime/fixed_q8_8.act");
    let mut values = BOUNDARIES.to_vec();
    values.extend([-384, -127, 129, 384]);
    for (mode, runtime) in lanes() {
        let compiled = compile_file(
            &source,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
        )
        .unwrap();
        for &value in &values {
            check(compiled.object_bytes(), mode, runtime, value, 0, |page| {
                numeric_expected(page, value, 0);
            });
        }
    }
}

#[test]
fn fixed_q8_8_binary_arithmetic_matches_host_oracle() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/runtime/fixed_q8_8.act");
    let mut pairs: Vec<_> = BOUNDARIES
        .iter()
        .flat_map(|&a| BOUNDARIES.iter().map(move |&b| (a, b)))
        .collect();
    // Deterministic full-width inputs: Numerical Recipes LCG, seed 0x51383821.
    let mut seed = 0x51383821u32;
    for _ in 0..128 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = (seed >> 16) as i16;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.push((a, (seed >> 16) as i16));
    }
    pairs.extend([
        (3, 2),
        (1, 256),
        (384, 512),
        (-1, 1),
        (-257, 128),
        (32767, 512),
        (-256, 768),
        (-32768, -256),
    ]);
    assert_eq!(pairs.len(), 577);
    for (mode, runtime) in lanes() {
        let compiled = compile_file(
            &source,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
        )
        .unwrap();
        for &(a, b) in &pairs {
            check(compiled.object_bytes(), mode, runtime, a, b, |page| {
                numeric_expected(page, a, b)
            });
        }
    }
}

#[test]
fn fixed_q8_8_zero_division_faults_without_stores_or_resuming() {
    for function in ["Div", "FromRatio"] {
        for divisor in ["b", "0"] {
            let source = Source::new(&format!(
                "MODULE FAULT USE MATH.Q8_8 AS Q\nINT a=$6E0,b=$6E2,result=$601 BYTE state=$620,done=$6DF\n\
                 PROC Main() state=41 result=Q.{function}(a,{divisor}) state=42 done=$A5 RETURN ENDMODULE"
            ));
            for (mode, runtime) in lanes() {
                let compiled = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap();
                for a in [0, -32768, 256] {
                    execute(
                        compiled.object_bytes(),
                        mode,
                        runtime,
                        (a, 0),
                        true,
                        |page| page[0x20] = 41,
                    );
                }
            }
        }
    }
}

#[test]
fn fixed_q8_8_composition_preserves_calls_and_guarded_stores() {
    const SOURCE: &str = include_str!("../../../fixtures/runtime/fixed_q8_8_composition.act");
    const STAGED: &str = "  leftValue=Left()\n  rightValue=Right()\n  table(0)=Q.Mul(leftValue,rightValue)\n  leftValue=Left()\n  halfValue=Q.Mul(leftValue,Q.Half)\n  rightValue=Right()\n  destination(1)=Q.Div(halfValue,rightValue)";
    const NESTED: &str =
        "  table(0)=Q.Mul(Left(),Right())\n  destination(1)=Q.Div(Q.Mul(Left(),Q.Half),Right())";
    assert_eq!(SOURCE.matches(STAGED).count(), 1);
    let inputs = [
        (384, 512),
        (-384, 768),
        (-1, 1),
        (1, -1),
        (-32768, -256),
        (32767, 512),
        (-32768, -32768),
        (32767, 32767),
        (0, 256),
        (257, -129),
        (-257, 128),
        (16384, 3),
    ];
    for nested in [false, true] {
        let text = if nested {
            SOURCE.replace(STAGED, NESTED)
        } else {
            SOURCE.to_string()
        };
        let source = Source::new(&text);
        for (mode, runtime) in
            lanes().filter(|(mode, _)| !nested || *mode != CompileMode::Compatibility)
        {
            let compiled = compile_file(
                &source.0,
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|e| panic!("nested={nested} {mode:?}/{runtime:?}: {e}"));
            for (a, b) in inputs {
                check(compiled.object_bytes(), mode, runtime, a, b, |page| {
                    let (a, b) = (i64::from(a), i64::from(b));
                    let signed = |v: i64| (v + 32768).rem_euclid(65536) - 32768;
                    let product = signed(a * b / 256);
                    let half = signed(a / 2);
                    put(page, 0xFD, product);
                    put(page, 0xFF, half * 256 / b);
                    put(page, 1, a * 256 / b);
                    put(page, 3, product);
                    let velocity = signed(b * 256 / 3);
                    let step = signed(velocity / 2) / 2;
                    let position = signed(a + 3 * step);
                    put(page, 5, position);
                    put(page, 7, position / 256);
                    put(page, 9, -a);
                    put(page, 11, a - b);
                    put(page, 13, a + b);
                    page[15] = u8::from(a < b);
                    page[16] = u8::from(a == b);
                    page[0x30] = 4;
                    page[0x31] = 50; // Call order: Left, Right, Left, Right, in base 3.
                });
            }
        }
    }
}

#[test]
fn fixed_q8_8_sample_prints_documented_results() {
    const SAMPLE: &str = include_str!("../../../samples/fixed-point/q8_8.act");
    const README: &str = include_str!("../../../samples/fixed-point/README.md");
    let expected: Vec<_> = README
        .split_once("```text\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0
        .bytes()
        .map(|b| if b == b'\n' { 0x9B } else { b })
        .collect();
    // Preserve the sample's calculations and I/O, then stop before returning
    // to a host shell. The build catalog separately compiles the original file.
    let end = SAMPLE.rfind("RETURN").unwrap();
    let source = Source::new(&format!("{} DO OD\n{}", &SAMPLE[..end], &SAMPLE[end..]));
    for (mode, runtime) in lanes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
        )
        .unwrap();
        let (mut vm, profile) = vm_for(runtime);
        if runtime == Runtime::Standalone {
            vm.load_image_bytes(
                ImageKind::Rom,
                "altirraos-xl.rom",
                OS_ROM_BASE,
                std::fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roms/altirraos-xl.rom"),
                )
                .unwrap(),
            )
            .unwrap();
        }
        vm.load_atari_object_for_execution(profile, compiled.object_bytes())
            .unwrap();
        let max_steps = 500_000;
        let outcome = VmRunner::new(vm).run(RunRequest {
            max_steps,
            history_len: 8,
            ..Default::default()
        });
        assert_eq!(outcome.stop_reason(), StopReason::StepLimit { max_steps });
        assert_eq!(
            outcome.vm.bus().cio_channel0_output(),
            expected,
            "sample {mode:?}/{runtime:?}: {:?}",
            outcome.report
        );
    }
}
