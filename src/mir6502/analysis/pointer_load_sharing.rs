//! Exact carry-preserving store motion around two redundant ordinary reads.
#![allow(dead_code)] // The first slice exposes an inert proof query.
use super::{call_predicates::ordinary_source, record_bases::MirPhysicalMemory, sites::*};
use crate::mir6502::ir::*;

pub(in crate::mir6502) const WINDOW: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum Blocker {
    Site,
    Shape,
    Storage,
    Placement,
    Overlap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct Shape {
    pub source: [MirMem; 2],
    pub capture: MirFixedZpSlot,
    pub pointer: MirFixedZpSlot,
}

fn load(op: &MirOp) -> Option<&MirMem> {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(m),
            width: MirWidth::Byte,
        } => Some(m),
        _ => None,
    }
}
fn store(op: &MirOp) -> Option<MirFixedZpSlot> {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(m)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } => Some(*m),
        _ => None,
    }
}
fn add(op: &MirOp, value: u8, carry: MirCarryIn, out: MirCarryOut) -> bool {
    matches!(op, MirOp::Binary { op: MirBinaryOp::Add, dst: MirDef::Reg(MirReg::A), left: MirValue::Def(MirDef::Reg(MirReg::A)), right: MirValue::ConstU8(v), width: MirWidth::Byte, carry_in: Some(c), carry_out } if *v==value && *c==carry && *carry_out==out)
}

pub(in crate::mir6502) fn shape(ops: &[MirOp]) -> Result<Shape, Blocker> {
    let [
        lo,
        low_add,
        cap_lo,
        hi,
        high_add,
        cap_hi,
        reload_lo,
        ptr_lo,
        reload_hi,
        ptr_hi,
        read,
    ] = ops
    else {
        return Err(Blocker::Shape);
    };
    let source = [
        load(lo).ok_or(Blocker::Shape)?.clone(),
        load(hi).ok_or(Blocker::Shape)?.clone(),
    ];
    let capture = store(cap_lo).ok_or(Blocker::Shape)?;
    let pointer = store(ptr_lo).ok_or(Blocker::Shape)?;
    if !add(low_add, 1, MirCarryIn::Clear, MirCarryOut::Produce)
        || !add(high_add, 0, MirCarryIn::FromPrevious, MirCarryOut::Ignore)
        || load(reload_lo) != Some(&source[0])
        || load(reload_hi) != Some(&source[1])
        || capture.0.checked_add(1).map(MirFixedZpSlot) != store(cap_hi)
        || pointer.0.checked_add(1).map(MirFixedZpSlot) != store(ptr_hi)
        || !matches!(read, MirOp::LoadIndirect { consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo }), dst: MirDef::Reg(MirReg::A), offset: 0 } if *lo==pointer)
    {
        return Err(Blocker::Shape);
    }
    Ok(Shape {
        source,
        capture,
        pointer,
    })
}

/// The retained operations are cloned, including the exact terminal consumer.
pub(in crate::mir6502) fn replacement(original: &[MirOp]) -> Option<Vec<MirOp>> {
    shape(original).ok()?;
    Some(
        [0, 7, 1, 2, 3, 9, 4, 5, 10]
            .map(|i| original[i].clone())
            .to_vec(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct Proof {
    pub routine: RoutineId,
    pub point: MirProgramPoint,
    pub shape: Shape,
    pub source_address: u16,
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
    let high = match &s.source[0] {
        MirMem::Param { id, offset } => MirMem::Param {
            id: *id,
            offset: offset.checked_add(1).ok_or(Blocker::Storage)?,
        },
        MirMem::Local { id, offset } => MirMem::Local {
            id: *id,
            offset: offset.checked_add(1).ok_or(Blocker::Storage)?,
        },
        _ => return Err(Blocker::Storage),
    };
    if high != s.source[1] || !ordinary_source(routine, &s.source[0], MirWidth::Word) {
        return Err(Blocker::Storage);
    }
    let address = memory
        .address(routine.id, &s.source[0])
        .ok_or(Blocker::Placement)?;
    let high_address = address.checked_add(1).ok_or(Blocker::Placement)?;
    if memory.address(routine.id, &s.source[1]) != Some(high_address) {
        return Err(Blocker::Placement);
    }
    // P is an existing backend scratch pair. C is an ordinary RAM capture,
    // with no hardware or ABI observer inside the exact window. S cannot
    // change under any retained store, including a separately named alias.
    if s.capture.0 == 255 || !matches!(s.pointer.0, 0xAC | 0xAE) {
        return Err(Blocker::Placement);
    }
    let ranges = [address, u16::from(s.capture.0), u16::from(s.pointer.0)];
    if (0..3).any(|a| (a + 1..3).any(|b| ranges[a].abs_diff(ranges[b]) < 2)) {
        return Err(Blocker::Overlap);
    }
    if !s.source.iter().all(|m| memory.ordinary(routine.id, m)) {
        return Err(Blocker::Storage);
    }
    Ok(Proof {
        routine: routine.id,
        point,
        shape: s,
        source_address: address,
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
#[path = "pointer_load_sharing_tests.rs"]
pub(in crate::mir6502) mod tests;
