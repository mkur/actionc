//! Whole-word provenance for the audited machine interpreter's byte carriers.
use super::*;
use crate::mir6502::analysis::address_ranges::{AbiByte, CoherentWordRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lane {
    word: usize,
    high: bool,
    min: u16,
    max: u16,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct WordLanes {
    registers: [Option<Lane>; 3],
    cells: BTreeMap<u16, Lane>,
}
impl WordLanes {
    pub fn seed(
        words: &[CoherentWordRange],
        registers: &[Option<u8>; 3],
        cells: &BTreeMap<u16, u8>,
    ) -> Option<Self> {
        let mut result = Self::default();
        let mut seen = Vec::new();
        let read = |home: AbiByte| match home {
            AbiByte::Register(r) => registers[reg_index(r)],
            AbiByte::Cell(a) => cells.get(&u16::from(a)).copied(),
        };
        for (word, range) in words.iter().enumerate() {
            if range.min > range.max || range.lo == range.hi {
                return None;
            }
            if let (Some(lo), Some(hi)) = (read(range.lo), read(range.hi)) {
                if !(range.min..=range.max).contains(&u16::from_le_bytes([lo, hi])) {
                    return None;
                }
            }
            for (home, high) in [(range.lo, false), (range.hi, true)] {
                if seen.contains(&home) {
                    return None;
                }
                seen.push(home);
                if let Some(byte) = read(home) {
                    let compatible = if high {
                        (range.min >> 8..=range.max >> 8).contains(&u16::from(byte))
                    } else if range.max - range.min >= 255 {
                        true
                    } else if range.min >> 8 == range.max >> 8 {
                        (range.min as u8..=range.max as u8).contains(&byte)
                    } else {
                        byte >= range.min as u8 || byte <= range.max as u8
                    };
                    if !compatible {
                        return None;
                    }
                }
                let lane = Some(Lane {
                    word,
                    high,
                    min: range.min,
                    max: range.max,
                });
                match home {
                    AbiByte::Register(r) => result.registers[reg_index(r)] = lane,
                    AbiByte::Cell(a) => {
                        result.cells.insert(u16::from(a), lane?);
                    }
                }
            }
        }
        Some(result)
    }
    pub fn join(&mut self, other: &Self) {
        for (lane, incoming) in self.registers.iter_mut().zip(other.registers) {
            if *lane != incoming {
                *lane = None;
            }
        }
        self.cells
            .retain(|a, lane| other.cells.get(a) == Some(lane));
    }
    pub fn load(&mut self, register: usize, address: Option<u16>) {
        self.registers[register] = address.and_then(|a| self.cells.get(&a).copied());
    }
    pub fn store(&mut self, register: usize, address: u16) {
        self.cells.remove(&address);
        if let Some(lane) = self.registers[register] {
            self.cells.insert(address, lane);
        }
    }
    pub fn forget_register(&mut self, register: usize) {
        self.registers[register] = None;
    }
    pub fn transfer(&mut self, dst: usize, src: usize) {
        self.registers[dst] = self.registers[src];
    }
    pub fn write(&mut self, address: u16) {
        self.cells.remove(&address);
    }
    pub fn pointer(&self, lo: u16, constants: &BTreeMap<u16, u8>) -> Option<(u16, u16)> {
        if let (Some(lo), Some(hi)) = (constants.get(&lo), constants.get(&lo.checked_add(1)?)) {
            let value = u16::from_le_bytes([*lo, *hi]);
            return Some((value, value));
        }
        let a = self.cells.get(&lo)?;
        let b = self.cells.get(&lo.checked_add(1)?)?;
        if a.high || !b.high || a.word != b.word || a.min != b.min || a.max != b.max {
            return None;
        }
        Some((a.min, a.max))
    }
}
fn reg_index(reg: MirReg) -> usize {
    match reg {
        MirReg::A => 0,
        MirReg::X => 1,
        MirReg::Y => 2,
    }
}
