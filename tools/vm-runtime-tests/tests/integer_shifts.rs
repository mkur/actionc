use actionc::compiler::{CompileMode, CompileOptions, CompilerPhase, Runtime, compile_file};
use actionc::includes::{ModuleLoadOptions, load_compilation};
use actionc_vm::{CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const BASE: u16 = 0x0600;
const BYTES: usize = 0x0A00;
const VALUES: usize = 0x07FF - BASE as usize;
const COUNTS: usize = 0x09FF - BASE as usize;
const OUT_I: usize = 0x0BFF - BASE as usize;
const OUT_LI: usize = 0x0DFF - BASE as usize;
const MODULE: &str = include_str!("../../../embedded/modules/math/integer.act");
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Source(PathBuf);
impl Source {
    fn create(text: &str, runtime: Runtime, module_copy: bool) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-integer-shifts-vm-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let mut lf = text.replace("\r\n", "\n");
        if module_copy {
            // Reserved embedded modules cannot be overridden on disk. Rename
            // only the module/import to compile the same body from a CRLF file.
            assert_eq!(lf.matches("USE MATH.INTEGER AS BITS").count(), 1);
            lf = lf.replace("USE MATH.INTEGER AS BITS", "USE SHIFT_COPY AS BITS");
            let module = MODULE.replace("\r\n", "\n");
            assert_eq!(module.matches("MODULE MATH.INTEGER").count(), 1);
            std::fs::write(
                path.join("shift_copy.act"),
                module
                    .replace("MODULE MATH.INTEGER", "MODULE SHIFT_COPY")
                    .replace('\n', "\r\n"),
            )
            .unwrap();
        }
        std::fs::write(
            path.join("main.act"),
            if runtime == Runtime::Standalone || module_copy {
                lf.replace('\n', "\r\n")
            } else {
                lf
            },
        )
        .unwrap();
        if module_copy {
            let loaded =
                load_compilation(&path.join("main.act"), &ModuleLoadOptions::default()).unwrap();
            assert!(
                !loaded
                    .modules
                    .iter()
                    .any(|m| m.origin.to_string() == "<embedded:MATH.INTEGER>")
            );
            assert!(loaded.modules.iter().any(|m| {
                m.origin
                    .host_path()
                    .is_some_and(|p| p.file_name().is_some_and(|name| name == "shift_copy.act"))
            }));
        }
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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
            .map(move |r| (mode, r))
    })
}

fn counts() -> Vec<u8> {
    (0..=33).chain([127, 128, 255]).collect()
}

fn boundaries() -> Vec<i32> {
    vec![
        i32::MIN,
        i32::MIN + 1,
        -65537,
        -65536,
        -32769,
        -32768,
        -32767,
        -257,
        -256,
        -255,
        -129,
        -128,
        -127,
        -3,
        -2,
        -1,
        0,
        1,
        2,
        3,
        127,
        128,
        129,
        255,
        256,
        257,
        32767,
        32768,
        32769,
        65535,
        65536,
        65537,
        0x40000000,
        i32::MAX - 1,
        i32::MAX,
    ]
}

fn expected_i(value: i32, count: u8) -> [u8; 2] {
    // Mathematical floor division, independent of the Action! complement code.
    ((value as i16 as i64).div_euclid(1i64 << count.min(15)) as i16).to_le_bytes()
}

fn expected_li(value: i32, count: u8) -> [u8; 4] {
    ((value as i64).div_euclid(1i64 << count.min(31)) as i32).to_le_bytes()
}

