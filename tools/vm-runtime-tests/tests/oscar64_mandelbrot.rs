//! Integer oracle and execution checks for the pinned Oscar64 Mandelbrot port.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunOutcome,
    RunRequest, StopReason, VmRunner,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "support/vbxe.rs"]
mod vbxe;

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Source(PathBuf);
impl Source {
    fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "actionc-mandelbrot-{}-{}.act",
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

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn project() -> PathBuf {
    root().join("samples/graphics/mandelbrot")
}
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

fn vm_for(image: &[u8], runtime: Runtime, os: bool) -> CompilerVm {
    let mut vm = CompilerVm::default();
    if runtime == Runtime::ActionCart {
        vm.load_image_bytes(
            ImageKind::Cartridge,
            "action.rom",
            DEFAULT_CART_BASE,
            std::fs::read(root().join("roms/action.rom")).unwrap(),
        )
        .unwrap();
    }
    if os || runtime == Runtime::ActionCart {
        vm.load_image_bytes(
            ImageKind::Rom,
            "altirraos-xl.rom",
            OS_ROM_BASE,
            std::fs::read(root().join("roms/altirraos-xl.rom")).unwrap(),
        )
        .unwrap();
    }
    let profile = if runtime == Runtime::ActionCart {
        ExecutionProfile::CartridgeObject
    } else {
        ExecutionProfile::StandaloneObject
    };
    let loaded = vm.load_atari_object_for_execution(profile, image).unwrap();
    assert!(
        loaded
            .segments
            .iter()
            .all(|s| s.end < 0x600 || s.start > 0x700)
    );
    vm.bus_mut().ram_mut().write(0x700, 0xEA); // Explicit test completion endpoint.
    vm
}

fn finish(vm: CompilerVm, max_steps: u64) -> RunOutcome {
    let outcome = VmRunner::new(vm).run(RunRequest {
        max_steps,
        stop_after_pc: Some(0x700),
        history_len: 8,
    });
    assert_eq!(
        outcome.stop_reason(),
        StopReason::PcReached { pc: 0x700 },
        "{:?}",
        outcome.report
    );
    outcome
}

fn signed16(value: i64) -> i64 {
    (value + 32768).rem_euclid(65536) - 32768
}

fn coords(px: u16, py: u8) -> (i64, i64) {
    // Derive the original truncated constants from exact rational literals.
    let x_coefficient = 7 * 4096 * 256 / (2 * 160);
    let y_coefficient = 12 * 4096 * 256 / (5 * 100);
    (
        i64::from(px) * x_coefficient / 256 - 5 * 4096 / 2,
        i64::from(py) * y_coefficient / 256 - 6 * 4096 / 5,
    )
}

fn iterations(cx: i64, cy: i64, floor: bool) -> u8 {
    let (mut x, mut y) = (0i64, 0i64);
    for i in 0..32 {
        let (xx, yy) = (x * x, y * y);
        if xx + yy >= 4 * 4096 * 4096 {
            return i;
        }
        let next = signed16(signed16(xx / 4096) - signed16(yy / 4096) + cx);
        let product = if floor {
            (x * y).div_euclid(4096)
        } else {
            x * y / 4096
        };
        y = signed16(2 * signed16(product) + cy);
        x = next;
    }
    32
}

fn put(page: &mut [u8], at: usize, value: i64) {
    page[at..at + 2].copy_from_slice(&(value.rem_euclid(65536) as u16).to_le_bytes());
}

#[test]
fn oscar64_mandelbrot_coordinates_recurrence_and_repeated_calls_match_integer_oracle() {
    let mut pixels: Vec<_> = (0..160)
        .map(|x| (x, 50u8))
        .chain((0..100).map(|y| (80, y)))
        .collect();
    let mut seed = 0x4D424658u32;
    for _ in 0..128 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let px = ((seed >> 16) % 160) as u16;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pixels.push((px, ((seed >> 16) % 100) as u8));
    }
    let mut sensitive = Vec::new();
    let mut interior = 0;
    let mut updates = 0;
    for py in 0..100 {
        for px in 0..160 {
            let (cx, cy) = coords(px, py);
            let count = iterations(cx, cy, true);
            interior += usize::from(count == 32);
            updates += usize::from(count);
            if count != iterations(cx, cy, false) {
                sensitive.push((px, py));
            }
        }
    }
    assert_eq!((sensitive.len(), interior, updates), (180, 3167, 151649));
    pixels.extend(sensitive.into_iter().take(12));
    pixels.extend([(0, 0), (159, 0), (0, 99), (159, 99)]);
    assert_eq!(pixels.len(), 404);
    assert_eq!(iterations(0, 0, true), 32);
    assert_eq!(iterations(8192, 0, true), 1);
    let raw = [
        (-32768, -32768),
        (0, 0),
        (4096, 0),
        (-4096, 0),
        (8192, 0),
        (-8192, 0),
        (0, 8192),
        (8191, 0),
        (8193, 0),
        (32767, 32767),
        (-10240, -4915),
        (-3072, 512),
    ];
    for (mode, runtime) in lanes() {
        let compiled = compile_file(
            root().join("fixtures/runtime/oscar64/mbfixed.act"),
            &CompileOptions::for_mode(mode)
                .with_runtime(runtime)
                .with_module_path(project()),
        )
        .unwrap();
        for (index, &(px, py)) in pixels.iter().enumerate() {
            let (a, b) = raw[index % raw.len()];
            let mut page = vec![0xCC; 256];
            put(&mut page, 0xE0, i64::from(px));
            page[0xE2] = py;
            put(&mut page, 0xE4, a);
            put(&mut page, 0xE6, b);
            let mut vm = vm_for(compiled.object_bytes(), runtime, false);
            for (offset, &byte) in page.iter().enumerate() {
                vm.bus_mut().ram_mut().write(0x600 + offset as u16, byte);
            }
            let (cx, cy) = coords(px, py);
            put(&mut page, 1, cx);
            put(&mut page, 3, cy);
            page[5] = iterations(cx, cy, true);
            page[6] = page[5];
            page[7] = iterations(a, b, true);
            page[255] = 0xA5;
            let outcome = finish(vm, 2_000_000);
            let actual: Vec<_> = (0x600..=0x6FF).map(|a| outcome.memory().read(a)).collect();
            assert_eq!(
                actual, page,
                "{mode:?}/{runtime:?} pixel({px},{py}) raw({a},{b}): {:?}",
                outcome.report
            );
        }
    }
}

