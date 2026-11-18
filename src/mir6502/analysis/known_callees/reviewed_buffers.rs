//! Instantiate a reviewed argument-relative footprint in one caller's state.
use super::*;
use crate::callee6502::{CallFacts, DataRange};
use crate::mir6502::analysis::address_ranges::{AbiByte, CoherentWordRange};
use crate::mir6502::analysis::record_bases::MirPhysicalMemory;
use crate::mir6502::ir::MirParamLane;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Buffer {
    low: AbiByte,
    high: AbiByte,
    count: AbiByte,
    zero_means_256: bool,
    write: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::mir6502) struct ReviewedBuffers {
    owner: RoutineId,
    buffers: Vec<Buffer>,
    fixed_reads: Vec<DataRange<crate::nir::SymbolId>>,
    fixed_writes: Vec<DataRange<crate::nir::SymbolId>>,
}

impl ReviewedBuffers {
    pub fn from_facts(
        routine: &MirRoutine,
        facts: &CallFacts<crate::nir::SymbolId>,
    ) -> Option<Self> {
        if facts.buffers.is_empty() {
            return None;
        }
        let params = routine.frame.direct_parameter_abi.as_ref()?;
        let lanes =
            crate::mir6502::abi::argument_lanes(crate::mir6502::abi::parameter_bytes(params)?)?;
        let lane = |param: u16, byte: usize, width: u16| {
            let index = usize::from(param.checked_sub(1)?);
            if params.get(index)?.width().bytes() != width {
                return None;
            }
            let start = usize::from(crate::mir6502::abi::parameter_bytes(params.get(..index)?)?);
            Some(match lanes.get(start + byte)? {
                MirParamLane::Reg(reg) => AbiByte::Register(*reg),
                MirParamLane::ZeroPage(slot) => AbiByte::Cell(slot.0),
            })
        };
        let buffers = facts
            .buffers
            .iter()
            .map(|b| {
                Some(Buffer {
                    low: lane(b.pointer, 0, 2)?,
                    high: lane(b.pointer, 1, 2)?,
                    count: lane(b.count, 0, 1)?,
                    zero_means_256: b.zero_means_256,
                    write: b.write,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            owner: routine.id,
            buffers,
            fixed_reads: facts.reads.clone()?,
            fixed_writes: facts.writes.clone()?,
        })
    }

    pub fn writes(
        &self,
        registers: [Option<u8>; 3],
        cells: &BTreeMap<u16, u8>,
        memory: &MirPhysicalMemory,
        ranges: &[CoherentWordRange],
    ) -> Option<BTreeSet<u16>> {
        let read = |lane| match lane {
            AbiByte::Register(MirReg::A) => registers[0],
            AbiByte::Register(MirReg::X) => registers[1],
            AbiByte::Register(MirReg::Y) => registers[2],
            AbiByte::Cell(a) => cells.get(&u16::from(a)).copied(),
        };
        let address = |r: &DataRange<crate::nir::SymbolId>, n| match r.global {
            Some(id) => memory.assembly_address(self.owner, &MirMem::Global { id, offset: n }),
            None => Some(n),
        };
        let mut writes = BTreeSet::new();
        let mut scratch = BTreeSet::new();
        for (ranges, write) in [(&self.fixed_reads, false), (&self.fixed_writes, true)] {
            for r in ranges {
                for n in r.start..=r.end {
                    let a = address(r, n)?;
                    if a >= 0x8000 || (write && a >= 0x100 && !memory.contains_write(a, 1)) {
                        return None;
                    }
                    scratch.insert(a);
                    if write {
                        writes.insert(a);
                    }
                }
            }
        }
        // A known span and the separately proven coherent word must agree with
        // all known register/ABI bytes. Never combine unrelated endpoint lanes.
        super::bounded_memory::validate_word_lanes(ranges, &registers, cells)?;
        for b in &self.buffers {
            let count = crate::callee6502::buffer_length(read(b.count), b.zero_means_256);
            if count == 0 {
                continue;
            }
            let (min, max) = match (read(b.low), read(b.high)) {
                (Some(lo), Some(hi)) => {
                    let p = u16::from_le_bytes([lo, hi]);
                    (p, p)
                }
                _ => {
                    let range = ranges.iter().find(|r| r.lo == b.low && r.hi == b.high)?;
                    (range.min, range.max)
                }
            };
            let last = max.checked_add(count - 1)?;
            let size = last.checked_sub(min)?.checked_add(1)?;
            if !ranges.is_empty() {
                if !memory.retention_ram_range(min, u32::from(last) + 1) {
                    return None;
                }
            } else if last >= 0x8000 || (b.write && !memory.contains_write(min, size)) {
                return None;
            }
            if b.write {
                // Overwriting a pointer/count carrier could change later loop addresses.
                if scratch.range(min..=last).next().is_some() || min < 0x400 {
                    return None;
                }
                writes.extend(min..=last);
            }
        }
        Some(writes)
    }
}

// Numeric pointers passed to a reviewed leaf expose their entire possible
// span. An unknown pointee stays an invocation effect; it does not create a
// literal-address reference to every unrelated routine's private storage.
pub(in crate::mir6502) fn numeric_buffer_observations(
    program: &MirProgram,
) -> Vec<(RoutineId, u16, u16, bool)> {
    use crate::mir6502::analysis::sites::MirSite;
    use crate::mir6502::runtime_contracts::buffers::{BufferAccess, BufferExtent, argument_values};
    let mut accesses = BTreeMap::new();
    for r in &program.routines {
        if !r
            .frame
            .callee_review
            .as_ref()
            .is_some_and(|review| !review.facts.buffers.is_empty())
        {
            continue;
        }
        let Some(facts) = super::reviewed_contract(program, r) else {
            continue;
        };
        let Some(params) = &r.frame.direct_parameter_abi else {
            continue;
        };
        let offset = |n: u16| {
            crate::mir6502::abi::parameter_bytes(params.get(..usize::from(n.checked_sub(1)?))?)
        };
        for b in &facts.buffers {
            if let (Some(pointer_byte), Some(argument_byte)) = (offset(b.pointer), offset(b.count))
            {
                accesses
                    .entry(r.id)
                    .or_insert_with(Vec::new)
                    .push(BufferAccess {
                        pointer_byte,
                        extent: BufferExtent::ByteCount {
                            argument_byte,
                            zero_means_256: b.zero_means_256,
                        },
                        write: b.write,
                    });
            }
        }
    }
    let mut observations = Vec::new();
    if accesses.is_empty() {
        return observations;
    }
    for r in &program.routines {
        let mut values = None;
        for block in &r.blocks {
            for (op_index, call) in block.ops.iter().enumerate() {
                let MirOp::Call {
                    target: MirCallTarget::Routine(id),
                    ..
                } = call
                else {
                    continue;
                };
                let Some(buffers) = accesses.get(id) else {
                    continue;
                };
                let site = MirSite::Op {
                    block: block.id,
                    op_index,
                };
                let values = values
                    .get_or_insert_with(|| argument_values(program, r))
                    .as_ref();
                for b in buffers {
                    if let (Some(address), size @ 1..) =
                        b.project(program, call, values.map(|v| (v, site)))
                    {
                        observations.push((r.id, address, size, b.write));
                    }
                }
            }
        }
    }
    observations
}
