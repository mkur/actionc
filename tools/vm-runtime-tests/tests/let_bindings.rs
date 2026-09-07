use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn execute(image: &[u8], runtime: Runtime) -> Vec<u8> {
    let mut vm = CompilerVm::default();
    let profile = if runtime == Runtime::ActionCart {
        for (kind, name, base) in [
            (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
            (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
        ] {
            vm.load_image_bytes(
                kind,
                name,
                base,
                std::fs::read(root().join("roms").join(name)).unwrap(),
            )
            .unwrap();
        }
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    vm.load_atari_object_for_execution(profile, image).unwrap();
    let result = VmRunner::new(vm).run(RunRequest {
        max_steps: 30_000,
        history_len: 16,
        ..RunRequest::default()
    });
    assert_eq!(
        result.stop_reason(),
        StopReason::StepLimit { max_steps: 30_000 },
        "{:?}",
        result.report
    );
    (0x0600..0x060D)
        .map(|address| result.memory().read(address))
        .collect()
}

#[test]
fn let_initializes_on_execution_and_preserves_snapshots_across_calls_and_loops() {
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                root().join("fixtures/runtime/let_bindings.act"),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|e| panic!("{mode:?}/{runtime:?}: {e}"));
            assert_eq!(
                execute(compiled.object_bytes(), runtime),
                [10, 11, 101, 10, 7, 8, 9, 11, 6, 42, 42, 7, 0xA5],
                "{mode:?}/{runtime:?}"
            );
        }
    }
}
