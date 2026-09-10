use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{ImageKind, OS_ROM_BASE, RunRequest, StopReason, VmRunner};
use std::path::Path;
#[path = "support/fixed_point.rs"]
mod support;
use support::{Source, check, execute, lanes, put, vm_for};

const BOUNDARIES: &[i16] = &[
    -32768, -32767, -1024, -513, -512, -257, -256, -129, -128, -1, 0, 1, 127, 128, 255, 256, 257,
    512, 1024, 32766, 32767,
];

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
