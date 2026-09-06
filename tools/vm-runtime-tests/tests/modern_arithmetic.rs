use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest, VmRunner,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Source(PathBuf);
impl Source {
    fn new(source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-arithmetic-{}-{}.act",
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

fn run(image: &[u8], runtime: Runtime, a: u16, b: u16) -> Vec<u8> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = CompilerVm::default();
    let profile = match runtime {
        Runtime::Standalone => ExecutionProfile::StandaloneObject,
        Runtime::ActionCart => {
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
            .all(|s| s.end < 0x600 || s.start > 0x6FF)
    );
    for address in 0x600..=0x6FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    for (address, value) in [(0x6E0, a), (0x6E2, b)] {
        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
            vm.bus_mut().ram_mut().write(address + i as u16, byte);
        }
    }
    let result = VmRunner::new(vm).run(RunRequest {
        max_steps: 15_000,
        history_len: 4,
        ..Default::default()
    });
    if result.memory().read(0x6FF) == 0xCC {
        assert_eq!(
            result.report.registers.a, 1,
            "noncompletion must be the arithmetic fault"
        );
        let pc = result.report.registers.pc;
        assert_eq!(
            (
                result.memory().read(pc),
                result.memory().read(pc.wrapping_add(1))
            ),
            (0xB0, 0xFE)
        );
    }
    (0x600..=0x6FF)
        .map(|address| result.memory().read(address))
        .collect()
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
fn byte_type(ty: &str) -> bool {
    matches!(ty, "BYTE" | "CHAR")
}

#[test]
fn typed_literals_constants_and_static_images_share_division_semantics() {
    let source = Source::new(
        "CONST INT sq=INT(-513)/INT(256) \
         CONST INT sr=INT(-513) MOD INT(256) \
         CONST INT nq=INT(7)/INT(-3) \
         CONST INT nr=INT(7) MOD INT(-3) \
         CONST CARD uq=CARD(65535)/CARD(2) \
         CONST CARD ur=CARD(65535) MOD CARD(2) \
         CONST INT oq=INT($8000)/INT($FFFF) \
         CONST INT ore=INT($8000) MOD INT($FFFF) \
         INT ARRAY initial(8)=[sq sr nq nr uq ur oq ore] \
         CARD ARRAY output(16)=$600 BYTE done=$6FF \
         PROC Main() BYTE i \
           FOR i=0 TO 7 DO output(i)=initial(i) OD \
           output(8)=INT(-513)/INT(256) output(9)=INT(-513) MOD INT(256) \
           output(10)=INT(7)/INT(-3) output(11)=INT(7) MOD INT(-3) \
           output(12)=CARD(65535)/CARD(2) output(13)=CARD(65535) MOD CARD(2) \
           output(14)=INT($8000)/INT($FFFF) output(15)=INT($8000) MOD INT($FFFF) \
           done=$A5 RETURN",
    );
    let expected = [65534, 65535, 65534, 1, 32767, 1, 32768, 0];
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                &source.0,
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap();
            let bytes = run(compiled.object_bytes(), runtime, 0, 1);
            for index in 0..16 {
                assert_eq!(
                    word(&bytes, index * 2),
                    expected[index % 8],
                    "{mode:?}/{runtime:?}/{index}"
                );
            }
            assert!(bytes[32..0xE0].iter().all(|byte| *byte == 0xCC));
            assert_eq!(bytes[255], 0xA5);
        }
    }
}

#[test]
fn separate_divmod_keeps_fresh_source_captures() {
    for ty in ["INT", "CARD"] {
        let source = Source::new(&format!(
            "{ty} a=$6E0,b=$6E2 CARD qout=$600,rout=$602 BYTE done=$6FF \
             PROC Pair({ty} x,y) {ty} q,r q=x/y r=x MOD y qout=q rout=r RETURN \
             PROC Main() Pair(a,b) done=$A5 RETURN"
        ));
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                &source.0,
                &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
            )
            .unwrap();
            // Source parameters are reloaded around the intervening local
            // store. Adjacent captured-MIR fusion must not equate those loads.
            assert!(!compiled.source_listing().contains("::DivMod"));
            for (a, b) in [
                (65535, 2),
                (32768, 65535),
                (65529, 3),
                (7, 65533),
                (65529, 65533),
                (50000, 40000),
                (513, 256),
                (1, 0),
            ] {
                let bytes = run(compiled.object_bytes(), runtime, a, b);
                if b == 0 {
                    assert_eq!(bytes[255], 0xCC);
                    continue;
                }
                let (a, b) = if ty == "INT" {
                    (i64::from(a as i16), i64::from(b as i16))
                } else {
                    (i64::from(a), i64::from(b))
                };
                let q = a.abs() / b.abs() * if (a < 0) != (b < 0) { -1 } else { 1 };
                let r = a - q * b;
                assert_eq!(
                    (word(&bytes, 0), word(&bytes, 2)),
                    (q as u16, r as u16),
                    "{ty}/{runtime:?}/{a}/{b}"
                );
                assert_eq!(bytes[255], 0xA5);
            }
        }
    }
}