#[test]
fn oscar64_mandelbrot_probe_prints_documented_results() {
    const PROBE: &str = include_str!("../../../samples/graphics/mandelbrot/probe.act");
    const README: &str = include_str!("../../../samples/graphics/mandelbrot/README.md");
    let text = PROBE.replace("PROC Main()", "PROC TestStop=$0700()\nPROC Main()");
    let end = text.rfind("RETURN").unwrap();
    let source = Source::new(&format!("{} TestStop()\n{}", &text[..end], &text[end..]));
    let expected: Vec<_> = README
        .split_once("```text\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0
        .bytes()
        .map(|b| if b == b'\n' { 0x9B } else { b })
        .collect();
    for (mode, runtime) in lanes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode)
                .with_runtime(runtime)
                .with_module_path(project()),
        )
        .unwrap();
        let outcome = finish(vm_for(compiled.object_bytes(), runtime, true), 500_000);
        assert_eq!(
            outcome.vm.bus().cio_channel0_output(),
            expected,
            "probe {mode:?}/{runtime:?}"
        );
    }
}

fn bitmap(rows: &[u8]) -> Vec<u8> {
    // Decode the original C64 pattern pairs as independent per-pixel colors,
    // with a continuous physical-row phase and independently sampled pixels.
    let patterns = [
        (0xFFu8, 0xFFu8),
        (0xEE, 0xBB),
        (0xAA, 0xAA),
        (0x88, 0x22),
        (0x44, 0x11),
        (0x55, 0x55),
        (0xDD, 0x77),
        (0x33, 0xCC),
    ];
    let mut image = vec![0; 40 * 192];
    for &py in rows {
        for px in 0..160 {
            let (cx, cy) = viewport_coords(px, py, 160);
            let count = iterations(cx, cy, true);
            if count < 32 {
                let (upper, lower) = patterns[usize::from(count % 8)];
                let shift = 6 - 2 * (px % 4);
                let pattern = if py % 2 == 0 { upper } else { lower };
                let color = (pattern >> shift) & 3;
                image[usize::from(py) * 40 + usize::from(px / 4)] |= color << shift;
            }
        }
    }
    image
}

