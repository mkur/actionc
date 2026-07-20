use std::collections::BTreeSet;

use super::cfg::NirCfg;
use crate::analysis::dominance::Dominance;
use crate::nir::BlockId;

/// Dominance facts derived from one immutable NIR CFG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::nir) struct NirDominance {
    shared: Dominance<BlockId>,
}

impl NirDominance {
    pub(in crate::nir) fn from_cfg(cfg: &NirCfg) -> Self {
        Self {
            shared: Dominance::from_graph(cfg),
        }
    }

    pub(in crate::nir) fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        self.shared.dominates(dominator, block)
    }

    pub(in crate::nir) fn root(&self) -> Option<BlockId> {
        self.shared.root()
    }

    #[allow(dead_code)] // Used by dominance-scoped optimizer slices.
    pub(in crate::nir) fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        self.shared.immediate_dominator(block)
    }

    #[allow(dead_code)] // Used by dominance-scoped optimizer slices.
    pub(in crate::nir) fn children(&self, block: BlockId) -> &[BlockId] {
        self.shared.children(block)
    }

    #[allow(dead_code)] // Used by dominance tests and later optimizer slices.
    pub(in crate::nir) fn dominance_frontier(&self, block: BlockId) -> &BTreeSet<BlockId> {
        self.shared.dominance_frontier(block)
    }

    pub(in crate::nir) fn pruned_iterated_frontier(
        &self,
        definitions: &BTreeSet<BlockId>,
        live_in: &BTreeSet<BlockId>,
    ) -> BTreeSet<BlockId> {
        self.shared.pruned_iterated_frontier(definitions, live_in)
    }

    #[allow(dead_code)] // Used by loop-aware optimizer slices.
    pub(in crate::nir) fn is_backedge(&self, from: BlockId, to: BlockId) -> bool {
        self.shared.is_backedge(from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{NirBlock, NirEdge, NirRoutine, NirTerminator, NirValue};

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
    fn computes_diamond_immediate_dominators_and_tree() {
        let routine = routine(vec![
            block(
                0,
                "entry",
                NirTerminator::Branch {
                    condition: NirValue::ConstU8(1),
                    then_edge: edge(1),
                    else_edge: edge(2),
                },
            ),
            block(1, "left", NirTerminator::Goto(edge(3))),
            block(2, "right", NirTerminator::Goto(edge(3))),
            block(3, "join", NirTerminator::Return(None)),
        ]);
        let cfg = NirCfg::from_routine(&routine);
        let dominance = NirDominance::from_cfg(&cfg);

        assert!(dominance.dominates(BlockId(0), BlockId(3)));
        assert!(!dominance.dominates(BlockId(1), BlockId(3)));
        assert_eq!(dominance.immediate_dominator(BlockId(3)), Some(BlockId(0)));
        assert_eq!(
            dominance.children(BlockId(0)),
            &[BlockId(1), BlockId(2), BlockId(3)]
        );
        assert_eq!(
            dominance.dominance_frontier(BlockId(1)),
            &BTreeSet::from([BlockId(3)])
        );
        assert_eq!(
            dominance.pruned_iterated_frontier(
                &BTreeSet::from([BlockId(1), BlockId(2)]),
                &BTreeSet::from([BlockId(3)]),
            ),
            BTreeSet::from([BlockId(3)])
        );
    }

    #[test]
    fn identifies_loop_backedges() {
        let routine = routine(vec![
            block(0, "entry", NirTerminator::Goto(edge(1))),
            block(
                1,
                "header",
                NirTerminator::Branch {
                    condition: NirValue::ConstU8(1),
                    then_edge: edge(2),
                    else_edge: edge(3),
                },
            ),
            block(2, "body", NirTerminator::Goto(edge(1))),
            block(3, "exit", NirTerminator::Return(None)),
        ]);
        let cfg = NirCfg::from_routine(&routine);
        let dominance = NirDominance::from_cfg(&cfg);

        assert_eq!(dominance.immediate_dominator(BlockId(2)), Some(BlockId(1)));
        assert!(dominance.is_backedge(BlockId(2), BlockId(1)));
        assert!(!dominance.is_backedge(BlockId(1), BlockId(2)));
        assert!(
            dominance
                .pruned_iterated_frontier(
                    &BTreeSet::from([BlockId(2)]),
                    &BTreeSet::from([BlockId(1)]),
                )
                .contains(&BlockId(1))
        );
    }
}
