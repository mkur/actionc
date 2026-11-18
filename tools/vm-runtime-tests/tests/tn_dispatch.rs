//! Execute TN's real key lookup, dispatcher and Handle loop with UI/command
//! entry points stubbed. These checks do not execute MyDOS disk operations.
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use actionc::compiler::{CompileMode, CompileOptions, CompiledProgram, Runtime, compile_file};
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunHooks, VmRunner,
};

#[allow(dead_code)]
#[path = "support/fixed_point.rs"]
mod fixed_point;

#[path = "support/tn_copy.rs"]
mod copy;
#[path = "support/tn_directory.rs"]
mod directory;
#[path = "support/tn_machine.rs"]
mod machine;
#[path = "support/tn_menus.rs"]
mod menus;
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
        if name == "Range" {
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
    ram.write(files_address, files);
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
        trace.extend(["Close", "Range"]);
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
    trace.extend(["Close", "Range"]);
    trace
}

fn check_conversion_leaf(compiled: &CompiledProgram, listing: &str, name: &str, map: &str, values: [u8; 4]) {
    use actionc_vm::{AddressRange, BusAccess};
    let leaf = routine_address(listing, name);
    let table = global_address(listing, map);
    let mut vm = machine::load(compiled);
    vm.bus_mut().add_watch_range(AddressRange { start: 0, end: u16::MAX });
    let [lo, hi] = leaf.to_le_bytes();
    for (carry, set_carry) in [(false, 0x18), (true, 0x38)] {
        vm.bus_mut().ram_mut().map(ENTRY, &[set_carry, 0xa9, 0, 0x20, lo, hi, 0xea]).unwrap();
        for ch in 0..=255u8 {
            let index = (ch >> 5) & 3;
            let expected = (ch & 0x9f) | values[usize::from(index)];
            let saved = [ch, !ch, 0x42, 0x81];
            let ram = vm.bus_mut().ram_mut();
            ram.write(ENTRY + 2, ch);
            ram.write(0xa0, !expected);
            ram.map(0xac, &saved).unwrap();
            vm.set_pc(ENTRY);
            vm.bus_mut().clear_events();
            let sp = vm.cpu().registers().sp;
            let stop = ENTRY + 6;
            let run = VmRunner::new(vm).run(RunRequest {
                max_steps: 32, stop_after_pc: Some(stop), history_len: 0,
            });
            assert_eq!(run.stop_reason(), StopReason::PcReached { pc: stop });
            vm = run.into_vm();
            let stack = [0x100 + u16::from(sp), 0x100 + u16::from(sp.wrapping_sub(1))];
            let mut fetches = Vec::new();
            let mut stack_accesses = Vec::new();
            let mut data = Vec::new();
            for e in vm.bus().events() {
                if e.access == BusAccess::Read && ((ENTRY..=stop).contains(&e.address)
                    || (leaf..leaf + 17).contains(&e.address)) {
                    fetches.push(e.address);
                } else if stack.contains(&e.address) {
                    stack_accesses.push((e.access, e.address));
                } else {
                    data.push((e.access, e.address));
                }
            }
            // The runner stops after executing the NOP at the return address.
            assert!(fetches.into_iter().eq((ENTRY..stop).chain(leaf..leaf + 17).chain([stop])));
            assert_eq!(stack_accesses, [
                (BusAccess::Write, stack[0]), (BusAccess::Write, stack[1]),
                (BusAccess::Read, stack[1]), (BusAccess::Read, stack[0]),
            ]);
            assert_eq!(data, [(BusAccess::Read, table + u16::from(index)), (BusAccess::Write, 0xa0)],
                "{name}({ch}) carry={carry}");
            assert_eq!(vm.cpu().registers().sp, sp);
            assert_eq!((vm.cpu().registers().a, vm.bus().ram().read(0xa0)), (expected, expected));
            assert_eq!(machine::bytes(&vm, 0xac, 4), saved);
        }
    }
}

struct Direct;
impl VmRunHooks for Direct { type Error = String; }

fn run_init(compiled: &CompiledProgram, listing: &str, shift: u32) -> CompilerVm {
    let table = global_address(listing, "quickmul");
    let mut vm = machine::load(compiled);
    vm.bus_mut().ram_mut().map(table, &[0xa5; 48]).unwrap();
    vm = machine::call(vm, &mut Direct, routine_address(listing, "Init"), &[]);
    for row in 0..24 {
        let offset = 40u16 * row;
        assert_eq!(vm.bus().ram().read(table + row), offset as u8, "Init low byte, row {row}");
        assert_eq!(vm.bus().ram().read(table + 24 + row), (offset >> shift) as u8,
            "Init RSH {shift}, row {row}");
    }
    vm
}