fn execute(image: &[u8], runtime: Runtime, initial: &[u8], expected: &[u8], label: &str) {
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
        ExecutionProfile::StandaloneObject
    };
    let loaded = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        loaded
            .segments
            .iter()
            .all(|s| s.end < BASE || s.start >= 0x1000)
    );
    vm.bus_mut().ram_mut().map(BASE, initial).unwrap();
    let limit = 1_000_000;
    let mut steps = 0;
    while vm.bus().ram().read(0x06FF) != 0xA5 && steps < limit {
        vm.step_cpu()
            .unwrap_or_else(|error| panic!("{label}: {error:?}"));
        steps += 1;
    }
    assert_eq!(
        vm.bus().ram().read(0x06FF),
        0xA5,
        "{label}: step limit; PC=${:04X}",
        vm.cpu().registers().pc
    );
    for (offset, &value) in expected.iter().enumerate() {
        let address = BASE + offset as u16;
        assert_eq!(
            vm.bus().ram().read(address),
            value,
            "{label}: ${address:04X}"
        );
    }
}

fn call_source(
    nested: &str,
    staged: &str,
    mode: CompileMode,
    runtime: Runtime,
    module_copy: bool,
) -> Source {
    let source = Source::create(nested, runtime, module_copy);
    if mode != CompileMode::Compatibility {
        return source;
    }
    let error = compile_file(
        source.0.join("main.act"),
        &CompileOptions::for_mode(mode)
            .with_runtime(runtime)
            .with_origin(0x2000),
    )
    .expect_err("compat must reject nested routine-call arguments");
    assert_eq!(error.diagnostics().len(), 2, "{runtime:?}: {error}");
    assert!(
        error
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.phase == CompilerPhase::Codegen
                && diagnostic.message
                    == "compat profile rejects function calls as routine call arguments"),
        "{runtime:?}: {error}"
    );
    // Keep the same arithmetic, call counts and memory oracle in compat by
    // spelling out its required argument temporaries in the source.
    Source::create(staged, runtime, module_copy)
}

fn check_dynamic(module_copy: bool) {
    let text = |calls: &str| {
        format!(
            "MODULE TEST USE MATH.INTEGER AS BITS\n\
        BYTE size=$0600,done=$06FF,current\nCARD calls=$0601\n\
        LONGINT ARRAY values(64)=$07FF\nBYTE ARRAY counts(64)=$09FF\n\
        INT ARRAY outI(64)=$0BFF\nLONGINT ARRAY outLI(64)=$0DFF\n\
        LONGINT FUNC ReadValue() calls==+1 RETURN(values(current))\n\
        BYTE FUNC ReadCount() calls==+1 RETURN(counts(current))\n\
        PROC Main() LONGINT stagedValue BYTE stagedCount\n calls=0\n\
          FOR current=0 TO size-1 DO\n\
            {calls}\n\
          OD done=$A5 RETURN ENDMODULE\n"
        )
    };
    let nested = text(
        "outI(current)=BITS.AsrI(INT(ReadValue()),ReadCount())\n\
         outLI(current)=BITS.AsrLI(ReadValue(),ReadCount())",
    );
    let staged = text(
        "stagedValue=ReadValue() stagedCount=ReadCount()\n\
         outI(current)=BITS.AsrI(INT(stagedValue),stagedCount)\n\
         stagedValue=ReadValue() stagedCount=ReadCount()\n\
         outLI(current)=BITS.AsrLI(stagedValue,stagedCount)",
    );
    let mut cases: Vec<_> = boundaries()
        .into_iter()
        .flat_map(|v| counts().into_iter().map(move |c| (v, c)))
        .collect();
    // Cover every BYTE count for both signs without crossing all values with
    // the oversized counts, which have the same sign-fill behavior.
    cases.extend((0..=u8::MAX).flat_map(|count| [(-3, count), (3, count)]));
    let mut seed = 0xA51A51u32;
    for _ in 0..128 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let value = seed as i32;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        cases.push((value, (seed % 34) as u8));
        cases.push((value, seed as u8));
    }
    if module_copy {
        cases = vec![
            (i32::MIN, 1),
            (-3, 1),
            (i32::MAX, 0),
            (i32::MAX, 32),
            (-3, 128),
            (3, u8::MAX),
        ];
    }
    for (mode, runtime) in lanes() {
        let source = call_source(&nested, &staged, mode, runtime, module_copy);
        let compiled = compile_file(
            source.0.join("main.act"),
            &CompileOptions::for_mode(mode)
                .with_runtime(runtime)
                .with_origin(0x2000),
        )
        .unwrap();
        for (batch, cases) in cases.chunks(64).enumerate() {
            let mut initial = vec![0xCC; BYTES];
            initial[0] = cases.len() as u8;
            for (i, &(value, count)) in cases.iter().enumerate() {
                initial[VALUES + i * 4..VALUES + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
                initial[COUNTS + i] = count;
            }
            let mut expected = initial.clone();
            expected[1..3].copy_from_slice(&(cases.len() as u16 * 4).to_le_bytes());
            expected[0xFF] = 0xA5;
            for (i, &(value, count)) in cases.iter().enumerate() {
                expected[OUT_I + i * 2..OUT_I + i * 2 + 2]
                    .copy_from_slice(&expected_i(value, count));
                expected[OUT_LI + i * 4..OUT_LI + i * 4 + 4]
                    .copy_from_slice(&expected_li(value, count));
            }
            execute(
                compiled.object_bytes(),
                runtime,
                &initial,
                &expected,
                &format!("dynamic {mode:?}/{runtime:?}/batch {batch}"),
            );
        }
    }
}

