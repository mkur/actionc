use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::codegen::{
    CODE_ORIGIN, CodegenProfile, format_load_file, generate_profile_with_origin,
};
use actionc::compiler::{
    CompileErrorKind, CompileMode, CompileOptions, CompileWarning, CompilerPhase, DiagnosticSite,
    Runtime, compile_file,
};
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502::{self, Mir6502Config};
use actionc::nir;
use actionc::semantic::{analyze, ir};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("actionc-api-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).expect("create compiler API test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn hello_world() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("hello-world.act")
}

fn write_source(temp: &TestDir, name: &str, source: &str) -> PathBuf {
    let path = temp.path().join(name);
    fs::write(&path, source).expect("write Action source");
    path
}

fn baseline_object(path: &Path, mode: CompileMode) -> Vec<u8> {
    let loaded = load_program_with_expanded_source(path).expect("load baseline program");
    let model = analyze(&loaded.program).expect("analyze baseline program");
    let output = match mode {
        CompileMode::Compatibility => {
            generate_profile_with_origin(&loaded.program, CODE_ORIGIN, CodegenProfile::Compat)
                .expect("compile compatibility baseline")
        }
        CompileMode::Optimized => {
            generate_profile_with_origin(&loaded.program, CODE_ORIGIN, CodegenProfile::Modern)
                .expect("compile optimized baseline")
        }
        CompileMode::Mir6502 => {
            let semir = ir::lower_program(&loaded.program, &model);
            let lowered = nir::lower_program(&semir);
            let optimized = nir::optimize_program(&lowered).expect("optimize NIR baseline");
            mir6502::generate_output_with_config(
                &optimized,
                CODE_ORIGIN,
                &Mir6502Config::optimized(),
            )
            .expect("compile MIR6502 baseline")
        }
    };
    format_load_file(&output)
}

#[test]
fn compatibility_api_matches_the_existing_classic_pipeline() {
    let source = hello_world();
    let compiled = compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Compatibility),
    )
    .expect("compile through reusable API");

    let loaded = load_program_with_expanded_source(&source).expect("load baseline program");
    let baseline = generate_profile_with_origin(&loaded.program, 0x3000, CodegenProfile::Compat)
        .expect("compile through existing classic pipeline");

    assert_eq!(compiled.object_bytes(), format_load_file(&baseline));
    assert_eq!(compiled.origin(), baseline.origin);
    assert_eq!(compiled.run_address(), baseline.run_address);
}

#[test]
fn compile_options_preserve_project_root_and_ordered_module_paths() {
    let options = CompileOptions::default()
        .with_project_root("project")
        .with_module_path("first")
        .with_module_path("second");

    assert_eq!(options.project_root(), Some(Path::new("project")));
    assert_eq!(
        options.module_paths(),
        [PathBuf::from("first"), PathBuf::from("second")]
    );
}

#[test]
fn compiled_program_formats_a_mads_compatible_source_listing() {
    let compiled = compile_file(
        hello_world(),
        &CompileOptions::for_mode(CompileMode::Compatibility),
    )
    .expect("compile source listing input");

    let listing = compiled.source_listing();
    assert!(listing.contains("; ===== PROC Main"));
    assert!(listing.contains("JSR.A $A46C"));
    assert!(listing.contains("| PrintE(\"Hello, world!\")"));
    assert!(listing.contains("ORG $02E2\n        DTA A(proc_main)"));
}

#[test]
fn all_public_modes_match_the_existing_pipelines() {
    let source = hello_world();
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile {mode:?} through reusable API: {error}"));

        assert_eq!(compiled.object_bytes(), baseline_object(&source, mode));
    }
}

#[test]
fn compatibility_api_honors_an_explicit_nondefault_origin() {
    let compiled = compile_file(
        hello_world(),
        &CompileOptions::for_mode(CompileMode::Compatibility).with_origin(0x3a00),
    )
    .expect("compile at an explicit origin");

    assert_eq!(compiled.origin(), 0x3a00);
    assert_eq!(&compiled.object_bytes()[2..4], &0x3a00u16.to_le_bytes());
}

