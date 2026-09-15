//! CPU qualification harness. No compiler, Atari OS, or SNES devices are involved.

pub use wdc65816_emu::core::{Registers, Wdc65816};
use wdc65816_emu::traits::BusInterface;

pub const ADDRESS_SPACE: usize = 1 << 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read(u32, u8),
    Write(u32, u8),
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trace {
    pub cycle: u64,
    pub access: Access,
}

#[derive(Clone, Debug)]
struct Region {
    start: u32,
    end: u32,
    writable: bool,
}

/// Explicitly mapped memory. Guard failures are host assertions, not CPU ABORTs.
#[derive(Clone)]
pub struct Bus {
    ram: Vec<u8>,
    regions: Vec<Region>,
    pub trace: Vec<Trace>,
    pub cycles: u64,
    pub irq: bool,
    /// Raw active-high level presented to the core; no pulse stretching.
    pub nmi: bool,
    pub nmi_acknowledgements: u64,
}

impl Default for Bus {
    fn default() -> Self {
        Self {
            ram: vec![0; ADDRESS_SPACE],
            regions: vec![],
            trace: vec![],
            cycles: 0,
            irq: false,
            nmi: false,
            nmi_acknowledgements: 0,
        }
    }
}

impl Bus {
    pub fn map(&mut self, start: u32, bytes: &[u8], writable: bool) {
        let end = usize::try_from(start)
            .unwrap()
            .checked_add(bytes.len())
            .unwrap();
        assert!(!bytes.is_empty() && end <= ADDRESS_SPACE);
        assert!(
            self.regions
                .iter()
                .all(|r| end <= r.start as usize || start >= r.end)
        );
        self.ram[start as usize..end].copy_from_slice(bytes);
        self.regions.push(Region {
            start,
            end: end as u32,
            writable,
        });
    }

    pub fn map_ram(&mut self, start: u32, length: usize) {
        self.map(start, &vec![0; length], true);
    }

    pub fn peek(&self, address: u32) -> u8 {
        assert!(
            self.regions
                .iter()
                .any(|r| address >= r.start && address < r.end)
        );
        self.ram[address as usize]
    }

    pub fn word(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.peek(address), self.peek(address + 1)])
    }

    pub fn writes(&self) -> Vec<(u32, u8)> {
        self.trace
            .iter()
            .filter_map(|t| match t.access {
                Access::Write(a, v) => Some((a, v)),
                _ => None,
            })
            .collect()
    }

    fn record(&mut self, access: Access) {
        self.trace.push(Trace {
            cycle: self.cycles,
            access,
        });
    }
}

impl BusInterface for Bus {
    fn read(&mut self, address: u32) -> u8 {
        assert!(
            self.regions
                .iter()
                .any(|r| address >= r.start && address < r.end),
            "unmapped read at ${address:06X}, cycle {}",
            self.cycles
        );
        let value = self.ram[address as usize];
        self.record(Access::Read(address, value));
        value
    }

    fn write(&mut self, address: u32, value: u8) {
        assert!(
            self.regions
                .iter()
                .any(|r| address >= r.start && address < r.end && r.writable),
            "unmapped or read-only write at ${address:06X}, cycle {}",
            self.cycles
        );
        self.ram[address as usize] = value;
        self.record(Access::Write(address, value));
    }

    fn idle(&mut self) {
        self.record(Access::Idle);
    }
    fn nmi(&self) -> bool {
        self.nmi
    }
    fn acknowledge_nmi(&mut self) {
        self.nmi_acknowledgements += 1;
    }
    fn irq(&self) -> bool {
        self.irq
    }
    fn halt(&self) -> bool {
        false
    }
    fn reset(&self) -> bool {
        false
    }
}

#[derive(Clone, Default)]
pub struct Machine {
    pub cpu: Wdc65816,
    pub bus: Bus,
}

impl Machine {
    /// Start a literal code fixture in native mode, M=X=0, decimal clear, IRQ masked.
    pub fn native(entry: u32) -> Self {
        assert!((entry as usize) < ADDRESS_SPACE);
        let mut machine = Self::default();
        let mut registers = machine.cpu.registers().clone();
        registers.emulation_mode = false;
        registers.p = 0x04.into();
        registers.pbr = (entry >> 16) as u8;
        registers.pc = entry as u16;
        registers.s = 0x3FFF;
        machine.cpu.set_registers(registers);
        machine.bus.map_ram(0x3000, 0x1000);
        machine
    }

    pub fn pc(&self) -> u32 {
        let r = self.cpu.registers();
        (u32::from(r.pbr) << 16) | u32::from(r.pc)
    }

    pub fn tick(&mut self) {
        self.cpu.tick(&mut self.bus);
        self.bus.cycles += 1;
    }

    /// One instruction or hardware interrupt entry, with a finite cycle budget.
    pub fn step(&mut self) {
        for _ in 0..32 {
            self.tick();
            if !self.cpu.is_mid_instruction() {
                return;
            }
        }
        panic!("instruction budget exceeded: {:?}", self.cpu);
    }

    pub fn run_until(&mut self, budget: u64, mut done: impl FnMut(&Self) -> bool) -> bool {
        for _ in 0..budget {
            if done(self) {
                return true;
            }
            self.tick();
        }
        done(self)
    }

    /// Stop before fetching the named address, at an instruction boundary.
    pub fn run_to(&mut self, address: u32, budget: u64) {
        assert!(
            self.run_until(budget, |m| !m.cpu.is_mid_instruction() && m.pc() == address),
            "cycle budget exhausted waiting for ${address:06X}: {:?}",
            self.cpu
        );
    }
}
