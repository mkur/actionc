use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Source(PathBuf);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn probe(mode: CompileMode, runtime: Runtime, signed: bool, input: bool, callback: bool) -> Vec<u8> {
    let ty = if signed { "LONGINT" } else { "LONGCARD" };
    let suffix = if signed { "LI" } else { "LC" };
    let (declaration, preparation, call) = if input {
        (
            String::new(),
            "SYS.Open(0,\"Q:\",4,0)".to_string(),
            format!("SYS.Input{suffix}()"),
        )
    } else if callback {
        (
            format!("{ty} FUNC POINTER parse(STRING source)"),
            format!("parse=@SYS.Val{suffix}"),
            "parse(text)".to_string(),
        )
    } else {
        (
            String::new(),
            String::new(),
            format!("SYS.Val{suffix}(text)"),
        )
    };
    let source = Source(std::env::temp_dir().join(format!(
        "actionc-long-io-{}-{}.act",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::write(&source.0, format!(
        "MODULE PROBE USE SYS\nBYTE ARRAY text(256)=$800 {ty} result=$600 BYTE state=$604\n{declaration}\nPROC Finished=$7F0()\nPROC Main()\n{preparation}\nstate=41 result={call} state=42 Finished() RETURN\nENDMODULE"
    )).unwrap();
    compile_file(
        &source.0,
        &CompileOptions::for_mode(mode)
            .with_runtime(runtime)
            .with_origin(0x3000),
    )
    .unwrap()
    .object_bytes()
    .to_vec()
}

fn check(image: &[u8], runtime: Runtime, text: &[u8], input: bool, expected: Result<u32, u8>) {
    let mut vm = CompilerVm::default();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    vm.load_image_bytes(
        ImageKind::Rom,
        "altirraos-xl.rom",
        OS_ROM_BASE,
        std::fs::read(root.join("roms/altirraos-xl.rom")).unwrap(),
    )
    .unwrap();
    let profile = if runtime == Runtime::ActionCart {
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            DEFAULT_CART_BASE,
            std::fs::read(root.join("roms/action.rom")).unwrap(),
        )
        .unwrap();
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x8FF)
    );
    for address in 0x600..=0x8FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    if input {
        vm.bus_mut().queue_scripted_cio_input_bytes(text);
    } else {
        assert!(text.len() <= 255);
        vm.bus_mut().ram_mut().write(0x800, text.len() as u8);
        for (offset, byte) in text.iter().enumerate() {
            vm.bus_mut().ram_mut().write(0x801 + offset as u16, *byte);
        }
    }
    // Capture the public Error ABI, then return with decimal mode set.
    vm.bus_mut()
        .ram_mut()
        .map(
            0x780,
            &[
                0x8D, 0x00, 0x07, 0x8E, 0x01, 0x07, 0x8C, 0x02, 0x07, 0xEE, 0x03, 0x07, 0xA9, 17,
                0xA2, 34, 0xA0, 51, 0xF8, 0x60,
            ],
        )
        .unwrap();
    vm.bus_mut().ram_mut().write(0x703, 0);
    if runtime == Runtime::ActionCart {
        vm.bus_mut()
            .ram_mut()
            .map(0x04CB, &[0x4C, 0x80, 0x07])
            .unwrap();
    } else {
        vm.bus_mut().ram_mut().write_word(0x000A, 0x0780);
    }
    let max_steps = 500_000;
    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps,
        stop_after_pc: Some(0x7F0),
        history_len: 8,
    });
    if let Ok(expected) = expected {
        assert_eq!(
            outcome.stop_reason(),
            StopReason::PcReached { pc: 0x7F0 },
            "{runtime:?} {text:?}: {:?}",
            outcome.report
        );
        assert_eq!(
            (0x600..0x604)
                .map(|a| outcome.memory().read(a))
                .collect::<Vec<_>>(),
            expected.to_le_bytes(),
            "{runtime:?} {text:?}"
        );
        assert_eq!(outcome.memory().read(0x604), 42);
        assert_eq!(outcome.memory().read(0x703), 0);
    } else {
        assert_eq!(
            outcome.stop_reason(),
            StopReason::StepLimit { max_steps },
            "{runtime:?} {text:?}: {:?}",
            outcome.report
        );
        assert_eq!(
            (0x700..=0x703)
                .map(|a| outcome.memory().read(a))
                .collect::<Vec<_>>(),
            [expected.unwrap_err(), 0, expected.unwrap_err(), 1],
            "{runtime:?} {text:?}: {:?}",
            outcome.report
        );
        assert_eq!(
            (0x600..0x605)
                .map(|a| outcome.memory().read(a))
                .collect::<Vec<_>>(),
            [0xCC, 0xCC, 0xCC, 0xCC, 41],
            "failed parse must not return/store a value"
        );
        assert_eq!(outcome.report.registers.status & 8, 0);
        assert_eq!(outcome.report.registers.a, expected.unwrap_err());
        assert_eq!(outcome.report.registers.y, expected.unwrap_err());
        let pc = outcome.report.registers.pc;
        assert_eq!(
            [outcome.memory().read(pc), outcome.memory().read(pc + 1)],
            [0xB0, 0xFE]
        );
    }
}

