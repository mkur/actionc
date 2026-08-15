#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use action_compiler_vm::{CompilerVm, ExecutionProfile, RunRequest, StopReason, VmRunner};
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};

    const RESULT_START: u16 = 0x0600;

    struct MemoryExpectation<'a> {
        start: u16,
        bytes: &'a [u8],
    }

    fn runtime_fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures")
            .join("runtime")
            .join(name)
    }

    fn assert_runtime_case(
        label: &str,
        fixture: &str,
        mode: CompileMode,
        max_steps: u64,
        expectations: &[MemoryExpectation<'_>],
    ) {
        let fixture = runtime_fixture(fixture);
        let compiled = compile_file(&fixture, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile {label} with {mode:?}: {error}"));
        let mut vm = CompilerVm::default();
        let load = vm
            .load_atari_object_for_execution(
                ExecutionProfile::StandaloneObject,
                compiled.object_bytes(),
            )
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
}