fn viewport_coords(px: u16, py: u8, width: i64) -> (i64, i64) {
    (
        i64::from(px) * (7 * 4096 / 2) / width - 5 * 4096 / 2,
        i64::from(py) * (2 * (6 * 4096 / 5)) / 192 - 6 * 4096 / 5,
    )
}

#[test]
fn oscar64_mandelbrot_viewport_coordinates_match_wide_integer_mapping() {
    let source = Source::new(
        "MODULE VIEWPORT_TEST USE FRACTAL.MANDELBROT AS MB\n\
         INT cx=$601,cy=$603 CARD px=$6E0,py=$6E2,width=$6E4,height=$6E6 BYTE done=$6FF\n\
         PROC TestStop=$0700()\n\
         PROC Main() cx=MB.ViewportX(px,width) cy=MB.ViewportY(py,height)\n\
         done=$A5 TestStop() DO OD RETURN ENDMODULE",
    );
    let mut cases: Vec<_> = [160u16, 320]
        .into_iter()
        .flat_map(|width| (0..width).map(move |px| (px, px % 192, width, 192u16)))
        .collect();
    cases.extend([
        (0, 0, 1, 1),
        (1, 1, 2, 2),
        (65534, 65534, 65535, 65535),
        (32767, 32767, 32768, 32768),
    ]);
    assert_eq!(cases.len(), 484);
    for (mode, runtime) in lanes() {
        let compiled = compile_file(
            &source.0,
            &CompileOptions::for_mode(mode)
                .with_runtime(runtime)
                .with_module_path(project()),
        )
        .unwrap();
        for &(px, py, width, height) in &cases {
            let mut page = vec![0xCC; 256];
            for (offset, value) in [(0xE0, px), (0xE2, py), (0xE4, width), (0xE6, height)] {
                put(&mut page, offset, i64::from(value));
            }
            let mut vm = vm_for(compiled.object_bytes(), runtime, false);
            for (offset, &byte) in page.iter().enumerate() {
                vm.bus_mut().ram_mut().write(0x600 + offset as u16, byte);
            }
            put(
                &mut page,
                1,
                i64::from(px) * 14336 / i64::from(width) - 10240,
            );
            put(
                &mut page,
                3,
                i64::from(py) * 9830 / i64::from(height) - 4915,
            );
            page[255] = 0xA5;
            let outcome = finish(vm, 100_000);
            let actual: Vec<_> = (0x600..=0x6FF).map(|a| outcome.memory().read(a)).collect();
            assert_eq!(
                actual, page,
                "{mode:?}/{runtime:?} ({px},{py}) in {width}x{height}"
            );
        }
    }
}

fn render(mode: CompileMode, runtime: Runtime, full: bool) {
    const SAMPLE: &str = include_str!("../../../samples/graphics/mandelbrot/mbfixed.act");
    let mut text = SAMPLE.replace("PROC Main()", "PROC TestStop=$0700()\nPROC Main()");
    assert_eq!(text.matches("  DO OS.ATRACT=0 OD").count(), 1);
    text = text.replace("  DO OS.ATRACT=0 OD", "  TestStop() DO OD");
    let rows: Vec<_> = if full {
        (0..192).collect()
    } else {
        vec![0, 1, 23, 24, 48, 49, 95, 96, 167, 168, 190, 191]
    };
    if !full {
        assert_eq!(text.matches("    DrawRow(py)").count(), 1);
        let condition = rows
            .iter()
            .map(|y| format!("py={y}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        text = text.replace(
            "    DrawRow(py)",
            &format!("    IF {condition} THEN DrawRow(py) FI"),
        );
    }
    let source = Source::new(&text);
    let compiled = compile_file(
        &source.0,
        &CompileOptions::for_mode(mode)
            .with_runtime(runtime)
            .with_module_path(project()),
    )
    .unwrap();
    let outcome = finish(
        vm_for(compiled.object_bytes(), runtime, true),
        if full { 2_000_000_000 } else { 500_000_000 },
    );
    // Channel 6: AUX1 selects read/write with no text window; AUX2 is mode 31.
    assert_eq!(outcome.memory().read(0x3AA), 12);
    assert_eq!(outcome.memory().read(0x3AB), 31);
    assert_eq!(outcome.vm.bus().graphics_mode(), Some(12)); // VM records AUX1.
    // The pinned VM models CIO S: pixels, not ANTIC screen-memory packing.
    // Pack its observed colors independently for comparison with the oracle.
    let mut actual = vec![0u8; 40 * 192];
    for row in 0..=255u8 {
        for column in 0..320u16 {
            let color = outcome.vm.bus().graphics_pixel(column, row);
            if row < 192 && column < 160 {
                assert!(color < 4, "invalid color {color} at ({column},{row})");
                actual[usize::from(row) * 40 + usize::from(column / 4)] |=
                    color << (6 - 2 * (column % 4));
            } else {
                assert_eq!(color, 0, "pixel outside viewport ({column},{row})");
            }
        }
    }
    let expected = bitmap(&rows);
    for (index, (&actual, &expected)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(
            actual,
            expected,
            "{mode:?}/{runtime:?} full={full}, row={}, byte={}: {:?}",
            index / 40,
            index % 40,
            outcome.report
        );
    }
    for (address, value) in [(0x2C4, 0x26), (0x2C5, 0x1C), (0x2C6, 0x9A), (0x2C8, 0)] {
        assert_eq!(
            outcome.memory().read(address),
            value,
            "palette {mode:?}/{runtime:?}"
        );
    }
    println!(
        "Mandelbrot {mode:?}/{runtime:?} full={full}: {} steps, {} cycles, {} object bytes",
        outcome.report.completed_steps,
        outcome.report.cycles,
        compiled.object_bytes().len()
    );
    if let Ok(dir) = std::env::var("ACTIONC_MANDELBROT_ARTIFACT_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let name = format!(
            "{mode:?}-{runtime:?}-{}",
            if full { "full" } else { "rows" }
        );
        std::fs::write(Path::new(&dir).join(format!("{name}.bin")), actual).unwrap();
        std::fs::write(
            Path::new(&dir).join(format!("{name}.txt")),
            format!("{:?}\n", outcome.report),
        )
        .unwrap();
    }
}

