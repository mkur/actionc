use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use std::path::Path;
use std::process::Command;

fn fixture_nir(optimized: bool) -> actionc::nir::NirProgram {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/register_copy_forwarding.act");
    let loaded = actionc::includes::load_program_with_expanded_source(&path).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
        &loaded.program,
        &model,
    ));
    if optimized {
        actionc::nir::optimize_program(&raw).unwrap()
    } else {
        raw
    }
}

#[test]
fn register_copy_feature_is_independent_and_costed_on_neutral_sources() {
    assert!(!actionc::mir6502::Mir6502Config::default().enable_register_copy_forwarding);
    assert!(actionc::mir6502::Mir6502Config::optimized().enable_register_copy_forwarding);
    for optimized_nir in [false, true] {
        let nir = fixture_nir(optimized_nir);
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut config = actionc::mir6502::Mir6502Config::optimized();
                config.enable_register_copy_forwarding = false;
                let old = actionc::mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                config.enable_register_copy_forwarding = true;
                let new = actionc::mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                assert!(
                    new.bytes.len() < old.bytes.len(),
                    "{runtime:?}/raw={}/{origin:04X}",
                    !optimized_nir
                );
                for name in ["CopyCounted", "SendString"] {
                    let size = |output: &actionc::codegen::CodegenOutput| {
                        let range = output
                            .map
                            .routine_ranges
                            .iter()
                            .find(|r| r.name == name)
                            .unwrap();
                        range.end - range.start
                    };
                    assert!(size(&new) < size(&old));
                }
            }
        }
    }
}

#[test]
fn copy_candidate_reporting_preserves_the_emitted_image() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("fixtures/runtime/register_copy_forwarding.act");
    let output =
        std::env::temp_dir().join(format!("actionc-copy-census-{}.xex", std::process::id()));
    let baseline = compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(Runtime::ActionCart),
    )
    .unwrap();
    let reported = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .env("ACTIONC_MIR6502_PEEPHOLES", "sites")
        .args(["--mode", "mir6502", "--runtime", "cart", "-o"])
        .arg(&output)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        reported.status.success(),
        "{}",
        String::from_utf8_lossy(&reported.stderr)
    );
    assert_eq!(baseline.object_bytes(), std::fs::read(&output).unwrap());
    std::fs::remove_file(output).unwrap();
    let report = String::from_utf8(reported.stderr).unwrap();
    assert!(
        report.contains("register-copy-transfer-candidate"),
        "{report}"
    );
    assert!(report.contains("register-copy-load-candidate"), "{report}");
    assert!(report.contains("available=Some(Y)"), "{report}");
}
