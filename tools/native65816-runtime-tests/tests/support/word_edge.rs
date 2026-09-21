//! Test-only, mode-aware decoding of the complete two-phase word edge shape.
use super::*;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub sites: Vec<u32>,
    pub moves: Vec<((bool, u16), u8, u8)>, // source, staging, destination
    pub target: u32,
    pub end: u32,
}

pub fn decode(bus: &Bus, mut pc: u32, range: Range<u32>) -> Option<Window> {
    let mut sites = vec![];
    let end = range.end;
    if !range.contains(&pc) || pc + 2 > end {
        return None;
    }
    if bus.ram[pc as usize..pc as usize + 2] == [0xc2, 0x20] {
        sites.push(pc);
        pc += 2;
    }
    let mut pairs = vec![];
    while pc + 2 <= end && bus.ram[pc as usize] != 0x5c {
        let stack = bus.ram[pc as usize] == 0xa3;
        if !stack && bus.ram[pc as usize] != 0xa9 {
            return None;
        }
        let size = if stack { 2 } else { 3 };
        if pc + size + 2 > end || bus.ram[(pc + size) as usize] != 0x83 {
            return None;
        }
        let source = bus.value(pc + 1, (size - 1) as usize) as u16;
        let destination = bus.ram[(pc + size + 1) as usize];
        if !(1..=254).contains(&destination) || (stack && !(1..=254).contains(&source)) {
            return None;
        }
        sites.extend([pc, pc + size]);
        pairs.push(((stack, source), destination));
        pc += size + 2;
    }
    if pairs.is_empty() || pairs.len() % 2 != 0 || pc + 4 > end {
        return None;
    }
    let target = bus.value(pc + 1, 3);
    if !range.contains(&target) {
        return None;
    }
    let n = pairs.len() / 2;
    let mut moves = vec![];
    let overlap = |a: u8, b: u8| a.abs_diff(b) < 2;
    for i in 0..n {
        let (source, stage) = pairs[i];
        let (staged, dest) = pairs[n + i];
        if staged != (true, u16::from(stage)) {
            return None;
        }
        // Current allocation reserves distinct four-byte staging slots and
        // disjoint destinations. Never mistake arbitrary LDA/STA runs for edges.
        if i > 0 && u16::from(stage) != u16::from(pairs[i - 1].1) + 4 {
            return None;
        }
        if pairs[..n]
            .iter()
            .any(|&(src, s)| overlap(dest, s) || (src.0 && overlap(src.1 as u8, stage)))
        {
            return None;
        }
        if pairs[n..n + i].iter().any(|&(_, d)| overlap(dest, d)) {
            return None;
        }
        moves.push((source, stage, dest));
    }
    sites.push(pc);
    Some(Window {
        sites,
        moves,
        target,
        end: pc + 4,
    })
}

/// Count at the first LDA only, so an optional REP does not count twice.
pub fn reached(
    cpu: &Machine,
    bus: &Bus,
    routines: &[actionc::mir65816::image::Routine],
) -> Option<Window> {
    assert!(cpu.is_instruction_boundary());
    if cpu.registers().p & 0x30 != 0 || !matches!(bus.ram[cpu.pc() as usize], 0xa3 | 0xa9) {
        return None;
    }
    let r = routines
        .iter()
        .find(|r| (r.address..r.address + r.size).contains(&cpu.pc()))?;
    decode(bus, cpu.pc(), r.address..r.address + r.size)
}