#[test]
fn tn_init_keeps_its_word_across_shift_helper_calls() {
    let loaded = actionc::includes::load_program_with_expanded_source(root().join("samples/tn/modern/TN.ACT")).unwrap();
    let source = loaded.source.replace("\r\n", "\n");
    let original = "quickmul(i + SCREEN_ROWS) = h rsh 8";
    assert_eq!(source.matches(original).count(), 1);
    for (shift, newline) in [(5, "\n"), (6, "\r\n")] {
        let variant = source.replace(original, &format!("quickmul(i + SCREEN_ROWS) = h rsh {shift}"))
            .replace('\n', newline);
        let file = fixed_point::Source::new(&variant);
        let compiled = compile_file(&file.0, &CompileOptions::for_mode(CompileMode::Mir6502)
            .with_runtime(Runtime::ActionCart).with_origin(0x3000)).unwrap();
        let listing = compiled.source_listing();
        let init = listing.lines().skip_while(|l| *l != "proc_Init:")
            .take_while(|l| !l.starts_with("; ===== END PROC Init"));
        assert!(init.into_iter().any(|l| l.contains("JSR.A $A0E6")), "missing RShift in Init");
        assert_zp_home(&listing, "local_Init_h", 2);
        run_init(&compiled, &listing, shift);
    }
}

fn assert_zp_home(listing: &str, label: &str, width: u16) {
    let prefix = format!("{label} = $");
    let home = listing.lines().find_map(|line| line.strip_prefix(&prefix))
        .and_then(|hex| u16::from_str_radix(hex, 16).ok())
        .unwrap_or_else(|| panic!("missing ZP home for {label}"));
    assert!(home <= 0x100 - width, "{label} needs {width} zero-page bytes");
}

fn check_init_and_calcadr(compiled: &CompiledProgram, listing: &str) {
    use actionc_vm::{AddressRange, BusAccess};
    let [lo, hi] = routine_address(listing, "CalcAdr").to_le_bytes();
    let table = global_address(listing, "quickmul");
    let mut vm = run_init(compiled, listing, 8);
    vm.bus_mut().add_watch_range(AddressRange { start: 0, end: u16::MAX });
    for row in 0..=255u8 {
        // The two indexed reads are defined by the CPU even beyond the table.
        let address = table + u16::from(row);
        let offset = u16::from_le_bytes([vm.bus().ram().read(address), vm.bus().ram().read(address + 24)]);
        let base = 0x710fu16;
        let column = row.wrapping_add(79);
        let (sum, carry) = base.overflowing_add(offset);
        let expected = sum.wrapping_add(u16::from(carry)).wrapping_add(u16::from(column));
        for (flag, poison) in [(0x18, 0x35), (0x38, 0xca)] {
            let saved_y = row ^ poison;
            let saved_pairs = [row, !row, poison, !poison];
            let ram = vm.bus_mut().ram_mut();
            ram.write_word(0x58, base);
            ram.write(0xfe, row);
            ram.write(0xff, column);
            ram.write_word(0xe6, !expected);
            ram.map(0xac, &saved_pairs).unwrap();
            // Vary the incoming A, X, Y and carry before the real JSR.
            ram.map(ENTRY, &[0xa9, poison, 0xa2, !poison, 0xa0, saved_y, flag, 0x20, lo, hi, 0xea]).unwrap();
            vm.set_pc(ENTRY);
            let sp = vm.cpu().registers().sp;
            vm.bus_mut().clear_events();
            let stop = ENTRY + 10;
            let run = VmRunner::new(vm).run(RunRequest { max_steps: 32, stop_after_pc: Some(stop), history_len: 0 });
            assert_eq!(run.stop_reason(), StopReason::PcReached { pc: stop });
            vm = run.into_vm();
            assert_eq!(machine::bytes(&vm, 0xe6, 2), expected.to_le_bytes(), "row {row}");
            assert_eq!(machine::bytes(&vm, 0xac, 4), saved_pairs);
            assert_eq!((vm.cpu().registers().y, vm.cpu().registers().sp), (saved_y, sp));
            let stack = [0x100 + u16::from(sp), 0x100 + u16::from(sp.wrapping_sub(1))];
            for event in vm.bus().events().iter().filter(|e| e.access == BusAccess::Write) {
                assert!(stack.contains(&event.address) || matches!(event.address, 0xe6 | 0xe7));
            }
        }
    }
}

