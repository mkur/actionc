//! Bounded-growth aggregate-copy selection. Stage the entire source in private
//! storage, then write the destination; do not expose one live temp per byte.
use super::{FreshTemps, temp_widths::collect_routine_temp_widths};
use crate::mir6502::ir::*;
use crate::nir::LocalId;
use std::collections::BTreeMap;

const MAX_SCALAR_COPY_BYTES: u16 = 32;

pub(super) fn expand_large_aggregate_copies(routine: &mut MirRoutine) -> usize {
    let widths = collect_routine_temp_widths(routine);
    let eligible = |op: &MirOp| {
        matches!(op, MirOp::CopyBytes { destination, source, size, .. }
        if *size > MAX_SCALAR_COPY_BYTES && supported(destination, &widths) && supported(source, &widths))
    };
    let Some(size) = routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .filter(|op| eligible(op))
        .filter_map(|op| match op {
            MirOp::CopyBytes { size, .. } => Some(*size),
            _ => None,
        })
        .max()
    else {
        return 0;
    };
    let next_local = routine
        .frame
        .locals
        .iter()
        .filter_map(|slot| match slot.base {
            MirStorageBase::Local(id) | MirStorageBase::LocalAlias { id, .. } => Some(id.0),
            _ => None,
        })
        .max()
        .map_or(0, |id| id + 1);
    let scratch = MirMem::Local {
        id: LocalId(next_local),
        offset: 0,
    };
    let counter = MirMem::Local {
        id: LocalId(next_local + 1),
        offset: 0,
    };
    let next_slot = routine
        .frame
        .locals
        .iter()
        .map(|slot| slot.id.0)
        .max()
        .map_or(0, |id| id + 1);
    for (index, name, size, scalar_width) in [
        (0, "$aggregate-copy-buffer", size, None),
        (1, "$aggregate-copy-index", 2, Some(MirWidth::Word)),
    ] {
        routine.frame.locals.push(MirStorageSlot {
            id: MirStorageId(next_slot + index),
            name: Some(name.into()),
            storage: MirStorageClass::Scalar,
            storage_size: size,
            scalar_width,
            base: MirStorageBase::Local(LocalId(next_local + index)),
            offset: 0,
            mutable: true,
            init: None,
        });
    }
    let mut next_block = routine
        .blocks
        .iter()
        .map(|block| block.id.0)
        .max()
        .unwrap_or(0)
        + 1;
    let mut fresh = FreshTemps::new(&routine.temps);
    let mut selected = 0;
    for mut original in std::mem::take(&mut routine.blocks) {
        let ops = std::mem::take(&mut original.ops);
        let terminator = original.terminator.clone();
        let mut current = original;
        for op in ops {
            if !eligible(&op) {
                current.ops.push(op);
                continue;
            }
            let MirOp::CopyBytes {
                destination,
                source,
                size,
                destination_volatile,
                source_volatile,
            } = op
            else {
                unreachable!()
            };
            selected += 1;
            let dest = capture_address(
                destination,
                &widths,
                &mut current.ops,
                &mut fresh,
                &mut routine.temps,
            );
            let src = capture_address(
                source,
                &widths,
                &mut current.ops,
                &mut fresh,
                &mut routine.temps,
            );
            let buffer = capture_address(
                MirAddr::Direct(scratch.clone()),
                &widths,
                &mut current.ops,
                &mut fresh,
                &mut routine.temps,
            );
            let stage = MirBlockId(next_block);
            let between = MirBlockId(next_block + 1);
            let write = MirBlockId(next_block + 2);
            let resume = MirBlockId(next_block + 3);
            next_block += 4;
            if source_volatile {
                current.ops.push(super::copies::volatile_barrier());
            }
            current.ops.push(reset(&counter));
            current.terminator = MirTerminator::Jump(MirEdge::plain(stage));
            routine.blocks.push(current);
            routine.blocks.push(copy_loop(
                stage,
                between,
                src,
                buffer.clone(),
                size,
                &counter,
                &mut fresh,
                &mut routine.temps,
            ));
            let mut middle = block(between);
            if source_volatile {
                middle.ops.push(super::copies::volatile_barrier());
            }
            if destination_volatile {
                middle.ops.push(super::copies::volatile_barrier());
            }
            middle.ops.push(reset(&counter));
            middle.terminator = MirTerminator::Jump(MirEdge::plain(write));
            routine.blocks.push(middle);
            routine.blocks.push(copy_loop(
                write,
                resume,
                buffer,
                dest,
                size,
                &counter,
                &mut fresh,
                &mut routine.temps,
            ));
            current = block(resume);
            if destination_volatile {
                current.ops.push(super::copies::volatile_barrier());
            }
        }
        current.terminator = terminator;
        routine.blocks.push(current);
    }
    selected
}

