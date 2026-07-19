use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::cfg::NirCfg;
use crate::nir::BlockId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::nir) enum NirDataflowDirection {
    #[allow(dead_code)] // Reserved for the first forward routine analysis client.
    Forward,
    Backward,
}

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::nir) struct NirDataflowResult<State> {
    in_states: BTreeMap<BlockId, State>,
    out_states: BTreeMap<BlockId, State>,
    evaluations: usize,
}

impl<State> NirDataflowResult<State> {
    pub(in crate::nir) fn in_state(&self, block: BlockId) -> Option<&State> {
        self.in_states.get(&block)
    }

    pub(in crate::nir) fn out_state(&self, block: BlockId) -> Option<&State> {
        self.out_states.get(&block)
    }

    pub(in crate::nir) fn evaluations(&self) -> usize {
        self.evaluations
    }
}

pub(in crate::nir) fn solve_dataflow<Problem>(
    cfg: &NirCfg,
    problem: &Problem,
) -> NirDataflowResult<Problem::State>
where
    Problem: NirDataflowProblem,
{
    let direction = problem.direction();
    let order = match direction {
        NirDataflowDirection::Forward => cfg.reverse_postorder(),
        NirDataflowDirection::Backward => cfg.postorder(),
    };
    let mut in_states = order
        .iter()
        .copied()
        .map(|block| (block, problem.bottom()))
        .collect::<BTreeMap<_, _>>();
    let mut out_states = order
        .iter()
        .copied()
        .map(|block| (block, problem.bottom()))
        .collect::<BTreeMap<_, _>>();
    let mut worklist = order.iter().copied().collect::<VecDeque<_>>();
    let mut queued = order.iter().copied().collect::<BTreeSet<_>>();
    let mut evaluations = 0usize;

    while let Some(block) = worklist.pop_front() {
        queued.remove(&block);
        evaluations = evaluations.saturating_add(1);

        let (next_in, next_out) = match direction {
            NirDataflowDirection::Forward => {
                let mut input = problem.bottom();
                if let Some(boundary) = problem.boundary(block) {
                    problem.join(&mut input, &boundary);
                }
                for predecessor in cfg.predecessors(block) {
                    if let Some(state) = out_states.get(predecessor) {
                        problem.join(&mut input, state);
                    }
                }
                let output = problem.transfer(block, &input);
                (input, output)
            }
            NirDataflowDirection::Backward => {
                let mut output = problem.bottom();
                if let Some(boundary) = problem.boundary(block) {
                    problem.join(&mut output, &boundary);
                }
                for successor in cfg.successors(block) {
                    if let Some(state) = in_states.get(successor) {
                        problem.join(&mut output, state);
                    }
                }
                let input = problem.transfer(block, &output);
                (input, output)
            }
        };

        let input_changed = in_states.get(&block) != Some(&next_in);
        let output_changed = out_states.get(&block) != Some(&next_out);
        if input_changed {
            in_states.insert(block, next_in);
        }
        if output_changed {
            out_states.insert(block, next_out);
        }

        let propagates = match direction {
            NirDataflowDirection::Forward => output_changed,
            NirDataflowDirection::Backward => input_changed,
        };
        if !propagates {
            continue;
        }
        let adjacent = match direction {
            NirDataflowDirection::Forward => cfg.successors(block),
            NirDataflowDirection::Backward => cfg.predecessors(block),
        };
        for adjacent in adjacent {
            if cfg.reachable().contains(adjacent) && queued.insert(*adjacent) {
                worklist.push_back(*adjacent);
            }
        }
    }

    NirDataflowResult {
        in_states,
        out_states,
        evaluations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{NirBlock, NirRoutine, NirTerminator, NirValue};

    struct ReachableHistory {
        entry: BlockId,
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

    fn block(id: u32, label: &str, terminator: NirTerminator) -> NirBlock {
        NirBlock {
            id: BlockId(id),
            label: label.to_string(),
            ops: Vec::new(),
            terminator,
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
                        then_label: "left".to_string(),
                        else_label: "right".to_string(),
                    },
                ),
                block(1, "left", NirTerminator::Goto("join".to_string())),
                block(2, "right", NirTerminator::Goto("join".to_string())),
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
}