fn check_store_and_value(compiled: &CompiledProgram, listing: &str) {
    use actionc_vm::{AddressRange, BusAccess};

    let store = routine_address(listing, "Store");
    let value = routine_address(listing, "Value");
    let mut vm = machine::load(compiled);
    vm.bus_mut().add_watch_range(AddressRange { start: 0, end: u16::MAX });

    for address in [0x08ffu16, 0x0900, 0xac, 0xad, 0xae, 0xaf, 0xa0, 0xa1] {
        let [lo, hi] = address.to_le_bytes();
        let ram = vm.bus_mut().ram_mut();
        ram.map(0xac, &[0x12, 0x34, 0x56, 0x78]).unwrap();
        ram.write(address, 0x69);
        let pairs = machine::bytes(&vm, 0xac, 4);
        // Value installs p at $A0/$A1 before reading through it.
        let expected = match address { 0xa0 => lo, 0xa1 => hi, _ => 0x69 };
        let sp = vm.cpu().registers().sp;
        let stack = [0x100 + u16::from(sp), 0x100 + u16::from(sp.wrapping_sub(1))];
        vm.bus_mut().clear_events();
        vm = machine::call(vm, &mut Direct, value, &[lo, hi, 0x97]);
        assert_eq!(vm.cpu().registers().x, hi);
        assert_eq!(vm.cpu().registers().a, expected);
        assert_eq!(vm.bus().ram().read(0xa0), expected);
        assert_eq!(machine::bytes(&vm, 0xac, 4), pairs);
        for event in vm.bus().events().iter().filter(|e| e.access == BusAccess::Write) {
            assert!(stack.contains(&event.address) || matches!(event.address, 0xa0 | 0xa1),
                "Value(${address:04X}) wrote ${:04X}", event.address);
        }

        vm = machine::call(vm, &mut Direct, store, &[lo, hi, 0xc5]);
        assert_eq!(vm.bus().ram().read(address), 0xc5, "Store(${address:04X})");
        assert_eq!(vm.cpu().registers().x, hi);
    }
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
            if mode == CompileMode::Mir6502 {
                if let Some(dir) = std::env::var_os("ACTIONC_WORD_LOAD_MEASURED_IMAGES")
                    .or_else(|| std::env::var_os("ACTIONC_ARGUMENT_X_BITWISE_MEASURED_IMAGES"))
                    .or_else(|| std::env::var_os("ACTIONC_CALLEE_ENTRY_FLAGS_MEASURED_IMAGES"))
                    .or_else(|| std::env::var_os("ACTIONC_ARGUMENT_X_MEASURED_IMAGES"))
                    .or_else(|| std::env::var_os("ACTIONC_POINTER_LOAD_MEASURED_IMAGES"))
                    .or_else(|| std::env::var_os("ACTIONC_POINTER_MEASURED_IMAGES"))
                    .or_else(|| std::env::var_os("ACTIONC_ARGUMENT_MEASURED_IMAGES"))
                {
                    let path = PathBuf::from(dir).join(format!(
                        "{}-ActionCart-true.xex", source.trim_end_matches(".ACT")
                    ));
                    assert_eq!(
                        compiled.object_bytes(), std::fs::read(&path).unwrap(),
                        "measured image differs: {}", path.display()
                    );
                }
            }
            let listing = compiled.source_listing();
            if mode == CompileMode::Mir6502 && !debug {
                for (label, width) in [
                    ("param_Convert_s", 2u16), ("param_Putchar_ch", 1),
                    ("param_Tag_i", 1), ("local_Tag_a", 1), ("local_Init_h", 2),
                    ("param_Draw_cnt", 1), ("param_Inv_cnt", 1), ("local_Push_captured", 2),
                ] {
                    assert_zp_home(&listing, label, width);
                }
            }
            check_store_and_value(&compiled, &listing);
            if !debug {
                check_init_and_calcadr(&compiled, &listing);
                check_conversion_leaf(&compiled, &listing, "Ascii", "asciiMap", [0x20, 0x40, 0, 0x60]);
                check_conversion_leaf(&compiled, &listing, "Internal", "internalMap", [0x40, 0, 0x20, 0x60]);
            }
            let handle = routine_address(&listing, "Handle");
            let files_address = global_address(&listing, "active");
            let mut entries = BTreeMap::new();
            for name in ["Inv", "Close", "CloseAll", "Range"].into_iter().chain(
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
            menus::check(&compiled);
        }
    }
}
