//! Panel transitions execute intact, including MovePage, sorting, tagging and
//! directory conversion. Only screen output and CIO directory input are stubbed.
use super::{global_address, root, routine_address};
use actionc::compiler::CompiledProgram;
use actionc_vm::{
    CompilerVm, DEFAULT_CART_BASE, ExecutionProfile, ImageKind, OS_ROM_BASE, RunRequest,
    StopReason, VmRunHooks, VmRunner,
};
use std::collections::BTreeMap;

const ENTRY: u16 = 0x0600;
const STOP: u16 = ENTRY + 7;
const RETURN: u16 = 0x0610;
const DRIVE: u16 = 0x070B;

struct Directory {
    entries: BTreeMap<u16, &'static str>,
    ioerr: u16,
    current_dir: u16,
    row: usize,
    opens: Vec<(u8, u16)>,
}
impl VmRunHooks for Directory {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let Some(&name) = self.entries.get(&vm.cpu().registers().pc) else {
            return Ok(());
        };
        if name == "Open" {
            let ram = vm.bus().ram();
            self.opens.push((
                ram.read(DRIVE),
                ram.read_word(ram.read_word(self.current_dir)),
            ));
            self.row = 0;
            vm.bus_mut().ram_mut().write(self.ioerr, 1);
        } else if name == "Input" {
            // Input(BYTE channel, CARD buffer, BYTE length) receives the buffer
            // in X/Y and the fourth argument byte in $A3.
            let registers = vm.cpu().registers();
            let buffer = u16::from_le_bytes([registers.x, registers.y]);
            assert_eq!(vm.bus().ram().read(0xA3), 19);
            let row = match self.row {
                0 => Some(&b"  A       TXT 001\x9B"[..]),
                1 => Some(&b"999 FREE SECTORS\x9B"[..]),
                _ => None,
            };
            let ram = vm.bus_mut().ram_mut();
            if let Some(row) = row {
                let mut bytes = [0x9B; 19];
                bytes[..row.len()].copy_from_slice(row);
                ram.map(buffer, &bytes)?;
                ram.write(self.ioerr, 1);
            } else {
                ram.write(self.ioerr, 136);
            }
            self.row += 1;
        }
        vm.set_pc(RETURN);
        Ok(())
    }
}

fn call(mut vm: CompilerVm, hooks: &mut Directory, address: u16, a: u8, x: u8) -> CompilerVm {
    vm.bus_mut()
        .ram_mut()
        .map(
            ENTRY,
            &[
                0xA9,
                a,
                0xA2,
                x,
                0x20,
                address as u8,
                (address >> 8) as u8,
                0xEA,
            ],
        )
        .unwrap();
    vm.set_pc(ENTRY);
    let sp = vm.cpu().registers().sp;
    let result = VmRunner::new(vm)
        .run_with_hooks(
            RunRequest {
                max_steps: 100_000,
                stop_after_pc: Some(STOP),
                history_len: 8,
            },
            hooks,
        )
        .unwrap();
    assert_eq!(
        result.stop_reason(),
        StopReason::PcReached { pc: STOP },
        "{:?}",
        result.report
    );
    assert_eq!(
        result.report.registers.sp, sp,
        "panel call leaked stack space"
    );
    result.into_vm()
}

// Compare semantic state, independent of the old packed panel layout.
fn state(vm: &CompilerVm, active: u16, tags: u16) -> Vec<u16> {
    let ram = vm.bus().ram();
    vec![ram.read_word(active), ram.read(active + 4).into(),
         ram.read(active + 5).into(), u16::from(ram.read(ram.read_word(tags) + 11)) * 0x7F,
         ram.read_word(active + 2), ram.read(active + 6).into(),
         ram.read(active + 7).into(), ram.read(active + 8).into()]
}