#[test]
fn oscar64_mandelbrot_atari_rows_match_packed_bitmap_in_all_lanes() {
    for (mode, runtime) in lanes() {
        render(mode, runtime, false);
    }
}

#[test]
fn oscar64_mandelbrot_full_atari_image_matches_integer_oracle() {
    render(CompileMode::Mir6502, Runtime::Standalone, true);
}

fn vbxe_source(full: bool) -> (Source, Vec<u8>) {
    const SAMPLE: &str = include_str!("../../../samples/graphics/mandelbrot/mbfixed-vbxe.act");
    let mut text = SAMPLE.replace("PROC Main()", "PROC TestStop=$0700()\nPROC Main()");
    assert_eq!(text.matches("  DO OS.ATRACT=0 OD").count(), 1);
    text = text.replace("  DO OS.ATRACT=0 OD", "  TestStop() DO OD");
    // Missing hardware returns from Main before the final display hold.
    assert_eq!(text.matches("    RETURN\n  FI").count(), 1);
    text = text.replace("    RETURN\n  FI", "    TestStop() RETURN\n  FI");
    let rows: Vec<_> = if full {
        (0..192).collect()
    } else {
        vec![0, 15, 16, 95, 96, 191]
    };
    if !full {
        assert_eq!(text.matches("    DrawRow(py)").count(), 1);
        let condition = rows
            .iter()
            .map(|y| format!("py={y}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        text = text.replace(
            "    DrawRow(py)",
            &format!("    IF {condition} THEN DrawRow(py) FI"),
        );
    }
    (Source::new(&text), rows)
}

fn vbxe_compile(source: &Source, mode: CompileMode) -> actionc::compiler::CompiledProgram {
    compile_file(
        &source.0,
        &CompileOptions::for_mode(mode)
            .with_runtime(Runtime::Standalone)
            .with_module_path(project())
            .with_module_path(root().join("samples/vbxe")),
    )
    .unwrap()
}

fn vbxe_vm(
    compiled: &actionc::compiler::CompiledProgram,
    device: Option<(u16, u8)>,
) -> (CompilerVm, vbxe::Vbxe) {
    let mut vm = vm_for(compiled.object_bytes(), Runtime::Standalone, true);
    // A MEMAC window must never cover executable code, static data, or helpers.
    let mut parser = CompilerVm::default();
    let loaded = parser
        .load_atari_object_for_execution(
            ExecutionProfile::StandaloneObject,
            compiled.object_bytes(),
        )
        .unwrap();
    assert!(
        loaded
            .segments
            .iter()
            .all(|s| s.end < 0xA000 || s.start > 0xBFFF)
    );
    let device = vbxe::Vbxe::new(&mut vm, device);
    (vm, device)
}

fn vbxe_finish(vm: CompilerVm, device: &mut vbxe::Vbxe, budget: u64) -> RunOutcome {
    let outcome = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: budget,
                stop_after_pc: Some(0x700),
                history_len: 8,
            },
            device,
        )
        .unwrap();
    assert_eq!(
        outcome.stop_reason(),
        StopReason::PcReached { pc: 0x700 },
        "{:?}",
        outcome.report
    );
    device.flush(&outcome.vm);
    outcome
}

