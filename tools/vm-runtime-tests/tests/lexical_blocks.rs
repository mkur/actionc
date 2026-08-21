use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture() -> PathBuf {
    repository_root().join("fixtures/runtime/lexical_blocks.act")
}

fn vm_for_runtime(runtime: Runtime) -> CompilerVm {
    let mut vm = CompilerVm::default();
    if runtime == Runtime::ActionCart {
        let roms = repository_root().join("roms");
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            DEFAULT_CART_BASE,
            std::fs::read(roms.join("action.rom")).expect("read Action! cartridge"),
        )
        .expect("load Action! cartridge");
        vm.load_image_bytes(
            ImageKind::Rom,
            "altirraos-xl.rom",
            OS_ROM_BASE,
            std::fs::read(roms.join("altirraos-xl.rom")).expect("read Atari OS ROM"),
        )
        .expect("load Atari OS ROM");
    }
    vm
}

#[test]
fn lexical_blocks_preserve_bindings_in_every_backend_and_runtime() {
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                fixture(),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| panic!("compile {mode:?}/{runtime:?}: {error}"));
            let profile = match runtime {
                Runtime::ActionCart => ExecutionProfile::CartridgeObject,
                Runtime::Standalone => ExecutionProfile::StandaloneObject,
            };
            let mut vm = vm_for_runtime(runtime);
            vm.load_atari_object_for_execution(profile, compiled.object_bytes())
                .unwrap_or_else(|error| panic!("load {mode:?}/{runtime:?}: {error}"));
            let outcome = VmRunner::new(vm).run(RunRequest {
                max_steps: 1_000,
                history_len: 8,
                ..RunRequest::default()
            });
            assert_eq!(
                outcome.stop_reason(),
                StopReason::StepLimit { max_steps: 1_000 },
                "unexpected stop for {mode:?}/{runtime:?}: {:?}",
                outcome.report
            );
            let actual = (0x0600..=0x0607)
                .map(|address| outcome.memory().read(address))
                .collect::<Vec<_>>();
            assert_eq!(
                actual,
                [10, 20, 30, 20, 10, 40, 10, 0xA5],
                "lexical results for {mode:?}/{runtime:?}"
            );
        }
    }
}
