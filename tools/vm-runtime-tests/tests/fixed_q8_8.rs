use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::Path;

const BOUNDARIES: &[i16] = &[
    -32768, -32767, -1024, -513, -512, -257, -256, -129, -128, -1, 0, 1, 127, 128, 255, 256, 257,
    512, 1024, 32766, 32767,
];

fn lanes() -> impl Iterator<Item = (CompileMode, Runtime)> {
    [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ]
    .into_iter()
    .flat_map(|mode| {
        [Runtime::ActionCart, Runtime::Standalone]
            .into_iter()
            .map(move |runtime| (mode, runtime))
    })
}

fn vm_for(runtime: Runtime) -> (CompilerVm, ExecutionProfile) {
    let mut vm = CompilerVm::default();
    let profile = if runtime == Runtime::ActionCart {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (kind, name, base) in [
            (ImageKind::Cartridge, "action.rom", DEFAULT_CART_BASE),
            (ImageKind::Rom, "altirraos-xl.rom", OS_ROM_BASE),
        ] {
            vm.load_image_bytes(
                kind,
                name,
                base,
                std::fs::read(root.join("roms").join(name)).unwrap(),
            )
            .unwrap();
        }
        ExecutionProfile::CartridgeObject
    } else {
        // Numeric standalone calls must work without either ROM.
        ExecutionProfile::StandaloneObject
    };
    (vm, profile)
}

fn put(page: &mut [u8], offset: usize, value: i64) {
    // Explicit modulo-65536 conversion, independent of compiler evaluators.
    let bits = value.rem_euclid(65536) as u16;
    page[offset..offset + 2].copy_from_slice(&bits.to_le_bytes());
}

fn check(
    image: &[u8],
    mode: CompileMode,
    runtime: Runtime,
    a: i16,
    b: i16,
    expected: impl FnOnce(&mut [u8]),
) {
    let (mut vm, profile) = vm_for(runtime);
    let load = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        load.segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x7FF)
    );
    let mut page = vec![0xCC; 0x120];
    put(&mut page, 0xE0, i64::from(a));
    put(&mut page, 0xE2, i64::from(b));
    for (offset, &value) in page.iter().enumerate() {
        vm.bus_mut().ram_mut().write(0x600 + offset as u16, value);
    }
    expected(&mut page);
    page[0xDF] = 0xA5;
    let max_steps = 25_000;
    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps,
        history_len: 8,
        ..Default::default()
    });
    assert_eq!(
        outcome.stop_reason(),
        StopReason::StepLimit { max_steps },
        "{mode:?}/{runtime:?} ({a},{b}): {:?}",
        outcome.report
    );
    let actual: Vec<_> = (0x600..=0x71F)
        .map(|address| outcome.memory().read(address))
        .collect();
    assert_eq!(
        actual, page,
        "{mode:?}/{runtime:?} ({a},{b}): {:?}",
        outcome.report
    );
}

#[test]
fn fixed_q8_8_conversions_and_constants_match_host_oracle() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/runtime/fixed_q8_8.act");
    let mut values = BOUNDARIES.to_vec();
    values.extend([-384, -127, 129, 384]);
    for (mode, runtime) in lanes() {
        let compiled = compile_file(
            &source,
            &CompileOptions::for_mode(mode).with_runtime(runtime),
        )
        .unwrap();
        for &value in &values {
            check(compiled.object_bytes(), mode, runtime, value, 0, |page| {
                let a = i64::from(value);
                put(page, 1, a * 256);
                put(page, 3, a / 256);
                for (index, raw) in [256, 128, 1, -32768, 32767].into_iter().enumerate() {
                    put(page, 0x11 + index * 2, raw);
                }
            });
        }
    }
}