#[test]
fn standalone_sys_warning_is_structured_and_backend_independent() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "sys-warning.act",
        "MODULE APP\n\
         USE SYS\n\
         BYTE ARRAY buffer(1)\n\
         PROC Main() SYS.Zero(buffer,1) PrintE(\"Ready\") RETURN\n\
         ENDMODULE\n",
    );
    let expected = [CompileWarning::StandaloneGplRuntime {
        sys_routines: vec!["SYS.PrintE".to_string(), "SYS.Zero".to_string()],
        helpers: Vec::new(),
    }];

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(
            &source,
            &CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone),
        )
        .unwrap_or_else(|error| panic!("compile standalone SYS warning in {mode:?}: {error}"));
        assert_eq!(compiled.warnings(), expected, "{mode:?}");
    }

    let cart = compile_file(&source, &CompileOptions::for_mode(CompileMode::Optimized))
        .expect("compile cart SYS program");
    assert!(cart.warnings().is_empty());

    let helper_only = write_source(
        &temp,
        "helper-only.act",
        "CARD left,right,value\n\
         PROC Main() left=300 right=300 value=left*right RETURN\n",
    );
    let helper_only = compile_file(
        &helper_only,
        &CompileOptions::for_mode(CompileMode::Optimized).with_runtime(Runtime::Standalone),
    )
    .expect("compile helper-only standalone program");
    assert_eq!(
        helper_only.warnings(),
        [CompileWarning::StandaloneGplRuntime {
            sys_routines: Vec::new(),
            helpers: vec!["MultI".to_string()],
        }]
    );
}

#[test]
fn origin_precedence_is_consistent_in_all_modes() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "origin.act",
        "SET $E=$4000\nSET $491=$4000\nPROC Main()\nRETURN\n",
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let implicit = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile implicit-origin {mode:?}: {error}"));
        assert_eq!(implicit.origin(), 0x4000);

        let explicit = compile_file(
            &source,
            &CompileOptions::for_mode(mode).with_origin(CODE_ORIGIN),
        )
        .unwrap_or_else(|error| panic!("compile explicit-origin {mode:?}: {error}"));
        assert_eq!(explicit.origin(), CODE_ORIGIN);
    }
}

#[test]
fn source_annotations_fill_defaults_but_do_not_override_an_explicit_mode() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "annotated.act",
        ";@actionc profile modern\n;@actionc backend mir6502\nPROC Main()\nRETURN\n",
    );

    let implicit = compile_file(&source, &CompileOptions::default()).expect("compile annotations");
    let explicit_mir = compile_file(&source, &CompileOptions::for_mode(CompileMode::Mir6502))
        .expect("compile explicit MIR6502");
    assert_eq!(implicit.object_bytes(), explicit_mir.object_bytes());

    let explicit_compatibility = compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Compatibility),
    )
    .expect("compile explicit compatibility");
    assert_eq!(
        explicit_compatibility.object_bytes(),
        baseline_object(&source, CompileMode::Compatibility)
    );
}

#[test]
fn nir_preflight_failure_is_returned_as_a_source_diagnostic() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "retarget.act",
        "PROC A() RETURN PROC T() PROC Main() T=A T() RETURN",
    );

    let error = compile_file(&source, &CompileOptions::for_mode(CompileMode::Mir6502)).unwrap_err();

    assert_eq!(error.kind(), CompileErrorKind::Compilation);
    assert_eq!(error.diagnostics()[0].phase, CompilerPhase::Nir);
    assert!(
        error.diagnostics()[0]
            .message
            .contains("legacy routine-name retargeting")
    );
    assert!(matches!(
        &error.diagnostics()[0].site,
        DiagnosticSite::Source { origin, .. }
            if origin.host_path() == Some(source.as_path())
    ));
}

