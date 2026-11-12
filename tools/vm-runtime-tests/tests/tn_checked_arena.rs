//! Calls the maintained library, including released-byte and window checkpoints.
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc_vm::{BusAccess, CompilerVm, RunRequest, StopReason, VmRunHooks, VmRunner};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
#[path = "support/tn_machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod support;
#[path = "support/tn_symbols.rs"]
mod symbols;

struct Observe {
    high_water: u16,
    calls: usize,
}
impl VmRunHooks for Observe {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        if vm.cpu().registers().pc == machine::ENTRY {
            self.calls += 1;
        }
        // Observe complete primitive results, not the transient two-byte
        // representation while a word store is halfway through executing.
        if vm.bus().ram().read(vm.cpu().registers().pc) == 0x60 {
            self.high_water = self.high_water.max(vm.bus().ram().read_word(0xE8));
        }
        Ok(())
    }
}

struct Sources(PathBuf);
impl Sources {
    fn new(crlf: bool) -> Self {
        let path =
            std::env::temp_dir().join(format!("actionc-arena-{}-{crlf}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        for name in ["TN.ACT", "TNDBG.ACT", "LIB.ACT"] {
            let text = std::fs::read_to_string(root().join("samples/tn/modern").join(name))
                .unwrap()
                .replace("\r\n", "\n");
            std::fs::write(
                path.join(name),
                if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text
                },
            )
            .unwrap();
        }
        Self(path)
    }
}
impl Drop for Sources {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compile(
    sources: &Sources,
    name: &str,
    mode: CompileMode,
    runtime: Runtime,
) -> (Vec<u8>, String) {
    // An explicitly selected freshly built release compiler avoids spending
    // minutes in debug materialization. CI normally exercises the linked crate.
    if let Some(compiler) = std::env::var_os("ACTIONC_ARENA_COMPILER") {
        let output = std::process::Command::new(compiler)
            .args([
                "--mode",
                if mode == CompileMode::Mir6502 {
                    "mir6502"
                } else {
                    "optimized"
                },
                "--runtime",
                if runtime == Runtime::ActionCart {
                    "cart"
                } else {
                    "standalone"
                },
            ])
            .arg("-o")
            .arg(sources.0.join("image.xex"))
            .arg("--listing")
            .arg(sources.0.join("image.asm"))
            .arg(sources.0.join(name))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        (
            std::fs::read(sources.0.join("image.xex")).unwrap(),
            std::fs::read_to_string(sources.0.join("image.asm")).unwrap(),
        )
    } else {
        let compiled = compile_file(
            sources.0.join(name),
            &CompileOptions::for_mode(mode).with_runtime(runtime),
        )
        .unwrap();
        (compiled.object_bytes().to_vec(), compiled.source_listing())
    }
}

fn parameter(listing: &str, routine: &str, name: &str) -> u16 {
    let label = format!("param_{routine}_{name}:");
    listing
        .lines()
        .skip_while(|l| *l != label)
        .find_map(|l| {
            let (_, comment) = l.split_once(';')?;
            let address = comment.trim_start().strip_prefix('$')?.split_once(':')?.0;
            u16::from_str_radix(address, 16).ok()
        })
        .unwrap()
}

fn failures(image: &[u8], listing: &str, runtime: Runtime) -> usize {
    let mut cases = vec![];
    for (name, arg) in [
        ("Alloc", 1u16),
        ("Free", 0x2000),
        ("Push", 0xBEEF),
        ("Pull", 0),
        ("RestorePos", 0),
    ] {
        for cursor in [0u16, 0xE8, 0x1FFF, 0x2801, 0xFFFF] {
            cases.push((name, cursor, arg, 1));
        }
    }
    cases.extend([
        ("Alloc", 0x2000, 0x801, 2),
        ("Alloc", 0x2000, 0xFFFF, 2),
        ("Alloc", 0x2800, 1, 2),
        ("Alloc", 0x27FF, 2, 2),
        ("Free", 0x2000, 0x2001, 3),
        ("Free", 0x2800, 0x2801, 3),
        ("Free", 0x2103, 0x1FFF, 3),
        ("Free", 0x2103, 0xFFFF, 3),
        ("Push", 0x2800, 0xBEEF, 2),
        ("Push", 0x27FF, 0xBEEF, 2),
        ("Pull", 0x2000, 0, 4),
        ("Pull", 0x2001, 0, 4),
        ("RestorePos", 0x2000, 0, 4),
        ("RestorePos", 0x2001, 0, 4),
        ("RestorePos", 0x2002, 0, 4),
        ("RestorePos", 0x2003, 0, 4),
    ]);
    let fault = symbols::global_address(listing, "ArenaFault");
    let stop = symbols::routine_address(listing, "ArenaStop");
    let end = listing
        .lines()
        .find(|l| l.starts_with("; ===== PROC ArenaStop "))
        .unwrap()
        .split("..$")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let end = u16::from_str_radix(end, 16).unwrap();
    for &(name, cursor, arg, reason) in &cases {
        let (mut vm, profile) = support::vm_for(runtime);
        vm.load_atari_object_for_execution(profile, image).unwrap();
        let ram = vm.bus_mut().ram_mut();
        ram.map(0x1FFF, &vec![0x5A; 0xA02]).unwrap();
        ram.map(0x5A, &[1, 2, 3, 4]).unwrap();
        ram.write_word(0xE8, cursor);
        let entry = symbols::routine_address(listing, name);
        let [lo, hi] = arg.to_le_bytes();
        ram.map(
            machine::ENTRY,
            &[
                0xA9,
                lo,
                0xA2,
                hi,
                0x20,
                entry as u8,
                (entry >> 8) as u8,
                0x4C,
                machine::STOP as u8,
                (machine::STOP >> 8) as u8,
            ],
        )
        .unwrap();
        vm.set_pc(machine::ENTRY);
        vm.bus_mut().clear_events();
        let steps = 1500;
        let result = VmRunner::new(vm).run(RunRequest {
            max_steps: steps,
            stop_after_pc: Some(machine::STOP),
            history_len: 8,
        });
        assert_eq!(
            result.stop_reason(),
            StopReason::StepLimit { max_steps: steps },
            "{name}/{cursor:04X}/{arg}: {:?}",
            result.report
        );
        assert!(
            (stop..end).contains(&result.report.registers.pc),
            "{:?}",
            result.report
        );
        let vm = result.into_vm();
        assert_eq!(
            vm.bus().ram().read(fault),
            reason,
            "{name}/{cursor:04X}/{arg}"
        );
        assert_eq!(vm.bus().ram().read_word(0xE8), cursor);
        assert_eq!(machine::bytes(&vm, 0x1FFF, 0xA02), vec![0x5A; 0xA02]);
        assert_eq!(machine::bytes(&vm, 0x5A, 4), [1, 2, 3, 4]);
        assert!(
            !vm.bus()
                .events()
                .iter()
                .any(|e| e.access == BusAccess::Write
                    && ((0x1FFF..=0x2A00).contains(&e.address)
                        || (0xE8..=0xE9).contains(&e.address)
                        || (0x5A..=0x5D).contains(&e.address))),
            "failing {name} changed protected bytes"
        );
    }
    cases.len()
}

#[test]
fn real_arena_success_and_window_checkpoints_with_lf_and_crlf() {
    let mut calls = 0;
    let mut measurements = String::new();
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for name in ["TN.ACT", "TNDBG.ACT"] {
                let mut image = None;
                for crlf in [false, true] {
                    let sources = Sources::new(crlf);
                    eprintln!("{name}/{mode:?}/{runtime:?}/crlf={crlf}");
                    let (bytes, listing) = compile(&sources, name, mode, runtime);
                    if let Some(image) = &image {
                        assert_eq!(image, &bytes);
                    } else {
                        image = Some(bytes.clone());
                    }
                    if !crlf {
                        if let Some(file) = std::env::var_os("ACTIONC_ARENA_VM_RESULTS") {
                            let path = PathBuf::from(file);
                            let parent = path.parent().unwrap();
                            std::fs::create_dir_all(parent).unwrap();
                            std::fs::write(
                                parent.join(format!("vm-{name}-{mode:?}-{runtime:?}.xex")),
                                &bytes,
                            )
                            .unwrap();
                        }
                    }
                    let routine = |n| symbols::routine_address(&listing, n);
                    let global = |n| symbols::global_address(&listing, n);
                    let (mut vm, profile) = support::vm_for(runtime);
                    vm.load_atari_object_for_execution(profile, &bytes).unwrap();
                    vm.bus_mut().ram_mut().write(machine::RETURN, 0x60);
                    let mut hooks = Observe {
                        high_water: 0,
                        calls: 0,
                    };
                    vm = machine::call(vm, &mut hooks, routine("Init"), &[]);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2000);
                    for n in [0u16, 1, 0, 255, 256, 0x800 - 512] {
                        let before = vm.bus().ram().read_word(0xE8);
                        let started = vm.cpu().cycles();
                        vm = machine::call(vm, &mut hooks, routine("Alloc"), &n.to_le_bytes());
                        if !crlf {
                            measurements.push_str(&format!(
                                "{name}\t{mode:?}\t{runtime:?}\tAlloc\t{n}\t{before}\t{}\n",
                                vm.cpu().cycles() - started
                            ));
                        }
                        assert_eq!(vm.bus().ram().read_word(0xA0), before);
                        assert_eq!(vm.bus().ram().read_word(0xE8), before + n);
                    }
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2800);
                    vm = machine::call(vm, &mut hooks, routine("Alloc"), &[0, 0]);
                    assert_eq!(vm.bus().ram().read_word(0xA0), 0x2800);
                    vm = machine::call(vm, &mut hooks, routine("Free"), &0x2000u16.to_le_bytes());
                    vm = machine::call(vm, &mut hooks, routine("Push"), &0xBEEFu16.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(0x2000), 0xBEEF);
                    vm = machine::call(vm, &mut hooks, routine("Pull"), &[]);
                    assert_eq!(vm.bus().ram().read_word(0xA0), 0xBEEF);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2000);
                    // MkDir's released allocation remains writable scratch.
                    vm = machine::call(vm, &mut hooks, routine("Alloc"), &20u16.to_le_bytes());
                    vm = machine::call(vm, &mut hooks, routine("Free"), &0x2000u16.to_le_bytes());
                    vm.bus_mut()
                        .ram_mut()
                        .map(0x2000, &[3, b'D', b':', b'X'])
                        .unwrap();
                    assert_eq!(machine::bytes(&vm, 0x2000, 4), [3, b'D', b':', b'X']);
                    // Exercise real drawing and image restoration without CIO.
                    vm.bus_mut().ram_mut().write_word(0x58, 0x7000);
                    let screen = (0..960)
                        .map(|i| (i as u8).wrapping_mul(17))
                        .collect::<Vec<_>>();
                    vm.bus_mut().ram_mut().map(0x7000, &screen).unwrap();
                    let position = [0, 0, 39, 23];
                    vm.bus_mut().ram_mut().map(0x5A, &position).unwrap();
                    vm.bus_mut().ram_mut().write(0x1FFF, 0xA5);
                    vm.bus_mut().ram_mut().write(0x2800, 0x5A);
                    vm = machine::call(vm, &mut hooks, routine("Window"), &[1, 1, 10, 4]);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2000 + 4 + 55 + 2 + 4);
                    assert_eq!(vm.bus().ram().read(global("openwindows")), 1);
                    vm = machine::call(vm, &mut hooks, routine("CloseWindow"), &[]);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2000);
                    assert_eq!(vm.bus().ram().read(global("openwindows")), 0);
                    assert_eq!(machine::bytes(&vm, 0x5A, 4), position);
                    assert_eq!(machine::bytes(&vm, 0x7000, 960), screen);
                    assert_eq!(vm.bus().ram().read(0x1FFF), 0xA5);
                    assert_eq!(vm.bus().ram().read(0x2800), 0x5A);
                    assert_eq!(hooks.high_water, 0x2800);
                    // Accept odd and current marks; there is no alignment or
                    // historical-mark requirement. Exercise exact push space.
                    vm.bus_mut().ram_mut().write_word(0xE8, 0x27FE);
                    vm = machine::call(vm, &mut hooks, routine("Push"), &0xCAFEu16.to_le_bytes());
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x2800);
                    vm = machine::call(vm, &mut hooks, routine("Pull"), &[]);
                    assert_eq!(vm.bus().ram().read_word(0xA0), 0xCAFE);
                    for mark in [0x27FEu16, 0x2001, 0x2000] {
                        vm = machine::call(vm, &mut hooks, routine("Free"), &mark.to_le_bytes());
                        assert_eq!(vm.bus().ram().read_word(0xE8), mark);
                    }
                    hooks.high_water = 0x2000;
                    vm = machine::call(vm, &mut hooks, routine("SwapScr"), &[]);
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x23E8);
                    // NewDrive -> Format -> confirmation uses these nested
                    // window geometries. Execute their real save/restore code.
                    for geometry in [[2, 8, 36, 2], [1, 7, 37, 4], [10, 9, 15, 4]] {
                        vm = machine::call(vm, &mut hooks, routine("Window"), &geometry);
                    }
                    for _ in 0..3 {
                        vm = machine::call(vm, &mut hooks, routine("CloseWindow"), &[]);
                    }
                    assert_eq!(vm.bus().ram().read_word(0xE8), 0x23E8);
                    assert_eq!(machine::bytes(&vm, 0x7000, 960), screen);
                    if name == "TNDBG.ACT" {
                        vm = machine::call(vm, &mut hooks, routine("Window"), &[1, 2, 38, 21]);
                        assert_eq!(vm.bus().ram().read_word(0xE8), 0x274C);
                        vm = machine::call(vm, &mut hooks, routine("CloseWindow"), &[]);
                        assert_eq!(vm.bus().ram().read_word(0xE8), 0x23E8);
                    }
                    assert_eq!(vm.bus().ram().read(global("ArenaFault")), 0);
                    assert_eq!(
                        hooks.high_water,
                        if name == "TNDBG.ACT" { 0x274C } else { 0x2583 }
                    );
                    eprintln!("UI high-water: ${:04X}", hooks.high_water);
                    if !crlf {
                        measurements.push_str(&format!(
                            "{name}\t{mode:?}\t{runtime:?}\tUI-high-water\t0\t{}\t0\n",
                            hooks.high_water
                        ));
                    }
                    // Two short lengths can overwrite t's parameter home.
                    // This is valid generic input, without a length overflow.
                    let t_home = parameter(&listing, "Strcat", "t");
                    let dst = t_home - 4;
                    vm.bus_mut().ram_mut().write(dst, 3);
                    vm.bus_mut().ram_mut().map(0x7200, &[2, 0, 0x73]).unwrap();
                    vm.bus_mut().ram_mut().write(0x7300, 5);
                    let mut args = dst.to_le_bytes().to_vec();
                    args.extend(0x7200u16.to_le_bytes());
                    let started = vm.cpu().cycles();
                    vm = machine::call(vm, &mut hooks, routine("Strcat"), &args);
                    if !crlf {
                        measurements.push_str(&format!(
                            "{name}\t{mode:?}\t{runtime:?}\tStrcat-alias-3-2\t2\t{dst}\t{}\n",
                            vm.cpu().cycles() - started
                        ));
                    }
                    assert_eq!(vm.bus().ram().read_word(t_home), 0x7300);
                    assert_eq!(vm.bus().ram().read(dst), 8);
                    calls += hooks.calls + failures(&bytes, &listing, runtime);
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("ACTIONC_ARENA_VM_RESULTS") {
        std::fs::write(path, measurements).unwrap();
    }
    eprintln!(
        "checked-arena success/fault controls: {calls} calls, both backends/runtimes and LF/CRLF"
    );
}

