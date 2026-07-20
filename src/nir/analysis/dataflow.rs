#[cfg(test)]
use std::collections::BTreeSet;

use super::cfg::NirCfg;
use crate::analysis::dataflow::{
    DataflowProblem, DataflowResult, solve_dataflow as solve_shared_dataflow,
};
use crate::nir::BlockId;

pub(in crate::nir) use crate::analysis::dataflow::DataflowDirection as NirDataflowDirection;

/// A finite, monotone block data-flow problem.
///
/// `join` and `transfer` must be deterministic. The solver rebuilds the state
/// at a block from its boundary and adjacent blocks on every evaluation.
pub(in crate::nir) trait NirDataflowProblem {
    type State: Clone + Eq;

    fn direction(&self) -> NirDataflowDirection;
    fn bottom(&self) -> Self::State;
    fn boundary(&self, block: BlockId) -> Option<Self::State>;
    fn join(&self, into: &mut Self::State, other: &Self::State);
    fn transfer(&self, block: BlockId, state: &Self::State) -> Self::State;

    /// Whether a forward-flow fact may propagate from `from` to `to`.
    ///
    /// The default keeps every CFG edge executable. Sparse forward clients may
    /// override this when the source block's output proves that an edge cannot
    /// execute, for example after resolving a constant branch condition.
    fn forward_edge_is_executable(
        &self,
        _from: BlockId,
        _to: BlockId,
        _from_out: &Self::State,
    ) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::nir) struct NirDataflowResult<State> {
    shared: DataflowResult<BlockId, State>,
}

impl<State> NirDataflowResult<State> {
    pub(in crate::nir) fn in_state(&self, block: BlockId) -> Option<&State> {
        self.shared.in_state(block)
    }

    pub(in crate::nir) fn out_state(&self, block: BlockId) -> Option<&State> {
        self.shared.out_state(block)
    }

    pub(in crate::nir) fn evaluations(&self) -> usize {
        self.shared.evaluations()
    }
}

struct NirProblemAdapter<'a, Problem>(&'a Problem);

impl<Problem> DataflowProblem<NirCfg> for NirProblemAdapter<'_, Problem>
where
    Problem: NirDataflowProblem,
{
    type State = Problem::State;

    fn direction(&self) -> NirDataflowDirection {
        self.0.direction()
    }

    fn bottom(&self) -> Self::State {
        self.0.bottom()
    }

    fn boundary(&self, block: BlockId) -> Option<Self::State> {
        self.0.boundary(block)
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        self.0.join(into, other);
    }

    fn transfer(&self, block: BlockId, state: &Self::State) -> Self::State {
        self.0.transfer(block, state)
    }

    fn forward_edge_is_executable(
        &self,
        from: BlockId,
        to: BlockId,
        from_out: &Self::State,
    ) -> bool {
        self.0.forward_edge_is_executable(from, to, from_out)
    }
}

