//! Independent MyDOS text/row/name oracle, executed against the complete TN.
use super::{global_address, machine, routine_address};
use actionc::compiler::CompiledProgram;
use actionc_vm::{CompilerVm, VmRunHooks};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct File {
    pub name: String,
    pub extension: String,
    pub directory: bool,
    pub protected: bool,
    pub sectors: u16,
    pub long_row: bool,
}

pub fn files(count: usize) -> Vec<File> {
    (0..count)
        .rev()
        .map(|i| File {
            name: if i == 0 {
                "A".into()
            } else {
                format!("FILE{i:04}")
            },
            extension: if i % 9 == 0 {
                ""
            } else if i % 2 == 0 {
                "BIN"
            } else {
                "TXT"
            }
            .into(),
            directory: i % 9 == 0,
            protected: i % 5 == 0 && i % 9 != 0,
            sectors: (i * 13 + 1) as u16,
            long_row: i % 2 == 0,
        })
        .collect()
}

impl File {
    pub fn input(&self) -> Vec<u8> {
        let text = format!(
            "{}{}{:<8}{:<3}{}{:03}",
            if self.protected { '*' } else { ' ' },
            if self.directory { ':' } else { ' ' },
            self.name,
            self.extension,
            if self.long_row { "  " } else { " " },
            self.sectors
        );
        let mut bytes = text.into_bytes();
        bytes.push(155);
        bytes.resize(19, 155);
        bytes
    }

    pub fn row(&self, tagged: bool) -> Vec<u8> {
        let text = format!(
            "{}{:<8}|{:<3}|{:03}",
            if self.directory {
                ':'
            } else if self.protected {
                '*'
            } else {
                ' '
            },
            self.name,
            self.extension,
            self.sectors
        );
        let mut row = vec![if tagged { 0x7F } else { 0 }];
        row.extend(text.bytes().map(screen));
        row
    }

    pub fn filename(&self) -> Vec<u8> {
        format!("D:{}.{}", self.name, self.extension).into_bytes()
    }
}

pub fn screen(byte: u8) -> u8 {
    (byte & 0x9F) | [0x40, 0, 0x20, 0x60][((byte >> 5) & 3) as usize]
}

pub struct Directory {
    pub entries: BTreeMap<u16, &'static str>,
    pub ioerr: u16,
    pub input: Vec<Vec<u8>>,
    pub row: usize,
    pub images: Vec<Vec<u8>>,
    pub names: Vec<(u8, Vec<u8>)>,
}

impl VmRunHooks for Directory {
    type Error = String;
    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let Some(&name) = self.entries.get(&vm.cpu().registers().pc) else {
            return Ok(());
        };
        let r = vm.cpu().registers();
        let (a, x, y) = (r.a, r.x, r.y);
        match name {
            "Open" => {
                let mode = vm.bus().ram().read(0xA3);
                self.names
                    .push((mode, machine::counted(vm, u16::from_le_bytes([x, y]))));
                if mode == 6 {
                    self.row = 0;
                }
                vm.bus_mut().ram_mut().write(self.ioerr, 1);
            }
            "Input" => {
                assert_eq!(vm.bus().ram().read(0xA3), 19);
                let ram = vm.bus_mut().ram_mut();
                if let Some(row) = self.input.get(self.row) {
                    ram.map(u16::from_le_bytes([x, y]), row)?;
                    ram.write(self.ioerr, 1);
                } else {
                    ram.write(self.ioerr, 136);
                }
                self.row += 1;
            }
            "PutImage" => self.images.push(machine::bytes(
                vm,
                u16::from_le_bytes([a, x]),
                y as usize + 1,
            )),
            _ => {}
        }
        vm.set_pc(machine::RETURN);
        Ok(())
    }
}

pub fn check(compiled: &CompiledProgram, _debug: bool) {
    let listing = compiled.source_listing();
    let global = |s| global_address(&listing, s);
    let routine = |s| routine_address(&listing, s);
    for count in [0, 1, 63, 64] {
        let input_files = files(count);
        let mut expected = input_files.clone();
        expected.sort_by_key(|f| (!f.directory, f.name.clone(), f.extension.clone()));
        let mut input: Vec<_> = input_files.iter().map(File::input).collect();
        let mut summary = b"999 FREE SECTORS\x9B".to_vec();
        summary.resize(19, 155);
        input.push(summary);
        let mut hooks = Directory {
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
            input,
            row: 0,
            images: vec![],
            names: vec![],
        };
        let mut vm = machine::load(compiled);
        for (address, value) in [(0x700, b'M'), (0x76F, 0xA9), (0x70A, 0xFF), (0x70B, 2)] {
            vm.bus_mut().ram_mut().write(address, value);
        }
        vm = machine::call(vm, &mut hooks, routine("InitPanels"), &[]);
        assert_eq!(vm.bus().ram().read(global("active")), count as u8);
        assert_eq!(hooks.row, count + 2, "files plus summary plus EOF");
        assert!(hooks.names.iter().all(|n| n == &(6, b"D:*.*".to_vec())));
        let table = vm.bus().ram().read_word(global("v"));
        for (i, file) in expected.iter().enumerate() {
            let address = vm.bus().ram().read_word(table + i as u16 * 2);
            assert_eq!(
                machine::bytes(&vm, address, 18),
                file.row(false),
                "{file:?}"
            );
            vm = machine::call(vm, &mut hooks, routine("Convert"), &address.to_le_bytes());
            assert_eq!(machine::counted(&vm, global("fname")), file.filename());
        }
        let mut expected_summary: Vec<_> =
            b"999 FREE SECTORS".iter().copied().map(screen).collect();
        expected_summary.resize(17, 0);
        assert!(hooks.images.iter().any(|row| row == &expected_summary));
        for start in [0, count.saturating_sub(16)] {
            hooks.images.clear();
            vm = machine::call(vm, &mut hooks, routine("Draw"), &[start as u8]);
            let rows: Vec<_> = expected
                .iter()
                .skip(start)
                .take(16)
                .map(|f| f.row(false))
                .collect();
            assert_eq!(&hooks.images[1..], rows);
        }
        if count != 0 {
            // Mixed tagging from none selects all, while mixed tagging from all
            // clears all: retain this remembered toggle-all direction.
            for (routine_name, arguments, selected) in [
                ("Tag", vec![0], 1),
                ("TagAll", vec![], if count == 1 { 0 } else { count }),
            ] {
                vm = machine::call(vm, &mut hooks, routine(routine_name), &arguments);
                assert_eq!(vm.bus().ram().read(global("tagged")), selected as u8);
            }
            if count > 1 {
                vm = machine::call(vm, &mut hooks, routine("Tag"), &[0]);
                assert_eq!(vm.bus().ram().read(global("tagged")), count as u8 - 1);
                vm = machine::call(vm, &mut hooks, routine("TagAll"), &[]);
                assert_eq!(vm.bus().ram().read(global("tagged")), 0);
            }
        }
    }
}