#[test]
fn native_real_core_arithmetic_compiles_with_both_backends_and_runtimes() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "native-real.act",
        "REAL value PROC Main() value=1.25+2 RETURN",
    );

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                &source,
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| panic!("compile native REAL for {mode:?}/{runtime}: {error}"));
            assert_eq!(compiled.runtime(), runtime);
            assert!(
                compiled
                    .object_bytes()
                    .windows(3)
                    .any(|bytes| bytes == [0x20, 0x66, 0xDA]),
                "expected FADD call for {mode:?}/{runtime}"
            );
            let listing = compiled.source_listing();
            assert!(listing.contains("ATARI_FPP_FADD"));
            assert!(listing.contains("Atari OS ROM"));
        }
    }

    let compatibility = compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Compatibility),
    )
    .unwrap_err();
    assert_eq!(
        compatibility.diagnostics()[0].phase,
        CompilerPhase::Semantic
    );
    assert!(
        compatibility
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown type `REAL`"))
    );
}

#[test]
fn optimized_classic_folds_integer_real_constants_and_negates_without_fpp() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "native-real-compact.act",
        r#"
            REAL byte_value, char_value, card_value, int_value, negated

            PROC Main()
              byte_value=BYTE(200)
              char_value=CHAR('A)
              card_value=CARD(65535)
              int_value=INT(-123)
              negated=-byte_value
            RETURN
        "#,
    );

    let compiled = compile_file(&source, &CompileOptions::for_mode(CompileMode::Optimized))
        .expect("compile compact classic native REAL operations");
    let object = compiled.object_bytes();

    assert!(
        !object.windows(3).any(|bytes| bytes == [0x20, 0xAA, 0xD9]),
        "constant integer promotions must not call Atari FPP IFP"
    );
    assert!(
        !object.windows(3).any(|bytes| bytes == [0x20, 0x60, 0xDA]),
        "native REAL negation must not call Atari FPP FSUB"
    );
    assert!(
        object.windows(2).any(|bytes| bytes == [0x49, 0x80]),
        "native REAL negation should toggle the packed sign bit"
    );
}

#[test]
fn optimized_classic_pools_real_literals_and_uses_compact_copy_loops() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "native-real-copy-pool.act",
        r#"
            REAL first, second, copied
            REAL POINTER destination

            PROC StoreFirst()
              first=1.23456789
              copied=first
              destination=@second
              destination^=copied
            RETURN

            PROC Main()
              second=1.23456789
              StoreFirst()
            RETURN
        "#,
    );

    let compiled = compile_file(&source, &CompileOptions::for_mode(CompileMode::Optimized))
        .expect("compile pooled classic native REAL literals");
    let object = compiled.object_bytes();
    let literal = [0x40, 0x01, 0x23, 0x45, 0x67, 0x89];

    assert_eq!(
        object
            .windows(literal.len())
            .filter(|bytes| *bytes == literal)
            .count(),
        1,
        "identical executable REAL literals should share one pool entry"
    );
    assert!(
        object.windows(11).any(|bytes| {
            bytes[0] == 0xA2
                && bytes[1] == 5
                && bytes[2] == 0xBD
                && bytes[5] == 0x9D
                && bytes[8] == 0xCA
                && bytes[9] == 0x10
        }),
        "direct REAL copies should use an X-indexed descending loop"
    );
    assert!(
        object.windows(11).any(|bytes| {
            bytes[0] == 0xA0
                && bytes[1] == 0
                && bytes[2] == 0xB9
                && bytes[5] == 0x48
                && bytes[6] == 0xC8
                && bytes[7] == 0xC0
                && bytes[8] == 6
                && bytes[9] == 0xD0
        }),
        "an indirect REAL copy should use a compact staged load loop"
    );
    assert!(
        object.windows(8).any(|bytes| {
            bytes[0] == 0xA0
                && bytes[1] == 5
                && bytes[2] == 0x68
                && bytes[3] == 0x91
                && bytes[5] == 0x88
                && bytes[6] == 0x10
        }),
        "an indirect REAL copy should use a compact staged store loop"
    );
}

