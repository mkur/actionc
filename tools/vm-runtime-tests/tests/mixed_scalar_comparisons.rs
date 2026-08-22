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
    repository_root().join("fixtures/runtime/mixed_scalar_comparisons.act")
}

fn cartridge_vm() -> CompilerVm {
    let mut vm = CompilerVm::default();
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
    vm
}

#[test]
fn mixed_scalar_comparisons_match_original_action() {
    let expected = [0x0E, 0x32, 0x32, 0x0E, 0x0E, 0x32, 0x0E, 0x32, 0xA5];
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                fixture(),
                &CompileOptions::for_mode(mode).with_runtime(runtime),
            )
            .unwrap_or_else(|error| panic!("compile {runtime:?}/{mode:?}: {error}"));
            let (mut vm, profile) = match runtime {
                Runtime::ActionCart => (cartridge_vm(), ExecutionProfile::CartridgeObject),
                Runtime::Standalone => {
                    (CompilerVm::default(), ExecutionProfile::StandaloneObject)
                }
            };
            vm.load_atari_object_for_execution(profile, compiled.object_bytes())
                .unwrap_or_else(|error| panic!("load {runtime:?}/{mode:?}: {error}"));
            let outcome = VmRunner::new(vm).run(RunRequest {
                max_steps: 20_000,
                history_len: 8,
                ..RunRequest::default()
            });
            assert_eq!(
                outcome.stop_reason(),
                StopReason::StepLimit { max_steps: 20_000 },
                "unexpected stop for {runtime:?}/{mode:?}: {:?}",
                outcome.report
            );
            let actual = (0x0600..=0x0608)
                .map(|address| outcome.memory().read(address))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{runtime:?}/{mode:?}");
        }
    }
}
