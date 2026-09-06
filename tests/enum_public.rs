use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use std::path::Path;
use std::process::Command;

#[test]
fn enum_public_api_and_cli_use_semantics_in_every_modern_route() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("fixtures/runtime/enum_types.act");
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            compile_file(&path, &CompileOptions::for_mode(mode).with_runtime(runtime)).unwrap();
        }
        let errors = compile_file(
            &path,
            &CompileOptions::for_mode(CompileMode::Compatibility).with_runtime(runtime),
        )
        .unwrap_err();
        assert!(
            errors
                .to_string()
                .contains("ENUM requires the modern profile"),
            "{errors}"
        );
        for source in [None, Some("ast"), Some("semir")] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_actionc-emit"));
            command.args([
                "--profile",
                "modern",
                "--backend",
                "classic",
                "--runtime",
                if runtime == Runtime::ActionCart {
                    "cart"
                } else {
                    "standalone"
                },
                "--emit-code",
            ]);
            if let Some(source) = source {
                command.args(["--codegen-source", source]);
            }
            let output = command.arg(&path).output().unwrap();
            assert!(
                output.status.success(),
                "{runtime:?}/{source:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.stdout.is_empty());
        }
    }
}

#[test]
fn raw_ast_codegen_rejects_unresolved_enums_instead_of_guessing_byte_layout() {
    for source in [
        "TYPE E=ENUM [A] PROC Main() RETURN",
        "E FUNC Read() RETURN(0)",
        "PROC Main() E FUNC POINTER callback RETURN",
        "PROC Main()\nBEGIN\nTYPE E=ENUM [A]\nEND\nRETURN",
    ] {
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
        for profile in [
            actionc::codegen::CodegenProfile::Modern,
            actionc::codegen::CodegenProfile::Compat,
        ] {
            let errors =
                actionc::codegen::generate_profile_with_origin(&ast, 0x3000, profile).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|e| e.message.contains("requires semantic lowering")),
                "{source}: {errors:?}"
            );
        }
    }
}
