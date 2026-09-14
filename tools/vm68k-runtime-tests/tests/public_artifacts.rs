mod common;
#[path = "common/compiler.rs"]
mod compiler;
use actionc::{
    compiler::native::artifacts::{Location, NativeArtifact},
    mir68k::image::SymbolView,
    runtime_fault::RuntimeFault,
};
use actionc_vm68k_tests::{Machine, Outcome};
use std::{fs, process::Command};

const SOURCE: &str = r#"MODULE WORKFLOW
PUBLIC LONGINT result
PUBLIC INT signedWord
PUBLIC BYTE marker
PUBLIC LONGCARD ARRAY table(3)=[1 2]
LONGCARD POINTER cursor
LONGINT FUNC Sum(INT seed)
 VOLATILE INT local
 local=seed
RETURN(LONGINT(local)+LONGINT(table(1)))
PROC Entry()
 signedWord=-300
 result=Sum(signedWord)
 marker=$81
 cursor=table
 cursor^=$12345678
 table(2)=$80000000
RETURN
ENDMODULE
"#;

#[test]
fn compiler_bundles_execute_after_source_removal_and_relocation() {
    for (options, crlf) in [
        (&["--origin", "0x10000", "--no-opt"][..], false),
        (&["--origin", "0x23400"][..], true),
        (&["--origin", "0x23400", "--no-codegen-opt"][..], false),
    ] {
        let source = common::Source::new(&SOURCE.replace('\n', if crlf { "\r\n" } else { "\n" }));
        let root = source.0.parent().unwrap();
        let path = root.join("old bundle/image.json");
        compiler::compile(&source.0, &path, options);
        let manifest = fs::read_to_string(&path).unwrap();
        fs::write(
            &path,
            manifest.replace('\n', if crlf { "\r\n" } else { "\n" }),
        )
        .unwrap();
        fs::remove_file(&source.0).unwrap();
        let moved = root.join("new bundle");
        fs::rename(path.parent().unwrap(), &moved).unwrap();
        let artifact = NativeArtifact::load(moved.join("image.json")).unwrap();
        let mut vm = Machine::from_image(&artifact).unwrap();
        assert_eq!(
            vm.read_scalar(artifact.symbol("WORKFLOW.result").unwrap())
                .unwrap(),
            0
        );
        assert_eq!(
            vm.read_array(artifact.symbol("WORKFLOW.table").unwrap())
                .unwrap(),
            [1, 2, 0]
        );
        vm.run(10000).assert_completed();
        for (name, value) in [
            ("WORKFLOW.result", (-298i32) as u32),
            ("WORKFLOW.signedWord", (-300i16) as u16 as u32),
            ("WORKFLOW.marker", 0x81),
        ] {
            assert_eq!(
                vm.read_scalar(artifact.symbol(name).unwrap()).unwrap(),
                value
            );
        }
        assert_eq!(
            vm.read_array(artifact.symbol("WORKFLOW.table").unwrap())
                .unwrap(),
            [0x12345678, 2, 0x80000000]
        );
        let frame = artifact
            .manifest
            .symbols
            .iter()
            .find(|s| matches!(s.location, Location::Frame { .. }))
            .unwrap();
        assert!(frame.address().is_err());
        let cwd = root.join("unrelated working directory");
        fs::create_dir(&cwd).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-vm68k-tests"))
            .current_dir(&cwd)
            .arg("--image")
            .arg(moved.join("image.json"))
            .args(["--budget", "10000"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Completed after"));
        assert!(text.contains("= -298 ($fffffed6)"), "{text}");
        assert!(text.contains("= -300 ($0000fed4)"), "{text}");
    }
}

#[test]
fn image_execution_retains_fault_latch_permissions_and_abi_checks() {
    let source = common::Source::new(
        "VOLATILE LONGINT numerator,divisor\nLONGINT result\nBYTE before,after\nPROC Entry() before=7 result=numerator/divisor after=9 RETURN",
    );
    let path = source.0.parent().unwrap().join("fault.json");
    let artifact = compiler::compile(&source.0, &path, &[]);
    let mut vm = Machine::from_image(&artifact).unwrap();
    vm.write_scalar(artifact.symbol("numerator").unwrap(), 1234)
        .unwrap();
    let run = vm.run(10000);
    assert!(
        matches!(
            run.outcome,
            Outcome::RuntimeFault(RuntimeFault::DivisionByZero)
        ),
        "{run:?}"
    );
    for (name, expected) in [("before", 7), ("after", 0), ("result", 0)] {
        assert_eq!(
            vm.read_scalar(artifact.symbol(name).unwrap()).unwrap(),
            expected
        );
    }
    let repeated = vm.run(10000);
    assert_eq!(repeated.steps, 0);
    assert!(matches!(
        repeated.outcome,
        Outcome::RuntimeFault(RuntimeFault::DivisionByZero)
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-vm68k-tests"))
        .arg("--image")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("DivisionByZero"));

    // Literal MC68000 instructions exercise the unchanged VM guards through an
    // imported image, independently of instruction selection.
    for (words, abi) in [
        (vec![0x742a_u16, 0x4e75], true),
        (vec![0x13c0, 0x0001, 0x0000], false),
    ] {
        let mut altered = artifact.clone();
        let segment = altered.segments.iter_mut().find(|s| s.executable).unwrap();
        let start = (altered.manifest.entry - segment.address) as usize;
        let bytes: Vec<_> = words.into_iter().flat_map(u16::to_be_bytes).collect();
        segment.bytes[start..start + bytes.len()].copy_from_slice(&bytes);
        let run = Machine::from_image(&altered).unwrap().run(100);
        if abi {
            assert!(matches!(run.outcome, Outcome::AbiViolation(_)), "{run:?}");
        } else {
            assert!(
                matches!(run.outcome, Outcome::MemoryViolation(_)),
                "{run:?}"
            );
        }
    }
}

#[test]
fn image_runner_rejects_compile_options_invalid_images_and_reserved_mappings() {
    let source = common::Source::new("PROC Entry() RETURN");
    let path = source.0.parent().unwrap().join("image.json");
    compiler::compile(&source.0, &path, &[]);
    for flags in [
        &["--origin", "0x20000"][..],
        &["--no-opt"],
        &["--no-codegen-opt"],
        &["--dump", "other"],
        &["probe.act"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-vm68k-tests"))
            .arg("--image")
            .arg(&path)
            .args(flags)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--image accepts only --budget"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-vm68k-tests"))
        .arg("--image")
        .arg(&path)
        .args(["--budget", "0"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("BudgetExhausted"));
    for content in ["{}", "{\"format\":\"wrong\"}", "not json"] {
        fs::write(&path, content).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-vm68k-tests"))
            .arg("--image")
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Completed"));
    }
    for origin in ["0x400", "0xe0000", "0x100000"] {
        let image = compiler::compile(&source.0, &path, &["--origin", origin]);
        assert!(Machine::from_image(&image).is_err(), "{origin}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_actionc-vm68k-tests"))
        .arg(&source.0)
        .args(["--origin", "0X23400", "--no-opt", "--budget", "100"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
