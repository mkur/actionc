//! Exact byte-load sharing between two complete, disjoint direct copies.
use super::{call_predicates::ordinary_source, record_bases::MirPhysicalMemory, sites::*};
use crate::mir6502::ir::*;

pub(in crate::mir6502) const WINDOW: usize = 8;
pub(in crate::mir6502) const RETAINED: [usize; 6] = [0, 1, 5, 2, 3, 7];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum Blocker {
    Site,
    Shape,
    Storage,
    Footprint,
    Placement,
    Overlap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct Shape {
    pub source: [MirMem; 2],
    pub first: [MirMem; 2],
    pub second: [MirMem; 2],
}

fn load(op: &MirOp) -> Option<&MirMem> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some(mem),
        _ => None,
    }
}
fn store(op: &MirOp) -> Option<&MirMem> {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } => Some(mem),
        _ => None,
    }
}

pub(in crate::mir6502) fn shape(ops: &[MirOp]) -> Result<Shape, Blocker> {
    let [
        lo,
        first_lo,
        hi,
        first_hi,
        again_lo,
        second_lo,
        again_hi,
        second_hi,
    ] = ops
    else {
        return Err(Blocker::Shape);
    };
    let source = [
        load(lo).ok_or(Blocker::Shape)?.clone(),
        load(hi).ok_or(Blocker::Shape)?.clone(),
    ];
    if load(again_lo) != Some(&source[0]) || load(again_hi) != Some(&source[1]) {
        return Err(Blocker::Shape);
    }
    Ok(Shape {
        source,
        first: [
            store(first_lo).ok_or(Blocker::Shape)?.clone(),
            store(first_hi).ok_or(Blocker::Shape)?.clone(),
        ],
        second: [
            store(second_lo).ok_or(Blocker::Shape)?.clone(),
            store(second_hi).ok_or(Blocker::Shape)?.clone(),
        ],
    })
}

pub(in crate::mir6502) fn replacement(original: &[MirOp]) -> Option<Vec<MirOp>> {
    shape(original).ok()?;
    Some(RETAINED.map(|i| original[i].clone()).to_vec())
}

fn next(mem: &MirMem) -> Option<MirMem> {
    Some(match mem {
        MirMem::Global { id, offset } => MirMem::Global {
            id: *id,
            offset: offset.checked_add(1)?,
        },
        MirMem::Local { id, offset } => MirMem::Local {
            id: *id,
            offset: offset.checked_add(1)?,
        },
        MirMem::Param { id, offset } => MirMem::Param {
            id: *id,
            offset: offset.checked_add(1)?,
        },
        _ => return None,
    })
}

fn ordinary_word(
    routine: &MirRoutine,
    pair: &[MirMem; 2],
    memory: &MirPhysicalMemory,
    address: u16,
) -> bool {
    next(&pair[0]).as_ref() == Some(&pair[1])
        && match &pair[0] {
            MirMem::Global { .. } => true,
            MirMem::Param { .. } | MirMem::Local { .. } => {
                ordinary_source(routine, &pair[0], MirWidth::Word)
            }
            _ => false,
        }
        && pair.iter().all(|mem| memory.ordinary(routine.id, mem))
        && memory.contains_write(address, 2)
}

fn private_byte(routine: &MirRoutine, mem: &MirMem) -> bool {
    match mem {
        MirMem::ZeroPage(id) => {
            routine.frame.virtual_zero_page.contains(id)
                || routine
                    .frame
                    .zero_page_allocations
                    .iter()
                    .any(|a| a.slot == *id)
        }
        MirMem::Spill { id, offset } => *offset == 0 && routine.frame.spills.contains(id),
        _ => false,
    }
}

fn destination(
    routine: &MirRoutine,
    pair: &[MirMem; 2],
    memory: &MirPhysicalMemory,
    address: u16,
) -> bool {
    if let [MirMem::FixedZeroPage(lo), MirMem::FixedZeroPage(hi)] = pair {
        return matches!(lo.0, 0xAA | 0xAC | 0xAE) && lo.0.checked_add(1) == Some(hi.0);
    }
    ordinary_word(routine, pair, memory, address)
        || pair
            .iter()
            .all(|mem| private_byte(routine, mem) && memory.ordinary(routine.id, mem))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct Proof {
    pub routine: RoutineId,
    pub point: MirProgramPoint,
    pub shape: Shape,
    pub addresses: [u16; 3],
}

pub(in crate::mir6502) fn prove(
    routine: &MirRoutine,
    generation: MirRoutineGeneration,
    point: MirProgramPoint,
    memory: &MirPhysicalMemory,
) -> Result<Proof, Blocker> {
    if point.generation != generation {
        return Err(Blocker::Site);
    }
    let MirSite::Op { block, op_index } = point.site else {
        return Err(Blocker::Site);
    };
    let b = routine
        .blocks
        .iter()
        .find(|b| b.id == block)
        .ok_or(Blocker::Site)?;
    let ops = b
        .ops
        .get(op_index..op_index.checked_add(WINDOW).ok_or(Blocker::Site)?)
        .ok_or(Blocker::Site)?;
    let s = shape(ops)?;
    let mut addresses = [0; 3];
    for (i, pair) in [&s.source, &s.first, &s.second].into_iter().enumerate() {
        let lo = memory
            .address(routine.id, &pair[0])
            .ok_or(Blocker::Placement)?;
        let hi = memory
            .address(routine.id, &pair[1])
            .ok_or(Blocker::Placement)?;
        if lo.checked_add(1) != Some(hi) {
            return Err(Blocker::Footprint);
        }
        if !memory.execution_disjoint(lo) || !memory.execution_disjoint(hi) {
            return Err(Blocker::Storage);
        }
        addresses[i] = lo;
    }
    if !ordinary_word(routine, &s.source, memory, addresses[0])
        || !destination(routine, &s.first, memory, addresses[1])
        || !destination(routine, &s.second, memory, addresses[2])
    {
        return Err(Blocker::Storage);
    }
    if (0..3).any(|a| (a + 1..3).any(|b| addresses[a].abs_diff(addresses[b]) < 2)) {
        return Err(Blocker::Overlap);
    }
    // All crossed accesses are complete ordinary, disjoint direct copies.
    // The final A and N/Z come from the same unchanged high-byte source;
    // stores affect no registers or flags and every destination store remains.
    Ok(Proof {
        routine: routine.id,
        point,
        shape: s,
        addresses,
    })
}

pub(in crate::mir6502) fn validate(
    proof: &Proof,
    routine: &MirRoutine,
    generation: MirRoutineGeneration,
    memory: &MirPhysicalMemory,
    original: &[MirOp],
    selected: &[MirOp],
) -> bool {
    proof.routine == routine.id
        && prove(routine, generation, proof.point, memory).as_ref() == Ok(proof)
        && replacement(original).as_deref() == Some(selected)
        && shape(original).as_ref() == Ok(&proof.shape)
}

#[cfg(test)]
#[path = "word_load_sharing_tests.rs"]
pub(in crate::mir6502) mod tests;