pub(in crate::nir) fn solve_dataflow<Problem>(
    cfg: &NirCfg,
    problem: &Problem,
) -> NirDataflowResult<Problem::State>
where
    Problem: NirDataflowProblem,
{
    NirDataflowResult {
        shared: solve_shared_dataflow(cfg, &NirProblemAdapter(problem)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{NirBlock, NirRoutine, NirTerminator, NirValue};

    struct ReachableHistory {
        entry: BlockId,
    }

    struct SparseReachableHistory {
        entry: BlockId,
        branch: BlockId,
        taken: BlockId,
    }

    impl NirDataflowProblem for ReachableHistory {
        type State = BTreeSet<BlockId>;

        fn direction(&self) -> NirDataflowDirection {
            NirDataflowDirection::Forward
        }

        fn bottom(&self) -> Self::State {
            BTreeSet::new()
        }

        fn boundary(&self, block: BlockId) -> Option<Self::State> {
            (block == self.entry).then(|| BTreeSet::from([block]))
        }

        fn join(&self, into: &mut Self::State, other: &Self::State) {
            into.extend(other);
        }

        fn transfer(&self, block: BlockId, state: &Self::State) -> Self::State {
            let mut state = state.clone();
            state.insert(block);
            state
        }
    }

    impl NirDataflowProblem for SparseReachableHistory {
        type State = Option<BTreeSet<BlockId>>;

        fn direction(&self) -> NirDataflowDirection {
            NirDataflowDirection::Forward
        }

        fn bottom(&self) -> Self::State {
            None
        }

        fn boundary(&self, block: BlockId) -> Option<Self::State> {
            (block == self.entry).then(|| Some(BTreeSet::new()))
        }

        fn join(&self, into: &mut Self::State, other: &Self::State) {
            let Some(other) = other else {
                return;
            };
            if let Some(into) = into {
                into.extend(other);
            } else {
                *into = Some(other.clone());
            }
        }

        fn transfer(&self, block: BlockId, state: &Self::State) -> Self::State {
            let mut state = state.clone()?;
            state.insert(block);
            Some(state)
        }

        fn forward_edge_is_executable(
            &self,
            from: BlockId,
            to: BlockId,
            from_out: &Self::State,
        ) -> bool {
            from_out.is_some() && (from != self.branch || to == self.taken)
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

    fn edge(target: u32) -> crate::nir::NirEdge {
        crate::nir::NirEdge {
            target: BlockId(target),
            args: Vec::new(),
        }
    }

    #[test]
    fn forward_solver_joins_diamond_predecessors_deterministically() {
        let routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
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
            ],
        };
        let cfg = NirCfg::from_routine(&routine);
        let result = solve_dataflow(&cfg, &ReachableHistory { entry: BlockId(0) });

        assert_eq!(
            result.in_state(BlockId(3)),
            Some(&BTreeSet::from([BlockId(0), BlockId(1), BlockId(2)]))
        );
        assert_eq!(
            result.out_state(BlockId(3)),
            Some(&BTreeSet::from([
                BlockId(0),
                BlockId(1),
                BlockId(2),
                BlockId(3)
            ]))
        );
        assert!(result.evaluations() >= routine.blocks.len());
    }

    #[test]
    fn forward_solver_excludes_non_executable_edges_from_joins() {
        let routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
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
            ],
        };
        let cfg = NirCfg::from_routine(&routine);
        let result = solve_dataflow(
            &cfg,
            &SparseReachableHistory {
                entry: BlockId(0),
                branch: BlockId(0),
                taken: BlockId(1),
            },
        );

        assert_eq!(result.in_state(BlockId(2)), Some(&None));
        assert_eq!(result.out_state(BlockId(2)), Some(&None));
        assert_eq!(
            result.in_state(BlockId(3)),
            Some(&Some(BTreeSet::from([BlockId(0), BlockId(1)])))
        );
    }

    #[test]
    fn sparse_forward_solver_converges_with_a_dead_loop_exit() {
        let routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                block(0, "entry", NirTerminator::Goto(edge(1))),
                block(
                    1,
                    "loop",
                    NirTerminator::Branch {
                        condition: NirValue::ConstU8(1),
                        then_edge: edge(2),
                        else_edge: edge(3),
                    },
                ),
                block(2, "body", NirTerminator::Goto(edge(1))),
                block(3, "exit", NirTerminator::Return(None)),
            ],
        };
        let cfg = NirCfg::from_routine(&routine);
        let result = solve_dataflow(
            &cfg,
            &SparseReachableHistory {
                entry: BlockId(0),
                branch: BlockId(1),
                taken: BlockId(2),
            },
        );

        assert_eq!(result.in_state(BlockId(3)), Some(&None));
        assert_eq!(
            result.out_state(BlockId(1)),
            Some(&Some(BTreeSet::from([BlockId(0), BlockId(1), BlockId(2),])))
        );
    }
}