fn block(id: MirBlockId) -> MirBlock {
    MirBlock {
        id,
        label: format!("aggregate-copy:{}", id.0),
        params: vec![],
        ops: vec![],
        terminator: MirTerminator::Unreachable,
    }
}

fn reset(counter: &MirMem) -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(counter.clone()),
        src: MirValue::ConstU16(0),
        width: MirWidth::Word,
    }
}

fn copy_loop(
    id: MirBlockId,
    exit: MirBlockId,
    source: MirValue,
    destination: MirValue,
    size: u16,
    counter: &MirMem,
    fresh: &mut FreshTemps,
    temps: &mut Vec<MirTemp>,
) -> MirBlock {
    let mut block = block(id);
    let index = fresh.fresh(temps);
    let byte = fresh.fresh(temps);
    let next = fresh.fresh(temps);
    let cond = fresh.fresh(temps);
    block.ops.push(MirOp::Load {
        dst: MirDef::VTemp(index),
        src: MirAddr::Direct(counter.clone()),
        width: MirWidth::Word,
    });
    let address = |base| MirAddr::ComputedIndex {
        base,
        index: value(index),
        elem_size: 1,
        offset: 0,
    };
    block.ops.push(MirOp::Load {
        dst: MirDef::VTemp(byte),
        src: address(source),
        width: MirWidth::Byte,
    });
    block.ops.push(MirOp::Store {
        dst: address(destination),
        src: value(byte),
        width: MirWidth::Byte,
    });
    block.ops.push(binary(
        MirBinaryOp::Add,
        next,
        value(index),
        MirValue::ConstU16(1),
    ));
    block.ops.push(MirOp::Store {
        dst: MirAddr::Direct(counter.clone()),
        src: value(next),
        width: MirWidth::Word,
    });
    block.ops.push(MirOp::Compare {
        op: MirCompareOp::Ne,
        dst: MirCondDest::Temp(cond),
        left: value(next),
        right: MirValue::ConstU16(size),
        width: MirWidth::Word,
        signed: false,
    });
    block.terminator = MirTerminator::Branch {
        cond: MirCond::BoolValue(value(cond)),
        then_edge: MirEdge::plain(id),
        else_edge: MirEdge::plain(exit),
    };
    block
}

fn value(id: MirTempId) -> MirValue {
    MirValue::Def(MirDef::VTemp(id))
}
fn binary(op: MirBinaryOp, dst: MirTempId, left: MirValue, right: MirValue) -> MirOp {
    MirOp::Binary {
        op,
        dst: MirDef::VTemp(dst),
        left,
        right,
        width: MirWidth::Word,
        carry_in: None,
        carry_out: MirCarryOut::Ignore,
    }
}

fn index_width(index: &MirValue, widths: &BTreeMap<MirTempId, MirWidth>) -> Option<MirWidth> {
    match index {
        MirValue::ConstU8(_) => Some(MirWidth::Byte),
        MirValue::ConstU16(_) => Some(MirWidth::Word),
        MirValue::Def(MirDef::VTemp(id)) => widths.get(id).copied(),
        _ => None,
    }
}

fn supported(addr: &MirAddr, widths: &BTreeMap<MirTempId, MirWidth>) -> bool {
    match addr {
        MirAddr::Direct(_) | MirAddr::PointerCell { .. } | MirAddr::Deref { .. } => true,
        MirAddr::ComputedIndex {
            index, elem_size, ..
        }
        | MirAddr::PointerIndex {
            index, elem_size, ..
        } => *elem_size > 0 && index_width(index, widths).is_some(),
        _ => false,
    }
}

