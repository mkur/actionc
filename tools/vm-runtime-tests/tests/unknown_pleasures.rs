//! Compile the beginner sample with the real cartridge and compare its picture.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    ACTION_MONITOR_KEY_CODE, CompilerVm, CpuStep, ExecutionProfile, Hotpatch, PcTrigger,
    RunOutcome, RunRequest, ScheduledAction, ScheduledActions, StopReason, VmRunHooks, VmRunner,
};
use std::path::{Path, PathBuf};

const SAMPLE: &str =
    include_str!("../../../samples/graphics/unknown-pleasures/PLEASURE.ACT");
const DATA: &str = include_str!("../../../samples/graphics/unknown-pleasures/UPDATA.ACT");
const CHECKPOINT: u16 = 0x0700;

fn atascii(text: &str) -> Vec<u8> {
    text.bytes()
        .map(|b| if b == b'\n' { 0x9B } else { b })
        .collect()
}

struct Monitor(ScheduledActions);
impl VmRunHooks for Monitor {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        self.0.apply_before_step(vm).map(|_| ())
    }
    fn stop_reason(&self, vm: &CompilerVm, step: &CpuStep) -> Option<StopReason> {
        (self.0.pending().is_empty() && vm.bus().scripted_cio_input_is_idle())
            .then_some(StopReason::ScriptedInputIdle { pc: step.pc })
    }
}

fn cartridge_compile(source: &str) -> Vec<u8> {
    let mut vm = CompilerVm::default();
    vm.prepare_execution_profile(ExecutionProfile::OriginalCompiler)
        .unwrap();
    vm.apply_hotpatch(Hotpatch::ActionQueuedInput).unwrap();
    vm.apply_hotpatch(Hotpatch::ActionHeadlessGetkey).unwrap();
    vm.add_host_file_bytes(
        "PLEASURE.ACT",
        atascii(&format!("SET $E=$2C00\nSET $491=$2C00\n{source}")),
    );
    vm.add_host_file_bytes("UPDATA.ACT", atascii(DATA));
    vm.add_host_output("PLEASURE.COM");
    vm.reset_cpu();
    let mut monitor = Monitor(ScheduledActions::new([
        ScheduledAction::queue_key_code(PcTrigger::at(0xA2E0), ACTION_MONITOR_KEY_CODE),
        ScheduledAction::queue_cio_input(
            PcTrigger::at_after(0xB2F5, 0xA2E0),
            atascii("C \"H:PLEASURE.ACT\"\nW \"H:PLEASURE.COM\"\n"),
        ),
    ]));
    let outcome = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: 20_000_000,
                history_len: 12,
                ..RunRequest::default()
            },
            &mut monitor,
        )
        .unwrap();
    assert!(
        matches!(outcome.stop_reason(), StopReason::ScriptedInputIdle { .. }),
        "{:?}",
        outcome.report
    );
    let output = outcome.vm.bus().decoded_cio_channel0_output();
    assert!(!output.to_ascii_lowercase().contains("error"), "{output}");
    let object = outcome
        .vm
        .host_file_bytes("PLEASURE.COM")
        .expect("cartridge must write the object");
    assert!(object.len() > 15000, "{output}");
    object.to_vec()
}

fn checkpoint(vm: CompilerVm) -> RunOutcome {
    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps: 20_000_000,
        stop_after_pc: Some(CHECKPOINT),
        history_len: 12,
    });
    assert_eq!(
        outcome.stop_reason(),
        StopReason::PcReached { pc: CHECKPOINT },
        "{:?}",
        outcome.report
    );
    outcome
}