#[test]
fn integer_shifts_dynamic_counts_and_argument_calls_match_floor_division() {
    check_dynamic(false);
}

#[test]
fn integer_shifts_crlf_module_body_compiles_and_executes() {
    check_dynamic(true);
}

#[test]
fn integer_shifts_constant_counts_and_nested_calls_match_floor_division() {
    let counts = counts();
    assert!(counts.len() < 64, "reserve an output slot for nested calls");
    let mut text = "MODULE TEST USE MATH.INTEGER AS BITS\n\
        LONGINT value=$07FF BYTE done=$06FF\n\
        INT ARRAY outI(64)=$0BFF LONGINT ARRAY outLI(64)=$0DFF\n\
        PROC Main() INT stagedI LONGINT stagedLI\n"
        .to_owned();
    for (i, count) in counts.iter().enumerate() {
        text.push_str(&format!(
            "outI({i})=BITS.AsrI(INT(value),${count:X}) outLI({i})=BITS.AsrLI(value,${count:X})\n"
        ));
    }
    let composed = counts.len();
    let nested = format!(
        "{text}outI({composed})=BITS.AsrI(BITS.AsrI(INT(value),1),2)\n\
        outLI({composed})=BITS.AsrLI(BITS.AsrLI(value,1),2)\ndone=$A5 RETURN ENDMODULE\n"
    );
    let staged = format!(
        "{text}stagedI=BITS.AsrI(INT(value),1) outI({composed})=BITS.AsrI(stagedI,2)\n\
        stagedLI=BITS.AsrLI(value,1) outLI({composed})=BITS.AsrLI(stagedLI,2)\n\
        done=$A5 RETURN ENDMODULE\n"
    );
    for (mode, runtime) in lanes() {
        let source = call_source(&nested, &staged, mode, runtime, false);
        let compiled = compile_file(
            source.0.join("main.act"),
            &CompileOptions::for_mode(mode)
                .with_runtime(runtime)
                .with_origin(0x2000),
        )
        .unwrap();
        for value in boundaries() {
            let mut initial = vec![0xCC; BYTES];
            initial[VALUES..VALUES + 4].copy_from_slice(&value.to_le_bytes());
            let mut expected = initial.clone();
            expected[0xFF] = 0xA5;
            for (i, count) in counts.iter().copied().chain([3]).enumerate() {
                expected[OUT_I + i * 2..OUT_I + i * 2 + 2]
                    .copy_from_slice(&expected_i(value, count));
                expected[OUT_LI + i * 4..OUT_LI + i * 4 + 4]
                    .copy_from_slice(&expected_li(value, count));
            }
            execute(
                compiled.object_bytes(),
                runtime,
                &initial,
                &expected,
                &format!("constant {mode:?}/{runtime:?}/{value}"),
            );
        }
    }
}