#[test]
fn optimized_classic_stages_cast_wrapped_call_arguments() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "cast-wrapped-call-arguments.act",
        r#"
            INT left, right
            CARD input

            PROC Capture(CARD first BYTE second)
            RETURN

            PROC Consume(INT value)
            RETURN

            PROC Main()
              Capture(left+right,BYTE(left-right))
              Consume(input-700)
            RETURN
        "#,
    );

    compile_file(&source, &CompileOptions::for_mode(CompileMode::Optimized))
        .expect("compile cast-wrapped computed call arguments");
}

#[test]
fn every_native_real_fpp_call_restores_binary_decimal_mode() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "native-real-fpp-decimal.act",
        r#"
            REAL left, right, result
            INT integer

            PROC Main()
              left=integer
              integer=INT(left)
              result=left+right
              result=left-right
              result=left*right
              result=left/right
            RETURN
        "#,
    );

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile native REAL FPP calls for {mode:?}: {error}"));
        for address in [0xD9AAu16, 0xD9D2, 0xDA66, 0xDA60, 0xDADB, 0xDB28] {
            let [lo, hi] = address.to_le_bytes();
            assert!(
                compiled
                    .object_bytes()
                    .windows(4)
                    .any(|bytes| bytes == [0x20, lo, hi, 0xD8]),
                "expected JSR ${address:04X}; CLD for {mode:?}"
            );
        }
    }
}

#[test]
fn loop_conditions_accept_all_original_binary_operators_in_all_pipelines() {
    let temp = TestDir::new();
    let mut source = String::from(
        "CONST BYTE UNDERFLOW=$FF\n\
         BYTE left,right\n\
         CARD wide\n\
         PROC Main()\n\
           left=2 right=1 wide=1\n",
    );

    for op in [
        "+", "-", "*", "/", " MOD ", " LSH ", " RSH ", "&", "%", "!", "=", "#", "<", "<=", ">",
        ">=",
    ] {
        source.push_str(&format!(
            "  WHILE left{op}right DO EXIT OD\n  DO UNTIL left{op}right OD\n"
        ));
    }

    source.push_str("  WHILE left#UNDERFLOW DO EXIT OD\n  DO UNTIL left#UNDERFLOW OD\n");
    for op in ["=", "#", "<", "<=", ">", ">="] {
        source.push_str(&format!(
            "  WHILE left{op}wide DO EXIT OD\n\
               WHILE wide{op}left DO EXIT OD\n\
               DO UNTIL left{op}wide OD\n\
               DO UNTIL wide{op}left OD\n"
        ));
    }
    source.push_str("RETURN\n");
    let source = write_source(&temp, "loop-operators.act", &source);

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            compile_file(
                &source,
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| {
                panic!("compile loop operators for {mode:?}/{runtime}: {error}")
            });
        }
    }
}

