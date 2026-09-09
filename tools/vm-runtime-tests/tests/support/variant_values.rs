//! Variant values execute against a host memory oracle, including fault paths.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, RunRequest, StopReason, VmRunner,
};

fn execute(image: &[u8], runtime: Runtime, fault: bool) -> Vec<u8> {
    execute_watched(image, runtime, fault, &[], &|_| {})
}

fn execute_watched(
    image: &[u8],
    runtime: Runtime,
    fault: bool,
    watch: &[u16],
    observe: &impl Fn(&[actionc_vm::BusEvent]),
) -> Vec<u8> {
    let mut vm = CompilerVm::default();
    vm.load_image_bytes(
        ImageKind::Rom,
        "altirraos-xl.rom",
        actionc_vm::OS_ROM_BASE,
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roms/altirraos-xl.rom"),
        )
        .unwrap(),
    )
    .unwrap();
    let profile = if runtime == Runtime::ActionCart {
        let rom = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roms/action.rom");
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            DEFAULT_CART_BASE,
            std::fs::read(rom).unwrap(),
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
            .all(|s| s.end < 0x600 || s.start > 0xAFF)
    );
    for address in 0x600..=0xAFF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0xB00 || s.start > 0xBFF)
    );
    vm.bus_mut()
        .ram_mut()
        .map(
            0xB80,
            &[
                0x8D, 0x20, 0x0B, 0x8E, 0x21, 0x0B, 0x8C, 0x22, 0x0B, // capture A/X/Y
                0xEE, 0x23, 0x0B, // invocation count
                0xAD, 0x00, 0x06, 0x8D, 0x24, 0x0B, // observe last application store
                0xF8, 0x60, // returning Error handler leaves decimal mode set
            ],
        )
        .unwrap();
    vm.bus_mut().ram_mut().write(0xB23, 0);
    match runtime {
        Runtime::ActionCart => vm
            .bus_mut()
            .ram_mut()
            .map(0x04CB, &[0x4C, 0x80, 0x0B])
            .unwrap(),
        Runtime::Standalone => vm.bus_mut().ram_mut().write_word(0x000A, 0x0B80),
    }
    for address in watch {
        vm.bus_mut().add_watchpoint(*address);
    }
    vm.bus_mut().clear_events();
    let result = VmRunner::new(vm).run(RunRequest {
        max_steps: 300_000,
        history_len: 16,
        ..RunRequest::default()
    });
    assert_eq!(
        result.stop_reason(),
        StopReason::StepLimit { max_steps: 300_000 },
        "{:?}",
        result.report
    );
    assert_eq!(
        result.memory().read(0xB23),
        u8::from(fault),
        "unexpected Error dispatch: {:?}",
        result.report
    );
    if fault {
        assert_eq!(
            (0xB20..=0xB24)
                .map(|a| result.memory().read(a))
                .collect::<Vec<_>>(),
            [100, 0, 100, 1, 41]
        );
        assert_eq!(result.report.registers.status & 8, 0);
        let pc = result.report.registers.pc;
        assert_eq!(
            (
                result.memory().read(pc),
                result.memory().read(pc.wrapping_add(1))
            ),
            (0xB0, 0xFE)
        );
    }
    observe(result.vm.bus().events());
    (0x600..=0xAFF).map(|a| result.memory().read(a)).collect()
}

fn compare(actual: Vec<u8>, expected: &[u8], path: &str) {
    assert_eq!(actual.len(), expected.len(), "{path}: oracle extent");
    let differences: Vec<_> = actual
        .iter()
        .zip(expected)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| (format!("${:04X}", 0x600 + i), *a, *b))
        .collect();
    assert!(
        differences.is_empty(),
        "{path}: (address, actual, expected) {differences:?}"
    );
}

#[allow(dead_code)]
pub fn check(source: &str, expected: &[u8]) {
    check_with_fault(source, expected, false);
}

#[allow(dead_code)]
pub fn check_with_fault(source: &str, expected: &[u8], fault: bool) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let mut options = actionc::semantic::SemanticOptions::modern();
    options.algebraic_types.variants = true;
    let model = actionc::semantic::analyze_with_options(&ast, options).unwrap();
    let semir = actionc::semantic::ir::lower_program(&ast, &model);
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "actionc-variants-{}-{}.act",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let compiled = compile_file(
                &path,
                &CompileOptions::for_mode(mode)
                    .with_runtime(runtime)
                    .with_origin(0x3000),
            )
            .unwrap();
            compare(
                execute(compiled.object_bytes(), runtime, fault),
                expected,
                &format!("{mode:?}/{runtime:?}"),
            );
        }
        for optimized in [false, true] {
            let nir = actionc::nir::lower_program(&semir);
            actionc::nir::verify_program(&nir).unwrap();
            let nir = if optimized {
                actionc::nir::optimize_program(&nir).unwrap()
            } else {
                nir
            };
            let output = actionc::mir6502::generate_output_with_config_and_runtime(
                &nir,
                0x3000,
                &actionc::mir6502::Mir6502Config::default(),
                runtime,
            )
            .unwrap();
            compare(
                execute(&actionc::codegen::format_load_file(&output), runtime, fault),
                expected,
                &format!("MIR/{runtime:?}/optimized={optimized}"),
            );
        }
    }
    std::fs::remove_file(path).unwrap();
}

/// Exercise staged semantic capabilities without opening a public source gate.
#[allow(dead_code)]
pub fn check_semir(semir: &actionc::semantic::ir::SemProgram, expected: &[u8], fault: bool) {
    check_semir_watched(semir, expected, fault, &[], |_, _| {});
}

#[allow(dead_code)]
pub fn check_semir_watched(
    semir: &actionc::semantic::ir::SemProgram,
    expected: &[u8],
    fault: bool,
    watch: &[u16],
    observe: impl Fn(&str, &[actionc_vm::BusEvent]),
) {
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = actionc::codegen::generate_semir_profile_at_origin_with_runtime(
            semir,
            0x3000,
            actionc::codegen::CodegenProfile::Modern,
            runtime,
        )
        .unwrap();
        let path = format!("classic/{runtime:?}");
        compare(
            execute_watched(
                &actionc::codegen::format_load_file(&output),
                runtime,
                fault,
                watch,
                &|events| observe(&path, events),
            ),
            expected,
            &format!("classic/{runtime:?}"),
        );
        let raw = actionc::nir::lower_program(semir);
        actionc::nir::verify_program(&raw).unwrap();
        for optimized in [false, true] {
            let nir = if optimized {
                actionc::nir::optimize_program(&raw).unwrap()
            } else {
                raw.clone()
            };
            actionc::nir::verify_program(&nir).unwrap();
            let output = actionc::mir6502::generate_output_with_config_and_runtime(
                &nir,
                0x3000,
                &actionc::mir6502::Mir6502Config::default(),
                runtime,
            )
            .unwrap();
            let path = format!("MIR/{runtime:?}/optimized={optimized}");
            compare(
                execute_watched(
                    &actionc::codegen::format_load_file(&output),
                    runtime,
                    fault,
                    watch,
                    &|events| observe(&path, events),
                ),
                expected,
                &format!("MIR/{runtime:?}/optimized={optimized}"),
            );
        }
    }
}
