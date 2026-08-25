#[cfg(test)]
mod atari_fpp_oracle;

#[cfg(test)]
mod sys_coverage;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
    use actionc_vm::{
        CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunOutcome,
        RunRequest, StopReason, VmRunner,
    };

    const RESULT_START: u16 = 0x0600;
    const ERROR_TRAP: u16 = 0x0700;

    struct MemoryExpectation<'a> {
        start: u16,
        bytes: &'a [u8],
    }

    fn runtime_fixture(name: &str) -> PathBuf {
        repository_root()
            .join("fixtures")
            .join("runtime")
            .join(name)
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn vm_for_profile(profile: ExecutionProfile) -> CompilerVm {
        let mut vm = CompilerVm::default();
        if profile == ExecutionProfile::CartridgeObject {
            let roms = repository_root().join("roms");
            let cartridge = std::fs::read(roms.join("action.rom")).expect("read Action! cartridge");
            let os = std::fs::read(roms.join("altirraos-xl.rom")).expect("read Atari OS ROM");
            vm.load_image_bytes(
                ImageKind::Cartridge,
                "action.rom",
                DEFAULT_CART_BASE,
                cartridge,
            )
            .expect("load Action! cartridge");
            vm.load_image_bytes(ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE, os)
                .expect("load Atari OS ROM");
        }
        vm
    }

    fn assert_runtime_case(
        label: &str,
        fixture: &str,
        mode: CompileMode,
        max_steps: u64,
        expectations: &[MemoryExpectation<'_>],
    ) {
        assert_runtime_case_with_profile(
            label,
            fixture,
            mode,
            ExecutionProfile::StandaloneObject,
            max_steps,
            expectations,
        );
    }

    fn assert_cartridge_runtime_case(
        label: &str,
        fixture: &str,
        mode: CompileMode,
        max_steps: u64,
        expectations: &[MemoryExpectation<'_>],
    ) {
        assert_runtime_case_with_profile(
            label,
            fixture,
            mode,
            ExecutionProfile::CartridgeObject,
            max_steps,
            expectations,
        );
    }

    fn assert_runtime_case_with_profile(
        label: &str,
        fixture: &str,
        mode: CompileMode,
        profile: ExecutionProfile,
        max_steps: u64,
        expectations: &[MemoryExpectation<'_>],
    ) {
        let fixture = runtime_fixture(fixture);
        let compiled = compile_file(&fixture, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile {label} with {mode:?}: {error}"));
        let mut vm = vm_for_profile(profile);
        let load = vm
            .load_atari_object_for_execution(profile, compiled.object_bytes())
            .unwrap_or_else(|error| panic!("load {label} with {mode:?}: {error}"));
        assert_eq!(load.run_address, Some(compiled.run_address()));

        let outcome = VmRunner::new(vm).run(RunRequest {
            max_steps,
            history_len: 8,
            ..RunRequest::default()
        });
        assert_eq!(
            outcome.stop_reason(),
            StopReason::StepLimit { max_steps },
            "unexpected VM stop for {label} with {mode:?}: {:?}",
            outcome.report
        );

        for expectation in expectations {
            let actual: Vec<_> = (0..expectation.bytes.len())
                .map(|offset| {
                    outcome
                        .memory()
                        .read(expectation.start.wrapping_add(offset as u16))
                })
                .collect();
            assert_eq!(
                actual, expectation.bytes,
                "VM memory at ${:04X} for {label} with {mode:?}",
                expectation.start
            );
        }
    }

    fn assert_both_backends(
        label: &str,
        fixture: &str,
        max_steps: u64,
        expectations: &[MemoryExpectation<'_>],
    ) {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            assert_runtime_case(label, fixture, mode, max_steps, expectations);
        }
    }

    fn assert_both_cartridge_backends(
        label: &str,
        fixture: &str,
        max_steps: u64,
        expectations: &[MemoryExpectation<'_>],
    ) {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            assert_cartridge_runtime_case(label, fixture, mode, max_steps, expectations);
        }
    }

    fn hex_bytes(hex: &str) -> Vec<u8> {
        hex.split_ascii_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).expect("valid expected byte"))
            .collect()
    }

    fn run_standalone_fixture(name: &str, mode: CompileMode, max_steps: u64) -> RunOutcome {
        run_runtime_fixture(name, mode, Runtime::Standalone, false, max_steps)
    }

    fn run_runtime_fixture(
        name: &str,
        mode: CompileMode,
        runtime: Runtime,
        load_os: bool,
        max_steps: u64,
    ) -> RunOutcome {
        run_runtime_fixture_with_setup(name, mode, runtime, load_os, max_steps, |_| {})
    }

    fn run_runtime_fixture_with_setup(
        name: &str,
        mode: CompileMode,
        runtime: Runtime,
        load_os: bool,
        max_steps: u64,
        setup: impl FnOnce(&mut CompilerVm),
    ) -> RunOutcome {
        let fixture = runtime_fixture(name);
        let compiled = compile_file(
            &fixture,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
        )
        .unwrap_or_else(|error| panic!("compile {runtime:?} {name} with {mode:?}: {error}"));
        let profile = match runtime {
            Runtime::ActionCart => ExecutionProfile::CartridgeObject,
            Runtime::Standalone => ExecutionProfile::StandaloneObject,
        };
        let mut vm = vm_for_profile(profile);
        if load_os && runtime == Runtime::Standalone {
            let os = repository_root().join("roms/altirraos-xl.rom");
            vm.load_image_bytes(
                ImageKind::Rom,
                "altirraos-xl.rom",
                OS_ROM_BASE,
                std::fs::read(os).expect("read Atari OS ROM"),
            )
            .expect("load Atari OS ROM for standalone runtime call");
        }
        vm.load_atari_object_for_execution(profile, compiled.object_bytes())
            .unwrap_or_else(|error| panic!("load {runtime:?} {name} with {mode:?}: {error}"));
        setup(&mut vm);
        VmRunner::new(vm).run(RunRequest {
            max_steps,
            history_len: 8,
            ..RunRequest::default()
        })
    }

    #[test]
    fn native_real_core_arithmetic_uses_atari_fpp_in_both_runtimes() {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let outcome =
                    run_runtime_fixture("native_real_core.act", mode, runtime, true, 10_000);
                let result = (0..6)
                    .map(|offset| outcome.memory().read(0x0600 + offset))
                    .collect::<Vec<_>>();
                assert_eq!(
                    result,
                    [0x40, 0x03, 0, 0, 0, 0],
                    "native REAL result for {mode:?}/{runtime:?}: {:?}",
                    outcome.report
                );
                for (address, expected) in [
                    (0x0606, [0x40, 0x03, 0x25, 0, 0, 0]),
                    (0x060C, [0xBF, 0x75, 0, 0, 0, 0]),
                    (0x0612, [0x40, 0x02, 0x50, 0, 0, 0]),
                    (0x0618, [0x3F, 0x62, 0x50, 0, 0, 0]),
                ] {
                    let actual = (0..6)
                        .map(|offset| outcome.memory().read(address + offset))
                        .collect::<Vec<_>>();
                    assert_eq!(actual, expected, "{mode:?}/{runtime:?} at ${address:04X}");
                }
            }
        }
    }

    #[test]
    fn native_real_assignment_is_overlap_safe() {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let outcome =
                    run_runtime_fixture("native_real_overlap.act", mode, runtime, true, 1_000);
                let destination = (0..6)
                    .map(|offset| outcome.memory().read(0x0602 + offset))
                    .collect::<Vec<_>>();
                assert_eq!(
                    destination,
                    [0x44, 0x12, 0x34, 0x56, 0x78, 0x90],
                    "{mode:?}/{runtime:?}"
                );
            }
        }
    }

    #[test]
    fn native_real_control_and_conversion_surface_runs_in_both_runtimes() {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let outcome =
                    run_runtime_fixture("native_real_control.act", mode, runtime, true, 100_000);
                let bytes = |address: u16, length: u16| {
                    (0..length)
                        .map(|offset| outcome.memory().read(address + offset))
                        .collect::<Vec<_>>()
                };
                assert_eq!(bytes(0x0600, 6), [0xC1, 0x01, 0x23, 0, 0, 0]);
                assert_eq!(bytes(0x0606, 6), [0x41, 0x01, 0x23, 0, 0, 0]);
                assert_eq!(bytes(0x060C, 2), [0x85, 0xFF]);
                assert_eq!(bytes(0x060E, 2), [0xFF, 0xFF]);
                assert_eq!(bytes(0x0610, 1), [0xFF]);
                assert_eq!(bytes(0x0611, 3), [1, 3, 65]);
                assert_eq!(bytes(0x0614, 6), [0, 1, 1, 1, 0, 0]);
                assert_eq!(bytes(0x061A, 4), [1, 1, 1, 1]);
                assert_eq!(bytes(0x061E, 2), [2, 0]);
            }
        }
    }

    #[test]
    fn native_real_aggregate_storage_runs_in_both_runtimes() {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let outcome =
                    run_runtime_fixture("native_real_storage.act", mode, runtime, true, 100_000);
                let bytes = |address: u16| {
                    (0..6)
                        .map(|offset| outcome.memory().read(address + offset))
                        .collect::<Vec<_>>()
                };
                for (address, expected) in [
                    (0x0600, [0x40, 0x04, 0x75, 0, 0, 0]),
                    (0x0606, [0x40, 0x01, 0x25, 0, 0, 0]),
                    (0x060C, [0xC0, 0x02, 0x50, 0, 0, 0]),
                    (0x0612, [0x40, 0x03, 0, 0, 0, 0]),
                    (0x0618, [0xC0, 0x04, 0x50, 0, 0, 0]),
                    (0x061E, [0x40, 0x05, 0x50, 0, 0, 0]),
                    (0x0624, [0xC0, 0x07, 0x50, 0, 0, 0]),
                    (0x062A, [0x40, 0x08, 0x50, 0, 0, 0]),
                    (0x0630, [0x40, 0x01, 0x25, 0, 0, 0]),
                    (0x0636, [0x40, 0x09, 0x25, 0, 0, 0]),
                ] {
                    assert_eq!(
                        bytes(address),
                        expected,
                        "{mode:?}/{runtime:?} at ${address:04X}"
                    );
                }
            }
        }
    }

    #[test]
    fn native_real_library_runs_in_both_backends_and_runtimes() {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let outcome =
                    run_runtime_fixture("native_real_library.act", mode, runtime, true, 1_000_000);
                let bytes = |address: u16, length: u16| {
                    (0..length)
                        .map(|offset| outcome.memory().read(address + offset))
                        .collect::<Vec<_>>()
                };
                let context = format!("{mode:?}/{runtime:?}: {:?}", outcome.report);
                assert_eq!(
                    bytes(0x0606, 6),
                    [0x3F, 0x99, 0x99, 0x99, 0x99, 0x98],
                    "{context}"
                );
                assert_eq!(
                    bytes(0x060C, 6),
                    [0x40, 0x99, 0x99, 0x99, 0x99, 0x98],
                    "{context}"
                );
                assert_eq!(
                    bytes(0x0612, 6),
                    [0x3B, 0x04, 0x60, 0x51, 0x70, 0x18],
                    "{context}"
                );
                assert_eq!(bytes(0x0618, 6), [0x40, 0x02, 0, 0, 0, 0], "{context}");
                assert_eq!(bytes(0x061E, 6), [0x40, 0x02, 0, 0, 0, 0], "{context}");
                assert_eq!(bytes(0x0624, 2), [1, b'2'], "{context}");
                assert_eq!(bytes(0x0638, 1), [0xA5], "{context}");
                assert_eq!(bytes(0x0700, 6), [0, 0, 0, 0, 0, 0], "{context}");
                assert_eq!(bytes(0x0706, 6), [0x40, 0x01, 0, 0, 0, 0], "{context}");
                assert_eq!(
                    bytes(0x070C, 6),
                    [0x40, 0x01, 0x41, 0x42, 0x13, 0x56],
                    "{context}"
                );
                assert_eq!(bytes(0x0712, 6), [0x40, 0x02, 0, 0, 0, 0], "{context}");
                assert_eq!(bytes(0x0718, 6), [0x3F, 0x01, 0, 0, 0, 0], "{context}");
                assert_eq!(bytes(0x071E, 6), [0x41, 0x01, 0, 0, 0, 0], "{context}");
            }
        }
    }

    #[test]
    fn initialized_arrays_execute_through_the_vm_library() {
        assert_both_backends(
            "initialized arrays",
            "initialized_arrays.act",
            200,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &[0x02, 0x22, 0x22, 0x05, 0x44, 0x44],
            }],
        );
    }

    #[test]
    fn selectively_linked_sargs_executes_without_a_cartridge() {
        let max_steps = 1_000;
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let outcome = run_standalone_fixture("standalone_sargs.act", mode, max_steps);
            assert_eq!(outcome.stop_reason(), StopReason::StepLimit { max_steps });
            assert_eq!(outcome.memory().read(RESULT_START), 4, "{mode:?}");
        }
    }

    #[test]
    fn selectively_linked_arithmetic_executes_without_a_cartridge() {
        let max_steps = 10_000;
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let outcome = run_standalone_fixture("standalone_arithmetic.act", mode, max_steps);
            assert_eq!(outcome.stop_reason(), StopReason::StepLimit { max_steps });
            assert_eq!(
                (0..10)
                    .map(|offset| outcome.memory().read(RESULT_START + offset))
                    .collect::<Vec<_>>(),
                hex_bytes("6c 7f 99 02 05 00 a0 91 46 02"),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn descending_for_steps_execute_in_all_public_modes() {
        let max_steps = 10_000;
        for mode in [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ] {
            let outcome = run_standalone_fixture("descending_for_steps.act", mode, max_steps);
            assert_eq!(
                outcome.stop_reason(),
                StopReason::StepLimit { max_steps },
                "{mode:?}: {:?}",
                outcome.report
            );
            assert_eq!(
                (0..7)
                    .map(|offset| outcome.memory().read(RESULT_START + offset))
                    .collect::<Vec<_>>(),
                [3, 2, 3, 2, 3, 2, 0xA5],
                "{mode:?}: {:?}",
                outcome.report
            );
        }
    }

    #[test]
    fn resident_sys_zero_matches_under_both_runtimes_and_backends() {
        let max_steps = 1_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "standalone_sys_memory.act",
                    mode,
                    runtime,
                    false,
                    max_steps,
                );
                assert_eq!(outcome.stop_reason(), StopReason::StepLimit { max_steps });
                assert_eq!(
                    (0..8)
                        .map(|offset| outcome.memory().read(RESULT_START + offset))
                        .collect::<Vec<_>>(),
                    vec![0; 8],
                    "{runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_sys_block_operations_match_under_both_runtimes_and_backends() {
        let max_steps = 2_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "standalone_sys_blocks.act",
                    mode,
                    runtime,
                    false,
                    max_steps,
                );
                assert_eq!(outcome.stop_reason(), StopReason::StepLimit { max_steps });
                for start in [0x0600, 0x0610] {
                    assert_eq!(
                        (0..4)
                            .map(|offset| outcome.memory().read(start + offset))
                            .collect::<Vec<_>>(),
                        vec![0x5A; 4],
                        "{runtime:?}/{mode:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn printh_implementations_print_hex_and_preserve_following_output() {
        let max_steps = 20_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome =
                    run_runtime_fixture("resident_printh.act", mode, runtime, true, max_steps);
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    outcome.vm.bus().cio_channel0_output(),
                    b"$1234\x9BX",
                    "following Put must work for {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_sys_strings_match_under_both_runtimes_and_backends() {
        let max_steps = 5_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "standalone_sys_strings_runtime.act",
                    mode,
                    runtime,
                    false,
                    max_steps,
                );
                assert_eq!(outcome.stop_reason(), StopReason::StepLimit { max_steps });
                assert_eq!(
                    (0..4)
                        .map(|offset| outcome.memory().read(0x0600 + offset))
                        .collect::<Vec<_>>(),
                    b"\x03ABC",
                    "SCopy with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    (0..3)
                        .map(|offset| outcome.memory().read(0x0610 + offset))
                        .collect::<Vec<_>>(),
                    b"\x02AB",
                    "SCopyS with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    outcome.memory().read(0x0620),
                    1,
                    "SCompare with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_sys_graphics_state_matches_under_both_runtimes_and_backends() {
        let max_steps = 2_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "standalone_sys_graphics_runtime.act",
                    mode,
                    runtime,
                    false,
                    max_steps,
                );
                assert_eq!(outcome.stop_reason(), StopReason::StepLimit { max_steps });
                assert_eq!(
                    outcome.memory().read(0x0054),
                    0x56,
                    "ROWCRS with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    outcome.memory().read(0x0055),
                    0x34,
                    "COLCRS low with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    outcome.memory().read(0x0056),
                    0x12,
                    "COLCRS high with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    outcome.memory().read(0x02C6),
                    0xAC,
                    "COLOR2 with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_sys_output_matches_under_both_runtimes_and_backends() {
        let max_steps = 20_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "standalone_sys_output_runtime.act",
                    mode,
                    runtime,
                    true,
                    max_steps,
                );
                assert_eq!(outcome.stop_reason(), StopReason::StepLimit { max_steps });
                assert_eq!(
                    outcome.vm.bus().cio_channel0_output(),
                    b"value=\x9B",
                    "CIO output with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_numeric_values_match_under_both_runtimes_and_backends() {
        let max_steps = 50_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "resident_numeric_values.act",
                    mode,
                    runtime,
                    false,
                    max_steps,
                );
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    (0..2)
                        .map(|offset| outcome.memory().read(0x0600 + offset))
                        .collect::<Vec<_>>(),
                    hex_bytes("00 ff"),
                    "ValB with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    (0..6)
                        .map(|offset| outcome.memory().read(0x0610 + offset))
                        .collect::<Vec<_>>(),
                    hex_bytes("00 00 d2 04 ff ff"),
                    "ValC with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    (0..8)
                        .map(|offset| outcome.memory().read(0x0620 + offset))
                        .collect::<Vec<_>>(),
                    hex_bytes("00 80 ff ff 00 00 ff 7f"),
                    "ValI with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_numeric_strings_match_under_both_runtimes_and_backends() {
        let max_steps = 100_000;
        let expected: &[(u16, &[u8])] = &[
            (0x0600, b"\x010"),
            (0x0610, b"\x03255"),
            (0x0620, b"\x010"),
            (0x0630, b"\x041234"),
            (0x0640, b"\x0565535"),
            (0x0650, b"\x06-32768"),
            (0x0660, b"\x02-1"),
            (0x0670, b"\x0532767"),
        ];
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "resident_numeric_strings.act",
                    mode,
                    runtime,
                    true,
                    max_steps,
                );
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                for &(start, bytes) in expected {
                    assert_eq!(
                        (0..bytes.len())
                            .map(|offset| outcome.memory().read(start + offset as u16))
                            .collect::<Vec<_>>(),
                        bytes,
                        "string at ${start:04X} with {runtime:?}/{mode:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn resident_numeric_output_matches_under_both_runtimes_and_backends() {
        let max_steps = 150_000;
        let expected = [
            &b"0|255\x9B"[..],
            &b"42|7\x9B"[..],
            &b"0|65535\x9B"[..],
            &b"1234|42\x9B"[..],
            &b"-1|32767\x9B"[..],
            &b"-1234|-32768\x9B"[..],
        ]
        .concat();
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "resident_numeric_output.act",
                    mode,
                    runtime,
                    true,
                    max_steps,
                );
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    outcome.vm.bus().cio_channel0_output(),
                    expected.as_slice(),
                    "numeric CIO output with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_console_input_matches_under_both_runtimes_and_backends() {
        let max_steps = 200_000;
        let input =
            b"42\x9B255\x9B1234\x9B65535\x9B-1234\x9B-32768\x9BZABC\x9BDEFG\x9BWXYZ\x9BLAST\x9B";
        let expected: &[(u16, &[u8])] = &[
            (0x0600, &[42, 255, b'Z']),
            (0x0610, &[0xD2, 0x04, 0xFF, 0xFF]),
            (0x0620, &[0x2E, 0xFB, 0x00, 0x80]),
            (0x0640, b"\x03ABC"),
            (0x0650, b"\x04DEFG"),
            (0x0660, b"\x04WXYZ"),
            (0x0670, b"\x04LAST"),
        ];
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture_with_setup(
                    "resident_console_input.act",
                    mode,
                    runtime,
                    true,
                    max_steps,
                    |vm| vm.bus_mut().queue_scripted_cio_input_bytes(input),
                );
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                for &(start, bytes) in expected {
                    assert_eq!(
                        (0..bytes.len())
                            .map(|offset| outcome.memory().read(start + offset as u16))
                            .collect::<Vec<_>>(),
                        bytes,
                        "input result at ${start:04X} with {runtime:?}/{mode:?}"
                    );
                }
                assert_eq!(outcome.vm.bus().cio_summary().opens, 1);
                assert_eq!(outcome.vm.bus().cio_summary().closes, 1);
                assert_eq!(outcome.vm.bus().cio_summary().reads, 11);
                assert_eq!(
                    outcome.vm.bus().cio_summary().bytes_read,
                    input.len() as u64
                );
            }
        }
    }

    #[test]
    fn resident_device_io_matches_under_both_runtimes_and_backends() {
        let max_steps = 150_000;
        let expected: &[(u16, &[u8])] = &[
            (0x0600, b"\x05FIRST"),
            (0x0610, b"\x06SECOND"),
            (0x0620, b"\x06SECOND"),
            (0x0630, &[0x00, 0x00, 0x06]),
        ];
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture_with_setup(
                    "resident_device_io.act",
                    mode,
                    runtime,
                    true,
                    max_steps,
                    |vm| {
                        vm.bus_mut()
                            .add_host_file("INPUT.TXT", b"FIRST\nSECOND\n".to_vec());
                        vm.bus_mut().add_host_output("OUTPUT.TXT");
                    },
                );
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                for &(start, bytes) in expected {
                    assert_eq!(
                        (0..bytes.len())
                            .map(|offset| outcome.memory().read(start + offset as u16))
                            .collect::<Vec<_>>(),
                        bytes,
                        "device-I/O result at ${start:04X} with {runtime:?}/{mode:?}"
                    );
                }
                assert_eq!(
                    outcome.vm.bus().host_file_bytes("OUTPUT.TXT"),
                    Some(&b"OUT\x9B"[..]),
                    "host output with {runtime:?}/{mode:?}"
                );
                let summary = outcome.vm.bus().cio_summary();
                assert_eq!(summary.opens, 2);
                assert_eq!(summary.closes, 2);
                assert_eq!(summary.statuses, 1);
                assert_eq!(summary.reads, 3);
                assert_eq!(summary.writes, 2);
                assert_eq!(summary.bytes_read, 20);
                assert_eq!(summary.bytes_written, 4);
                assert!(
                    outcome
                        .vm
                        .bus()
                        .cio_observations()
                        .iter()
                        .any(|observation| observation.command == 0x26 && observation.handled),
                    "NOTE must be handled for {runtime:?}/{mode:?}"
                );
                assert!(
                    outcome
                        .vm
                        .bus()
                        .cio_observations()
                        .iter()
                        .any(|observation| observation.command == 0x25 && observation.handled),
                    "POINT must be handled for {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_graphics_io_matches_under_both_runtimes_and_backends() {
        let max_steps = 100_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome =
                    run_runtime_fixture("resident_graphics_io.act", mode, runtime, true, max_steps);
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    (0..4)
                        .map(|offset| outcome.memory().read(0x0600 + offset))
                        .collect::<Vec<_>>(),
                    [3, 5, 7, 0xA6],
                    "graphics results with {runtime:?}/{mode:?}"
                );
                assert_eq!(outcome.vm.bus().graphics_mode(), Some(0x1C));
                assert_eq!(outcome.vm.bus().graphics_pixel(10, 20), 5);
                assert_eq!(outcome.vm.bus().graphics_pixel(11, 21), 5);
                assert_eq!(outcome.vm.bus().graphics_pixel(13, 23), 7);
                let summary = outcome.vm.bus().cio_summary();
                assert_eq!(summary.opens, 2);
                assert_eq!(summary.closes, 2);
                assert_eq!(summary.reads, 3);
                assert_eq!(summary.writes, 1);
                for command in [0x11, 0x12] {
                    assert!(
                        outcome
                            .vm
                            .bus()
                            .cio_observations()
                            .iter()
                            .any(|observation| {
                                observation.command == command && observation.handled
                            }),
                        "graphics command ${command:02X} must be handled for {runtime:?}/{mode:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn resident_formatted_output_matches_under_both_runtimes_and_backends() {
        let max_steps = 100_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "resident_formatted_output.act",
                    mode,
                    runtime,
                    true,
                    max_steps,
                );
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    outcome.vm.bus().cio_channel0_output(),
                    b"ABC\x9BD\x9B%|TXT|1234|-1|$BEEF\x9B",
                    "formatted output with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_memory_and_string_helpers_match_under_both_runtimes_and_backends() {
        let max_steps = 50_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture(
                    "resident_memory_strings.act",
                    mode,
                    runtime,
                    false,
                    max_steps,
                );
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    (0..6)
                        .map(|offset| outcome.memory().read(0x0600 + offset))
                        .collect::<Vec<_>>(),
                    b"\x05AxyDE",
                    "SAssign result with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    (0..3)
                        .map(|offset| outcome.memory().read(0x0620 + offset))
                        .collect::<Vec<_>>(),
                    [0x34, 0xEF, 0xBE],
                    "Peek/Poke result with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn resident_hardware_helpers_match_under_both_runtimes_and_backends() {
        let max_steps = 50_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let outcome = run_runtime_fixture_with_setup(
                    "resident_hardware_helpers.act",
                    mode,
                    runtime,
                    false,
                    max_steps,
                    |vm| {
                        vm.bus_mut().write(0xD20A, 0x80);
                        vm.bus_mut().ram_mut().write(0x0272, 0x66);
                        vm.bus_mut().write(0xD300, 0xA5);
                        vm.bus_mut().write(0xD012, 0x7F);
                    },
                );
                assert_eq!(
                    outcome.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected VM stop for {runtime:?}/{mode:?}: {:?}",
                    outcome.report
                );
                assert_eq!(
                    (0..11)
                        .map(|offset| outcome.memory().read(0x0600 + offset))
                        .collect::<Vec<_>>(),
                    [5, 0x66, 4, 0, 5, 10, 0x7F, 0x34, 0xA7, 0, 0],
                    "hardware helper results with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    fn run_exception_fixture(
        fixture: &str,
        mode: CompileMode,
        runtime: Runtime,
        max_steps: u64,
    ) -> RunOutcome {
        run_runtime_fixture_with_setup(fixture, mode, runtime, false, max_steps, |vm| {
            let spin = ERROR_TRAP + 14;
            let [spin_low, spin_high] = spin.to_le_bytes();
            vm.bus_mut()
                .ram_mut()
                .map(
                    ERROR_TRAP,
                    &[
                        0x8D, 0x00, 0x06, // STA $0600
                        0x8E, 0x01, 0x06, // STX $0601
                        0x8C, 0x02, 0x06, // STY $0602
                        0xA9, 0xA5, // LDA #$A5
                        0x8D, 0x03, 0x06, // STA $0603
                        0x4C, spin_low, spin_high, // JMP spin
                    ],
                )
                .expect("install error-vector trap");
            match runtime {
                Runtime::ActionCart => vm
                    .bus_mut()
                    .ram_mut()
                    .map(0x04CB, &[0x4C, 0x00, 0x07])
                    .expect("redirect cartridge Error entry"),
                Runtime::Standalone => vm.bus_mut().ram_mut().write_word(0x000A, ERROR_TRAP),
            }
        })
    }

    #[test]
    fn resident_exception_entries_match_under_both_runtimes_and_backends() {
        let max_steps = 2_000;
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let error = run_exception_fixture("resident_error.act", mode, runtime, max_steps);
                assert_eq!(
                    error.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected Error stop for {runtime:?}/{mode:?}: {:?}",
                    error.report
                );
                assert_eq!(
                    (0..4)
                        .map(|offset| error.memory().read(0x0600 + offset))
                        .collect::<Vec<_>>(),
                    [0x11, 0x22, 0x33, 0xA5],
                    "Error context with {runtime:?}/{mode:?}"
                );

                let break_call =
                    run_exception_fixture("resident_break.act", mode, runtime, max_steps);
                assert_eq!(
                    break_call.stop_reason(),
                    StopReason::StepLimit { max_steps },
                    "unexpected Break stop for {runtime:?}/{mode:?}: {:?}",
                    break_call.report
                );
                assert_eq!(
                    break_call.memory().read(0x0600),
                    0x80,
                    "Break error code with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    break_call.memory().read(0x0602),
                    0x80,
                    "Break Y context with {runtime:?}/{mode:?}"
                );
                assert_eq!(
                    break_call.memory().read(0x0603),
                    0xA5,
                    "Break trap marker with {runtime:?}/{mode:?}"
                );
            }
        }
    }

    #[test]
    fn kalscope_backend_contracts_execute_through_the_vm_library() {
        assert_both_backends(
            "KALSCOPE backend contracts",
            "kalscope_backend_contracts.act",
            120,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &[0x12, 0x34, 0x82, 0x84],
            }],
        );
    }

    #[test]
    fn direct_word_compares_execute_through_the_vm_library() {
        assert_both_backends(
            "direct word compares",
            "direct_word_compare_runtime.act",
            2_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &[
                    0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x01, 0x00,
                ],
            }],
        );
    }

    #[test]
    fn direct_byte_array_indexes_execute_through_the_vm_library() {
        let expected = hex_bytes("a5 a4 da 25 5a d1 d2 e1 e2 a5 00 00 01 00 ff ff 00 00");
        assert_both_backends(
            "direct BYTE array indexes",
            "direct_byte_array_indexes.act",
            800_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn scaled_card_indexes_execute_through_the_vm_library() {
        let expected = hex_bytes(
            "00 11 01 22 7f 33 80 44 ff 55 80 44 7f 33 ff 55 7f a1 80 66 ff 77 7f 88 \
             80 99 80 aa 01 6a 7f ff ef be",
        );
        assert_runtime_case(
            "scaled CARD indexes",
            "scaled_card_indexes.act",
            CompileMode::Optimized,
            1_600,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn ordered_absolute_sub_executes_through_the_vm_library() {
        let expected = hex_bytes("a9 4e 0f");
        assert_runtime_case(
            "ordered absolute subtraction",
            "ordered_absolute_sub_runtime.act",
            CompileMode::Mir6502,
            1_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn paired_word_arithmetic_compare_executes_through_the_vm_library() {
        let expected = hex_bytes("01 01 00 00 00 01 01 00 01 00 a5");
        assert_both_backends(
            "paired word arithmetic compare",
            "paired_word_arithmetic_compare.act",
            5_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn direct_action_word_arithmetic_args_execute_through_the_vm_library() {
        let expected = hex_bytes(
            "34 12 00 01 45 23 ff 00 56 34 00 00 00 01 67 45 ff 00 78 56 00 01 89 67 \
             9a 78 00 01 a5",
        );
        assert_both_cartridge_backends(
            "direct Action word arithmetic arguments",
            "direct_action_word_arithmetic_args.act",
            5_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn indexed_byte_fixed_action_args_execute_through_the_vm_library() {
        let expected = hex_bytes("34 12 55 00 cd ab 11 00 22 00 33 00 44 00 55 00 a5");
        assert_both_cartridge_backends(
            "indexed BYTE fixed Action arguments",
            "indexed_byte_fixed_action_args.act",
            12_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn dual_indexed_word_compares_execute_through_the_vm_library() {
        let expected = hex_bytes("01 01 01 01 00 00 00 00 01 a5");
        assert_both_cartridge_backends(
            "dual indexed word compares",
            "dual_indexed_word_compares.act",
            2_500,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn indirect_call_fields_execute_through_the_vm_library() {
        let expected = hex_bytes("c8 02 11 11 fe 70 34 12 cd ab 22 22 a3 00 78 56 bc 9a");
        assert_cartridge_runtime_case(
            "indirect call fields",
            "indirect_call_fields.act",
            CompileMode::Mir6502,
            3_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn dual_pointer_word_transfers_execute_through_the_vm_library() {
        let shared =
            hex_bytes("34 12 78 56 ef be fe ca 57 13 68 24 00 01 68 24 03 05 03 05 00 80 00 00");
        assert_cartridge_runtime_case(
            "dual-pointer word transfers",
            "dual_pointer_word_transfers.act",
            CompileMode::Optimized,
            2_200,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &shared,
            }],
        );

        let mir6502 = hex_bytes(
            "34 12 78 56 ef be fe ca 57 13 68 24 00 01 68 24 03 05 03 05 00 80 00 00 \
             34 12 01 02 01 02",
        );
        assert_cartridge_runtime_case(
            "dual-pointer word transfers",
            "dual_pointer_word_transfers.act",
            CompileMode::Mir6502,
            2_200,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &mir6502,
            }],
        );
    }

    #[test]
    fn kalscope_codegen_patterns_execute_through_the_vm_library() {
        let results = hex_bytes("11 22 33 44 af 45 af 45 82 84 1f");
        assert_both_cartridge_backends(
            "KALSCOPE codegen patterns",
            "kalscope_codegen_patterns.act",
            40_000,
            &[
                MemoryExpectation {
                    start: RESULT_START,
                    bytes: &results,
                },
                MemoryExpectation {
                    start: 0x0610,
                    bytes: &[0xA5],
                },
            ],
        );
    }

    #[test]
    fn turtle_word_placement_executes_through_the_vm_library() {
        let expected = hex_bytes("58 1b 01 00 40 1f 34 08 01 01 a5 7d 00 04");
        assert_both_cartridge_backends(
            "TURTLE word placement",
            "turtle_word_placement.act",
            10_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn allocate_executes_through_the_vm_library() {
        let expected = hex_bytes(
            "a1 09 00 00 00 70 00 00 10 71 10 00 08 74 00 74 08 00 80 75 08 00 00 76 \
             30 00 00 00 10 78 20 00 00 00 30 79 00 79 00 00 00 7a 00 01 00 00",
        );
        assert_both_cartridge_backends(
            "ALLOCATE",
            "allocate_runtime.act",
            6_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn sort_executes_through_the_vm_library() {
        let expected = hex_bytes(
            "a5 0b d1 d2 c2 c1 c4 c3 d2 d1 d4 d3 e2 e1 e4 e3 \
             80 00 ff \
             00 00 01 7f 80 80 ff \
             ff 80 80 7f 01 00 00 \
             00 00 01 7f 80 80 ff \
             00 00 ff 00 ff 00 00 01 ff 7f 00 80 ff ff \
             ff ff 00 80 ff 7f 00 01 ff 00 ff 00 00 00 \
             00 80 ff ff ff ff 00 00 01 00 ff 7f \
             ff 7f 01 00 00 00 ff ff ff ff 00 80 \
             01 41 02 41 02 41 03 41 01 42 \
             01 42 03 41 02 41 02 41 01 41",
        );
        assert_both_cartridge_backends(
            "SORT",
            "sort_runtime.act",
            200_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn signed_return_word_zero_compares_execute_through_the_vm_library() {
        let expected = hex_bytes(
            "01 01 00 00 00 00 01 01 01 01 00 00 00 00 01 01 00 01 00 01 00 01 00 01 \
             00 00 01 01 01 01 00 00 00 00 01 01 01 01 00 00 a5",
        );
        assert_cartridge_runtime_case(
            "signed return-word zero compares",
            "signed_return_word_zero_compares.act",
            CompileMode::Mir6502,
            5_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn signed_word_relation_matrix_executes_through_the_vm_library() {
        let expected = hex_bytes(
            "00 01 00 01 01 01 00 00 01 01 00 00 01 01 00 00 01 01 00 00 \
             00 00 01 01 00 01 00 01 01 01 00 00 01 01 00 00 01 01 00 00 \
             00 00 01 01 00 00 01 01 00 01 00 01 01 01 00 00 01 01 00 00 \
             00 00 01 01 00 00 01 01 00 00 01 01 00 01 00 01 01 01 00 00 \
             00 00 01 01 00 00 01 01 00 00 01 01 00 00 01 01 00 01 00 01 a5",
        );
        assert_cartridge_runtime_case(
            "signed word relation matrix",
            "signed_word_relation_matrix.act",
            CompileMode::Mir6502,
            12_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &expected,
            }],
        );
    }

    #[test]
    fn circle_int_math_executes_through_the_vm_library() {
        let classic = hex_bytes(
            "40 9c ff 7f 00 00 80 7f 00 02 80 05 80 01 01 00 00 00 00 01 01 00 01 01 \
             00 01 00 00 00 a5",
        );
        assert_cartridge_runtime_case(
            "CIRCLE INT arithmetic",
            "circle_int_math.act",
            CompileMode::Optimized,
            20_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &classic,
            }],
        );

        let mir6502 = hex_bytes(
            "40 9c ff 7f 00 00 80 7f 00 02 80 05 80 01 01 00 00 00 00 01 01 01 00 01 \
             00 01 00 01 00 a5",
        );
        assert_cartridge_runtime_case(
            "CIRCLE INT arithmetic",
            "circle_int_math.act",
            CompileMode::Mir6502,
            20_000,
            &[MemoryExpectation {
                start: RESULT_START,
                bytes: &mir6502,
            }],
        );
    }
}
