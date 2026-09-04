use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, RunRequest, StopReason, VmRunner};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn dynamic_word_pointer_loop_preserves_zero_page_and_word_boundaries() {
    let fixture = repository_root().join("fixtures/runtime/dynamic_word_pointer_loop.act");
    let compiled = compile_file(
        fixture,
        &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(Runtime::Standalone),
    )
    .expect("compile dynamic word pointer loop");
    let mut vm = CompilerVm::default();
    vm.load_atari_object_for_execution(ExecutionProfile::StandaloneObject, compiled.object_bytes())
        .expect("load dynamic word pointer loop");
    let max_steps = 300_000;
    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps,
        history_len: 8,
        ..RunRequest::default()
    });

    assert_eq!(
        outcome.stop_reason(),
        StopReason::StepLimit { max_steps },
        "unexpected stop: {:?}",
        outcome.report
    );
    let results = (0x0600..=0x0606)
        .map(|address| outcome.memory().read(address))
        .collect::<Vec<_>>();
    assert_eq!(results, [0, 1, 255, 0, 1, 44, 0xA5]);
}