#[test]
fn standalone_runtime_linking_preserves_the_last_application_proc_as_runad() {
    let temp = TestDir::new();
    let source = write_source(
        &temp,
        "last-proc-entry.act",
        "BYTE result=$0600\n\
         PROC Main() result=1 RETURN\n\
         PROC KeepFourth(BYTE first,second,third,fourth) result=fourth RETURN\n\
         PROC Start() KeepFourth(1,2,3,4) RETURN\n\
         BYTE FUNC Helper() RETURN(0)\n",
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(
            &source,
            &CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone),
        )
        .unwrap_or_else(|error| panic!("compile last-PROC standalone {mode:?}: {error}"));
        let listing = compiled.source_listing();
        let start_header = listing
            .lines()
            .find(|line| line.contains("PROC Start $") && line.contains("..$"))
            .unwrap_or_else(|| panic!("missing Start listing header for {mode:?}:\n{listing}"));
        let start_entry_hex = start_header
            .split(" entry $")
            .nth(1)
            .and_then(|tail| tail.split_whitespace().next())
            .or_else(|| {
                start_header
                    .split("PROC Start $")
                    .nth(1)
                    .and_then(|tail| tail.split("..$").next())
            })
            .expect("find Start entry address");
        let start_entry = u16::from_str_radix(start_entry_hex, 16)
            .expect("parse Start entry address");

        assert_eq!(compiled.run_address(), start_entry, "{mode:?}");
        assert!(listing.contains("proc_syslib_sargs:"), "{mode:?}: {listing}");
        assert!(
            listing.contains("ORG $02E2\n        DTA A(proc_start)"),
            "{mode:?}: {listing}"
        );
    }
}

#[test]
fn historical_toolkit_real_source_remains_a_record_in_modern_backends() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/toolkit/modern/REAL.ACT");
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile Toolkit REAL.ACT with {mode:?}: {error}"));
    }
}

#[test]
fn include_diagnostics_keep_the_included_path() {
    let temp = TestDir::new();
    let source_dir = temp.path().join("source tree");
    fs::create_dir_all(&source_dir).expect("create spaced source directory");
    let source = source_dir.join("main.act");
    let included = source_dir.join("library.act");
    fs::write(&source, "INCLUDE \"library.act\"\nPROC Main() RETURN\n")
        .expect("write including source");
    fs::write(&included, "PROC Broken()\n  missing_symbol=1\nRETURN\n")
        .expect("write broken include");

    let error = compile_file(&source, &CompileOptions::default()).unwrap_err();

    assert_eq!(error.diagnostics()[0].phase, CompilerPhase::Semantic);
    assert!(matches!(
        &error.diagnostics()[0].site,
        DiagnosticSite::Source { origin, .. }
            if origin.host_path() == Some(included.as_path())
    ));
}

#[test]
fn repeated_and_concurrent_compilations_are_independent() {
    let source = hello_world();
    let first = compile_file(&source, &CompileOptions::for_mode(CompileMode::Optimized))
        .expect("first repeated compilation");
    let second = compile_file(&source, &CompileOptions::for_mode(CompileMode::Optimized))
        .expect("second repeated compilation");
    assert_eq!(first.object_bytes(), second.object_bytes());

    std::thread::scope(|scope| {
        let jobs = [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ]
        .map(|mode| {
            let source = source.clone();
            scope.spawn(move || {
                compile_file(&source, &CompileOptions::for_mode(mode))
                    .unwrap_or_else(|error| panic!("concurrent {mode:?} compilation: {error}"))
                    .object_bytes()
                    .to_vec()
            })
        });

        for (job, mode) in jobs.into_iter().zip([
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ]) {
            assert_eq!(
                job.join().expect("join compilation"),
                baseline_object(&source, mode)
            );
        }
    });
}

#[test]
fn invalid_source_returns_diagnostics_without_creating_outputs() {
    let temp = TestDir::new();
    let source = temp.path().join("broken.act");
    fs::write(&source, "PROC Main( RETURN").expect("write invalid source");

    let error = compile_file(&source, &CompileOptions::default()).unwrap_err();

    assert_eq!(error.kind(), CompileErrorKind::Compilation);
    assert_eq!(error.diagnostics()[0].phase, CompilerPhase::Frontend);
    assert!(error.diagnostics()[0].message.contains("expected"));
    assert!(matches!(
        &error.diagnostics()[0].site,
        DiagnosticSite::Source { origin, .. }
            if origin.host_path() == Some(source.as_path())
    ));
    assert_eq!(
        fs::read_dir(temp.path()).unwrap().count(),
        1,
        "the compiler API wrote an output file"
    );
}
