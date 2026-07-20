#![allow(dead_code)] // Query surface is consumed incrementally by later workflow slices.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::graph::DataflowGraph;
use crate::mir6502::ir::{MirBlockId, MirRoutine, MirTerminator};

/// Immutable control-flow facts for one MIR6502 routine generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirCfg {
    entry: Option<MirBlockId>,
    block_ids: BTreeSet<MirBlockId>,
    block_indices: BTreeMap<MirBlockId, usize>,
    predecessors: BTreeMap<MirBlockId, BTreeSet<MirBlockId>>,
    successors: BTreeMap<MirBlockId, BTreeSet<MirBlockId>>,
    reachable: BTreeSet<MirBlockId>,
    postorder: Vec<MirBlockId>,
    reverse_postorder: Vec<MirBlockId>,
    exits: BTreeSet<MirBlockId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirCfgError {
    pub block: MirBlockId,
    pub message: String,
}

impl MirCfg {
    pub(in crate::mir6502) fn from_routine(routine: &MirRoutine) -> Result<Self, Vec<MirCfgError>> {
        let entry = routine.blocks.first().map(|block| block.id);
        let mut block_ids = BTreeSet::new();
        let mut block_indices = BTreeMap::new();
        let mut predecessors = BTreeMap::new();
        let mut successors = BTreeMap::new();
        let mut errors = Vec::new();

        for (index, block) in routine.blocks.iter().enumerate() {
            if !block_ids.insert(block.id) {
                errors.push(MirCfgError {
                    block: block.id,
                    message: format!("duplicate MIR block id `b{}`", block.id.0),
                });
                continue;
            }
            block_indices.insert(block.id, index);
            predecessors.insert(block.id, BTreeSet::new());
            successors.insert(block.id, BTreeSet::new());
        }

        for block in &routine.blocks {
            for target in terminator_targets(&block.terminator) {
                if !block_ids.contains(&target) {
                    errors.push(MirCfgError {
                        block: block.id,
                        message: format!("CFG edge targets missing block `b{}`", target.0),
                    });
                    continue;
                }
                successors.entry(block.id).or_default().insert(target);
                predecessors.entry(target).or_default().insert(block.id);
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        let mut postorder = Vec::new();
        let mut reachable = BTreeSet::new();
        if let Some(entry) = entry {
            visit_postorder(entry, &successors, &mut reachable, &mut postorder);
        }
        let reverse_postorder = postorder.iter().rev().copied().collect();
        let exits = reachable
            .iter()
            .copied()
            .filter(|block| successors.get(block).is_none_or(BTreeSet::is_empty))
            .collect();

        Ok(Self {
            entry,
            block_ids,
            block_indices,
            predecessors,
            successors,
            reachable,
            postorder,
            reverse_postorder,
            exits,
        })
    }

    pub(in crate::mir6502) fn entry(&self) -> Option<MirBlockId> {
        self.entry
    }

    pub(in crate::mir6502) fn block_ids(&self) -> &BTreeSet<MirBlockId> {
        &self.block_ids
    }

    pub(in crate::mir6502) fn block_index(&self, block: MirBlockId) -> Option<usize> {
        self.block_indices.get(&block).copied()
    }

    pub(in crate::mir6502) fn predecessors(&self, block: MirBlockId) -> &BTreeSet<MirBlockId> {
        self.predecessors.get(&block).unwrap_or(&EMPTY_BLOCK_SET)
    }

    pub(in crate::mir6502) fn successors(&self, block: MirBlockId) -> &BTreeSet<MirBlockId> {
        self.successors.get(&block).unwrap_or(&EMPTY_BLOCK_SET)
    }

    pub(in crate::mir6502) fn reachable(&self) -> &BTreeSet<MirBlockId> {
        &self.reachable
    }

    pub(in crate::mir6502) fn postorder(&self) -> &[MirBlockId] {
        &self.postorder
    }

    pub(in crate::mir6502) fn reverse_postorder(&self) -> &[MirBlockId] {
        &self.reverse_postorder
    }

    pub(in crate::mir6502) fn exits(&self) -> &BTreeSet<MirBlockId> {
        &self.exits
    }
}

impl DataflowGraph for MirCfg {
    type Node = MirBlockId;

    fn entry(&self) -> Option<Self::Node> {
        self.entry
    }

    fn nodes(&self) -> &BTreeSet<Self::Node> {
        &self.block_ids
    }

    fn predecessors(&self, node: Self::Node) -> &BTreeSet<Self::Node> {
        self.predecessors.get(&node).unwrap_or(&EMPTY_BLOCK_SET)
    }

    fn successors(&self, node: Self::Node) -> &BTreeSet<Self::Node> {
        self.successors.get(&node).unwrap_or(&EMPTY_BLOCK_SET)
    }

    fn reachable(&self) -> &BTreeSet<Self::Node> {
        &self.reachable
    }

    fn postorder(&self) -> &[Self::Node] {
        &self.postorder
    }

    fn reverse_postorder(&self) -> &[Self::Node] {
        &self.reverse_postorder
    }
}

static EMPTY_BLOCK_SET: BTreeSet<MirBlockId> = BTreeSet::new();

fn terminator_targets(terminator: &MirTerminator) -> Vec<MirBlockId> {
    match terminator {
        MirTerminator::Jump(edge) => vec![edge.target],
        MirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge.target, else_edge.target],
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => Vec::new(),
    }
}

fn visit_postorder(
    block: MirBlockId,
    successors: &BTreeMap<MirBlockId, BTreeSet<MirBlockId>>,
    visited: &mut BTreeSet<MirBlockId>,
    postorder: &mut Vec<MirBlockId>,
) {
    if !visited.insert(block) {
        return;
    }
    if let Some(next) = successors.get(&block) {
        for successor in next {
            visit_postorder(*successor, successors, visited, postorder);
        }
    }
    postorder.push(block);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::dominance::Dominance;
    use crate::mir6502::ir::{
        MirBlock, MirCond, MirEdge, MirEffects, MirFrame, MirRoutineAbi, MirValue, RoutineId,
    };

    fn block(id: u32, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops: Vec::new(),
            terminator,
        }
    }

