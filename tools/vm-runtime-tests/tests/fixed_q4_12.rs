use actionc::compiler::{CompileOptions, compile_file};
use std::path::Path;
#[path = "support/fixed_point.rs"]
mod support;
use support::{Source, check, execute, lanes, put};

#[test]
fn fixed_q4_12_arithmetic_rounding_and_wide_squares_match_host_oracle() {
    let boundaries: &[i16] = &[
        -32768, -32767, -8192, -6144, -4097, -4096, -2049, -2048, -1, 0, 1, 7, 8, 2047, 2048, 4095,
        4096, 4097, 6144, 8192, 32766, 32767,
    ];
    let mut pairs: Vec<_> = boundaries
        .iter()
        .flat_map(|&a| boundaries.iter().map(move |&b| (a, b)))
        .collect();
    let mut seed = 0x51441221u32;
    for _ in 0..128 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = (seed >> 16) as i16;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.push((a, (seed >> 16) as i16));
    }
    pairs.extend([
        (3, 2),
        (-1, 1),
        (-4097, 2048),
        (32767, 8192),
        (-32768, -4096),
    ]);
    assert_eq!(pairs.len(), 617);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/runtime/fixed_q4_12.act");
    for (mode, runtime) in lanes() {
        let compiled =
            compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        for &(a, b) in &pairs {
            check(compiled.object_bytes(), mode, runtime, a, b, |page| {
                let (a, b) = (i64::from(a), i64::from(b));
                put(page, 1, a * 4096);
                put(page, 3, a / 4096);
                put(page, 5, a * b / 4096);
                if b != 0 {
                    put(page, 7, a * 4096 / b);
                    put(page, 9, a * 4096 / b);
                }
                put(page, 11, (a * b).div_euclid(4096));
                page[0xFD..0x101].copy_from_slice(&((a * a) as u32).to_le_bytes());
                for (i, value) in [4096, 2048, 1, -32768, 32767].into_iter().enumerate() {
                    put(page, 0x11 + 2 * i, value);
                }
            });
        }
    }
}

#[test]
fn fixed_q4_12_division_faults_cannot_store_or_resume() {
    for function in ["Div", "FromRatio"] {
        for divisor in ["b", "0"] {
            let source = Source::new(&format!(
                "MODULE FAULT USE MATH.Q4_12 AS Q INT a=$6E0,b=$6E2,result=$601 BYTE state=$620,done=$6DF\n\
                PROC Main() state=41 result=Q.{function}(a,{divisor}) state=42 done=$A5 RETURN ENDMODULE"
            ));
            for (mode, runtime) in lanes() {
                let compiled = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap();
                for a in [0, -32768, 4096] {
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
