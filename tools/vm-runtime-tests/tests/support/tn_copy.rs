//! Execute Copy against stateful file services, retaining its real panel and
//! directory routines. The tiny transfer arena forces continuation and EOF.
use super::{directory, global_address, machine, routine_address};
use actionc::compiler::CompiledProgram;
use actionc_vm::{CompilerVm, VmRunHooks};
use std::collections::BTreeMap;

struct Files {
    directory: directory::Directory,
    services: BTreeMap<u16, &'static str>,
    inputs: BTreeMap<Vec<u8>, Vec<u8>>,
    outputs: BTreeMap<Vec<u8>, Vec<u8>>,
    reading: Vec<u8>,
    writing: Vec<u8>,
    offset: usize,
    reads: Vec<(usize, u8)>,
    write_opens: usize,
    prompts: Vec<Vec<u8>>,
}

impl VmRunHooks for Files {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let pc = vm.cpu().registers().pc;
        let Some(&name) = self.services.get(&pc) else {
            return self.directory.before_step(vm);
        };
        let r = vm.cpu().registers();
        let (a, x, y) = (r.a, r.x, r.y);
        let address = u16::from_le_bytes([x, y]);
        match name {
            "Open" => {
                let mode = vm.bus().ram().read(0xA3);
                let filename = machine::counted(vm, address);
                match mode {
                    6 => return self.directory.before_step(vm),
                    4 => {
                        assert_eq!(a, 1);
                        assert!(self.inputs.contains_key(&filename));
                        self.reading = filename;
                        self.offset = 0;
                    }
                    8 => {
                        assert_eq!(a, 2);
                        self.write_opens += 1;
                        self.outputs.insert(filename.clone(), vec![]);
                        self.writing = filename;
                    }
                    _ => panic!("unexpected open mode {mode}"),
                }
                vm.bus_mut().ram_mut().write(self.directory.ioerr, 1);
            }
            "Bget" => {
                assert_eq!(a, 1);
                let requested = vm.bus().ram().read_word(0xA3) as usize;
                let data = &self.inputs[&self.reading];
                let n = requested.min(data.len() - self.offset);
                let status = if self.offset + n == data.len() {
                    136
                } else {
                    1
                };
                let ram = vm.bus_mut().ram_mut();
                if n > 0 {
                    ram.map(address, &data[self.offset..self.offset + n])?;
                }
                ram.write_word(0x358, n as u16);
                ram.write(self.directory.ioerr, status);
                self.offset += n;
                self.reads.push((n, status));
            }
            "Bput" => {
                assert_eq!(a, 2);
                let n = vm.bus().ram().read_word(0xA3) as usize;
                self.outputs
                    .get_mut(&self.writing)
                    .unwrap()
                    .extend(machine::bytes(vm, address, n));
                vm.bus_mut().ram_mut().write(self.directory.ioerr, 1);
            }
            "Print" => {
                let text = machine::counted(vm, u16::from_le_bytes([a, x]));
                if text.ends_with(b"disk !") {
                    self.prompts.push(text);
                }
            }
            _ => {}
        }
        vm.set_pc(machine::RETURN);
        Ok(())
    }
}

pub fn check(compiled: &CompiledProgram, debug: bool) {
    let listing = compiled.source_listing();
    let routine = |s| routine_address(&listing, s);
    let global = |s| global_address(&listing, s);
    for (lengths, ramdisk, arena) in [(vec![700, 64, 0], 4, 256), (vec![17; 20], 0xFF, 512)] {
        let mut files = directory::files(lengths.len());
        for file in &mut files {
            file.directory = false;
            file.protected = false;
        }
        files.sort_by_key(|f| (f.name.clone(), f.extension.clone()));
        let inputs = files
            .iter()
            .zip(&lengths)
            .enumerate()
            .map(|(index, (file, &length))| {
                (
                    file.filename(),
                    (0..length)
                        .map(|n| (n as u8).wrapping_add(index as u8 * 7))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut input = files.iter().map(directory::File::input).collect::<Vec<_>>();
        let mut summary = b"999 FREE SECTORS\x9B".to_vec();
        summary.resize(19, 155);
        input.push(summary);
        let mut hooks = Files {
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
                .map(|n| (routine(n), n))
                .collect(),
                ioerr: global("ioerr"),
                input,
                row: 0,
                images: vec![],
                names: vec![],
            },
            services: [
                "Open",
                "Bget",
                "Bput",
                "Print",
                "Window",
                "CloseWindow",
                "GetAnyKey",
                "CloseAll",
            ]
            .into_iter()
            .chain(debug.then_some("MarkCopy"))
            .map(|n| (routine(n), n))
            .collect(),
            inputs: inputs.clone(),
            outputs: BTreeMap::new(),
            reading: vec![],
            writing: vec![],
            offset: 0,
            reads: vec![],
            write_opens: 0,
            prompts: vec![],
        };
        let mut vm = machine::load(compiled);
        for (address, value) in [(0x700, b'M'), (0x76F, 0xA9), (0x70A, ramdisk), (0x70B, 2)] {
            vm.bus_mut().ram_mut().write(address, value);
        }
        vm = machine::call(vm, &mut hooks, routine("InitPanels"), &[]);
        let buffer = vm.bus().ram().read_word(global("buffer"));
        vm.bus_mut().ram_mut().write_word(0x2E5, buffer + arena);
        vm = machine::call(vm, &mut hooks, routine("TagAll"), &[]);
        vm = machine::call(vm, &mut hooks, routine("Copy"), &[]);
        assert_eq!(hooks.outputs, inputs, "copy bytes/EOF/continuation");
        assert_eq!(
            hooks.write_opens,
            lengths.len(),
            "partial files must remain open across chunks"
        );
        assert_eq!(vm.bus().ram().read(global("tagged")), 0);
        assert_eq!(vm.bus().ram().read(global("copyflag")), 0);
        assert_eq!(vm.bus().ram().read(0x70B), 2);
        if arena == 256 {
            assert_eq!(
                hooks.reads,
                [(256, 1), (256, 1), (188, 136), (64, 136), (0, 136)]
            );
            assert!(hooks.prompts.is_empty());
        } else {
            assert_eq!(
                hooks.prompts,
                [
                    b"Destination disk !".to_vec(),
                    b"Source disk !".to_vec(),
                    b"Destination disk !".to_vec()
                ]
            );
        }
    }
}
