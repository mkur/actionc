#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use action_compiler_vm::{CompilerVm, ExecutionProfile, RunRequest, StopReason, VmRunner};
    use actionc::compiler::{CompileMode, CompileOptions, compile_file};

    const RESULT_START: u16 = 0x0600;
    const EXPECTED_RESULTS: [u8; 6] = [0x02, 0x22, 0x22, 0x05, 0x44, 0x44];

    fn initialized_arrays_fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures")
            .join("runtime")
            .join("initialized_arrays.act")
    }

    #[test]
    fn initialized_arrays_execute_through_the_vm_library() {
        let fixture = initialized_arrays_fixture();
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let compiled =
                compile_file(&fixture, &CompileOptions::for_mode(mode)).unwrap_or_else(|error| {
                    panic!("compile initialized arrays with {mode:?}: {error}")
                });
            let mut vm = CompilerVm::default();
            let load = vm
                .load_atari_object_for_execution(
                    ExecutionProfile::StandaloneObject,
                    compiled.object_bytes(),
                )
                .unwrap_or_else(|error| panic!("load initialized arrays with {mode:?}: {error}"));
            assert_eq!(load.run_address, Some(compiled.run_address()));

            let outcome = VmRunner::new(vm).run(RunRequest {
                max_steps: 200,
                history_len: 8,
                ..RunRequest::default()
            });
            assert_eq!(
                outcome.stop_reason(),
                StopReason::StepLimit { max_steps: 200 },
                "unexpected VM stop for {mode:?}: {:?}",
                outcome.report
            );
            let actual = std::array::from_fn(|offset| {
                outcome
                    .memory()
                    .read(RESULT_START.wrapping_add(offset as u16))
            });
            assert_eq!(actual, EXPECTED_RESULTS, "VM results for {mode:?}");
        }
    }
}
