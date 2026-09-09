use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::{Path, PathBuf};

const SAMPLE: &str = include_str!("../../../samples/long-integers/long-integers.act");
const EXPECTED: &str = "\
LONGCARD / LONGINT (32-bit)

LONGCARD: unsigned
0! = 1
12! = 479001600
F(0) = 0
F(47) = 2971215073
maximum = 4294967295

LONGINT: signed
-F(46) = -1836311903
minimum = -2147483648
maximum = 2147483647
-70000 / 3 = -23333
-70000 MOD 3 = -1

Widen before arithmetic:
CARD $FFFF+1 = 0
LONGCARD($FFFF)+1 = 65536
";

struct Source(PathBuf);

impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn long_integers_sample_prints_documented_results_in_both_runtimes() {
    // Keep the completed demo in a loop instead of returning into a host shell.
    // All computations and the real runtime printing calls remain unchanged.
    let end = SAMPLE.rfind("RETURN").unwrap();
    let source = Source(std::env::temp_dir().join(format!(
        "actionc-long-integers-sample-{}.act",
        std::process::id()
    )));
    std::fs::write(
        &source.0,
        format!("{} DO OD\n{}", &SAMPLE[..end], &SAMPLE[end..]),
    )
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let expected: Vec<_> = EXPECTED
        .bytes()
        .map(|byte| if byte == b'\n' { 0x9B } else { byte })
        .collect();

    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(runtime),
        )
        .unwrap_or_else(|error| panic!("compile {runtime:?}: {error}"));
        let mut vm = CompilerVm::default();
        vm.load_image_bytes(
            ImageKind::Rom,
            "altirraos-xl.rom",
            OS_ROM_BASE,
            std::fs::read(root.join("roms/altirraos-xl.rom")).unwrap(),
        )
        .unwrap();
        let profile = if runtime == Runtime::ActionCart {
            vm.load_image_bytes(
                ImageKind::Cartridge,
                "action.rom",
                DEFAULT_CART_BASE,
                std::fs::read(root.join("roms/action.rom")).unwrap(),
            )
            .unwrap();
            ExecutionProfile::CartridgeObject
        } else {
            ExecutionProfile::StandaloneObject
        };
        vm.load_atari_object_for_execution(profile, compiled.object_bytes())
            .unwrap();
        let max_steps = 500_000;
        let outcome = VmRunner::new(vm).run(RunRequest {
            max_steps,
            history_len: 8,
            ..Default::default()
        });
        assert_eq!(
            outcome.stop_reason(),
            StopReason::StepLimit { max_steps },
            "{runtime:?}: {:?}",
            outcome.report
        );
        assert_eq!(
            outcome.vm.bus().cio_channel0_output(),
            expected,
            "sample output with {runtime:?}: {:?}",
            outcome.report
        );
    }
}
