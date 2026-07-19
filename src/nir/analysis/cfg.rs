use std::collections::{BTreeMap, BTreeSet};

use super::super::facts::BlockId;
use super::super::ir::{NirRoutine, NirTerminator};

/// Target-independent control-flow facts for one NIR routine.
///
/// Executable edges already carry stable `BlockId` identities. Display labels
/// never participate in graph construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::nir) struct NirCfg {
    entry: Option<BlockId>,
    block_ids: BTreeSet<BlockId>,
    block_indices: BTreeMap<BlockId, usize>,
    predecessors: BTreeMap<BlockId, BTreeSet<BlockId>>,
    successors: BTreeMap<BlockId, BTreeSet<BlockId>>,
    reachable: BTreeSet<BlockId>,
    postorder: Vec<BlockId>,
    reverse_postorder: Vec<BlockId>,
    exits: BTreeSet<BlockId>,
}

impl NirCfg {
    pub(in crate::nir) fn from_routine(routine: &NirRoutine) -> Self {
        let entry = routine.blocks.first().map(|block| block.id);
        let mut block_ids = BTreeSet::new();
        let mut block_indices = BTreeMap::new();
        let mut predecessors = BTreeMap::new();
        let mut successors = BTreeMap::new();

        for (index, block) in routine.blocks.iter().enumerate() {
            block_ids.insert(block.id);
            block_indices.entry(block.id).or_insert(index);
            predecessors.entry(block.id).or_insert_with(BTreeSet::new);
            successors.entry(block.id).or_insert_with(BTreeSet::new);
        }

        for block in &routine.blocks {
            for_each_target(&block.terminator, |target_id| {
                if !block_ids.contains(&target_id) {
                    return;
                }
                successors.entry(block.id).or_default().insert(target_id);
                predecessors.entry(target_id).or_default().insert(block.id);
            });
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

        Self {
            entry,
            block_ids,
            block_indices,
            predecessors,
            successors,
            reachable,
            postorder,
            reverse_postorder,
            exits,
        }
    }

    pub(in crate::nir) fn entry(&self) -> Option<BlockId> {
        self.entry
    }

    pub(in crate::nir) fn block_ids(&self) -> &BTreeSet<BlockId> {
        &self.block_ids
    }

    #[allow(dead_code)] // Consumed by the later use-def and data-flow slices.
    pub(in crate::nir) fn block_index(&self, block: BlockId) -> Option<usize> {
        self.block_indices.get(&block).copied()
    }

    pub(in crate::nir) fn predecessors(&self, block: BlockId) -> &BTreeSet<BlockId> {
        self.predecessors.get(&block).unwrap_or(&EMPTY_BLOCK_SET)
    }

    #[allow(dead_code)] // Consumed by the later use-def and data-flow slices.
    pub(in crate::nir) fn successors(&self, block: BlockId) -> &BTreeSet<BlockId> {
        self.successors.get(&block).unwrap_or(&EMPTY_BLOCK_SET)
    }

    pub(in crate::nir) fn reachable(&self) -> &BTreeSet<BlockId> {
        &self.reachable
    }

    #[allow(dead_code)] // Consumed by the later data-flow solver slice.
    pub(in crate::nir) fn postorder(&self) -> &[BlockId] {
        &self.postorder
    }

    #[allow(dead_code)] // Consumed by the later dominance and data-flow slices.
    pub(in crate::nir) fn reverse_postorder(&self) -> &[BlockId] {
        &self.reverse_postorder
    }

    #[allow(dead_code)] // Consumed by the later backward data-flow slice.
    pub(in crate::nir) fn exits(&self) -> &BTreeSet<BlockId> {
        &self.exits
    }
}

static EMPTY_BLOCK_SET: BTreeSet<BlockId> = BTreeSet::new();

fn for_each_target(terminator: &NirTerminator, mut visit: impl FnMut(BlockId)) {
    match terminator {
        NirTerminator::Goto(edge) => visit(edge.target),
        NirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            visit(then_edge.target);
            visit(else_edge.target);
        }
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Return(_)
        | NirTerminator::Exit
        | NirTerminator::Unknown(_) => {}
    }
}

