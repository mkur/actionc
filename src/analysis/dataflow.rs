use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::graph::DataflowGraph;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataflowDirection {
    Forward,
    Backward,
}

/// A finite, monotone block data-flow problem over a stable graph snapshot.
///
/// `join` and `transfer` must be deterministic. The solver rebuilds the state
/// at a node from its boundary and adjacent nodes on every evaluation.
pub(crate) trait DataflowProblem<Graph>
where
    Graph: DataflowGraph,
{
    type State: Clone + Eq;

    fn direction(&self) -> DataflowDirection;
    fn bottom(&self) -> Self::State;
    fn boundary(&self, node: Graph::Node) -> Option<Self::State>;
    fn join(&self, into: &mut Self::State, other: &Self::State);
    fn transfer(&self, node: Graph::Node, state: &Self::State) -> Self::State;

    /// Whether a forward-flow fact may propagate from `from` to `to`.
    ///
    /// Backward safety analyses always follow every structurally reachable
    /// edge. Sparse executable-edge filtering is intentionally forward-only.
    fn forward_edge_is_executable(
        &self,
        _from: Graph::Node,
        _to: Graph::Node,
        _from_out: &Self::State,
    ) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataflowResult<Node, State> {
    in_states: BTreeMap<Node, State>,
    out_states: BTreeMap<Node, State>,
    evaluations: usize,
}

impl<Node, State> DataflowResult<Node, State>
where
    Node: Ord,
{
    pub(crate) fn in_state(&self, node: Node) -> Option<&State> {
        self.in_states.get(&node)
    }

    pub(crate) fn out_state(&self, node: Node) -> Option<&State> {
        self.out_states.get(&node)
    }

    pub(crate) fn evaluations(&self) -> usize {
        self.evaluations
    }
}

pub(crate) fn solve_dataflow<Graph, Problem>(
    graph: &Graph,
    problem: &Problem,
) -> DataflowResult<Graph::Node, Problem::State>
where
    Graph: DataflowGraph,
    Problem: DataflowProblem<Graph>,
{
    let direction = problem.direction();
    let order = match direction {
        DataflowDirection::Forward => graph.reverse_postorder(),
        DataflowDirection::Backward => graph.postorder(),
    };
    let mut in_states = order
        .iter()
        .copied()
        .map(|node| (node, problem.bottom()))
        .collect::<BTreeMap<_, _>>();
    let mut out_states = order
        .iter()
        .copied()
        .map(|node| (node, problem.bottom()))
        .collect::<BTreeMap<_, _>>();
    let mut worklist = order.iter().copied().collect::<VecDeque<_>>();
    let mut queued = order.iter().copied().collect::<BTreeSet<_>>();
    let mut evaluations = 0usize;

    while let Some(node) = worklist.pop_front() {
        queued.remove(&node);
        evaluations = evaluations.saturating_add(1);

        let (next_in, next_out) = match direction {
            DataflowDirection::Forward => {
                let mut input = problem.bottom();
                if let Some(boundary) = problem.boundary(node) {
                    problem.join(&mut input, &boundary);
                }
                for predecessor in graph.predecessors(node) {
                    if let Some(state) = out_states.get(predecessor) {
                        if problem.forward_edge_is_executable(*predecessor, node, state) {
                            problem.join(&mut input, state);
                        }
                    }
                }
                let output = problem.transfer(node, &input);
                (input, output)
            }
            DataflowDirection::Backward => {
                let mut output = problem.bottom();
                if let Some(boundary) = problem.boundary(node) {
                    problem.join(&mut output, &boundary);
                }
                for successor in graph.successors(node) {
                    if let Some(state) = in_states.get(successor) {
                        problem.join(&mut output, state);
                    }
                }
                let input = problem.transfer(node, &output);
                (input, output)
            }
        };

        let input_changed = in_states.get(&node) != Some(&next_in);
        let output_changed = out_states.get(&node) != Some(&next_out);
        if input_changed {
            in_states.insert(node, next_in);
        }
        if output_changed {
            out_states.insert(node, next_out);
        }

        let propagates = match direction {
            DataflowDirection::Forward => output_changed,
            DataflowDirection::Backward => input_changed,
        };
        if !propagates {
            continue;
        }
        let adjacent = match direction {
            DataflowDirection::Forward => graph.successors(node),
            DataflowDirection::Backward => graph.predecessors(node),
        };
        for adjacent in adjacent {
            if graph.reachable().contains(adjacent) && queued.insert(*adjacent) {
                worklist.push_back(*adjacent);
            }
        }
    }

    DataflowResult {
        in_states,
        out_states,
        evaluations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestGraph {
        entry: Option<u8>,
        nodes: BTreeSet<u8>,
        predecessors: BTreeMap<u8, BTreeSet<u8>>,
        successors: BTreeMap<u8, BTreeSet<u8>>,
        reachable: BTreeSet<u8>,
        postorder: Vec<u8>,
        reverse_postorder: Vec<u8>,
    }

    impl TestGraph {
        fn new(nodes: &[u8], edges: &[(u8, u8)], entry: u8) -> Self {
            let nodes = nodes.iter().copied().collect::<BTreeSet<_>>();
            let mut predecessors = nodes
                .iter()
                .copied()
                .map(|node| (node, BTreeSet::new()))
                .collect::<BTreeMap<_, _>>();
            let mut successors = predecessors.clone();
            for &(from, to) in edges {
                successors.get_mut(&from).unwrap().insert(to);
                predecessors.get_mut(&to).unwrap().insert(from);
            }
            let mut reachable = BTreeSet::new();
            let mut postorder = Vec::new();
            fn visit(
                node: u8,
                successors: &BTreeMap<u8, BTreeSet<u8>>,
                reachable: &mut BTreeSet<u8>,
                postorder: &mut Vec<u8>,
            ) {
                if !reachable.insert(node) {
                    return;
                }
                for successor in &successors[&node] {
                    visit(*successor, successors, reachable, postorder);
                }
                postorder.push(node);
            }
            visit(entry, &successors, &mut reachable, &mut postorder);
            let reverse_postorder = postorder.iter().rev().copied().collect();
            Self {
                entry: Some(entry),
                nodes,
                predecessors,
                successors,
                reachable,
                postorder,
                reverse_postorder,
            }
        }
    }

    impl DataflowGraph for TestGraph {
        type Node = u8;

        fn entry(&self) -> Option<Self::Node> {
            self.entry
        }

        fn nodes(&self) -> &BTreeSet<Self::Node> {
            &self.nodes
        }

        fn predecessors(&self, node: Self::Node) -> &BTreeSet<Self::Node> {
            &self.predecessors[&node]
        }

        fn successors(&self, node: Self::Node) -> &BTreeSet<Self::Node> {
            &self.successors[&node]
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

    struct History {
        entry: u8,
        direction: DataflowDirection,
    }

    impl DataflowProblem<TestGraph> for History {
        type State = BTreeSet<u8>;

        fn direction(&self) -> DataflowDirection {
            self.direction
        }

        fn bottom(&self) -> Self::State {
            BTreeSet::new()
        }

        fn boundary(&self, node: u8) -> Option<Self::State> {
            (node == self.entry).then(|| BTreeSet::from([node]))
        }

        fn join(&self, into: &mut Self::State, other: &Self::State) {
            into.extend(other);
        }

        fn transfer(&self, node: u8, state: &Self::State) -> Self::State {
            let mut state = state.clone();
            state.insert(node);
            state
        }
    }

    struct SparseHistory {
        branch: u8,
        taken: u8,
    }

    impl DataflowProblem<TestGraph> for SparseHistory {
        type State = Option<BTreeSet<u8>>;

        fn direction(&self) -> DataflowDirection {
            DataflowDirection::Forward
        }

        fn bottom(&self) -> Self::State {
            None
        }

        fn boundary(&self, node: u8) -> Option<Self::State> {
            (node == 0).then(|| Some(BTreeSet::new()))
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

        fn transfer(&self, node: u8, state: &Self::State) -> Self::State {
            let mut state = state.clone()?;
            state.insert(node);
            Some(state)
        }

        fn forward_edge_is_executable(&self, from: u8, to: u8, from_out: &Self::State) -> bool {
            from_out.is_some() && (from != self.branch || to == self.taken)
        }
    }

    #[test]
    fn forward_solver_joins_diamond_predecessors_deterministically() {
        let graph = TestGraph::new(&[0, 1, 2, 3, 9], &[(0, 1), (0, 2), (1, 3), (2, 3)], 0);
        let result = solve_dataflow(
            &graph,
            &History {
                entry: 0,
                direction: DataflowDirection::Forward,
            },
        );

        assert_eq!(result.in_state(3), Some(&BTreeSet::from([0, 1, 2])));
        assert_eq!(result.out_state(3), Some(&BTreeSet::from([0, 1, 2, 3])));
        assert_eq!(result.in_state(9), None);
        assert!(result.evaluations() >= graph.reachable.len());
    }

    #[test]
    fn sparse_forward_solver_excludes_dead_edge_and_converges_through_loop() {
        let graph = TestGraph::new(&[0, 1, 2, 3], &[(0, 1), (1, 2), (1, 3), (2, 1)], 0);
        let result = solve_dataflow(
            &graph,
            &SparseHistory {
                branch: 1,
                taken: 2,
            },
        );

        assert_eq!(result.in_state(3), Some(&None));
        assert_eq!(result.out_state(1), Some(&Some(BTreeSet::from([0, 1, 2]))));
    }

    #[test]
    fn backward_solver_joins_multiple_exits() {
        let graph = TestGraph::new(&[0, 1, 2], &[(0, 1), (0, 2)], 0);
        let problem = History {
            entry: 2,
            direction: DataflowDirection::Backward,
        };
        let result = solve_dataflow(&graph, &problem);

        assert_eq!(result.out_state(0), Some(&BTreeSet::from([1, 2])));
        assert_eq!(result.in_state(0), Some(&BTreeSet::from([0, 1, 2])));
    }
}
