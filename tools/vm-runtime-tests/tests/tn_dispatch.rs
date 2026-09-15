//! Execute TN's real key lookup, dispatcher and Handle loop with UI/command
//! entry points stubbed. These checks do not execute MyDOS disk operations.
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, CompiledProgram, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunHooks, VmRunner,
};

#[path = "support/tn_copy.rs"]
mod copy;
#[path = "support/tn_directory.rs"]
mod directory;
#[path = "support/tn_machine.rs"]
mod machine;
#[path = "support/tn_panels.rs"]
mod panels;

const ENTRY: u16 = 0x0600;
const RETURN: u16 = 0x0610;
const KEY: u16 = 0x0611;
const STOP: u16 = 0x0620;
const COMMANDS: &[(u8, &str)] = &[
    (b'V', "View"),
    (b'A', "Attrib"),
    (b'D', "Delete"),
    (b'R', "Rename"),
    (b'S', "TagAll"),
    (b'C', "Copy"),
    (b'M', "MkDir"),
    (b'F', "Format"),
    (b'G', "Dbg"),
    (b'Q', "Quit"),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[path = "support/tn_symbols.rs"]
mod symbols;
use symbols::{global_address, routine_address};

struct DispatchHooks {
    entries: BTreeMap<u16, &'static str>,
    keys: VecDeque<u8>,
    trace: Vec<&'static str>,
    input_stack_depths: Vec<u8>,
}

impl VmRunHooks for DispatchHooks {
    type Error = String;

    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let Some(&name) = self.entries.get(&vm.cpu().registers().pc) else {
            return Ok(());
        };
        self.trace.push(name);
        if name == "FileRange" {
            self.input_stack_depths.push(vm.cpu().registers().sp);
            if let Some(key) = self.keys.pop_front() {
                // Match Range's CHAR result ABI in A and $A0, then really RTS.
                vm.bus_mut().ram_mut().write(KEY + 1, key);
                vm.set_pc(KEY);
            } else {
                vm.set_pc(STOP);
            }
        } else {
            // Real RTS also exercises dispatch's return path and stack balance.
            vm.set_pc(RETURN);
        }
        Ok(())
    }
}

fn run(
    compiled: &CompiledProgram,
    entries: &BTreeMap<u16, &'static str>,
    handle: u16,
    files_address: u16,
    files: u8,
    keys: &[u8],
    expected: &[&str],
) {
    let mut vm = CompilerVm::default();
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
    let loaded = vm
        .load_atari_object_for_execution(ExecutionProfile::CartridgeObject, compiled.object_bytes())
        .unwrap();
    assert!(
        loaded
            .segments
            .iter()
            .all(|s| s.end < ENTRY || s.start > STOP)
    );
    let ram = vm.bus_mut().ram_mut();
    ram.write_word(files_address, u16::from(files));
    ram.map(ENTRY, &[0x20, handle as u8, (handle >> 8) as u8, 0xEA])
        .unwrap();
    ram.map(RETURN, &[0x60, 0xA9, 0, 0x85, 0xA0, 0x60]).unwrap();
    ram.write(STOP, 0xEA);
    vm.set_pc(ENTRY);
    let mut hooks = DispatchHooks {
        entries: entries.clone(),
        keys: keys.iter().copied().collect(),
        trace: Vec::new(),
        input_stack_depths: Vec::new(),
    };
    let result = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: 20_000,
                stop_after_pc: Some(STOP),
                history_len: 8,
            },
            &mut hooks,
        )
        .unwrap();
    assert_eq!(
        result.stop_reason(),
        StopReason::PcReached { pc: STOP },
        "files={files}, keys={keys:?}: {:?}; trace={:?}",
        result.report,
        hooks.trace
    );
    assert_eq!(hooks.trace, expected, "files={files}, keys={keys:?}");
    assert_eq!(hooks.input_stack_depths.len(), keys.len() + 1);
    assert!(
        hooks
            .input_stack_depths
            .windows(2)
            .all(|pair| pair[0] == pair[1]),
        "dispatch leaked stack space: {:?}",
        hooks.input_stack_depths
    );
}

fn expected_trace(commands: &[Option<&'static str>]) -> Vec<&'static str> {
    let mut trace = vec!["Inv"];
    for &command in commands {
        trace.extend(["Close", "FileRange"]);
        if let Some(name) = command {
            trace.push(name);
        }
        if command == Some("Copy") {
            // Copy exits the inner loop, skipping CloseAll and restarting Inv.
            trace.push("Inv");
        } else {
            trace.push("CloseAll");
        }
    }
    trace.extend(["Close", "FileRange"]);
    trace
}

#[test]
fn tn_dispatch_and_panel_state_preserve_behavior() {
    // The maintained programs advertise cartridge builds in these two modes.
    // Compile each full program once, then exercise every command and repeated
    // dispatch and panel transitions on the same generated code. No source body
    // is copied into tests.
    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        for (source, debug) in [("TN.ACT", false), ("TNDBG.ACT", true)] {
            let compiled = compile_file(
                root().join("samples/tn/modern").join(source),
                &CompileOptions::for_mode(mode).with_runtime(Runtime::ActionCart),
            )
            .unwrap_or_else(|error| panic!("{source}/{mode:?}: {error}"));
            let listing = compiled.source_listing();
            let handle = routine_address(&listing, "Handle");
            let files_address = global_address(&listing, "active");
            let mut entries = BTreeMap::new();
            for name in ["Inv", "Close", "CloseAll", "FileRange"].into_iter().chain(
                COMMANDS
                    .iter()
                    .filter(|(_, name)| debug || *name != "Dbg")
                    .map(|(_, name)| *name),
            ) {
                assert!(
                    entries
                        .insert(routine_address(&listing, name), name)
                        .is_none()
                );
            }
            let crlf = listing.replace("\r\n", "\n").replace('\n', "\r\n");
            assert_eq!(routine_address(&crlf, "Handle"), handle);
            assert_eq!(global_address(&crlf, "active"), files_address);
            for (&address, &name) in &entries {
                assert_eq!(routine_address(&crlf, name), address);
            }
            eprintln!(
                "{source}/{mode:?}: {} load-file bytes",
                compiled.object_bytes().len()
            );
            for files in [0, 2] {
                let allowed = |name| {
                    (debug || name != "Dbg")
                        && (files != 0 || matches!(name, "MkDir" | "Format" | "Dbg" | "Quit"))
                };
                for &(key, name) in COMMANDS {
                    let expected = expected_trace(&[allowed(name).then_some(name)]);
                    run(
                        &compiled,
                        &entries,
                        handle,
                        files_address,
                        files,
                        &[key],
                        &expected,
                    );
                }
                run(
                    &compiled,
                    &entries,
                    handle,
                    files_address,
                    files,
                    b"?",
                    &expected_trace(&[None]),
                );
                let keys = b"VCMVGQ?C";
                let commands: Vec<_> = keys
                    .iter()
                    .map(|key| {
                        COMMANDS
                            .iter()
                            .find(|(candidate, _)| candidate == key)
                            .and_then(|(_, name)| allowed(name).then_some(*name))
                    })
                    .collect();
                run(
                    &compiled,
                    &entries,
                    handle,
                    files_address,
                    files,
                    keys,
                    &expected_trace(&commands),
                );
            }
            panels::check(&compiled, debug);
            directory::check(&compiled, debug);
            copy::check(&compiled, debug);
        }
    }
}
