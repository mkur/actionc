//! Retained-store byte forwarding through a bounded, unchanged argument gap.
#![allow(dead_code)] // Inert until the selection slice.
use super::{
    call_predicates::ordinary_source,
    effects::{MirFlagSet, classify_op},
    record_bases::MirPhysicalMemory,
    sites::*,
};
use crate::mir6502::{ir::*, rewrite::context::PostHomeRewriteContext};

pub(in crate::mir6502) const MAX_REGION: usize = 64;
pub(in crate::mir6502) const MAX_TAIL: usize = 3;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum Blocker {
    Site,
    Shape,
    Budget,
    Storage,
    Placement,
    Alias,
    IncomingX,
    XRead,
    XClobber,
    Gap,
    InsertedNz,
    RemovedNz,
    CallFlags,
    Call,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct Shape {
    pub home: MirMem,
    pub store: MirSite,
    pub reload: MirSite,
    pub call: MirSite,
}
impl Shape {
    pub fn range(&self) -> std::ops::Range<usize> {
        let MirSite::Op { op_index: s, .. } = self.store else {
            unreachable!()
        };
        let MirSite::Op { op_index: e, .. } = self.reload else {
            unreachable!()
        };
        s..e + 1
    }
}
pub(in crate::mir6502) fn store_home(op: &MirOp) -> Option<&MirMem> {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(h),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        } => Some(h),
        _ => None,
    }
}
pub(in crate::mir6502) fn shape(r: &MirRoutine, site: MirSite) -> Result<Shape, Blocker> {
    let MirSite::Op { block, op_index: s } = site else {
        return Err(Blocker::Site);
    };
    let b = r
        .blocks
        .iter()
        .find(|b| b.id == block)
        .ok_or(Blocker::Site)?;
    let h = store_home(b.ops.get(s).ok_or(Blocker::Site)?).ok_or(Blocker::Shape)?;
    for i in s + 1..b.ops.len().min(s + MAX_REGION) {
        if matches!(
            b.ops[i],
            MirOp::Call { .. }
                | MirOp::RuntimeHelper { .. }
                | MirOp::MachineBlock { .. }
                | MirOp::Barrier { .. }
        ) {
            return Err(Blocker::Shape);
        }
        if !matches!(&b.ops[i],MirOp::Load {dst:MirDef::Reg(MirReg::X),src:MirAddr::Direct(m),width:MirWidth::Byte} if m==h)
        {
            continue;
        }
        for k in i + 1..b.ops.len().min(i + MAX_TAIL + 2) {
            if k - s + 1 > MAX_REGION {
                return Err(Blocker::Budget);
            }
            if matches!(b.ops[k], MirOp::Call { .. }) {
                return Ok(Shape {
                    home: h.clone(),
                    store: site,
                    reload: MirSite::Op { block, op_index: i },
                    call: MirSite::Op { block, op_index: k },
                });
            }
            if !simple(&b.ops[k]) {
                break;
            }
        }
    }
    Err(if b.ops.len().saturating_sub(s) > MAX_REGION {
        Blocker::Budget
    } else {
        Blocker::Shape
    })
}
fn simple(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A | MirReg::Y),
            src: MirAddr::Direct(_),
            width: MirWidth::Byte
        } | MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A | MirReg::Y),
            width: MirWidth::Byte,
            ..
        } | MirOp::Store {
            dst: MirAddr::Direct(_),
            src: MirValue::Def(MirDef::Reg(MirReg::A | MirReg::Y)),
            width: MirWidth::Byte
        } | MirOp::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub,
            dst: MirDef::Reg(MirReg::A),
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            right: MirValue::ConstU8(_),
            width: MirWidth::Byte,
            ..
        } | MirOp::LoadIndirect {
            consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { .. }),
            dst: MirDef::Reg(MirReg::A),
            offset: 0..=255
        }
    )
}
fn ordinary(r: &MirRoutine, h: &MirMem, m: &MirPhysicalMemory) -> bool {
    match h {
        MirMem::FixedZeroPage(s) => (0xA0..=0xA4).contains(&s.0),
        MirMem::Spill { .. } | MirMem::ZeroPage(_) => m.ordinary(r.id, h),
        MirMem::Param { .. } | MirMem::Local { .. } => {
            ordinary_source(r, h, MirWidth::Byte) && m.ordinary(r.id, h)
        }
        _ => false,
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct Proof {
    pub routine: RoutineId,
    pub point: MirProgramPoint,
    pub shape: Shape,
    pub address: u16,
    pub tail_restores_nz: bool,
}

pub(in crate::mir6502) fn prove(
    r: &MirRoutine,
    c: &PostHomeRewriteContext<'_, '_>,
    point: MirProgramPoint,
    m: &MirPhysicalMemory,
) -> Result<Proof, Blocker> {
    if point.generation != c.generation() {
        return Err(Blocker::Site);
    }
    let s = shape(r, point.site)?;
    let range = s.range();
    let block = r
        .blocks
        .iter()
        .find(|b| b.id == s.store.block())
        .ok_or(Blocker::Site)?;
    let address = m.address(r.id, &s.home).ok_or(Blocker::Placement)?;
    if !ordinary(r, &s.home, m) {
        return Err(Blocker::Storage);
    }
    let MirSite::Op { op_index: call, .. } = s.call else {
        unreachable!()
    };
    let MirOp::Call {
        target: MirCallTarget::Routine(target),
        abi,
        args,
        effects,
        ..
    } = &block.ops[call]
    else {
        return Err(Blocker::Call);
    };
    if !(2..=5).contains(&args.len())
        || abi.params != args.iter().map(|a| a.home.clone()).collect::<Vec<_>>()
        || !args.iter().enumerate().all(|(i, a)| {
            a.width == MirWidth::Byte
                && a.home == crate::mir6502::abi::action_arg_home(i as u16, MirWidth::Byte)
        })
        || args[1].value != MirValue::Def(MirDef::Reg(MirReg::X))
    {
        return Err(Blocker::Call);
    }
    for op in &block.ops[range.start + 1..range.end - 1] {
        let e = classify_op(op);
        if e.reads_reg(MirReg::X) {
            return Err(Blocker::XRead);
        }
        if e.may_clobber_reg_compat(MirReg::X) {
            return Err(Blocker::XClobber);
        }
        if !simple(op)
            || e.memory.may_write_any
            || e.memory.indirect_writes
            || e.memory.opaque
            || e.homes.unknown_writes
        {
            return Err(Blocker::Gap);
        }
        for write in &e.memory.direct_writes {
            let base = m.address(r.id, &write.base).ok_or(Blocker::Placement)?;
            if address.wrapping_sub(base) < write.bytes {
                return Err(Blocker::Alias);
            }
            // Only ordinary direct writes and existing backend ABI/pointer
            // scratch stores occur in the admitted setup operations.
            if !m.ordinary(r.id, &write.base)
                && !matches!(&write.base,MirMem::FixedZeroPage(s) if (0xA0..=0xAF).contains(&s.0))
            {
                return Err(Blocker::Gap);
            }
        }
    }
    if !c.register_dead_after(MirReg::X, point).is_proven() {
        return Err(Blocker::IncomingX);
    }
    let nz = MirFlagSet {
        n: true,
        z: true,
        ..MirFlagSet::default()
    };
    if !c.flags_dead_after(nz, point).is_proven() {
        return Err(Blocker::InsertedNz);
    }
    let tail = &block.ops[range.end..call];
    if tail.len() > MAX_TAIL
        || tail.iter().any(|o| {
            !simple(o)
                || classify_op(o).reads_reg(MirReg::X)
                || classify_op(o).may_clobber_reg_compat(MirReg::X)
        })
    {
        return Err(Blocker::Gap);
    }
    // Tail operations are unchanged. A definite, modeled N/Z definition
    // restores their exact value before any opaque machine-call observation.
    let restored = tail.iter().any(|op| {
        let e = classify_op(op);
        e.machine.flag_writes.n && e.machine.flag_writes.z
    });
    if !restored
        && (effects.opaque || effects.reads.flags || !c.scalar_entry_ignores_flags(*target))
    {
        return Err(Blocker::CallFlags);
    }
    if !c.flags_dead_after(nz, c.point(s.reload)).is_proven() {
        return Err(Blocker::RemovedNz);
    }
    Ok(Proof {
        routine: r.id,
        point,
        shape: s,
        address,
        tail_restores_nz: restored,
    })
}
pub(in crate::mir6502) fn replacement(r: &MirRoutine, s: &Shape) -> Option<Vec<MirOp>> {
    let b = r.blocks.iter().find(|b| b.id == s.store.block())?;
    let range = s.range();
    let mut ops = vec![
        b.ops.get(range.start)?.clone(),
        MirOp::Move {
            dst: MirDef::Reg(MirReg::X),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ];
    ops.extend_from_slice(b.ops.get(range.start + 1..range.end - 1)?);
    Some(ops)
}
pub(in crate::mir6502) fn validate(
    p: &Proof,
    r: &MirRoutine,
    c: &PostHomeRewriteContext<'_, '_>,
    m: &MirPhysicalMemory,
    range: std::ops::Range<usize>,
    selected: &[MirOp],
) -> bool {
    p.routine == r.id
        && p.shape.range() == range
        && prove(r, c, p.point, m).as_ref() == Ok(p)
        && replacement(r, &p.shape).as_deref() == Some(selected)
}
#[cfg(test)]
#[path = "argument_x_forwarding_tests.rs"]
pub(in crate::mir6502) mod tests;