fn visit_postorder(
    block: BlockId,
    successors: &BTreeMap<BlockId, BTreeSet<BlockId>>,
    visited: &mut BTreeSet<BlockId>,
    postorder: &mut Vec<BlockId>,
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
    use crate::nir::{NirBlock, NirEdge, NirRoutine};

    fn edge(target: u32) -> NirEdge {
        NirEdge {
            target: BlockId(target),
            args: Vec::new(),
        }
    }

    fn routine(blocks: Vec<NirBlock>) -> NirRoutine {
        NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks,
        }
    }

    fn block(id: u32, label: &str, terminator: NirTerminator) -> NirBlock {
        NirBlock {
            id: BlockId(id),
            label: label.to_string(),
            params: Vec::new(),
            ops: Vec::new(),
            terminator,
        }
    }

    #[test]
    fn builds_deterministic_diamond_facts_using_block_ids() {
        let routine = routine(vec![
            block(
                0,
                "entry",
                NirTerminator::Branch {
                    condition: crate::nir::NirValue::ConstU8(1),
                    then_edge: edge(1),
                    else_edge: edge(2),
                },
            ),
            block(1, "left", NirTerminator::Goto(edge(3))),
            block(2, "right", NirTerminator::Goto(edge(3))),
            block(3, "exit", NirTerminator::Return(None)),
            block(9, "dead", NirTerminator::Return(None)),
        ]);

        let cfg = NirCfg::from_routine(&routine);

        assert_eq!(cfg.entry(), Some(BlockId(0)));
        assert_eq!(cfg.block_index(BlockId(2)), Some(2));
        assert_eq!(
            cfg.successors(BlockId(0)),
            &BTreeSet::from([BlockId(1), BlockId(2)])
        );
        assert_eq!(
            cfg.predecessors(BlockId(3)),
            &BTreeSet::from([BlockId(1), BlockId(2)])
        );
        assert_eq!(
            cfg.reachable(),
            &BTreeSet::from([BlockId(0), BlockId(1), BlockId(2), BlockId(3)])
        );
        assert_eq!(
            cfg.postorder(),
            &[BlockId(3), BlockId(1), BlockId(2), BlockId(0)]
        );
        assert_eq!(
            cfg.reverse_postorder(),
            &[BlockId(0), BlockId(2), BlockId(1), BlockId(3)]
        );
        assert_eq!(cfg.exits(), &BTreeSet::from([BlockId(3)]));
    }

    #[test]
    fn handles_loops_without_making_the_backedge_an_exit() {
        let routine = routine(vec![
            block(0, "entry", NirTerminator::Goto(edge(1))),
            block(
                1,
                "header",
                NirTerminator::Branch {
                    condition: crate::nir::NirValue::ConstU8(1),
                    then_edge: edge(2),
                    else_edge: edge(3),
                },
            ),
            block(2, "body", NirTerminator::Goto(edge(1))),
            block(3, "exit", NirTerminator::Exit),
        ]);

        let cfg = NirCfg::from_routine(&routine);

        assert_eq!(
            cfg.predecessors(BlockId(1)),
            &BTreeSet::from([BlockId(0), BlockId(2)])
        );
        assert_eq!(cfg.exits(), &BTreeSet::from([BlockId(3)]));
        assert_eq!(cfg.reachable().len(), 4);
        assert_eq!(cfg.reverse_postorder().first(), Some(&BlockId(0)));
    }

    #[test]
    fn leaves_missing_block_edges_unresolved_for_the_verifier() {
        let routine = routine(vec![block(0, "entry", NirTerminator::Goto(edge(9)))]);

        let cfg = NirCfg::from_routine(&routine);

        assert!(cfg.successors(BlockId(0)).is_empty());
        assert_eq!(cfg.reachable(), &BTreeSet::from([BlockId(0)]));
    }
}
