use actionc_vm::{CompilerVm, ExecutionProfile, RunRequest, StopReason, VmRunner};

#[test]
fn cartridge_error_reports_the_y_register() {
    // Original arbitrary/legacy codes and every actionc extension use the
    // unchanged cartridge formatter. A and X deliberately differ from Y.
    for code in [51, 100, 101, 102, 103, 104, 105, 106, 128, 136] {
        check_cartridge_error(code);
    }
}

fn check_cartridge_error(code: u8) {
    let mut vm = CompilerVm::default();
    vm.prepare_execution_profile(ExecutionProfile::OriginalCompiler)
        .unwrap();
    vm.reset_cpu();
    for _ in 0..1_000_000 {
        vm.step_cpu().unwrap();
    }
    assert_eq!(vm.bus().ram().read(0x04CB), 0x4C);
    vm.bus_mut()
        .ram_mut()
        .map(
            0x3000,
            &[
                0xA9, 0x11, // LDA #17
                0xA2, 0x22, // LDX #34
                0xA0, code, // LDY #reported code
                0x20, 0xCB, 0x04, // JSR Error
                0xA9, 0xA5, 0x8D, 0xFF, 0x06, // mark unexpected return
                0x4C, 0x0E, 0x30,
            ],
        )
        .unwrap();
    vm.set_pc(0x3000);
    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps: 300_000,
        // Pinned ROM: stop as SPLErr enters the monitor, before it clears the
        // diagnostic. MAIN.IO SysErr converts Y into the string at $0550.
        stop_after_pc: Some(0xB889),
        history_len: 8,
    });
    assert_eq!(
        outcome.stop_reason(),
        StopReason::PcReached { pc: 0xB889 },
        "Error must enter the monitor"
    );
    let text = code.to_string();
    let expected: Vec<_> = std::iter::once(text.len() as u8).chain(text.bytes()).collect();
    assert_eq!(
        (0x550..0x550 + expected.len() as u16)
            .map(|a| outcome.memory().read(a))
            .collect::<Vec<_>>(),
        expected,
        "Error({code})"
    );
    assert_ne!(outcome.memory().read(0x06FF), 0xA5);
}
