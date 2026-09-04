use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, ExecutionProfile, RunRequest, StopReason, VmRunner};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn unrolled_carry_driven_crc8_retains_source_semantics() {
    let fixture = repository_root().join("fixtures/runtime/small_counted_crc8.act");
    let compiled = compile_file(
        fixture,
        &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(Runtime::Standalone),
    )
    .expect("compile carry-driven CRC8 loop");
    let mut vm = CompilerVm::default();
    vm.load_atari_object_for_execution(ExecutionProfile::StandaloneObject, compiled.object_bytes())
        .expect("load carry-driven CRC8 loop");
    let max_steps = 100_000;
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
    assert_eq!(outcome.memory().read(0x0600), 0x37);
}
