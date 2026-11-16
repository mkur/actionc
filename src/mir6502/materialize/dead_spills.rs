use std::collections::BTreeSet;

use crate::mir6502::analysis::effects::{MirSpillByte, classify_op, classify_terminator};
use crate::mir6502::ir::{
    MirAddr, MirBlockId, MirMem, MirOp, MirRoutine, MirSpillId, MirTerminator, MirWidth,
};

#[allow(dead_code)]
pub(super) fn remove_dead_spill_stores(routine: &mut MirRoutine) {
    remove_dead_spill_stores_traced(routine, &crate::mir6502::trace::Trace::disabled());
}

pub(super) fn remove_dead_spill_stores_traced(
    routine: &mut MirRoutine,
    trace: &crate::mir6502::trace::Trace,
) {
    let snapshot = trace.routine_snapshot("compat-dead-spills-input", routine);
    let mut remove = BTreeSet::<(usize, usize)>::new();
    for (block_index, block) in routine.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let Some((spill, offset)) = op_store_spill_byte(op) else {
                continue;
            };
            let mut visited = BTreeSet::new();
            let retained = spill_byte_may_be_read_after(
                routine,
                block_index,
                op_index.saturating_add(1),
                spill,
                offset,
                &mut visited,
                &mut |reason, block_index, op| {
                    if trace.wants(routine.id) {
                        trace.event("compat-spill-stop", ||serde_json::json!({
                            "snapshot":snapshot,"routine":routine.id.0,
                            "home":crate::mir6502::trace::homes::memory(routine.id,&MirMem::Spill{id:spill,offset}),
                            "store":{"block":block.id.0,"op":op_index},"reason":reason,
                            "point":{"block":routine.blocks.get(block_index).map(|b|b.id.0),"op":op}
                        }));
                    }
                },
            );
            if trace.wants(routine.id) {
                trace.event("compat-spill-outcome", ||serde_json::json!({"snapshot":snapshot,"routine":routine.id.0,
                    "home":crate::mir6502::trace::homes::memory(routine.id,&MirMem::Spill{id:spill,offset}),
                    "store":{"block":block.id.0,"op":op_index},"retained":retained}));
            }
            if !retained {
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
    visited: &mut BTreeSet<(usize, usize)>,
    observe: &mut impl FnMut(&'static str, usize, Option<usize>),
) -> bool {
    // The initial scan may begin after a candidate store, while a backedge
    // re-enters the same block at op zero and can read the stored value first.
    if !visited.insert((block_index, start)) {
        observe("visited-cycle", block_index, Some(start));
        return false;
    }
    let Some(block) = routine.blocks.get(block_index) else {
        observe("missing-block", block_index, None);
        return true;
    };
    for (op_index, op) in block.ops.iter().enumerate().skip(start) {
        let effects = classify_op(op);
        if effects.reads_spill_byte_compat(spill, offset) {
            observe("explicit-read", block_index, Some(op_index));
            return true;
        }
        if effects.writes_spill_byte_compat(spill, offset) {
            observe("covering-overwrite", block_index, Some(op_index));
            return false;
        }
        if effects.may_read_unknown_spill_byte_compat() {
            observe("unknown-spill-reader", block_index, Some(op_index));
            return true;
        }
    }
    if classify_terminator(&block.terminator)
        .projected_spill_byte_reads
        .contains(&MirSpillByte { id: spill, offset })
    {
        observe("terminator-read", block_index, None);
        return true;
    }
    let read = block_successor_indices(routine, &block.terminator)
        .into_iter()
        .any(|successor_index| {
            spill_byte_may_be_read_after(
                routine,
                successor_index,
                0,
                spill,
                offset,
                visited,
                observe,
            )
        });
    if !read {
        observe("no-read-found", block_index, None);
    }
    read
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

#[cfg(test)]
mod trace_tests {
    use super::*;
    use crate::mir6502::ir::*;
    use crate::mir6502::trace::Mir6502Trace;

    #[test]
    fn trace_reports_actual_short_circuit_stops_without_changing_deletions() {
        let home = MirMem::Spill {
            id: MirSpillId(0),
            offset: 0,
        };
        let store = MirOp::Store {
            dst: MirAddr::Direct(home.clone()),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        };
        let read = MirOp::Load {
            dst: MirDef::Reg(MirReg::X),
            src: MirAddr::Direct(home),
            width: MirWidth::Byte,
        };
        let base = MirRoutine {
            id: RoutineId(0),
            name: "scan".into(),
            abi: MirRoutineAbi::Action,
            direct_result_abi: None,
            scalar_signature: None,
            inline: Default::default(),
            temps: vec![],
            effects: Default::default(),
            frame: MirFrame {
                spills: vec![MirSpillId(0)],
                ..Default::default()
            },
            blocks: vec![
                MirBlock {
                    id: MirBlockId(0),
                    label: "entry".into(),
                    params: vec![],
                    ops: vec![store.clone()],
                    terminator: MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
                },
                MirBlock {
                    id: MirBlockId(1),
                    label: "successor".into(),
                    params: vec![],
                    ops: vec![],
                    terminator: MirTerminator::Return,
                },
            ],
        };
        for (op, reason, retained) in [
            (Some(read), "explicit-read", true),
            (Some(store), "covering-overwrite", false),
            (
                Some(MirOp::RuntimeHelper {
                    helper: MirRuntimeHelper::Mul,
                    args: vec![],
                    result: None,
                    additional_results: vec![],
                    effects: Default::default(),
                }),
                "unknown-spill-reader",
                true,
            ),
            (
                Some(MirOp::MachineBlock {
                    id: MirMachineBlockId(0),
                    effects: Default::default(),
                }),
                "unknown-spill-reader",
                true,
            ),
            (None, "no-read-found", false),
        ] {
            let mut on = base.clone();
            on.blocks[1].ops = op.into_iter().collect();
            let mut off = on.clone();
            let trace = Mir6502Trace::default();
            remove_dead_spill_stores(&mut off);
            remove_dead_spill_stores_traced(&mut on, &trace.context());
            assert_eq!(on, off);
            let events = trace.events();
            assert!(
                events
                    .iter()
                    .any(|e| e.kind == "compat-spill-stop" && e.data["reason"] == reason),
                "{reason}"
            );
            let outcome = events
                .iter()
                .find(|e| e.kind == "compat-spill-outcome" && e.data["store"]["block"] == 0)
                .unwrap();
            assert_eq!(outcome.data["retained"], retained);
        }
    }
}
