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
