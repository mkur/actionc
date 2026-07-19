use std::collections::{BTreeMap, BTreeSet};

use super::cfg::NirCfg;
use crate::nir::BlockId;

/// Dominance facts derived from one immutable NIR CFG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::nir) struct NirDominance {
    dominators: BTreeMap<BlockId, BTreeSet<BlockId>>,
    immediate_dominators: BTreeMap<BlockId, BlockId>,
    children: BTreeMap<BlockId, Vec<BlockId>>,
}

impl NirDominance {
    pub(in crate::nir) fn from_cfg(cfg: &NirCfg) -> Self {
        let Some(entry) = cfg.entry() else {
            return Self {
                dominators: BTreeMap::new(),
                immediate_dominators: BTreeMap::new(),
                children: BTreeMap::new(),
            };
        };

        // Preserve the verifier's established treatment of unreachable blocks:
        // they participate in the set calculation but are excluded from the
        // immediate-dominator tree used by optimizer traversals.
        let all_blocks = cfg.block_ids().clone();
        let mut dominators = BTreeMap::new();
        for block in cfg.block_ids() {
            if *block == entry {
                dominators.insert(*block, BTreeSet::from([*block]));
            } else {
                dominators.insert(*block, all_blocks.clone());
            }
        }

        let mut changed = true;
        while changed {
            changed = false;
            for block in cfg.block_ids() {
                if *block == entry {
                    continue;
                }
                let predecessors = cfg.predecessors(*block);
                let mut next = if predecessors.is_empty() {
                    BTreeSet::new()
                } else {
                    let mut iter = predecessors.iter();
                    let first = iter
                        .next()
                        .and_then(|pred| dominators.get(pred))
                        .cloned()
                        .unwrap_or_default();
                    iter.fold(first, |acc, pred| {
                        acc.intersection(dominators.get(pred).unwrap_or(&EMPTY_BLOCK_SET))
                            .copied()
                            .collect()
                    })
                };
                next.insert(*block);
                if dominators.get(block) != Some(&next) {
                    dominators.insert(*block, next);
                    changed = true;
                }
            }
        }

        let mut immediate_dominators = BTreeMap::new();
        let mut children = cfg
            .reachable()
            .iter()
            .copied()
            .map(|block| (block, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for block in cfg.reachable() {
            if *block == entry {
                continue;
            }
            let strict = dominators
                .get(block)
                .into_iter()
                .flatten()
                .copied()
                .filter(|dominator| dominator != block)
                .collect::<Vec<_>>();
            let immediate = strict.iter().copied().find(|candidate| {
                !strict.iter().copied().any(|other| {
                    other != *candidate
                        && dominators
                            .get(&other)
                            .is_some_and(|set| set.contains(candidate))
                })
            });
            if let Some(immediate) = immediate {
                immediate_dominators.insert(*block, immediate);
                children.entry(immediate).or_default().push(*block);
            }
        }
        for child_blocks in children.values_mut() {
            child_blocks.sort_unstable();
        }

        Self {
            dominators,
            immediate_dominators,
            children,
        }
    }

    pub(in crate::nir) fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        self.dominators
            .get(&block)
            .is_some_and(|set| set.contains(&dominator))
    }

    #[allow(dead_code)] // Used by dominance-scoped optimizer slices.
    pub(in crate::nir) fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        self.immediate_dominators.get(&block).copied()
    }

    #[allow(dead_code)] // Used by dominance-scoped optimizer slices.
    pub(in crate::nir) fn children(&self, block: BlockId) -> &[BlockId] {
        self.children.get(&block).map_or(&[], Vec::as_slice)
    }

    #[allow(dead_code)] // Used by loop-aware optimizer slices.
    pub(in crate::nir) fn is_backedge(&self, from: BlockId, to: BlockId) -> bool {
        self.dominates(to, from)
    }
}

static EMPTY_BLOCK_SET: BTreeSet<BlockId> = BTreeSet::new();

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
    }
}
