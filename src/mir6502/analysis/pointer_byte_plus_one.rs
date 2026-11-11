//! A two-operation proof, independent of source names, callers and ABI shape.
#![allow(dead_code)] // Inert until the selected operation is introduced.
use super::{
    effects::{MirFlagSet, MirHomeByte},
    sites::{MirProgramPoint, MirSite},
};
use crate::mir6502::{ir::*, rewrite::context::PostHomeRewriteContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum Blocker {
    Site,
    Shape,
    Placement,
    Source,
    MachineState,
    Definition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) struct Shape {
    pub dst: MirFixedZpSlot,
    pub source: MirAddressConsumer,
    pub offset: u16,
}

/// Fixed pairs resolve to these exact physical bytes in every final layout.
pub(in crate::mir6502) fn valid_pairs(
    dst: MirFixedZpSlot,
    source: MirAddressConsumer,
    offset: u16,
) -> bool {
    let MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo }) = source else {
        return false;
    };
    dst.0 < 255
        && lo.0 < 255
        && offset <= 255
        && (u16::from(dst.0) + 1 < u16::from(lo.0) || u16::from(lo.0) + 1 < u16::from(dst.0))
}

pub(in crate::mir6502) fn shape(ops: &[MirOp]) -> Result<Shape, Blocker> {
    let [
        MirOp::OffsetPointerByIndirectByte {
            op: MirBinaryOp::Add,
            dst: MirMem::FixedZeroPage(dst),
            source,
            offset,
        },
        MirOp::AddByteToWordMem {
            mem: MirMem::FixedZeroPage(next),
            value: MirValue::ConstU8(1),
        },
    ] = ops
    else {
        return Err(Blocker::Shape);
    };
    if dst != next {
        return Err(Blocker::Shape);
    }
    if !valid_pairs(*dst, *source, *offset) {
        return Err(Blocker::Placement);
    }
    Ok(Shape {
        dst: *dst,
        source: *source,
        offset: *offset,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct Proof {
    pub routine: RoutineId,
    pub point: MirProgramPoint,
    pub shape: Shape,
}

pub(in crate::mir6502) fn prove(
    r: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    point: MirProgramPoint,
) -> Result<Proof, Blocker> {
    if point.generation != context.generation() {
        return Err(Blocker::Site);
    }
    let MirSite::Op { block, op_index } = point.site else {
        return Err(Blocker::Site);
    };
    let b = r
        .blocks
        .iter()
        .find(|b| b.id == block)
        .ok_or(Blocker::Site)?;
    let ops = b
        .ops
        .get(op_index..op_index.checked_add(2).ok_or(Blocker::Site)?)
        .ok_or(Blocker::Site)?;
    let shape = shape(ops)?;
    let MirPointerPair::Fixed { lo } = shape.source.pointer_pair() else {
        unreachable!();
    };
    for slot in [lo, MirFixedZpSlot(lo.0 + 1)] {
        if !context.incoming_home_defined(MirHomeByte::FixedZeroPage(slot), point) {
            return Err(Blocker::Source);
        }
    }
    let end = context.point(MirSite::Op {
        block,
        op_index: op_index + 1,
    });
    if [MirReg::A, MirReg::X]
        .iter()
        .any(|&reg| !context.register_dead_after(reg, end).is_proven())
        || !context.flags_dead_after(MirFlagSet::all(), end).is_proven()
    {
        return Err(Blocker::MachineState);
    }
    for slot in [shape.dst, MirFixedZpSlot(shape.dst.0 + 1)] {
        if !context
            .home_definition_dead_after(MirHomeByte::FixedZeroPage(slot), point, end)
            .is_proven()
        {
            return Err(Blocker::Definition);
        }
    }
    Ok(Proof {
        routine: r.id,
        point,
        shape,
    })
}

#[cfg(test)]
#[path = "pointer_byte_plus_one_tests.rs"]
mod tests;
