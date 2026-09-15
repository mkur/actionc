//! The external provider is simulated; cache publication, display, navigation,
//! selection, end discovery and panel activation execute TN's real routines.
use super::{directory, global_address, machine, routine_address};
use actionc::compiler::CompiledProgram;
use actionc_vm::{CompilerVm, VmRunHooks};
use std::collections::{BTreeMap, VecDeque};

const RESULT: u16 = 0x0630;
const FOUND: u16 = 0x0640;
const NAME: u16 = 0x0D00;
struct Paging {
    directory: directory::Directory,
    entries: BTreeMap<u16, &'static str>,
    batch: u16,
    tags: u16,
    total: u16,
    requests: Vec<u16>,
    status: u8,
    long_names: bool,
    owner: Option<u16>,
    lost_cursor: bool,
    reopens: usize,
    keys: VecDeque<u8>,
}
impl VmRunHooks for Paging {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let r = vm.cpu().registers();
        let Some(&name) = self.entries.get(&r.pc) else {
            return self.directory.before_step(vm);
        };
        let ordinal = u16::from_le_bytes([r.a, r.x]);
        let mut status = 1;
        if name == "Getchar" {
            status = self
                .keys
                .pop_front()
                .expect("navigation consumed expected keys");
        } else if name == "ReadWindow" {
            let tags = vm.bus().ram().read_word(self.tags);
            if self.owner.is_some_and(|owner| owner != tags) {
                self.reopens += 1;
                if self.lost_cursor {
                    status = 4;
                }
            }
            self.owner = Some(tags);
            self.requests.push(ordinal);
            if self.status != 1 {
                status = self.status;
            }
            if status == 1 {
                assert!(ordinal <= self.total);
                let count = (self.total - ordinal).min(64);
                let batch = vm.bus().ram().read_word(self.batch);
                let ram = vm.bus_mut().ram_mut();
                assert_eq!(
                    ram.read(batch + 1283),
                    0,
                    "invalidate before provider writes"
                );
                ram.write(batch + 1282, count as u8);
                ram.write(batch + 1284, u8::from(ordinal + count == self.total));
                ram.map(batch + 1349, &[32; 18])?;
                for slot in 0..count {
                    let entry = batch + slot * 20;
                    ram.write_word(entry, ordinal + slot);
                    ram.write_word(entry + 2, ordinal + slot);
                    ram.write(entry + 4, 4); // Truncated preview: never an operation name.
                    ram.write(entry + 5, 32);
                    ram.map(entry + 6, b"SAMEPREVIEW")?;
                    ram.map(entry + 17, b"   ")?; // Unknown size, not zero sectors.
                    ram.write(batch + 1285 + slot, slot as u8);
                }
            }
        } else {
            assert_eq!(name, "ResolveEntry");
            let ram = vm.bus().ram();
            let output = u16::from_le_bytes([r.y, ram.read(0xA3)]);
            let capacity = ram.read_word(0xA4);
            let exact = if self.long_names {
                format!("D:Same visible prefix, complete filename {ordinal:05}.bin")
            } else {
                format!("D:F{ordinal:07}.BIN")
            };
            if exact.len() + 1 > usize::from(capacity) {
                status = 2;
            } else {
                let ram = vm.bus_mut().ram_mut();
                ram.write(output, exact.len() as u8);
                ram.map(output + 1, exact.as_bytes())?;
            }
        }
        vm.bus_mut()
            .ram_mut()
            .map(RESULT, &[0xA9, status, 0x85, 0xA0, 0x60])?;
        vm.set_pc(RESULT);
        Ok(())
    }
}
fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn check(compiled: &CompiledProgram) {
    let listing = compiled.source_listing();
    let routine = |s| routine_address(&listing, s);
    let global = |s| global_address(&listing, s);
    let mut hooks = Paging {
        directory: directory::Directory {
            entries: [
                "Path",
                "DrawWinFrame",
                "Inv",
                "Close",
                "Open",
                "Input",
                "PutImage",
            ]
            .into_iter()
            .map(|s| (routine(s), s))
            .collect(),
            ioerr: global("ioerr"),
            input: vec![b"999 FREE SECTORS\x9B\x9B\x9B\x9B".to_vec()],
            row: 0,
            images: vec![],
            names: vec![],
        },
        entries: ["ReadWindow", "ResolveEntry", "Getchar"]
            .into_iter()
            .map(|s| (routine(s), s))
            .collect(),
        batch: global("currentbatch"),
        tags: global("currenttags"),
        total: 1089,
        requests: vec![],
        status: 1,
        long_names: false,
        owner: None,
        lost_cursor: false,
        reopens: 0,
        keys: VecDeque::new(),
    };
    let mut vm = machine::load(compiled);
    for (a, v) in [(0x700, b'M'), (0x76F, 0xA9), (0x70A, 0xFF), (0x70B, 2)] {
        vm.bus_mut().ram_mut().write(a, v);
    }
    vm = machine::call(vm, &mut hooks, routine("InitPanels"), &[]);
    let active = global("active");
    let right = global("righttags");
    let left = global("lefttags");
    // Fixture-owned RAM, below the program's $2C00 origin. Guard each bitmap.
    for (tags, backing) in [(right, 0x0800), (left, 0x0A02)] {
        vm.bus_mut().ram_mut().map(backing, &[0xA5; 514]).unwrap();
        vm = machine::call(
            vm,
            &mut hooks,
            routine("TagsInit"),
            &words(&[tags, backing + 1, 4096]),
        );
    }
    let mut extent = words(&[right, 1089]);
    extent.push(1);
    vm = machine::call(vm, &mut hooks, routine("TagsExtent"), &extent);
    vm = machine::call(vm, &mut hooks, routine("FetchWindow"), &words(&[0]));
    assert_eq!(vm.bus().ram().read_word(active), 1089);
    for ordinal in [7u16, 63, 64, 256, 1024, 1088] {
        vm = machine::call(vm, &mut hooks, routine("Tag"), &words(&[ordinal]));
    }
    for ordinal in [15u16, 63, 64, 255, 256, 1023, 1024, 1088, 64, 0] {
        hooks.directory.images.clear();
        vm = machine::call(vm, &mut hooks, routine("GoTo"), &words(&[ordinal]));
        assert_eq!(vm.bus().ram().read_word(active + 2), ordinal);
        let top = ordinal.saturating_sub(15);
        let batch = vm.bus().ram().read_word(global("currentbatch"));
        let first = vm.bus().ram().read_word(batch + 1280);
        let count = u16::from(vm.bus().ram().read(batch + 1282));
        assert!(first <= top && first + count >= top + 16);
        assert_eq!(hooks.directory.images.len(), 17);
        for (offset, row) in hooks.directory.images[1..].iter().enumerate() {
            assert_eq!(
                row[0],
                if [7, 63, 64, 256, 1024, 1088].contains(&(top + offset as u16)) {
                    0x7F
                } else {
                    0
                }
            );
        }
    }
    assert!(
        hooks.requests.contains(&49),
        "viewport crossing 63/64 needs an unaligned window"
    );
    assert_eq!(vm.bus().ram().read_word(right + 6), 6);

    // Execute the wide navigation helper, including ignored boundary keys.
    for (selected, keys, expected) in [
        (0, vec![b'-', b'X'], 0),
        (1088, vec![b'=', b'X'], 1088),
        (255, vec![b'='], 256),
        (256, vec![b'-'], 255),
    ] {
        vm.bus_mut().ram_mut().write_word(active + 2, selected);
        hooks.keys = keys.into();
        vm = machine::call(vm, &mut hooks, routine("FileRange"), &[]);
        assert_eq!(vm.bus().ram().read_word(active + 2), expected);
        assert!(hooks.keys.is_empty());
    }
    vm = machine::call(vm, &mut hooks, routine("SwapWin"), &[]);
    vm = machine::call(vm, &mut hooks, routine("FetchWindow"), &words(&[0]));
    vm = machine::call(vm, &mut hooks, routine("Tag"), &words(&[7]));
    vm = machine::call(vm, &mut hooks, routine("SwapWin"), &[]);
    vm = machine::call(vm, &mut hooks, routine("FetchWindow"), &words(&[1024]));
    assert!(
        hooks.reopens >= 2,
        "source cursor must be restored per panel"
    );
    assert_eq!(vm.bus().ram().read_word(left + 6), 1);
    assert_eq!(vm.bus().ram().read_word(right + 6), 6);
    // Iterate selected ordinals independently of the cache and resolve exact
    // names. All six entries deliberately have the same clipped preview.
    let mut from = 0;
    for expected in [7u16, 63, 64, 256, 1024, 1088] {
        vm = machine::call(vm, &mut hooks, routine("FindNext"), &words(&[from, FOUND]));
        assert_eq!(vm.cpu().registers().a, 1);
        assert_eq!(vm.bus().ram().read_word(FOUND), expected);
        vm = machine::call(vm, &mut hooks, routine("Convert"), &words(&[expected]));
        assert_eq!(
            machine::counted(&vm, global("fname")),
            format!("D:F{expected:07}.BIN").as_bytes()
        );
        from = expected + 1;
    }
    vm = machine::call(vm, &mut hooks, routine("FindNext"), &words(&[from, FOUND]));
    assert_eq!(vm.cpu().registers().a, 0);
    hooks.long_names = true;
    for ordinal in [63, 1024] {
        vm = machine::call(
            vm,
            &mut hooks,
            routine("ResolveEntry"),
            &words(&[ordinal, NAME, 128]),
        );
        assert_eq!(
            machine::counted(&vm, NAME),
            format!("D:Same visible prefix, complete filename {ordinal:05}.bin").as_bytes()
        );
    }
    let before = machine::bytes(&vm, NAME, 128);
    vm = machine::call(
        vm,
        &mut hooks,
        routine("ResolveEntry"),
        &words(&[1024, NAME, 15]),
    );
    assert_eq!(vm.cpu().registers().a, 2);
    assert_eq!(machine::bytes(&vm, NAME, 128), before);

    // Unknown length: tag-all is pending until the complete bounded scan.
    vm = machine::call(vm, &mut hooks, routine("InvalidateListing"), &[]);
    vm = machine::call(vm, &mut hooks, routine("FetchWindow"), &words(&[0]));
    vm = machine::call(vm, &mut hooks, routine("ToggleAllTags"), &words(&[right]));
    vm = machine::call(vm, &mut hooks, routine("Tag"), &words(&[7]));
    vm = machine::call(vm, &mut hooks, routine("GoTo"), &words(&[31]));
    vm = machine::call(vm, &mut hooks, routine("PrepareSelection"), &[]);
    assert_eq!(vm.cpu().registers().a, 1);
    assert_eq!(vm.bus().ram().read_word(right + 6), 1088);
    assert_eq!(vm.bus().ram().read_word(active + 2), 31);
    vm = machine::call(vm, &mut hooks, routine("GoLast"), &[]);
    assert_eq!(vm.bus().ram().read_word(active + 2), 1088);

    vm = machine::call(vm, &mut hooks, routine("InvalidateListing"), &[]);
    vm = machine::call(vm, &mut hooks, routine("FetchWindow"), &words(&[0]));
    vm.bus_mut().ram_mut().write_word(active + 2, 63);
    hooks.keys = vec![b'='].into();
    vm = machine::call(vm, &mut hooks, routine("FileRange"), &[]);
    assert_eq!(vm.bus().ram().read_word(active + 2), 64);
    vm = machine::call(vm, &mut hooks, routine("GoTo"), &words(&[31]));
    vm = machine::call(vm, &mut hooks, routine("ToggleAllTags"), &words(&[right]));
    hooks.status = 6;
    vm = machine::call(vm, &mut hooks, routine("PrepareSelection"), &[]);
    assert_eq!(vm.cpu().registers().a, 6);
    assert_eq!(
        vm.bus().ram().read_word(active + 2),
        31,
        "failed end scan preserves selection"
    );
    hooks.status = 1;

    hooks.total = 4097;
    vm = machine::call(vm, &mut hooks, routine("InvalidateListing"), &[]);
    vm = machine::call(vm, &mut hooks, routine("FetchWindow"), &words(&[0]));
    vm = machine::call(vm, &mut hooks, routine("ToggleAllTags"), &words(&[right]));
    vm = machine::call(vm, &mut hooks, routine("PrepareSelection"), &[]);
    assert_eq!(
        vm.cpu().registers().a,
        2,
        "no partial tag-all beyond capacity"
    );
    assert!(hooks.directory.names.iter().all(|(mode, _)| *mode == 6));
    vm = machine::call(vm, &mut hooks, routine("ClearTags"), &words(&[right]));
    vm = machine::call(vm, &mut hooks, routine("GoTo"), &words(&[4096]));
    assert_eq!(
        vm.bus().ram().read_word(active + 2),
        4096,
        "browsing exceeds bitmap capacity"
    );

    // Ordinary I/O failure cannot publish a partial cache. A changed listing
    // (including an unrepeatable cursor lost on panel activation) clears tags.
    hooks.status = 6;
    vm = machine::call(vm, &mut hooks, routine("FetchWindow"), &words(&[0]));
    assert_eq!(vm.cpu().registers().a, 6);
    let batch = vm.bus().ram().read_word(global("currentbatch"));
    assert_eq!(vm.bus().ram().read(batch + 1283), 0);
    hooks.status = 1;
    hooks.lost_cursor = true;
    hooks.owner = Some(left);
    let generation = vm.bus().ram().read_word(right + 8);
    vm = machine::call(vm, &mut hooks, routine("FetchWindow"), &words(&[0]));
    assert_eq!(vm.cpu().registers().a, 4);
    assert_eq!(vm.bus().ram().read_word(right + 8), generation + 1);
    assert_eq!(vm.bus().ram().read_word(right + 6), 0);
    assert_eq!(vm.bus().ram().read_word(active), 0);
    for address in [0x0800, 0x0A01, 0x0A02, 0x0C03] {
        assert_eq!(vm.bus().ram().read(address), 0xA5);
    }
}