#[test]
fn checked_decimal_parsers_cover_limits_syntax_and_returning_error_handlers() {
    for (mode, runtime) in [CompileMode::Compatibility, CompileMode::Optimized, CompileMode::Mir6502].into_iter().flat_map(|mode| [Runtime::ActionCart, Runtime::Standalone].into_iter().map(move |runtime| (mode, runtime))) {
        for signed in [false, true] {
            let image = probe(mode, runtime, signed, false, false);
            for (text, expected) in [
                ("0", 0),
                ("+0", 0),
                ("  +42  ", 42),
                ("65536", 65536),
                ("00000123", 123),
                ("2147483647", 2147483647),
            ] {
                check(&image, runtime, text.as_bytes(), false, Ok(expected));
            }
            for text in [
                "",
                " ",
                "+",
                "-",
                "1 2",
                "12x",
                "0x10",
                "$FF",
                "1.0",
                "--1",
                "+-1",
                "4294967296x",
                "99999999999999999999x",
            ] {
                check(&image, runtime, text.as_bytes(), false, Err(102));
            }
            check(&image, runtime, b"1\0", false, Err(102));
            for text in ["4294967296", "99999999999999999999"] {
                check(&image, runtime, text.as_bytes(), false, Err(103));
            }
            if signed {
                for (text, expected) in [
                    ("-2147483648", i32::MIN),
                    ("-1", -1),
                    (" -0 ", 0),
                    ("-70000", -70000),
                ] {
                    check(&image, runtime, text.as_bytes(), false, Ok(expected as u32));
                }
                for text in ["2147483648", "-2147483649", "4294967295"] {
                    check(&image, runtime, text.as_bytes(), false, Err(103));
                }
            } else {
                check(&image, runtime, b"4294967295", false, Ok(u32::MAX));
                for text in ["-1", "-0", "-4294967295"] {
                    check(&image, runtime, text.as_bytes(), false, Err(102));
                }
            }
        }
    }
}

#[test]
fn input_rejects_invalid_and_truncated_records_without_returning() {
    for (mode, runtime) in [CompileMode::Compatibility, CompileMode::Optimized, CompileMode::Mir6502].into_iter().flat_map(|mode| [Runtime::ActionCart, Runtime::Standalone].into_iter().map(move |runtime| (mode, runtime))) {
        for signed in [false, true] {
            let image = probe(mode, runtime, signed, true, false);
            for text in [b"\x9B".as_slice(), b"12x\x9B", b"4294967296x\x9B"] {
                check(&image, runtime, text, true, Err(102));
            }
            check(&image, runtime, b"4294967296\x9B", true, Err(103));
            let mut longest = vec![b'0'; 254];
            longest.push(0x9B);
            check(&image, runtime, &longest, true, Ok(0));
            let mut truncated = vec![b'0'; 255];
            truncated.extend_from_slice(b"x\x9B");
            check(&image, runtime, &truncated, true, Err(104));
        }
    }
}

#[test]
fn parser_function_pointers_keep_the_wide_result_abi() {
    for (mode, runtime) in [CompileMode::Compatibility, CompileMode::Optimized, CompileMode::Mir6502].into_iter().flat_map(|mode| [Runtime::ActionCart, Runtime::Standalone].into_iter().map(move |runtime| (mode, runtime))) {
        for signed in [false, true] {
            let image = probe(mode, runtime, signed, false, true);
            let (text, expected) = if signed {
                ("-2147483648", i32::MIN as u32)
            } else {
                ("4294967295", u32::MAX)
            };
            check(&image, runtime, text.as_bytes(), false, Ok(expected));
        }
    }
}