pub(super) fn check(compiled: &CompiledProgram, debug: bool) {
    let listing = compiled.source_listing();
    let global = |name| global_address(&listing, name);
    let routine = |name| routine_address(&listing, name);
    let active = global("active");
    let current_dir = global("currentdir");
    let tags = global("currenttags");
    let dirsectors = global("dirsectors");
    let batch = global("currentbatch");
    let winnum = global("winnum");
    let dummy = global("dummy");
    let mut entries = BTreeMap::new();
    for name in [
        "Path",
        "DrawWinFrame",
        "UpdDis",
        "Inv",
        "Close",
        "Open",
        "Input",
    ] {
        assert!(entries.insert(routine(name), name).is_none());
    }
    // Both MyDOS current-directory locations, with a separate RAM disk and
    // without one. All symbol addresses come from this compiled program.
    for (dostst, directory, ramdisk, other_drive) in [(0xA9, 0x07B8, 4, 4), (0, 0x07BB, 0xFF, 2)] {
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
            .load_atari_object_for_execution(
                ExecutionProfile::CartridgeObject,
                compiled.object_bytes(),
            )
            .unwrap();
        assert!(
            loaded
                .segments
                .iter()
                .all(|s| s.end < ENTRY || s.start > RETURN)
        );
        let ram = vm.bus_mut().ram_mut();
        ram.write(RETURN, 0x60);
        ram.write(0x700, b'M');
        ram.write(0x76F, dostst);
        ram.write(0x70A, ramdisk);
        ram.write(DRIVE, 2);
        ram.write_word(dummy, 0xADDE); // Guard immediately after active state.
        let mut hooks = Directory {
            entries: entries.clone(),
            ioerr: global("ioerr"),
            current_dir,
            row: 0,
            opens: Vec::new(),
        };
        vm = call(vm, &mut hooks, routine("InitPanels"), 0, 0);
        assert_eq!(vm.bus().ram().read_word(current_dir), directory);
        assert_eq!(hooks.opens, [(2, 0x169), (other_drive, 0x169)]);
        assert_eq!(state(&vm, active, tags), [1, 0, 2, 0, 0, 0, 0x69, 1]);
        assert_eq!(vm.bus().ram().read(winnum), 0);
        let right_path = vm.bus().ram().read_word(dirsectors);
        let right_table = vm.bus().ram().read_word(batch);
        let right_tags = vm.bus().ram().read_word(tags);
        let right_bits = vm.bus().ram().read_word(right_tags);
        assert_eq!(vm.bus().ram().read_word(right_tags + 6), 0);
        assert_eq!(vm.bus().ram().read_word(global("lefttags") + 6), 0);

        // Tagging operates on this panel's directory data and separate count.
        vm = call(vm, &mut hooks, routine("Tag"), 0, 0);
        assert_eq!(vm.bus().ram().read(right_bits), 1);
        assert_eq!(vm.bus().ram().read_word(right_tags + 6), 1);
        assert_eq!(state(&vm, active, tags)[3], 0x7F, "tags={:?}", (0..14).map(|o|vm.bus().ram().read(right_tags+o)).collect::<Vec<_>>());
        // Exercise selection/row fields through the real navigation routine.
        vm.bus_mut().ram_mut().write(active, 24);
        vm = call(vm, &mut hooks, routine("GoTo"), 19, 0);
        assert_eq!(&state(&vm, active, tags)[4..6], [19, 15]);
        let ram = vm.bus_mut().ram_mut();
        ram.write(active + 4, 4);
        ram.write(active + 5, 0xFE); // Deliberately stale OS shadows.
        ram.write_word(active + 7, 0xFFFF);
        ram.write(DRIVE, 3);
        ram.write_word(directory, 0x4321);
        ram.write_word(right_path, 0x2345);

        vm = call(vm, &mut hooks, routine("SwapWin"), 0, 0);
        assert_eq!(state(&vm, active, tags), [1, 0, u16::from(other_drive), 0, 0, 0, 0x69, 1]);
        assert_eq!(vm.bus().ram().read(winnum), 1);
        assert_eq!(vm.bus().ram().read(0x5A), 21);
        let left_path = vm.bus().ram().read_word(dirsectors);
        let left_table = vm.bus().ram().read_word(batch);
        assert_ne!(left_path, right_path);
        assert_ne!(left_table, right_table);
        assert_eq!(vm.bus().ram().read_word(vm.bus().ram().read_word(tags) + 6), 0);
        assert_eq!(vm.bus().ram().read_word(global("lefttags") + 6), 0);
        let ram = vm.bus_mut().ram_mut();
        ram.map(active, &[17, 0, 7, 0, 2, 0xFE, 7, 0xFF, 0xFF])
            .unwrap();
        ram.write(DRIVE, 5);
        ram.write_word(directory, 0x6543);
        ram.write_word(left_path, 0x4567);

        // Saving and restoring the same panel must capture the live OS values.
        vm = call(vm, &mut hooks, routine("SetWin"), 1, 0);
        assert_eq!(state(&vm, active, tags), [17, 2, 5, 0, 7, 7, 0x43, 0x65]);
        vm = call(vm, &mut hooks, routine("SwapWin"), 0, 0);
        assert_eq!(state(&vm, active, tags), [24, 4, 3, 0x7F, 19, 15, 0x21, 0x43]);
        assert_eq!(vm.bus().ram().read(DRIVE), 3);
        assert_eq!(vm.bus().ram().read_word(directory), 0x4321);
        assert_eq!(vm.bus().ram().read_word(dirsectors), right_path);
        assert_eq!(vm.bus().ram().read_word(batch), right_table);
        assert_eq!(vm.bus().ram().read_word(right_path), 0x2345);
        assert_eq!(vm.bus().ram().read_word(left_path), 0x4567);
        assert_eq!(vm.bus().ram().read_word(right_tags + 6), 1);
        assert_eq!(
            hooks.opens.len(),
            2,
            "switching must not reread a directory"
        );

        // Reload keeps the depth and live directory, but resets selection/tags.
        vm = call(vm, &mut hooks, routine("Dir"), 0, 0);
        assert_eq!(state(&vm, active, tags), [1, 4, 3, 0, 0, 0, 0x21, 0x43]);
        assert_eq!(vm.bus().ram().read_word(right_tags + 6), 0);
        assert_eq!(vm.bus().ram().read(right_bits), 0);
        assert_eq!(hooks.opens.last(), Some(&(3, 0x4321)));
        // Selecting another drive resets depth and selects its root directory.
        vm = call(vm, &mut hooks, routine("SetWin"), 0, 8);
        assert_eq!(state(&vm, active, tags), [1, 0, 8, 0, 0, 0, 0x69, 1]);
        assert_eq!(hooks.opens.last(), Some(&(8, 0x169)));
        vm = call(vm, &mut hooks, routine("SwapWin"), 0, 0);
        assert_eq!(state(&vm, active, tags), [17, 2, 5, 0, 7, 7, 0x43, 0x65]);
        assert_eq!(vm.bus().ram().read_word(batch), left_table);
        assert_eq!(vm.bus().ram().read_word(dirsectors), left_path);
        assert_eq!(vm.bus().ram().read(DRIVE), 5);
        assert_eq!(vm.bus().ram().read_word(directory), 0x6543);
        // InitPanels' persistent initialization guard must still work.
        vm = call(vm, &mut hooks, routine("InitPanels"), 0, 0);
        assert_eq!(state(&vm, active, tags), [17, 2, 5, 0, 7, 7, 0x43, 0x65]);
        assert_eq!(hooks.opens.len(), 4);
        assert_eq!(vm.bus().ram().read_word(dummy), 0xADDE);
        if debug {
            assert_eq!(vm.bus().ram().read(global("dbgsethits")), 9);
            assert_eq!(vm.bus().ram().read(global("dbgdirhits")), 1);
        }
    }
}
