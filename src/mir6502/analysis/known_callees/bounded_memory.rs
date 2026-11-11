//! Call-context memory footprints for a closed, inspectable leaf ISA subset.
//! No incoming pointer or arbitrary memory value receives an assumed bound.
use super::*;
use crate::mir6502::analysis::record_bases::MirPhysicalMemory;
use std::collections::VecDeque;

const MAX_BYTES: usize = 512;
const MAX_STEPS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct BoundedMachine {
    owner: RoutineId,
    bytes: Vec<MachineByte>,
    starts: BTreeSet<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    registers: [Option<u8>; 3],
    cells: BTreeMap<u16, u8>,
    // C/N/Z/V, independently known or unknown.
    flags: [Option<bool>; 4],
}
impl State {
    fn join(&mut self, other: &Self) -> bool {
        let old = self.clone();
        for (value, incoming) in self.registers.iter_mut().zip(other.registers) {
            if *value != incoming {
                *value = None;
            }
        }
        self.cells
            .retain(|address, value| other.cells.get(address) == Some(value));
        for (value, incoming) in self.flags.iter_mut().zip(other.flags) {
            if *value != incoming {
                *value = None;
            }
        }
        *self != old
    }
    fn nz(&mut self, value: Option<u8>) {
        self.flags[1] = value.map(|v| v & 128 != 0);
        self.flags[2] = value.map(|v| v == 0);
    }
}