fn capture_address(
    addr: MirAddr,
    widths: &BTreeMap<MirTempId, MirWidth>,
    ops: &mut Vec<MirOp>,
    fresh: &mut FreshTemps,
    temps: &mut Vec<MirTemp>,
) -> MirValue {
    let (mut base, index, offset) = match addr {
        MirAddr::Direct(mem) => {
            let temp = fresh.fresh(temps);
            ops.push(MirOp::LeaAddr {
                dst: MirDef::VTemp(temp),
                target: mem,
                width: MirWidth::Word,
            });
            return value(temp);
        }
        MirAddr::Deref { ptr, offset } => (ptr, None, offset),
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset,
        } => (base, Some((index, elem_size)), offset),
        MirAddr::PointerCell { ref ptr, offset }
        | MirAddr::PointerIndex {
            ref ptr, offset, ..
        } => {
            let temp = fresh.fresh(temps);
            ops.push(MirOp::Load {
                dst: MirDef::VTemp(temp),
                src: MirAddr::Direct(ptr.clone()),
                width: MirWidth::Word,
            });
            let index = match addr {
                MirAddr::PointerIndex {
                    index, elem_size, ..
                } => Some((index, elem_size)),
                _ => None,
            };
            (value(temp), index, offset)
        }
        _ => unreachable!("preflighted copy endpoint"),
    };
    if let Some((mut index, scale)) = index {
        if index_width(&index, widths) == Some(MirWidth::Byte) {
            let temp = fresh.fresh(temps);
            ops.push(MirOp::Extend {
                dst: MirDef::VTemp(temp),
                src: index,
                from_width: MirWidth::Byte,
                to_width: MirWidth::Word,
                signed: false,
            });
            index = value(temp);
        }
        if scale != 1 {
            let temp = fresh.fresh(temps);
            ops.push(binary(
                MirBinaryOp::Mul,
                temp,
                index,
                MirValue::ConstU16(scale),
            ));
            index = value(temp);
        }
        let temp = fresh.fresh(temps);
        ops.push(binary(MirBinaryOp::Add, temp, base, index));
        base = value(temp);
    }
    if offset != 0 {
        let temp = fresh.fresh(temps);
        ops.push(binary(
            MirBinaryOp::Add,
            temp,
            base,
            MirValue::ConstU16(offset),
        ));
        base = value(temp);
    }
    let temp = fresh.fresh(temps);
    ops.push(MirOp::Move {
        dst: MirDef::VTemp(temp),
        src: base,
        width: MirWidth::Word,
    });
    value(temp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_copy_growth_is_bounded_and_source_is_fully_staged() {
        let mut shape = None;
        for size in [33, 257, 4096, 65535] {
            let mut routine = MirRoutine {
                id: RoutineId(0),
                name: "copy".into(),
                abi: MirRoutineAbi::Action,
                frame: MirFrame::default(),
                temps: vec![],
                effects: MirEffects::default(),
                blocks: vec![MirBlock {
                    id: MirBlockId(0),
                    label: "entry".into(),
                    params: vec![],
                    ops: vec![MirOp::CopyBytes {
                        destination: MirAddr::Direct(MirMem::Absolute(0x7800)),
                        source: MirAddr::Direct(MirMem::Absolute(0x7000)),
                        size,
                        destination_volatile: true,
                        source_volatile: true,
                    }],
                    terminator: MirTerminator::Return,
                }],
            };
            assert_eq!(expand_large_aggregate_copies(&mut routine), 1);
            crate::mir6502::analysis::cfg::MirCfg::from_routine(&routine).unwrap();
            assert_eq!(routine.frame.locals.len(), 2);
            assert_eq!(routine.frame.locals[0].storage_size, size);
            assert_eq!(routine.frame.locals[1].storage_size, 2);
            let ops = routine
                .blocks
                .iter()
                .flat_map(|block| &block.ops)
                .collect::<Vec<_>>();
            assert!(!ops.iter().any(|op| matches!(op, MirOp::CopyBytes { .. })));
            assert_eq!(
                ops.iter()
                    .filter(|op| matches!(op, MirOp::Barrier { .. }))
                    .count(),
                4
            );
            let current_shape = (routine.blocks.len(), routine.temps.len(), ops.len());
            assert_eq!(*shape.get_or_insert(current_shape), current_shape);
            assert!(routine.temps.len() < 20);
            assert_eq!(routine.blocks.len(), 5);
            assert!(matches!(
                routine.blocks[4].terminator,
                MirTerminator::Return
            ));
            // Only the staging loop precedes the reset and destination loop.
            assert!(
                matches!(&routine.blocks[1].terminator, MirTerminator::Branch { then_edge, else_edge, .. }
                if then_edge.target == routine.blocks[1].id && else_edge.target == routine.blocks[2].id)
            );
            assert!(
                matches!(&routine.blocks[2].terminator, MirTerminator::Jump(edge) if edge.target == routine.blocks[3].id)
            );
        }
    }
}
