//! Opt-in experiment, compiled by window_frames.py against release VM libraries.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{CompilerVm, RunRequest, StopReason, VmRunHooks, VmRunner};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn build() -> PathBuf {
    std::env::var_os("ACTIONC_TN_FRAME_BUILD").unwrap().into()
}
#[allow(dead_code)]
#[path = "../../vm-runtime-tests/tests/support/fixed_point.rs"]
mod support;
#[path = "../../vm-runtime-tests/tests/support/tn_symbols.rs"]
mod symbols;
use symbols::{global_address, routine_address};

fn ranges(path: &Path) {
    let loaded = actionc::includes::load_program_with_expanded_source(path).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &loaded.program,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let raw = actionc::nir::lower_program(&actionc::semantic::ir::lower_program(
        &loaded.program,
        &model,
    ));
    for nir in [&raw, &actionc::nir::optimize_program(&raw).unwrap()] {
        actionc::nir::verify_program(nir).unwrap();
        let facts = actionc::nir::analyze_unsigned_ranges(nir).unwrap();
        for name in ["ArenaCursor", "Alloc"] {
            let r = nir.routines.iter().find(|r| r.name == name).unwrap();
            let range = facts.normal_return_range(r.id).unwrap();
            assert_eq!(
                (range.min(), range.max()),
                (0x2000, 0x2800),
                "{} {name}",
                path.display()
            );
        }
    }
}

#[test]
fn compile_images() {
    let mut rows = Vec::new();
    for variant in ["baseline", "linked", "trailer"] {
        for name in ["TN", "TNDBG"] {
            let source = build().join(variant).join(format!("{name}.ACT"));
            ranges(&source);
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    eprintln!("COMPILE {variant}/{name}/{mode:?}/{runtime:?}");
                    let compiled = compile_file(
                        &source,
                        &CompileOptions::for_mode(mode).with_runtime(runtime),
                    )
                    .unwrap();
                    let stem = build()
                        .join(variant)
                        .join(format!("{name}-{mode:?}-{runtime:?}"));
                    std::fs::write(stem.with_extension("xex"), compiled.object_bytes()).unwrap();
                    let listing = compiled.source_listing().replace("\r\n", "\n");
                    std::fs::write(stem.with_extension("asm"), &listing).unwrap();
                    let routines = listing
                        .lines()
                        .filter_map(|line| {
                            let line = line.strip_prefix("; ===== PROC ")?;
                            let (name, tail) = line.split_once(" $")?;
                            let (a, b) = tail.split_once("..$")?;
                            let end = b.split_whitespace().next()?;
                            Some((
                                name.to_string(),
                                u16::from_str_radix(end, 16).unwrap()
                                    - u16::from_str_radix(a, 16).unwrap(),
                            ))
                        })
                        .collect::<BTreeMap<_, _>>();
                    rows.push(serde_json::json!({"variant":variant,"program":name,"mode":format!("{mode:?}"),"runtime":format!("{runtime:?}"),"xex_bytes":compiled.object_bytes().len(),"routine_bytes":routines}));
                }
            }
        }
    }
    std::fs::write(
        build().join("images.json"),
        serde_json::to_string_pretty(&rows).unwrap() + "\n",
    )
    .unwrap();
}

struct Hooks {
    names: BTreeMap<u16, &'static str>,
    calls: BTreeMap<&'static str, usize>,
    peak: u16,
}
impl VmRunHooks for Hooks {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let pc = vm.cpu().registers().pc;
        if let Some(name) = self.names.get(&pc) {
            *self.calls.entry(name).or_default() += 1;
        }
        if vm.bus().ram().read(pc) == 0x60 {
            self.peak = self.peak.max(vm.bus().ram().read_word(0xE8));
        }
        Ok(())
    }
}
fn bytes(vm: &CompilerVm, adr: u16, n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| vm.bus().ram().read(adr + i as u16))
        .collect()
}
const ENTRY: u16 = 0x0600;
fn prepare(vm: &mut CompilerVm, entry: u16, args: &[u8]) {
    let byte = |i| args.get(i).copied().unwrap_or(0);
    vm.bus_mut()
        .ram_mut()
        .map(
            ENTRY,
            &[
                0xA9,
                byte(0),
                0xA2,
                byte(1),
                0xA0,
                byte(2),
                0x20,
                entry as u8,
                (entry >> 8) as u8,
                0x4C,
                0x10,
                0x06,
            ],
        )
        .unwrap();
    if args.len() > 3 {
        vm.bus_mut().ram_mut().map(0xA3, &args[3..]).unwrap();
    }
    vm.set_pc(ENTRY);
}
fn call(
    mut vm: CompilerVm,
    hooks: &mut Hooks,
    entry: u16,
    args: &[u8],
) -> (CompilerVm, u64, BTreeMap<&'static str, usize>) {
    prepare(&mut vm, entry, args);
    let started = vm.cpu().cycles();
    let sp = vm.cpu().registers().sp;
    hooks.calls.clear();
    let result = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: 2_000_000,
                stop_after_pc: Some(ENTRY + 9),
                history_len: 8,
            },
            hooks,
        )
        .unwrap();
    assert_eq!(
        result.stop_reason(),
        StopReason::PcReached { pc: ENTRY + 9 },
        "{:?}",
        result.report
    );
    assert_eq!(result.report.registers.sp, sp, "stack leak");
    let vm = result.into_vm();
    // Three immediate loads (6), JSR (6), final JMP (3). Stop after that
    // JMP, never execute the instruction at its destination. RTS is included.
    let cycles = vm.cpu().cycles() - started - 15;
    (vm, cycles, hooks.calls.clone())
}

