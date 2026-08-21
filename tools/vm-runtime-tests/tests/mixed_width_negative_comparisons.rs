use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, RunRequest, StopReason, VmRunner};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/runtime/mixed_width_negative_comparisons.act")
}

#[test]
fn negative_literal_comparisons_match_in_classic_and_mir6502() {
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let compiled = compile_file(
            fixture(),
            &CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone),
        )
        .unwrap_or_else(|error| panic!("compile {mode:?}: {error}"));
        let mut vm = CompilerVm::default();
        vm.load_atari_object_for_execution(
            ExecutionProfile::StandaloneObject,
            compiled.object_bytes(),
        )
        .unwrap_or_else(|error| panic!("load {mode:?}: {error}"));
        let outcome = VmRunner::new(vm).run(RunRequest {
            max_steps: 2_000,
            history_len: 8,
            ..RunRequest::default()
        });
        assert_eq!(
            outcome.stop_reason(),
            StopReason::StepLimit { max_steps: 2_000 },
            "unexpected stop for {mode:?}: {:?}",
            outcome.report
        );
        let actual = (0x0600..=0x0604)
            .map(|address| outcome.memory().read(address))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            [1, 0, 0, 0, 0xA5],
            "comparison results for {mode:?}"
        );
    }
}
