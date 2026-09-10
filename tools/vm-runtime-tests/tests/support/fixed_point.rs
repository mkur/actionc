use actionc::compiler::{CompileMode, Runtime};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunner,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub struct Source(pub PathBuf);
impl Source {
    pub fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-fixed-vm-{}-{}.act",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, text).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn lanes() -> impl Iterator<Item = (CompileMode, Runtime)> {
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

pub fn vm_for(runtime: Runtime) -> (CompilerVm, ExecutionProfile) {
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

pub fn put(page: &mut [u8], offset: usize, value: i64) {
    // Explicit modulo-65536 conversion, independent of compiler evaluators.
    let bits = value.rem_euclid(65536) as u16;
    page[offset..offset + 2].copy_from_slice(&bits.to_le_bytes());
}

pub fn check(
    image: &[u8],
    mode: CompileMode,
    runtime: Runtime,
    a: i16,
    b: i16,
    expected: impl FnOnce(&mut [u8]),
) {
    execute(image, mode, runtime, (a, b), false, expected);
}

pub fn execute(
    image: &[u8],
    mode: CompileMode,
    runtime: Runtime,
    (a, b): (i16, i16),
    fault: bool,
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
    if fault {
        // Capture Error's registers/count, set decimal mode, and deliberately
        // return. The arithmetic helper must clear D and never resume its caller.
        vm.bus_mut()
            .ram_mut()
            .map(
                0x780,
                &[
                    0x8D, 0x20, 7, 0x8E, 0x21, 7, 0x8C, 0x22, 7, 0xEE, 0x23, 7, 0xF8, 0x60,
                ],
            )
            .unwrap();
        vm.bus_mut().ram_mut().write(0x723, 0);
        match runtime {
            Runtime::ActionCart => vm
                .bus_mut()
                .ram_mut()
                .map(0x04CB, &[0x4C, 0x80, 7])
                .unwrap(),
            Runtime::Standalone => vm.bus_mut().ram_mut().write_word(0x000A, 0x0780),
        }
    }
    expected(&mut page);
    if !fault {
        page[0xDF] = 0xA5;
    }
    let max_steps = 60_000;
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
    if fault {
        assert_eq!(
            (0x720..=0x723)
                .map(|a| outcome.memory().read(a))
                .collect::<Vec<_>>(),
            [101, 0, 101, 1],
            "{mode:?}/{runtime:?} Error register/count capture"
        );
        assert_eq!(
            outcome.report.registers.status & 8,
            0,
            "decimal mode after returning Error"
        );
        let pc = outcome.report.registers.pc;
        assert_eq!(
            [outcome.memory().read(pc), outcome.memory().read(pc + 1)],
            [0xB0, 0xFE],
            "fault must stop at the non-returning guard"
        );
    }
}