#[test]
#[ignore = "requires the frozen unchecked controls; measures only valid allocator calls"]
fn unchecked_valid_allocation_cycle_controls() {
    let mut measurements = String::new();
    for mode in ["optimized", "mir6502"] {
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            let rt = if runtime == Runtime::ActionCart {
                "cart"
            } else {
                "standalone"
            };
            for name in ["TN", "TNDBG"] {
                let stem = root().join(format!("build/checked-arena/{name}-{mode}-{rt}"));
                let bytes = std::fs::read(stem.with_extension("xex")).unwrap();
                let listing = std::fs::read_to_string(stem.with_extension("asm"))
                    .unwrap()
                    .replace("\r\n", "\n");
                let (mut vm, profile) = support::vm_for(runtime);
                vm.load_atari_object_for_execution(profile, &bytes).unwrap();
                let mut hooks = Observe {
                    high_water: 0,
                    calls: 0,
                };
                vm = machine::call(
                    vm,
                    &mut hooks,
                    symbols::routine_address(&listing, "Init"),
                    &[],
                );
                for n in [0u16, 1, 0, 255, 256, 0x800 - 512] {
                    let before = vm.bus().ram().read_word(0xE8);
                    let started = vm.cpu().cycles();
                    vm = machine::call(
                        vm,
                        &mut hooks,
                        symbols::routine_address(&listing, "Alloc"),
                        &n.to_le_bytes(),
                    );
                    assert_eq!(vm.bus().ram().read_word(0xE8), before + n);
                    measurements.push_str(&format!(
                        "{name}.ACT\t{}\t{runtime:?}\tAlloc\t{n}\t{before}\t{}\n",
                        if mode == "optimized" {
                            "Optimized"
                        } else {
                            "Mir6502"
                        },
                        vm.cpu().cycles() - started
                    ));
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("ACTIONC_UNCHECKED_ARENA_VM_RESULTS") {
        std::fs::write(path, measurements).unwrap();
    }
}