#[test]
fn measure_frames() {
    let mut rows = Vec::new();
    for variant in ["baseline", "linked", "trailer"] {
        for name in ["TN", "TNDBG"] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                    let stem = build()
                        .join(variant)
                        .join(format!("{name}-{mode:?}-{runtime:?}"));
                    let image = std::fs::read(stem.with_extension("xex")).unwrap();
                    let listing = std::fs::read_to_string(stem.with_extension("asm"))
                        .unwrap()
                        .replace("\r\n", "\n");
                    let routine = |n| routine_address(&listing, n);
                    let global = |n| global_address(&listing, n);
                    for (case, geometries) in [
                        ("small", vec![[1u8, 1, 10, 4]]),
                        ("nested", vec![[2, 8, 36, 2], [1, 7, 37, 4], [10, 9, 15, 4]]),
                        ("debug", vec![[1, 2, 38, 21]]),
                        ("full-screen", vec![[0, 0, 39, 23]]),
                        ("exact-fit", vec![[1, 1, 10, 4]]),
                    ] {
                        let (mut vm, profile) = support::vm_for(runtime);
                        vm.load_atari_object_for_execution(profile, &image).unwrap();
                        let mut hooks = Hooks {
                            names: [
                                "ArenaCursor",
                                "Alloc",
                                "Free",
                                "Push",
                                "Pull",
                                "SavePos",
                                "RestorePos",
                            ]
                            .into_iter()
                            .map(|n| (routine(n), n))
                            .collect(),
                            calls: BTreeMap::new(),
                            peak: 0,
                        };
                        (vm, _, _) = call(vm, &mut hooks, routine("Init"), &[]);
                        vm.bus_mut().ram_mut().write_word(0x58, 0x9000);
                        let screen = (0..960)
                            .map(|i| (i as u8).wrapping_mul(17).wrapping_add(9))
                            .collect::<Vec<_>>();
                        vm.bus_mut().ram_mut().map(0x9000, &screen).unwrap();
                        vm.bus_mut().ram_mut().map(0x5A, &[0, 0, 39, 23]).unwrap();
                        vm.bus_mut().ram_mut().write(0x1FFF, 0xA5);
                        vm.bus_mut().ram_mut().write(0x2800, 0x5A);
                        (vm, _, _) = call(vm, &mut hooks, routine("SwapScr"), &[]);
                        assert_eq!(vm.bus().ram().read_word(0xE8), 0x23E8);
                        if case == "exact-fit" {
                            (vm, _, _) = call(
                                vm,
                                &mut hooks,
                                routine("Alloc"),
                                &(0x27BFu16 - 0x23E8).to_le_bytes(),
                            );
                        }
                        let start = vm.bus().ram().read_word(0xE8);
                        let permanent = bytes(&vm, 0x2000, 1000);
                        let mut stack = Vec::new();
                        let mut measurements = Vec::new();
                        hooks.peak = start;
                        for geometry in geometries {
                            let mark = vm.bus().ram().read_word(0xE8);
                            let old_position = bytes(&vm, 0x5A, 4);
                            let old_screen = bytes(&vm, 0x9000, 960);
                            let [x, y, dx, dy] = geometry;
                            let area = (u16::from(dx) + 1) * (u16::from(dy) + 1);
                            let previous = stack.last().map(|(mark, _, _)| *mark).unwrap_or(0u16);
                            let (next, cycles, calls) =
                                call(vm, &mut hooks, routine("Window"), &geometry);
                            vm = next;
                            assert_eq!(vm.bus().ram().read_word(0xE8), mark + area + 10);
                            assert_eq!(
                                vm.bus().ram().read(global("openwindows")),
                                stack.len() as u8 + 1
                            );
                            assert_eq!(bytes(&vm, 0x5A, 4), [x + 1, y + 1, dx - 1, dy - 1]);
                            let (parent, window, image_address) = if variant == "linked" {
                                assert_eq!(vm.bus().ram().read_word(mark), previous);
                                (mark + 2, mark + 6, mark + 10)
                            } else {
                                assert_eq!(vm.bus().ram().read_word(mark + 4 + area), mark + 4);
                                (mark, mark + 6 + area, mark + 4)
                            };
                            assert_eq!(bytes(&vm, parent, 4), old_position);
                            assert_eq!(bytes(&vm, window, 4), geometry);
                            let mut pixels = Vec::new();
                            for row in y..=y + dy {
                                for col in x..=x + dx {
                                    pixels
                                        .push(old_screen[usize::from(row) * 40 + usize::from(col)]);
                                }
                            }
                            assert_eq!(
                                bytes(&vm, image_address, area as usize),
                                pixels,
                                "wrong saved rectangle"
                            );
                            assert_eq!(
                                calls.get("ArenaCursor"),
                                Some(&if variant == "baseline" { 4 } else { 1 })
                            );
                            measurements.push(
                                serde_json::json!({"op":"open","cycles":cycles,"calls":calls}),
                            );
                            stack.push((mark, old_position, old_screen));
                        }
                        while let Some((mark, position, screen)) = stack.pop() {
                            let (next, cycles, calls) =
                                call(vm, &mut hooks, routine("CloseWindow"), &[]);
                            vm = next;
                            assert_eq!(vm.bus().ram().read_word(0xE8), mark);
                            assert_eq!(
                                vm.bus().ram().read(global("openwindows")),
                                stack.len() as u8
                            );
                            assert_eq!(bytes(&vm, 0x5A, 4), position);
                            assert_eq!(bytes(&vm, 0x9000, 960), screen);
                            assert_eq!(
                                calls.get("ArenaCursor"),
                                Some(&if variant == "baseline" { 4 } else { 1 })
                            );
                            measurements.push(
                                serde_json::json!({"op":"close","cycles":cycles,"calls":calls}),
                            );
                        }
                        assert_eq!(bytes(&vm, 0x2000, 1000), permanent);
                        assert_eq!(vm.bus().ram().read(global("ArenaFault")), 0);
                        assert_eq!(vm.bus().ram().read(0x1FFF), 0xA5);
                        assert_eq!(vm.bus().ram().read(0x2800), 0x5A);
                        rows.push(serde_json::json!({"variant":variant,"program":name,"mode":format!("{mode:?}"),"runtime":format!("{runtime:?}"),"case":case,"peak":hooks.peak,"measurements":measurements}));
                    }
                    // One byte short: both designs must stop before a write
                    // outside the arena. Candidates reserve atomically; the
                    // baseline can retain earlier successful sub-operations.
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &image).unwrap();
                    let mut hooks = Hooks {
                        names: BTreeMap::new(),
                        calls: BTreeMap::new(),
                        peak: 0,
                    };
                    (vm, _, _) = call(vm, &mut hooks, routine("Init"), &[]);
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x2000, &vec![0xA6; 0x801])
                        .unwrap();
                    vm.bus_mut().ram_mut().write_word(0xE8, 0x27C0);
                    vm.bus_mut().ram_mut().write_word(0x58, 0x9000);
                    vm.bus_mut().ram_mut().map(0x5A, &[0, 0, 39, 23]).unwrap();
                    prepare(&mut vm, routine("Window"), &[1, 1, 10, 4]);
                    let result = VmRunner::new(vm).run(RunRequest {
                        max_steps: 50_000,
                        history_len: 8,
                        ..RunRequest::default()
                    });
                    assert_eq!(
                        result.stop_reason(),
                        StopReason::StepLimit { max_steps: 50_000 }
                    );
                    let stop = routine("ArenaStop");
                    let stop_end = listing
                        .lines()
                        .find(|line| line.starts_with("; ===== PROC ArenaStop "))
                        .unwrap()
                        .split("..$")
                        .nth(1)
                        .unwrap()
                        .split_whitespace()
                        .next()
                        .unwrap();
                    let stop_end = u16::from_str_radix(stop_end, 16).unwrap();
                    assert!((stop..stop_end).contains(&result.report.registers.pc));
                    let vm = result.into_vm();
                    assert_eq!(vm.bus().ram().read(global("ArenaFault")), 2);
                    assert_eq!(vm.bus().ram().read(0x2800), 0xA6);
                    if variant != "baseline" {
                        assert_eq!(vm.bus().ram().read_word(0xE8), 0x27C0);
                        assert_eq!(bytes(&vm, 0x2000, 0x800), vec![0xA6; 0x800]);
                        assert_eq!(bytes(&vm, 0x5A, 4), [0, 0, 39, 23]);
                        assert_eq!(vm.bus().ram().read(global("openwindows")), 0);
                    }
                    rows.push(serde_json::json!({"variant":variant,"program":name,"mode":format!("{mode:?}"),"runtime":format!("{runtime:?}"),"case":"one-byte-short","fault":2,"cursor_after":vm.bus().ram().read_word(0xE8)}));
                }
            }
        }
    }
    std::fs::write(
        build().join("execution.json"),
        serde_json::to_string_pretty(&rows).unwrap() + "\n",
    )
    .unwrap();
}