fn vbxe_palette(palette: usize) -> Vec<[u8; 3]> {
    // The shared screen first initializes the complete cold-steel palette.
    // The fractal replaces entries 0..32 with black and four RGB gradient spans.
    let steel = [
        [0, 4, 10],
        [16, 29, 42],
        [78, 102, 117],
        [178, 195, 204],
        [255, 255, 255],
    ];
    // Preserve the approved preview's exact bytes independently of the
    // Action tables and interpolation code; no ignored build files are needed.
    let previews = include_str!("../../../fixtures/runtime/oscar64/mbfixed-vbxe-palettes.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(previews.len(), 6);
    let colors = previews[palette]
        .split_whitespace()
        .skip(1) // Human-readable palette name.
        .map(|hex| {
            let rgb = u32::from_str_radix(hex, 16).unwrap();
            [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
        })
        .collect::<Vec<_>>();
    assert_eq!(colors.len(), 33);
    (0..256)
        .map(|color| {
            if color <= 32 {
                return colors[color];
            }
            let segment = color / 64;
            let pos = color % 64;
            let span = if color < 192 { 64 } else { 63 };
            std::array::from_fn(|channel| {
                let lo = steel[segment][channel];
                let hi = steel[segment + 1][channel];
                (lo + (hi - lo) * pos as i32 / span) as u8
            })
        })
        .collect()
}

fn vbxe_render(mode: CompileMode, base: u16, full: bool) {
    let (source, rows) = vbxe_source(full);
    let compiled = vbxe_compile(&source, mode);
    let (mut vm, mut device) = vbxe_vm(&compiled, Some((base, 0xA3)));
    vm.bus_mut().write(0xD20A, 0); // Select Current for stable full-image artifacts.
    let outcome = vbxe_finish(
        vm,
        &mut device,
        if full { 4_000_000_000 } else { 500_000_000 },
    );
    let mut expected = vec![0xCC; 0x80000];
    expected[..16].copy_from_slice(&[
        0x24, 0, 23, 0x62, 8, 191, 0, 0x40, 0, 0, 2, 0x11, 0xFF, 0x24, 0x80, 23,
    ]);
    expected[0x4000..0x1C000].fill(0); // Clear initializes all banks, including stride padding.
    for &py in &rows {
        for px in 0..320 {
            let (cx, cy) = viewport_coords(px, py, 320);
            let count = iterations(cx, cy, true);
            expected[0x4000 + usize::from(py) * 512 + usize::from(px)] =
                if count == 32 { 0 } else { count + 1 };
        }
    }
    for (offset, (&actual, &expected)) in device.local.iter().zip(&expected).enumerate() {
        assert_eq!(
            actual, expected,
            "{mode:?} ${base:04X} full={full}, VBXE local ${offset:05X}"
        );
    }
    assert_eq!(device.palettes[1].as_slice(), vbxe_palette(0));
    for palette in [0, 2, 3] {
        assert_eq!(device.palettes[palette], [[0xCC; 3]; 256]);
    }
    assert_eq!(device.registers[0], 5); // XDL enabled, zero palette index is opaque.
    assert_eq!(&device.registers[1..4], &[0, 0, 0]);
    assert_eq!(device.registers[0x1E], 0xA9);
    assert_eq!(device.registers[0x1F], 0x9A); // Last of the twelve 8KB banks.
    assert_eq!(
        device.banks,
        std::iter::once(0)
            .chain((0..12).map(|i| 4 + 2 * i))
            .collect()
    );
    assert_eq!(outcome.vm.bus().io().read(0xD301), Some(0xFF));
    assert!(outcome.vm.bus().cio_channel0_output().is_empty());
    println!(
        "VBXE {mode:?} ${base:04X} full={full}: {} steps, {} cycles, {} object bytes",
        outcome.report.completed_steps,
        outcome.report.cycles,
        compiled.object_bytes().len()
    );
    if let Ok(dir) = std::env::var("ACTIONC_MANDELBROT_ARTIFACT_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let name = format!(
            "Vbxe-{mode:?}-{base:04X}-{}",
            if full { "full" } else { "rows" }
        );
        let image: Vec<u8> = (0..192)
            .flat_map(|y| {
                device.local[0x4000 + y * 512..0x4000 + y * 512 + 320]
                    .iter()
                    .copied()
            })
            .collect();
        std::fs::write(Path::new(&dir).join(format!("{name}.bin")), image).unwrap();
        std::fs::write(
            Path::new(&dir).join(format!("{name}.pal")),
            device.palettes[1].as_flattened(),
        )
        .unwrap();
        std::fs::write(
            Path::new(&dir).join(format!("{name}.txt")),
            format!("{:?}\n", outcome.report),
        )
        .unwrap();
    }
}

#[test]
fn oscar64_mandelbrot_vbxe_startup_selects_all_preview_palettes() {
    let (source, _) = vbxe_source(false);
    let text = std::fs::read_to_string(&source.0).unwrap();
    assert_eq!(text.matches("  COLORS.InstallRandom()").count(), 1);
    // Execute the real startup, stopping before the expensive pixel rendering.
    let source = Source::new(&text.replace(
        "  COLORS.InstallRandom()",
        "  COLORS.InstallRandom()\n  TestStop()",
    ));
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let compiled = vbxe_compile(&source, mode);
        // Exercise both ends of every SYS.Rand(6) bucket, including 0 and 255.
        for (random, palette) in [
            (0, 0),
            (42, 0),
            (43, 1),
            (85, 1),
            (86, 2),
            (127, 2),
            (128, 3),
            (170, 3),
            (171, 4),
            (213, 4),
            (214, 5),
            (255, 5),
        ] {
            let base = if random % 2 == 0 { 0xD640 } else { 0xD740 };
            let (mut vm, mut device) = vbxe_vm(&compiled, Some((base, 0xA3)));
            vm.bus_mut().write(0xD20A, random);
            vm.bus_mut().add_watch_range(actionc_vm::AddressRange {
                start: 0xD20A,
                end: 0xD20A,
            });
            vbxe_finish(vm, &mut device, 5_000_000);
            assert!(device.reads.contains(&0xD20A), "startup must sample POKEY");
            assert_eq!(
                device.palettes[1].as_slice(),
                vbxe_palette(palette),
                "{mode:?}, random byte {random}, palette {palette}",
            );
            for untouched in [0, 2, 3] {
                assert_eq!(device.palettes[untouched], [[0xCC; 3]; 256]);
            }
        }
    }
}