    fn routine(blocks: Vec<MirBlock>) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "Main".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn jump(target: u32) -> MirTerminator {
        MirTerminator::Jump(MirEdge::plain(MirBlockId(target)))
    }

    fn branch(then_target: u32, else_target: u32) -> MirTerminator {
        MirTerminator::Branch {
            cond: MirCond::BoolValue(MirValue::ConstU8(1)),
            then_edge: MirEdge::plain(MirBlockId(then_target)),
            else_edge: MirEdge::plain(MirBlockId(else_target)),
        }
    }

    #[test]
    fn builds_diamond_with_multiple_exits_and_unreachable_block() {
        let routine = routine(vec![
            block(0, branch(1, 2)),
            block(1, jump(3)),
            block(2, MirTerminator::Exit),
            block(3, MirTerminator::Return),
            block(9, MirTerminator::Return),
        ]);
        let cfg = MirCfg::from_routine(&routine).unwrap();

        assert_eq!(cfg.entry(), Some(MirBlockId(0)));
        assert_eq!(
            cfg.predecessors(MirBlockId(3)),
            &BTreeSet::from([MirBlockId(1)])
        );
        assert_eq!(
            cfg.successors(MirBlockId(0)),
            &BTreeSet::from([MirBlockId(1), MirBlockId(2)])
        );
        assert_eq!(cfg.exits(), &BTreeSet::from([MirBlockId(2), MirBlockId(3)]));
        assert!(!cfg.reachable().contains(&MirBlockId(9)));
        assert_eq!(cfg.block_index(MirBlockId(9)), Some(4));
    }

    #[test]
    fn handles_loops_and_is_stable_when_non_entry_blocks_reorder() {
        let first = routine(vec![
            block(0, jump(1)),
            block(1, branch(2, 3)),
            block(2, jump(1)),
            block(3, MirTerminator::Return),
        ]);
        let reordered = routine(vec![
            block(0, jump(1)),
            block(3, MirTerminator::Return),
            block(2, jump(1)),
            block(1, branch(2, 3)),
        ]);
        let first_cfg = MirCfg::from_routine(&first).unwrap();
        let reordered_cfg = MirCfg::from_routine(&reordered).unwrap();
        let dominance = Dominance::from_graph(&first_cfg);

        assert_eq!(first_cfg.block_ids(), reordered_cfg.block_ids());
        assert_eq!(first_cfg.reachable(), reordered_cfg.reachable());
        assert_eq!(first_cfg.postorder(), reordered_cfg.postorder());
        assert_eq!(
            first_cfg.reverse_postorder(),
            reordered_cfg.reverse_postorder()
        );
        assert_eq!(first_cfg.block_index(MirBlockId(1)), Some(1));
        assert_eq!(reordered_cfg.block_index(MirBlockId(1)), Some(3));
        assert!(first_cfg.exits().contains(&MirBlockId(3)));
        assert!(!first_cfg.exits().contains(&MirBlockId(1)));
        assert!(dominance.is_backedge(MirBlockId(2), MirBlockId(1)));
    }

    #[test]
    fn rejects_duplicate_ids_and_dangling_targets() {
        let duplicate = routine(vec![block(0, jump(9)), block(0, MirTerminator::Return)]);
        let errors = MirCfg::from_routine(&duplicate).unwrap_err();

        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("duplicate"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("missing block `b9`"))
        );
    }
}
