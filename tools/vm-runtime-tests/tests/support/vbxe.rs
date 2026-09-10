//! Focused VBXE model driven by actual VM bus events: revision reads,
//! MEMAC A's 8KB CPU window and palette autoincrement. Scanout, blitting,
//! interrupts and other MEMAC modes are outside this model.
use actionc_vm::{AddressRange, BusAccess, CompilerVm, VmRunHooks};
use std::collections::BTreeSet;

pub struct Vbxe {
    pub local: Vec<u8>,
    pub palettes: [[[u8; 3]; 256]; 4],
    pub registers: [u8; 32],
    pub writes: Vec<(u16, u8)>,
    pub reads: BTreeSet<u16>,
    pub banks: BTreeSet<u8>,
    device: Option<(u16, u8)>,
    mapped: Option<usize>,
    backing: Vec<u8>,
}

impl Vbxe {
    pub fn new(vm: &mut CompilerVm, device: Option<(u16, u8)>) -> Self {
        for base in [0xD640, 0xD740] {
            for offset in 0..32 {
                vm.bus_mut().write(base + offset, 0xFF);
            }
        }
        if let Some((base, minor)) = device {
            vm.bus_mut().write(base, 0x10);
            vm.bus_mut().write(base + 1, minor);
        }
        // Preserve ordinary RAM under the window, with BASIC initially enabled.
        vm.bus_mut().write(0xD301, 0xFD);
        let backing = vec![0x5A; 0x2000];
        vm.bus_mut().ram_mut().map(0xA000, &backing).unwrap();
        vm.bus_mut().clear_events();
        vm.bus_mut().add_watch_range(AddressRange {
            start: 0xD640,
            end: 0xD75F,
        });
        Self {
            local: vec![0xCC; 0x80000],
            palettes: [[[0xCC; 3]; 256]; 4],
            registers: [0; 32],
            writes: Vec::new(),
            reads: BTreeSet::new(),
            banks: BTreeSet::new(),
            device,
            mapped: None,
            backing,
        }
    }

    pub fn flush(&mut self, vm: &CompilerVm) {
        let destination = match self.mapped {
            Some(start) => &mut self.local[start..start + 0x2000],
            None => &mut self.backing,
        };
        for (offset, byte) in destination.iter_mut().enumerate() {
            *byte = vm.bus().ram().read(0xA000 + offset as u16);
        }
    }

    fn register_write(&mut self, address: u16, value: u8) -> Result<usize, String> {
        self.writes.push((address, value));
        let (base, _) = self
            .device
            .ok_or_else(|| format!("VBXE absent: write to ${address:04X}"))?;
        if !(base..base + 32).contains(&address) {
            return Err(format!("write to wrong VBXE page: ${address:04X}"));
        }
        let offset = usize::from(address - base);
        self.registers[offset] = value;
        match offset {
            5 if value > 3 => return Err(format!("invalid palette {value}")),
            6..=8 => {
                let palette = usize::from(self.registers[5]);
                let color = usize::from(self.registers[4]);
                self.palettes[palette][color][offset - 6] = value;
                if offset == 8 {
                    self.registers[4] = self.registers[4].wrapping_add(1);
                }
            }
            _ => {}
        }
        Ok(offset)
    }

    fn map_window(&mut self, vm: &mut CompilerVm) -> Result<(), String> {
        let control = self.registers[0x1E];
        let bank = self.registers[0x1F];
        let next = if control & 8 != 0 && bank & 0x80 != 0 {
            if control != 0xA9 {
                return Err(format!("unsupported MEMAC window control ${control:02X}"));
            }
            if vm.bus().io().read(0xD301).unwrap() & 2 == 0 {
                return Err("BASIC ROM still covers the MEMAC window".into());
            }
            self.banks.insert(bank & 0x7F);
            Some(usize::from(bank & 0x7F) * 0x1000)
        } else {
            None
        };
        if next == self.mapped {
            return Ok(());
        }
        self.flush(vm);
        let source = match next {
            Some(start) => self
                .local
                .get(start..start + 0x2000)
                .ok_or_else(|| "MEMAC window exceeds local memory".to_string())?,
            None => &self.backing,
        };
        vm.bus_mut().ram_mut().map(0xA000, source)?;
        self.mapped = next;
        Ok(())
    }
}

impl VmRunHooks for Vbxe {
    type Error = String;

    fn before_step(&mut self, vm: &mut CompilerVm) -> Result<(), Self::Error> {
        if vm.bus().events().is_empty() {
            return Ok(());
        }
        // Apply the preceding instruction's hardware writes before the next
        // CPU access, including mapping changes before window loads/stores.
        let events = vm.bus().events().to_vec();
        vm.bus_mut().clear_events();
        let (mut remap, mut revisions) = (false, false);
        for event in events {
            if event.access == BusAccess::Write {
                let offset = self.register_write(event.address, event.value)?;
                remap |= matches!(offset, 0x1E | 0x1F);
                revisions |= matches!(offset, 0 | 1);
            } else {
                self.reads.insert(event.address);
            }
        }
        if revisions {
            // Read/write aliases: video control and XDL ADR0 writes must not
            // change the core/minor revision values returned by subsequent reads.
            let (base, minor) = self.device.unwrap();
            vm.bus_mut().write(base, 0x10);
            vm.bus_mut().write(base + 1, minor);
            vm.bus_mut().clear_events();
        }
        if remap {
            self.map_window(vm)?;
        }
        Ok(())
    }
}