fn render(object: &[u8], runtime: Runtime) -> Vec<u8> {
    let mut vm = CompilerVm::default();
    vm.load_bundled_altirra_os().unwrap();
    let profile = if runtime == Runtime::ActionCart {
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let loaded = vm.load_atari_object_for_execution(profile, object).unwrap();
    assert_eq!(loaded.segments[0].start, 0x2C00);
    assert!(loaded.segments.iter().all(|s| s.end < 0x7000));
    assert!(
        loaded
            .segments
            .iter()
            .all(|s| s.end < CHECKPOINT || s.start > CHECKPOINT + 1)
    );
    // NOP stops the runner; RTS resumes the real program on the next run.
    vm.bus_mut()
        .ram_mut()
        .map(CHECKPOINT, &[0xEA, 0x60])
        .unwrap();
    let outcome = checkpoint(vm);
    assert_eq!(outcome.memory().read(0x3AB), 24);
    assert_eq!(outcome.memory().read(0x2C5), 14); // Graphics 8 white luminance.
    assert_eq!(outcome.memory().read(0x2C6), 0); // Black background.
    let mut pixels = Vec::new();
    for y in 0..=255 {
        for x in 0..320 {
            let pixel = outcome.vm.bus().graphics_pixel(x, y);
            if y < 192 {
                assert!(pixel <= 1);
                pixels.push(pixel);
            } else {
                assert_eq!(pixel, 0, "pixel outside the screen at ({x}, {y})");
            }
        }
    }
    let lit = pixels.iter().filter(|&&p| p == 1).count();
    assert!(lit > 10000, "only {lit} lit pixels");
    for y in 0..192 {
        assert!(pixels[y * 320..y * 320 + 10].iter().all(|&p| p == 0));
        assert!(pixels[y * 320 + 310..(y + 1) * 320].iter().all(|&p| p == 0));
    }
    let mut vm = outcome.vm;
    vm.bus_mut().ram_mut().write(0x02FC, 0); // A new key releases the hold loop.
    let exited = checkpoint(vm);
    assert_eq!(exited.memory().read(0x3AB), 0); // Graphics(0) restores text mode.
    assert_eq!(exited.memory().read(0x004D), 0);
    pixels
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn beginner_unknown_pleasures_matches_original_cartridge_in_all_backends_and_runtimes() {
    let heights: Vec<u8> = DATA
        .split_once("=[")
        .unwrap()
        .1
        .trim_end()
        .trim_end_matches(']')
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!(heights.len(), 15000);
    // FNV-1a of all 15,000 integer heights, verified against the pinned CSV.
    // This preserves the exact image without keeping a second integer table.
    let fingerprint = heights.iter().fold(0xcbf29ce484222325u64, |hash, &height| {
        (hash ^ u64::from(height)).wrapping_mul(0x100000001b3)
    });
    assert_eq!(fingerprint, 0xc62f8d8249054461, "integer pulse data changed");

    let full =
        include_str!("../../../samples/graphics/unknown-pleasures/unknown-pleasures-vbxe-data.inc");
    let vbxe: Vec<u8> = full
        .split_once("=[")
        .unwrap()
        .1
        .trim_end()
        .trim_end_matches(']')
        .split_whitespace()
        .map(|v| u8::from_str_radix(v.trim_start_matches('$'), 16).unwrap())
        .collect();
    assert_eq!(vbxe.len(), 80 * 300);
    for (i, &height) in heights.iter().enumerate() {
        // The two tables round the CSV independently. Corresponding samples
        // differ by at most half a pixel after scaling to quarter pixels.
        let trace = ((i / 300) * 80).div_ceil(50);
        let quarter_height = vbxe[trace * 300 + i % 300];
        assert!(
            (i16::from(height) * 4 - i16::from(quarter_height)).abs() <= 2,
            "Graphics 8 and VBXE samples disagree at index {i}"
        );
        let row = 181 - (i / 300) as i32 * 3 + 4 - i32::from(height);
        assert!(
            (0..192).contains(&row),
            "height at index {i} leaves the screen"
        );
    }

    // Checkpoints leave the rendering and keyboard loop intact.
    let source = SAMPLE
        .replace("PROC Main()", "PROC PictureDone=$0700()\nPROC Main()")
        .replace("  CH=$FF", "  CH=$FF\n  PictureDone()")
        .replace("  Graphics(0)", "  Graphics(0)\n  PictureDone()");
    let original_object = cartridge_compile(&source);
    let expected = render(&original_object, Runtime::ActionCart);
    if let Ok(dir) = std::env::var("ACTIONC_UNKNOWN_PLEASURES_ARTIFACT_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let mut pgm = b"P5\n320 192\n255\n".to_vec();
        for &pixel in &expected {
            pgm.push(pixel * 255);
        }
        std::fs::write(Path::new(&dir).join("cartridge.pgm"), pgm).unwrap();
    }
    let scratch = Scratch(
        std::env::temp_dir().join(format!("actionc-unknown-pleasures-{}", std::process::id())),
    );
    std::fs::create_dir_all(&scratch.0).unwrap();
    let path = scratch.0.join("PLEASURE.ACT");
    std::fs::write(&path, source).unwrap();
    std::fs::write(scratch.0.join("UPDATA.ACT"), DATA).unwrap();
    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let compiled = compile_file(
                &path,
                &CompileOptions::for_mode(mode)
                    .with_runtime(runtime)
                    .with_origin(0x2C00),
            )
            .unwrap();
            assert_eq!(
                render(compiled.object_bytes(), runtime),
                expected,
                "{mode:?}/{runtime:?}"
            );
            println!(
                "{mode:?}/{runtime:?}: {} object bytes, image matches cartridge",
                compiled.object_bytes().len()
            );
        }
    }
}
