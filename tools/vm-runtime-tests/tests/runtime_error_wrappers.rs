use actionc::compiler::Runtime;
use actionc::{codegen, mir6502, nir, semantic};
use actionc_vm::{CompilerVm, ExecutionProfile, ImageKind, RunRequest, StopReason, VmRunner};

fn execute(output: &codegen::CodegenOutput, runtime: Runtime, lane: &str) {
    let mut vm = CompilerVm::default();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    vm.load_image_bytes(
        ImageKind::Rom,
        "altirraos-xl.rom",
        actionc_vm::OS_ROM_BASE,
        std::fs::read(root.join("roms/altirraos-xl.rom")).unwrap(),
    )
    .unwrap();
    let profile = if runtime == Runtime::ActionCart {
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            actionc_vm::DEFAULT_CART_BASE,
            std::fs::read(root.join("roms/action.rom")).unwrap(),
        )
        .unwrap();
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let load = vm
        .load_atari_object_for_execution(profile, &codegen::format_load_file(output))
        .unwrap();
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x7FF)
    );
    for address in 0x600..=0x7FF {
        vm.bus_mut().ram_mut().write(address, 0xCC);
    }
    // This is a fixed-address test entry in both runtime modes. Capture the
    // public Error ABI, then return with decimal mode set and A/X/Y clobbered.
    vm.bus_mut()
        .ram_mut()
        .map(0x04CB, &[0x4C, 0x80, 0x07])
        .unwrap();
    vm.bus_mut()
        .ram_mut()
        .map(
            0x0780,
            &[
                0x8D, 0x00, 0x07, 0x8E, 0x01, 0x07, 0x8C, 0x02, 0x07, 0xEE, 0x03, 0x07, 0xA9, 17,
                0xA2, 34, 0xA0, 51, 0xF8, 0x60,
            ],
        )
        .unwrap();
    vm.bus_mut().ram_mut().write(0x703, 0);
    let result = VmRunner::new(vm).run(RunRequest {
        max_steps: 4_000,
        history_len: 8,
        ..RunRequest::default()
    });
    assert_eq!(
        result.stop_reason(),
        StopReason::StepLimit { max_steps: 4_000 },
        "{lane}/{runtime:?}: {:?}",
        result.report
    );
    assert_eq!(
        (0x700..=0x703)
            .map(|a| result.memory().read(a))
            .collect::<Vec<_>>(),
        [105, 0, 105, 1],
        "{lane}/{runtime:?}: {:?}",
        result.report
    );
    let mut expected = vec![0xCC; 0x100];
    expected[0] = 41;
    assert_eq!(
        (0x600..=0x6FF)
            .map(|a| result.memory().read(a))
            .collect::<Vec<_>>(),
        expected
    );
    let registers = &result.report.registers;
    assert_eq!([registers.a, registers.x, registers.y], [105, 0, 105]);
    assert_eq!(registers.status & 8, 0, "wrapper must clear decimal mode");
    assert_eq!(
        [
            result.memory().read(registers.pc),
            result.memory().read(registers.pc + 1)
        ],
        [0xB0, 0xFE],
        "a returning Error handler must not resume source code"
    );
}

fn check_wrapper(body: &str) {
    let source = format!(
        r#"
BYTE state=$0600
PROC Entry=$04CB(BYTE code,x,y)
PROC Wrapper(BYTE code)
{body}
RETURN
PROC Main()
state=41
Wrapper(105)
state=42
DO OD
RETURN
"#
    );
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
    let model = semantic::analyze(&ast).unwrap();
    let semir = semantic::ir::lower_program(&ast, &model);
    let raw = nir::lower_program(&semir);
    let optimized = nir::optimize_program(&raw).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let classic = codegen::generate_semir_profile_at_origin_with_runtime(
            &semir,
            0x3000,
            codegen::CodegenProfile::Modern,
            runtime,
        )
        .unwrap();
        execute(&classic, runtime, "classic");
        for (lane, program) in [("raw MIR", &raw), ("optimized MIR", &optimized)] {
            let output = mir6502::generate_output_with_config_and_runtime(
                program,
                0x3000,
                &mir6502::Mir6502Config::optimized(),
                runtime,
            )
            .unwrap();
            execute(&output, runtime, lane);
        }
    }
}

#[test]
fn symbolic_absolute_error_entry_executes_in_both_runtimes() {
    check_wrapper(
        r#"
ASM OPAQUE
    pha
    ldx #0
    tay
    jsr Entry
    cld
    pla
    ldx #0
    tay
    sec
stopped: bcs stopped
ENDASM
"#,
    );
}

#[test]
fn repeated_error_arguments_after_opaque_assembly_preserve_the_error_abi() {
    check_wrapper(
        r#"
ASM OPAQUE
    lda code
    pha
ENDASM
Entry(code,0,code)
ASM OPAQUE
    cld
    pla
    ldx #0
    tay
    sec
stopped: bcs stopped
ENDASM
"#,
    );
}
