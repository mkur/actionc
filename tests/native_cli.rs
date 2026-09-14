use actionc::{
    compiler::native::{self, NativeCompileOptions, artifacts::NativeArtifact},
    mir68k::image::ImageView,
};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct TestDir(PathBuf);
impl TestDir {
    fn new(source: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "actionc native cli {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("probe.act"), source).unwrap();
        Self(dir)
    }
    fn run(&self, emit: bool, args: &[&str]) -> Output {
        Command::new(if emit {
            env!("CARGO_BIN_EXE_actionc-emit")
        } else {
            env!("CARGO_BIN_EXE_actionc")
        })
        .current_dir(&self.0)
        .args(args)
        .arg("probe.act")
        .output()
        .unwrap()
    }
    fn image(&self) -> NativeArtifact {
        NativeArtifact::load(self.0.join("probe.native.json")).unwrap()
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
const SOURCE: &str =
    "LONGINT result\nPROC Entry()\n INT i\n FOR i=0 TO 10 DO result==+i OD\nRETURN\n";

#[test]
fn aliases_origins_and_independent_optimization_controls_match_native_api() {
    for (alias, origin, raw, conservative) in [
        ("motorola-68000", "0x10000", false, false),
        ("68k", "0X23400", true, false),
        ("68000", "$23400", false, true),
        ("m68000", "144384", true, true),
    ] {
        let temp = TestDir::new(SOURCE);
        let mut args = vec![
            "--origin",
            origin,
            "--target",
            alias,
            "--listing",
            "listing with spaces.txt",
        ];
        if raw {
            args.push("--no-opt");
        }
        if conservative {
            args.push("--no-codegen-opt");
        }
        success(temp.run(false, &args));
        let image = temp.image();
        let mut options = NativeCompileOptions {
            origin: if origin == "0x10000" {
                0x10000
            } else {
                0x23400
            },
            optimize: !raw,
            ..Default::default()
        };
        if conservative {
            options.codegen = actionc::mir68k::materialize::Options::conservative();
        }
        let api = native::compile_file(temp.0.join("probe.act"), &options)
            .unwrap()
            .image;
        assert_eq!(image.segments, api.segments);
        assert_eq!(image.entry(), api.entry);
        assert_eq!(image.zero_fill(), api.zero_fill);
        let listing = fs::read_to_string(temp.0.join("listing with spaces.txt")).unwrap();
        assert!(listing.contains("MachineProgram"));
        assert!(!temp.0.join("probe.xex").exists());
    }
}

#[test]
fn annotations_and_explicit_precedence_work_with_both_newlines() {
    for newline in ["\n", "\r\n"] {
        let source = format!(";@actionc target motorola-68000\n;@actionc backend mir68k\n{SOURCE}")
            .replace('\n', newline);
        let temp = TestDir::new(&source);
        success(temp.run(false, &[]));
        assert_eq!(temp.image().entry(), 0x10000);
        success(temp.run(
            false,
            &[
                "--target=68000",
                "--backend=mir68k",
                "--profile=modern",
                "--runtime",
                "bare",
            ],
        ));
        // Explicit settings also allow an annotated native source to be inspected
        // using another target, without retaining its backend selection.
        success(temp.run(
            true,
            &[
                "--target",
                "atari-6502",
                "--profile",
                "modern",
                "--backend",
                "mir6502",
                "--emit-nir",
            ],
        ));
        fs::write(temp.0.join("probe.act"), format!(";@actionc target atari-6502\n;@actionc profile legacy\n;@actionc backend classic\n{SOURCE}").replace('\n', newline)).unwrap();
        success(temp.run(
            false,
            &[
                "--target",
                "68k",
                "--backend",
                "mir68k",
                "--profile",
                "modern",
            ],
        ));
        assert_eq!(temp.run(false, &["--target", "68k"]).status.code(), Some(2));
    }
}

#[test]
fn native_modes_reject_conflicts_and_preserve_completed_outputs_on_failure() {
    let temp = TestDir::new(SOURCE);
    let path = temp.0.join("probe.native.json");
    success(temp.run(false, &["--target", "68k"]));
    let original = fs::read(&path).unwrap();
    for args in [
        vec!["--mode", "compatibility"],
        vec!["--mode", "optimized"],
        vec!["--mode", "mir6502"],
        vec!["--profile", "legacy"],
        vec!["--backend", "classic"],
        vec!["--backend", "mir6502"],
        vec!["--runtime", "cart"],
        vec!["--runtime", "standalone"],
        vec!["--origin", "0x10001"],
        vec!["--origin", "-1"],
        vec!["--origin", "4294967296"],
        vec!["--origin", "0x1000000"],
    ] {
        let mut flags = vec!["--target", "68k"];
        flags.extend(args);
        assert_eq!(temp.run(false, &flags).status.code(), Some(2), "{flags:?}");
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    for flags in [
        &["--no-opt"][..],
        &["--no-codegen-opt"],
        &["--backend", "mir68k"],
        &["--runtime", "bare"],
        &["--origin", "65536"],
    ] {
        assert_eq!(temp.run(false, flags).status.code(), Some(2), "{flags:?}");
    }
    for source in [
        "PROC Entry() missing=7 RETURN",
        "PROC Entry() PrintE(\"hello\") RETURN",
        "PROC Entry( RETURN",
        "BYTE FUNC Library() RETURN(1)",
    ] {
        fs::write(temp.0.join("probe.act"), source).unwrap();
        assert_eq!(
            temp.run(false, &["--target", "68k"]).status.code(),
            Some(1),
            "{source}"
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    fs::write(
        temp.0.join("probe.act"),
        "SET $491=$3000\nSET $E=$3000\nPROC Entry() RETURN",
    )
    .unwrap();
    assert_eq!(temp.run(false, &["--target", "68k"]).status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn stdout_modes_are_target_specific_and_shared_inspection_remains_available() {
    let temp = TestDir::new(SOURCE);
    for (mode, expected) in [
        ("--emit-code", "$00010000:"),
        ("--emit-listing", "MachineProgram"),
        ("--emit-map", "result"),
        ("--emit-tokens", "Span"),
        ("--emit-semir", "result"),
        ("--emit-nir", "result"),
        ("--emit-optimized-nir", "result"),
        ("--emit-nir-stats", "routine"),
    ] {
        let output = temp.run(true, &["--target", "68k", mode]);
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(expected),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        success(output);
    }
    for mode in [
        "--emit-load",
        "--emit-source-listing",
        "--emit-mir6502",
        "--emit-materialized-mir6502",
        "--emit-proofs",
        "--emit-proof-attempts",
        "--emit-link-plan",
    ] {
        let output = temp.run(true, &["--target", "68k", mode]);
        assert_eq!(output.status.code(), Some(2), "{mode}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unavailable for Motorola 68000"));
    }
    assert_eq!(
        temp.run(
            true,
            &[
                "--target",
                "wdc-65816-native",
                "--profile",
                "modern",
                "--emit-code"
            ]
        )
        .status
        .code(),
        Some(2)
    );
}

#[test]
fn module_search_paths_and_import_diagnostics_are_forwarded() {
    let temp = TestDir::new(
        "MODULE APP USE MATHS PUBLIC LONGINT result PROC Entry() result=MATHS.Value() RETURN ENDMODULE",
    );
    let dir = temp.0.join("modules with spaces");
    fs::create_dir(&dir).unwrap();
    let module = dir.join("maths.act");
    fs::write(
        &module,
        "MODULE MATHS PUBLIC LONGINT FUNC Value() RETURN(-300) ENDMODULE",
    )
    .unwrap();
    success(temp.run(
        false,
        &[
            "--target",
            "68k",
            "--module-path",
            "unused",
            "--module-path=modules with spaces",
        ],
    ));
    assert!(temp.image().symbol("APP.result").is_ok());
    let original = fs::read(&module).unwrap();
    let output = temp.run(
        false,
        &[
            "--target",
            "68k",
            "--module-path=modules with spaces",
            "-o",
            "modules with spaces/maths.act",
        ],
    );
    assert!(!output.status.success());
    assert_eq!(fs::read(&module).unwrap(), original);
    fs::write(
        &module,
        "MODULE MATHS PUBLIC LONGINT FUNC Value() RETURN(missing) ENDMODULE",
    )
    .unwrap();
    let output = temp.run(
        false,
        &["--target", "68k", "--module-path=modules with spaces"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("maths.act"));
}

#[test]
fn amiga_cli_selects_single_file_output_and_section_relative_inspection() {
    let temp = TestDir::new(
        ";@actionc target motorola-68000\n;@actionc backend mir68k\nBYTE result PROC Entry() result=1 PrintIE(-32768) RETURN",
    );
    success(temp.run(false, &["--runtime", "amiga"]));
    let bytes = fs::read(temp.0.join("probe.amiga")).unwrap();
    assert_eq!(&bytes[..4], &1011u32.to_be_bytes());
    assert!(!temp.0.join("probe.native.json").exists());
    let api = native::amiga::compile_file(temp.0.join("probe.act"), &Default::default()).unwrap();
    assert_eq!(bytes, api.executable.bytes);
    for (mode, expected) in [
        ("--emit-code", "HUNK+00000000:"),
        ("--emit-listing", "PlatformRoutine"),
        ("--emit-map", "result Section"),
        ("--emit-nir", "result"),
        ("--emit-optimized-nir", "result"),
    ] {
        let output = temp.run(true, &["--runtime", "amiga", mode]);
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(expected),
            "{mode}: {} / {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        success(output);
    }
}

#[test]
fn amiga_cli_rejects_incompatible_options_and_preserves_outputs_and_inputs() {
    let temp = TestDir::new("PROC Entry() PrintE(\"ok\") RETURN");
    let base = ["--target", "68k", "--runtime", "amiga"];
    success(temp.run(false, &base));
    let output = temp.0.join("probe.amiga");
    let original = fs::read(&output).unwrap();
    for flags in [
        vec!["--origin", "0x10000"],
        vec!["--mode", "optimized"],
        vec!["--profile", "legacy"],
        vec!["--backend", "classic"],
        vec!["--backend", "mir6502"],
        vec!["--target", "atari-6502"],
        vec!["--target", "wdc-65816-native"],
    ] {
        let args: Vec<_> = base.iter().copied().chain(flags).collect();
        assert_eq!(temp.run(false, &args).status.code(), Some(2), "{args:?}");
        assert_eq!(fs::read(&output).unwrap(), original);
    }
    for source in [
        "ORG $4000 PROC Entry() RETURN",
        "SET $E=$4000 PROC Entry() RETURN",
        "PROC Entry() Graphics(0) RETURN",
        "PROC Entry() missing=1 RETURN",
    ] {
        fs::write(temp.0.join("probe.act"), source).unwrap();
        let result = temp.run(false, &base);
        assert!(!result.status.success(), "{source}");
        assert_eq!(fs::read(&output).unwrap(), original);
    }
    fs::write(temp.0.join("local.inc"), "CARD value=[65]\n").unwrap();
    let source = "INCLUDE \"local.inc\"\nPROC Entry() PrintCE(value) RETURN";
    fs::write(temp.0.join("probe.act"), source).unwrap();
    for destination in ["probe.act", "local.inc"] {
        let before = fs::read(temp.0.join(destination)).unwrap();
        let args: Vec<_> = base.iter().copied().chain(["-o", destination]).collect();
        assert!(!temp.run(false, &args).status.success());
        assert_eq!(fs::read(temp.0.join(destination)).unwrap(), before);
        let args: Vec<_> = base
            .iter()
            .copied()
            .chain(["--listing", destination])
            .collect();
        assert!(!temp.run(false, &args).status.success());
        assert_eq!(fs::read(temp.0.join(destination)).unwrap(), before);
    }
    fs::write(temp.0.join("blocked"), b"file").unwrap();
    let args: Vec<_> = base
        .iter()
        .copied()
        .chain(["--listing", "blocked/list.txt"])
        .collect();
    assert!(!temp.run(false, &args).status.success());
    assert_eq!(fs::read(&output).unwrap(), original);
}