impl BoundedMachine {
    pub fn analyze(routine: &MirRoutine, program: &MirProgram) -> Option<Self> {
        let [block] = routine.blocks.as_slice() else {
            return None;
        };
        let [MirOp::MachineBlock { id, effects }] = block.ops.as_slice() else {
            return None;
        };
        if effects.opaque
            || effects.may_call_os
            || routine.effects.opaque
            || routine.effects.may_call_os
            || !matches!(block.terminator, MirTerminator::Unreachable)
        {
            return None;
        }
        let machine = program.machine_blocks.iter().find(|m| m.id == *id)?;
        // This proof adds no dependency on unresolved storage names or source
        // expressions. Existing typed relocations and numeric atoms suffice.
        if machine.items.iter().any(|item| {
            matches!(
                item,
                MirMachineItem::Name(_)
                    | MirMachineItem::AddressExpr {
                        atom: MirMachineAtom::Name(_),
                        ..
                    }
                    | MirMachineItem::AddressByte { .. }
                    | MirMachineItem::StringLiteral(_)
            )
        }) {
            return None;
        }
        let bytes = flatten_machine_items(&machine.items, routine, program)?;
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return None;
        }
        let starts = machine_instruction_starts(&bytes)?;
        Some(Self {
            owner: routine.id,
            bytes,
            starts,
        })
    }

    pub fn writes(
        &self,
        registers: [Option<u8>; 3],
        cells: BTreeMap<u16, u8>,
        memory: &MirPhysicalMemory,
    ) -> Option<BTreeSet<u16>> {
        let mut entries = BTreeMap::from([(
            0usize,
            State {
                registers,
                cells,
                flags: [None; 4],
            },
        )]);
        let mut pending = VecDeque::from([0usize]);
        let mut writes = BTreeSet::new();
        let mut returned = false;
        let mut budget = MAX_STEPS;
        while let Some(pc) = pending.pop_front() {
            budget = budget.checked_sub(1)?;
            let mut state = entries.get(&pc)?.clone();
            let opcode = self.bytes.get(pc)?.value?;
            let (_, _, len) = crate::codegen::decode_6502_opcode(opcode)?;
            let next = pc.checked_add(len)?;
            let byte = || self.bytes.get(pc + 1)?.value;
            let address = || {
                let low = self.bytes.get(pc + 1)?;
                if let Some(mem) = &low.memory {
                    return memory.address(self.owner, mem);
                }
                let lo = low.value?;
                if len == 2 {
                    Some(u16::from(lo))
                } else {
                    Some(u16::from_le_bytes([lo, self.bytes.get(pc + 2)?.value?]))
                }
            };
            let mut successors = vec![next];
            match opcode {
                0x60 if next == self.bytes.len() => {
                    returned = true;
                    continue;
                }
                0xA9 | 0xA2 | 0xA0 | 0xA5 | 0xA6 | 0xA4 | 0xAD | 0xAE | 0xAC => {
                    let r = match opcode {
                        0xA2 | 0xA6 | 0xAE => 1,
                        0xA0 | 0xA4 | 0xAC => 2,
                        _ => 0,
                    };
                    let value = if matches!(opcode, 0xA9 | 0xA2 | 0xA0) {
                        Some(byte()?)
                    } else {
                        let a = address()?;
                        if a >= 0x8000 {
                            return None;
                        }
                        state.cells.get(&a).copied()
                    };
                    state.registers[r] = value;
                    state.nz(value);
                }
                0x85 | 0x86 | 0x84 | 0x8D | 0x8E | 0x8C => {
                    let a = address()?;
                    if a >= 0x100 && !memory.contains_write(a, 1) {
                        return None;
                    }
                    writes.insert(a);
                    let r = match opcode {
                        0x86 | 0x8E => 1,
                        0x84 | 0x8C => 2,
                        _ => 0,
                    };
                    state.cells.remove(&a);
                    if let Some(value) = state.registers[r] {
                        state.cells.insert(a, value);
                    }
                }
                0xB1 | 0x91 => {
                    let lo = u16::from(byte()?);
                    if lo == 255 {
                        return None;
                    }
                    let base =
                        u16::from_le_bytes([*state.cells.get(&lo)?, *state.cells.get(&(lo + 1))?]);
                    let indices: Vec<u16> = match state.registers[2] {
                        Some(y) => vec![u16::from(y)],
                        None => (0..256).collect(),
                    };
                    for index in indices {
                        let a = base.checked_add(index)?;
                        if opcode == 0x91 {
                            // This also proves an indirect write cannot change
                            // pointer/ABI cells, stack or the executed code.
                            if !memory.contains_write(a, 1) {
                                return None;
                            }
                            writes.insert(a);
                            state.cells.remove(&a);
                        } else if a >= 0x8000 {
                            return None;
                        }
                    }
                    if opcode == 0xB1 {
                        state.registers[0] = None;
                        state.nz(None);
                    }
                }
                0x88 | 0xC8 | 0xCA | 0xE8 => {
                    let r = if matches!(opcode, 0x88 | 0xC8) { 2 } else { 1 };
                    let value = state.registers[r].map(|v| {
                        if matches!(opcode, 0x88 | 0xCA) {
                            v.wrapping_sub(1)
                        } else {
                            v.wrapping_add(1)
                        }
                    });
                    state.registers[r] = value;
                    state.nz(value);
                }
                0xAA | 0xA8 | 0x8A | 0x98 => {
                    let (dst, src) = match opcode {
                        0xAA => (1, 0),
                        0xA8 => (2, 0),
                        0x8A => (0, 1),
                        _ => (0, 2),
                    };
                    let value = state.registers[src];
                    state.registers[dst] = value;
                    state.nz(value);
                }
                0xC9 | 0xE0 | 0xC0 => {
                    let r = match opcode {
                        0xE0 => 1,
                        0xC0 => 2,
                        _ => 0,
                    };
                    let rhs = byte()?;
                    state.flags[0] = state.registers[r].map(|v| v >= rhs);
                    let result = state.registers[r].map(|v| v.wrapping_sub(rhs));
                    state.nz(result);
                }
                0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0 => {
                    let offset = i8::from_ne_bytes([byte()?]);
                    let target =
                        usize::try_from(isize::try_from(next).ok()? + isize::from(offset)).ok()?;
                    if !self.starts.contains(&target) {
                        return None;
                    }
                    let (flag, expected) = match opcode {
                        0x10 => (1, false),
                        0x30 => (1, true),
                        0x50 => (3, false),
                        0x70 => (3, true),
                        0x90 => (0, false),
                        0xB0 => (0, true),
                        0xD0 => (2, false),
                        _ => (2, true),
                    };
                    successors = match state.flags[flag] {
                        Some(v) if v == expected => vec![target],
                        Some(_) => vec![next],
                        None => vec![target, next],
                    };
                }
                0x18 => state.flags[0] = Some(false),
                0x38 => state.flags[0] = Some(true),
                0xB8 => state.flags[3] = Some(false),
                0xEA => {}
                // Calls, stack operations, decimal-mode changes, arithmetic,
                // indexed-X, unknown encodings and early returns fail closed.
                _ => return None,
            }
            for successor in successors {
                if !self.starts.contains(&successor) {
                    return None;
                }
                if let Some(old) = entries.get_mut(&successor) {
                    if old.join(&state) {
                        pending.push_back(successor);
                    }
                } else {
                    entries.insert(successor, state.clone());
                    pending.push_back(successor);
                }
            }
        }
        returned.then_some(writes)
    }
}

#[cfg(test)]
mod tests;
