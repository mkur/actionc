#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use action_compiler_vm::{
        CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
        StopReason, VmRunner,
    };
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};

    const RESULT_START: u16 = 0x0600;

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
