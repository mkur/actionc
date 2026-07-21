use std::collections::BTreeSet;

use crate::mir6502::analysis::effects::{MirSpillByte, classify_op, classify_terminator};
use crate::mir6502::ir::{
    MirAddr, MirBlockId, MirMem, MirOp, MirRoutine, MirSpillId, MirTerminator, MirWidth,
};

pub(super) fn remove_dead_spill_stores(routine: &mut MirRoutine) {
    let mut remove = BTreeSet::<(usize, usize)>::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let Some((spill, offset)) = op_store_spill_byte(op) else {
                continue;
            };
            let mut visited = BTreeSet::new();
            if !spill_byte_may_be_read_after(
                routine,
                block_index,
                op_index.saturating_add(1),
                spill,
                offset,
                &mut visited,
            ) {
                remove.insert((block_index, op_index));
            }
        }
    }

    for (block_index, block) in routine.blocks.iter_mut().enumerate() {
        let mut op_index = 0usize;
        block.ops.retain(|_| {
            let keep = !remove.contains(&(block_index, op_index));
            op_index = op_index.saturating_add(1);
            keep
        });
    }
}

fn spill_byte_may_be_read_after(
    routine: &MirRoutine,
    block_index: usize,
    start: usize,
    spill: MirSpillId,
    offset: u16,
    visited: &mut BTreeSet<usize>,
) -> bool {
    if !visited.insert(block_index) {
        return false;
    }
    let Some(block) = routine.blocks.get(block_index) else {
        return true;
    };
    for op in block.ops.iter().skip(start) {
        let effects = classify_op(op);
        if effects.reads_spill_byte_compat(spill, offset) {
            return true;
        }
        if effects.writes_spill_byte_compat(spill, offset) {
            return false;
        }
        if effects.may_read_unknown_spill_byte_compat() {
            return true;
        }
    }
    if classify_terminator(&block.terminator)
        .projected_spill_byte_reads
        .contains(&MirSpillByte { id: spill, offset })
    {
        return true;
    }
    block_successor_indices(routine, &block.terminator)
        .into_iter()
        .any(|successor_index| {
            spill_byte_may_be_read_after(routine, successor_index, 0, spill, offset, visited)
        })
}

fn op_store_spill_byte(op: &MirOp) -> Option<(MirSpillId, u16)> {
    let MirOp::Store {
        dst: MirAddr::Direct(MirMem::Spill { id, offset }),
        width: MirWidth::Byte,
        ..
    } = op
    else {
        return None;
    };
    Some((*id, *offset))
}

pub(super) fn block_successor_indices(
    routine: &MirRoutine,
    terminator: &MirTerminator,
) -> Vec<usize> {
    let mut successors = Vec::<MirBlockId>::new();
    match terminator {
        MirTerminator::Jump(edge) => successors.push(edge.target),
        MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            successors.push(then_edge.target);
            successors.push(else_edge.target);
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => {}
    }
    successors
        .into_iter()
        .filter_map(|id| routine.blocks.iter().position(|block| block.id == id))
        .collect()
}