#[test]
fn oscar64_mandelbrot_vbxe_rows_match_banked_pixels_and_palette_in_both_backends_and_pages() {
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for base in [0xD640, 0xD740] {
            vbxe_render(mode, base, false);
        }
    }
}

#[test]
fn oscar64_mandelbrot_vbxe_full_image_matches_integer_oracle() {
    vbxe_render(CompileMode::Mir6502, 0xD740, true);
}

#[test]
fn oscar64_mandelbrot_vbxe_missing_or_incompatible_hardware_reports_without_writes() {
    let (source, _) = vbxe_source(false);
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let compiled = vbxe_compile(&source, mode);
        for hardware in [None, Some((0xD640, 0x30)), Some((0xD740, 0x10))] {
            let (vm, mut device) = vbxe_vm(&compiled, hardware);
            let outcome = vbxe_finish(vm, &mut device, 500_000);
            assert_eq!(
                outcome.vm.bus().cio_channel0_output(),
                b"VBXE FX 1.2x is required.\x9B"
            );
            assert!(device.writes.is_empty());
            assert!(device.banks.is_empty());
            assert!(device.local.iter().all(|&b| b == 0xCC));
            assert!(
                device
                    .palettes
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|&b| b == 0xCC)
            );
            assert_eq!(outcome.vm.bus().io().read(0xD301), Some(0xFD));
            assert!((0xA000..=0xBFFF).all(|a| outcome.memory().read(a) == 0x5A));
            assert!(device.reads.contains(&0xD640));
            if hardware.is_none_or(|(base, _)| base == 0xD740) {
                assert!(device.reads.contains(&0xD740));
            }
        }
    }
}
