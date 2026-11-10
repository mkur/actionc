use actionc::{
    compiler::{CompileMode, CompileOptions, Runtime, compile_file},
    mir6502::{self, Mir6502Config},
};
use std::{path::Path, process::Command};

#[test]
fn pointer_retention_reporting_does_not_change_the_emitted_image() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/pointer_retention.act");
    let baseline = compile_file(&source, &CompileOptions::for_mode(CompileMode::Mir6502)).unwrap();
    let path =
        std::env::temp_dir().join(format!("actionc-pointer-report-{}.xex", std::process::id()));
    let result = Command::new(env!("CARGO_BIN_EXE_actionc"))
        .env("ACTIONC_MIR6502_PEEPHOLES", "sites")
        .args(["--mode", "mir6502", "-o"])
        .arg(&path)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(&path).unwrap(), baseline.object_bytes());
    std::fs::remove_file(path).unwrap();
    assert!(String::from_utf8_lossy(&result.stderr).contains("pointer-retention-setup"));
}

#[test]
fn pointer_retention_switches_preserve_baseline_and_exact_final_profitability() {
    assert!(!Mir6502Config::default().enable_pointer_call_retention);
    assert!(Mir6502Config::optimized().enable_pointer_call_retention);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/runtime/pointer_retention.act");
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
    for optimized in [false, true] {
        let nir = if optimized {
            actionc::nir::optimize_program(&raw).unwrap()
        } else {
            raw.clone()
        };
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for origin in [0x3000, 0x41C7] {
                let mut config = Mir6502Config::optimized();
                config.enable_pointer_call_retention = false;
                config.enable_pointer_loop_retention = false;
                config.enable_pointer_promotion = false;
                let old = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                config.enable_pointer_call_retention = true;
                let new = mir6502::generate_output_with_config_and_runtime(
                    &nir, origin, &config, runtime,
                )
                .unwrap();
                assert!(new.bytes.len() < old.bytes.len());
                let size = |o: &actionc::codegen::CodegenOutput| {
                    let r = o
                        .map
                        .routine_ranges
                        .iter()
                        .find(|r| r.name == "ReadAcrossCall")
                        .unwrap();
                    r.end - r.start
                };
                assert!(size(&new) < size(&old));
            }
        }
    }
}