#[test]
fn narrow_helpers_keep_asymmetric_inputs_and_wide_divisors() {
    for (left_ty, right_ty, expected) in [
        ("BYTE", "BYTE", "DivU8"),
        ("CARD", "BYTE", "DivU16U8"),
        ("BYTE", "CARD", "DivU16"),
    ] {
        let source = Source::new(&format!(
            "{left_ty} a=$6E0 {right_ty} b=$6E2 CARD q=$600,r=$602 BYTE done=$6FF \
             PROC Main() q=a/b r=a MOD b done=$A5 RETURN"
        ));
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                &source.0,
                &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
            )
            .unwrap();
            assert!(
                compiled.source_listing().contains(expected),
                "{left_ty}/{right_ty}/{runtime:?}: expected {expected}"
            );
            for (a, b) in [(65535u16, 2u16), (255, 256), (513, 7), (32768, 255), (1, 0)] {
                let bytes = run(compiled.object_bytes(), runtime, a, b);
                let a = if left_ty == "BYTE" { a & 255 } else { a };
                let b = if right_ty == "BYTE" { b & 255 } else { b };
                if b == 0 {
                    assert_eq!(bytes[255], 0xCC);
                    continue;
                }
                assert_eq!((word(&bytes, 0), word(&bytes, 2)), (a / b, a % b));
            }
        }
    }
}

#[test]
fn modern_divmod_all_input_types_profiles_and_link_modes() {
    let cases = [
        (0, 1),
        (1, 0),
        (65535, 1),
        (65535, 2),
        (32768, 2),
        (32768, 65535),
        (65535, 32768),
        (65023, 256),
        (65023, 65280),
        (513, 65280),
        (65529, 3),
        (7, 65533),
        (65529, 65533),
        (32767, 256),
        (65535, 65535),
        (256, 0),
        (1, 256),
        (255, 2),
        (0, 0),
        (32768, 0),
        (100, 7),
        (128, 129),
        (255, 255),
        (50000, 40000),
        (513, 3),
        (1000, 7),
        (32767, 3),
        (256, 3),
    ];
    for lt in ["BYTE", "CHAR", "INT", "CARD"] {
        for rt in ["BYTE", "CHAR", "INT", "CARD"] {
            let source = Source::new(&format!(
                "{lt} a=$06E0,cq=$0604,cr=$0606 {rt} b=$06E2 \
                 CARD q=$0600,r=$0602 BYTE done=$06FF \
                 PROC Main() q=a/b r=a MOD b cq=a cq==/b cr=a cr==MOD b done=$A5 RETURN"
            ));
            for mode in [
                CompileMode::Compatibility,
                CompileMode::Optimized,
                CompileMode::Mir6502,
            ] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let compiled = compile_file(
                        &source.0,
                        &CompileOptions::for_mode(mode).with_runtime(runtime),
                    )
                    .unwrap_or_else(|error| panic!("{lt}/{rt}/{mode:?}/{runtime:?}: {error}"));
                    for (a, b) in cases {
                        let label = format!("{lt}/{rt}/{mode:?}/{runtime:?}/{a}/{b}");
                        let bytes = run(compiled.object_bytes(), runtime, a, b);
                        assert_eq!(word(&bytes, 0xE0), a, "{label}: input a");
                        assert_eq!(word(&bytes, 0xE2), b, "{label}: input b");
                        assert!(
                            bytes[8..0xE0]
                                .iter()
                                .chain(bytes[0xE4..0xFF].iter())
                                .all(|v| *v == 0xCC),
                            "{label}: guards"
                        );
                        let left = if byte_type(lt) { a & 255 } else { a };
                        let right = if byte_type(rt) { b & 255 } else { b };
                        if right == 0 {
                            assert_eq!(bytes[255], 0xCC, "{label}: fault must not return");
                            assert!(
                                bytes[..8].iter().all(|v| *v == 0xCC),
                                "{label}: fault wrote a result"
                            );
                            continue;
                        }
                        let signed = lt != "CARD" && rt != "CARD" && (lt == "INT" || rt == "INT");
                        let (left, right) = if signed {
                            (i64::from(left as i16), i64::from(right as i16))
                        } else {
                            (i64::from(left), i64::from(right))
                        };
                        // Independent magnitude/sign oracle, including wrapped INT_MIN/-1.
                        let magnitude = left.abs() / right.abs();
                        let quotient = if (left < 0) != (right < 0) {
                            -magnitude
                        } else {
                            magnitude
                        };
                        let remainder = left - quotient * right;
                        let mask = if byte_type(lt) { 255 } else { 65535 };
                        let actual = [
                            word(&bytes, 0),
                            word(&bytes, 2),
                            word(&bytes, 4) & mask,
                            word(&bytes, 6) & mask,
                        ];
                        assert_eq!(
                            actual,
                            [
                                quotient as u16,
                                remainder as u16,
                                quotient as u16 & mask,
                                remainder as u16 & mask
                            ],
                            "{label}"
                        );
                        if byte_type(lt) {
                            assert_eq!(
                                (bytes[5], bytes[7]),
                                (0xCC, 0xCC),
                                "{label}: narrow guards"
                            );
                        }
                        assert_eq!(bytes[255], 0xA5, "{label}: completion");
                    }
                }
            }
        }
    }
}

