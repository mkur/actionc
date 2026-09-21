//! Decode only a candidate reached at a real A16 instruction boundary. This
//! recognizes the complete emitted comparison, including relocated JML targets.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Window {
    pub load: u32,
    pub cmp: u32,
    pub branch: u32,
    pub yes: u32,
    pub done: u32,
    pub end: u32,
    pub sources: [(bool, u16); 2], // stack-relative / immediate, encoded value
    pub dest: u8,
    pub predicate: u8,
}
impl Window {
    pub fn addresses(&self) -> Vec<u32> {
        vec![
            self.load,
            self.cmp,
            self.branch,
            self.branch + 2,
            self.branch + 6,
            self.branch + 8,
            self.branch + 10,
            self.yes,
            self.yes + 2,
            self.done,
            self.done + 2,
            self.end,
        ]
    }
}
pub fn window(
    cpu: &Machine,
    bus: &Bus,
    routines: &[actionc::mir65816::image::Routine],
) -> Option<Window> {
    assert!(cpu.is_instruction_boundary());
    if cpu.registers().p & 0x20 != 0 {
        return None;
    }
    let start = cpu.pc();
    let routine = routines
        .iter()
        .find(|r| (r.address..r.address + r.size).contains(&start))?;
    let mut at = start;
    let mut operand = |load: bool| -> Option<(bool, u16)> {
        let opcode = bus.ram[at as usize];
        let stack = opcode == if load { 0xa3 } else { 0xc3 };
        if !stack && opcode != if load { 0xa9 } else { 0xc9 } {
            return None;
        }
        let size = if stack { 2 } else { 3 };
        if at + size > routine.address + routine.size {
            return None;
        }
        let value = bus.value(at + 1, (size - 1) as usize) as u16;
        at += size;
        Some((stack, value))
    };
    let left = operand(true)?;
    let cmp = start + if left.0 { 2 } else { 3 };
    let right = operand(false)?;
    let branch = at;
    let end = branch + 22;
    if end > routine.address + routine.size {
        return None;
    }
    let code = &bus.ram[branch as usize..end as usize];
    if !matches!(code[0], 0x90 | 0xb0 | 0xd0 | 0xf0) {
        return None;
    }
    let yes = branch + 14;
    let done = branch + 18;
    let mut expected = vec![code[0], 4, 0x5c];
    expected.extend(&yes.to_le_bytes()[..3]);
    expected.extend([0xe2, 0x20, 0xa9, 0, 0x5c]);
    expected.extend(&done.to_le_bytes()[..3]);
    expected.extend([0xe2, 0x20, 0xa9, 1, 0xe2, 0x20, 0x83, code[21]]);
    if code != expected {
        return None;
    }
    Some(Window {
        load: start,
        cmp,
        branch,
        yes,
        done,
        end,
        sources: [left, right],
        dest: code[21],
        predicate: code[0] ^ 0x20,
    })
}

/// A reached A16 LDA/CMP and one dispatch to two complete edge trampolines.
/// Decode edge copies, including their real instruction boundaries, rather than
/// scanning arbitrary bytes for a CMP opcode or assuming empty successors.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FusedWindow {
    pub load: u32,
    pub cmp: u32,
    pub branch: u32,
    pub no: u32,
    pub yes: u32,
    pub sources: [(bool, u16); 2],
    pub predicate: u8,
    pub edges: [Vec<u32>; 2], // false / true instruction boundaries
    pub targets: [u32; 2],
}
impl FusedWindow {
    pub fn addresses(&self) -> Vec<u32> {
        [
            vec![self.load, self.cmp, self.branch, self.branch + 2],
            self.edges[0].clone(),
            self.edges[1].clone(),
        ]
        .concat()
    }
}
pub fn fused_window(
    cpu: &Machine,
    bus: &Bus,
    routines: &[actionc::mir65816::image::Routine],
) -> Option<FusedWindow> {
    assert!(cpu.is_instruction_boundary());
    if cpu.registers().p & 0x20 != 0 {
        return None;
    }
    let start = cpu.pc();
    let r = routines
        .iter()
        .find(|r| (r.address..r.address + r.size).contains(&start))?;
    fused_in_range(cpu, bus, r.address..r.address + r.size)
}
pub fn fused_in_range(
    cpu: &Machine,
    bus: &Bus,
    range: std::ops::Range<u32>,
) -> Option<FusedWindow> {
    assert!(cpu.is_instruction_boundary());
    let start = cpu.pc();
    if cpu.registers().p & 0x20 != 0 || !range.contains(&start) {
        return None;
    }
    let end = range.end;
    let mut at = start;
    let mut operand = |load: bool| -> Option<(bool, u16)> {
        if at + 2 > end {
            return None;
        }
        let op = bus.ram[at as usize];
        let stack = op == if load { 0xa3 } else { 0xc3 };
        if !stack && op != if load { 0xa9 } else { 0xc9 } {
            return None;
        }
        let size = if stack { 2 } else { 3 };
        if at + size > end {
            return None;
        }
        let value = bus.value(at + 1, (size - 1) as usize) as u16;
        at += size;
        Some((stack, value))
    };
    let left = operand(true)?;
    let right = operand(false)?;
    let branch = at;
    if branch + 6 > end {
        return None;
    }
    let code = &bus.ram[branch as usize..(branch + 6) as usize];
    if !matches!(code[0], 0x90 | 0xb0 | 0xd0 | 0xf0) || code[1..3] != [4, 0x5c] {
        return None;
    }
    let yes = bus.value(branch + 3, 3);
    let no = branch + 6;
    let edge = |mut pc: u32| -> Option<(Vec<u32>, u32, u32)> {
        if let Some(w) = super::word_edge::decode(bus, pc, range.clone()) {
            return Some((w.sites, w.target, w.end));
        }
        if pc + 2 > end || bus.ram[pc as usize..pc as usize + 2] != [0xe2, 0x20] {
            return None;
        }
        let mut sites = vec![pc];
        pc += 2;
        while pc + 2 <= end && bus.ram[pc as usize] != 0xc2 {
            // Every edge copy byte is one load followed by one store.
            let size = match bus.ram[pc as usize] {
                0xa3 | 0xa9 | 0xa5 => 2,
                0xaf => 4,
                _ => return None,
            };
            if pc + size + 2 > end {
                return None;
            }
            sites.push(pc);
            pc += size;
            if !matches!(bus.ram[pc as usize], 0x83 | 0x85) {
                return None;
            }
            sites.push(pc);
            pc += 2;
        }
        if pc + 6 > end || bus.ram[pc as usize..pc as usize + 3] != [0xc2, 0x20, 0x5c] {
            return None;
        }
        sites.extend([pc, pc + 2]);
        let target = bus.value(pc + 3, 3);
        if !range.contains(&target) {
            return None;
        }
        Some((sites, target, pc + 6))
    };
    let (false_sites, false_target, false_end) = edge(no)?;
    if yes != false_end {
        return None;
    }
    let (true_sites, true_target, _) = edge(yes)?;
    Some(FusedWindow {
        load: start,
        cmp: start + if left.0 { 2 } else { 3 },
        branch,
        no,
        yes,
        sources: [left, right],
        predicate: code[0] ^ 0x20,
        edges: [false_sites, true_sites],
        targets: [false_target, true_target],
    })
}