#[test]
fn modern_multiply_composition_constant_and_runtime_agree() {
    for constant in [false, true] {
        let operands = if constant {
            "BYTE(255)*BYTE(255)"
        } else {
            "a*b"
        };
        let source = Source::new(&format!(
            "BYTE a=$6E0,b=$6E2,done=$6FF INT q=$600,r=$602 CARD uq=$604,ur=$606 \
             PROC Main() q=({operands})/2 r=({operands}) MOD 2 \
             uq=CARD({operands})/CARD(2) ur=CARD({operands}) MOD CARD(2) done=$A5 RETURN"
        ));
        for mode in [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let compiled = compile_file(
                    &source.0,
                    &CompileOptions::for_mode(mode).with_runtime(runtime),
                )
                .unwrap();
                let bytes = run(compiled.object_bytes(), runtime, 255, 255);
                assert_eq!(
                    [
                        word(&bytes, 0),
                        word(&bytes, 2),
                        word(&bytes, 4),
                        word(&bytes, 6)
                    ],
                    [(-255i16) as u16, 65535, 32512, 1],
                    "{mode:?}/{runtime:?}/constant={constant}"
                );
                assert_eq!(bytes[255], 0xA5);
            }
        }
    }
}

#[test]
fn modern_fault_survives_dead_result_elimination_but_not_unexecuted_branch() {
    for operation in ["/", "MOD"] {
        for body in [
            format!("q=a {operation} b q=0"),
            format!("IF a THEN q=a {operation} b FI q=0"),
            format!("q=0*(a {operation} b)"),
            format!("q=(a {operation} b)*0"),
        ] {
            let conditional = body.starts_with("IF");
            let source = Source::new(&format!(
                "CARD a=$6E0,b=$6E2,q BYTE done=$6FF PROC Main() {body} done=$A5 RETURN"
            ));
            for mode in [
                CompileMode::Compatibility,
                CompileMode::Optimized,
                CompileMode::Mir6502,
            ] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let compiled = compile_file(
                        &source.0,
                        &CompileOptions::for_mode(mode).with_runtime(runtime),
                    )
                    .unwrap();
                    for a in [0, 1] {
                        let bytes = run(compiled.object_bytes(), runtime, a, 0);
                        assert_eq!(
                            bytes[255],
                            if conditional && a == 0 { 0xA5 } else { 0xCC },
                            "{mode:?}/{runtime:?}/{body}/a={a}"
                        );
                    }
                }
            }
        }
    }
}
