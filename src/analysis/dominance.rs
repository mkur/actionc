use std::collections::{BTreeMap, BTreeSet};

use super::graph::DataflowGraph;

/// Dominance facts derived from one immutable graph snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dominance<Node> {
    root: Option<Node>,
    dominators: BTreeMap<Node, BTreeSet<Node>>,
    immediate_dominators: BTreeMap<Node, Node>,
    children: BTreeMap<Node, Vec<Node>>,
    frontiers: BTreeMap<Node, BTreeSet<Node>>,
    empty_node_set: BTreeSet<Node>,
}

impl<Node> Dominance<Node>
where
    Node: Copy + Ord,
{
    pub(crate) fn from_graph<Graph>(graph: &Graph) -> Self
    where
        Graph: DataflowGraph<Node = Node>,
    {
        let Some(entry) = graph.entry() else {
            return Self {
                root: None,
                dominators: BTreeMap::new(),
                immediate_dominators: BTreeMap::new(),
                children: BTreeMap::new(),
                frontiers: BTreeMap::new(),
                empty_node_set: BTreeSet::new(),
            };
        };

        // Preserve the existing compiler policy for unreachable nodes: they
        // participate in the set calculation but are excluded from the
        // immediate-dominator tree used by optimizer traversals.
        let all_nodes = graph.nodes().clone();
        let mut dominators = BTreeMap::new();
        for node in graph.nodes() {
            if *node == entry {
                dominators.insert(*node, BTreeSet::from([*node]));
            } else {
                dominators.insert(*node, all_nodes.clone());
            }
        }

        let empty_node_set = BTreeSet::new();
        let mut changed = true;
        while changed {
            changed = false;
            for node in graph.nodes() {
                if *node == entry {
                    continue;
                }
                let predecessors = graph.predecessors(*node);
                let mut next = if predecessors.is_empty() {
                    BTreeSet::new()
                } else {
                    let mut iter = predecessors.iter();
                    let first = iter
                        .next()
                        .and_then(|predecessor| dominators.get(predecessor))
                        .cloned()
                        .unwrap_or_default();
                    iter.fold(first, |acc, predecessor| {
                        acc.intersection(dominators.get(predecessor).unwrap_or(&empty_node_set))
                            .copied()
                            .collect()
                    })
                };
                next.insert(*node);
                if dominators.get(node) != Some(&next) {
                    dominators.insert(*node, next);
                    changed = true;
                }
            }
        }

        let mut immediate_dominators = BTreeMap::new();
        let mut children = graph
            .reachable()
            .iter()
            .copied()
            .map(|node| (node, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for node in graph.reachable() {
            if *node == entry {
                continue;
            }
            let strict = dominators
                .get(node)
                .into_iter()
                .flatten()
                .copied()
                .filter(|dominator| dominator != node)
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
                immediate_dominators.insert(*node, immediate);
                children.entry(immediate).or_default().push(*node);
            }
        }
        for child_nodes in children.values_mut() {
            child_nodes.sort_unstable();
        }

        let mut frontiers = graph
            .reachable()
            .iter()
            .copied()
            .map(|node| (node, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for node in graph.reachable() {
            if graph.predecessors(*node).len() < 2 {
                continue;
            }
            let stop = immediate_dominators.get(node).copied();
            for predecessor in graph.predecessors(*node) {
                let mut runner = Some(*predecessor);
                while runner.is_some() && runner != stop {
                    let current = runner.expect("dominance-frontier runner");
                    frontiers.entry(current).or_default().insert(*node);
                    runner = immediate_dominators.get(&current).copied();
                }
            }
        }

        Self {
            root: Some(entry),
            dominators,
            immediate_dominators,
            children,
            frontiers,
            empty_node_set,
        }
    }

    pub(crate) fn dominates(&self, dominator: Node, node: Node) -> bool {
        self.dominators
            .get(&node)
            .is_some_and(|set| set.contains(&dominator))
    }

    pub(crate) fn root(&self) -> Option<Node> {
        self.root
    }

    pub(crate) fn immediate_dominator(&self, node: Node) -> Option<Node> {
        self.immediate_dominators.get(&node).copied()
    }

    pub(crate) fn children(&self, node: Node) -> &[Node] {
        self.children.get(&node).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn dominance_frontier(&self, node: Node) -> &BTreeSet<Node> {
        self.frontiers.get(&node).unwrap_or(&self.empty_node_set)
    }

    pub(crate) fn pruned_iterated_frontier(
        &self,
        definitions: &BTreeSet<Node>,
        live_in: &BTreeSet<Node>,
    ) -> BTreeSet<Node> {
        let mut result = BTreeSet::new();
        let mut work = definitions.clone();
        while let Some(node) = work.pop_first() {
            for frontier in self.dominance_frontier(node) {
                if !live_in.contains(frontier) || !result.insert(*frontier) {
                    continue;
                }
                if !definitions.contains(frontier) {
                    work.insert(*frontier);
                }
            }
        }
        result
    }

    pub(crate) fn is_backedge(&self, from: Node, to: Node) -> bool {
        self.dominates(to, from)
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
        fn new(nodes: &[u8], edges: &[(u8, u8)], entry: Option<u8>) -> Self {
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
            if let Some(entry) = entry {
                visit(entry, &successors, &mut reachable, &mut postorder);
            }
            let reverse_postorder = postorder.iter().rev().copied().collect();
            Self {
                entry,
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

    #[test]
    fn computes_diamond_tree_frontier_and_unreachable_policy() {
        let graph = TestGraph::new(&[0, 1, 2, 3, 9], &[(0, 1), (0, 2), (1, 3), (2, 3)], Some(0));
        let dominance = Dominance::from_graph(&graph);

        assert_eq!(dominance.root(), Some(0));
        assert!(dominance.dominates(0, 3));
        assert!(!dominance.dominates(1, 3));
        assert_eq!(dominance.immediate_dominator(3), Some(0));
        assert_eq!(dominance.children(0), &[1, 2, 3]);
        assert_eq!(dominance.dominance_frontier(1), &BTreeSet::from([3]));
        assert_eq!(dominance.immediate_dominator(9), None);
        assert_eq!(dominance.children(9), &[]);
    }

    #[test]
    fn identifies_loop_backedges_and_pruned_iterated_frontiers() {
        let graph = TestGraph::new(&[0, 1, 2, 3], &[(0, 1), (1, 2), (1, 3), (2, 1)], Some(0));
        let dominance = Dominance::from_graph(&graph);

        assert!(dominance.is_backedge(2, 1));
        assert!(!dominance.is_backedge(1, 2));
        assert_eq!(
            dominance.pruned_iterated_frontier(&BTreeSet::from([2]), &BTreeSet::from([1]),),
            BTreeSet::from([1])
        );
    }

    #[test]
    fn empty_graph_has_no_root_or_facts() {
        let graph = TestGraph::new(&[], &[], None);
        let dominance = Dominance::from_graph(&graph);

        assert_eq!(dominance.root(), None);
        assert!(!dominance.dominates(0, 0));
        assert!(dominance.dominance_frontier(0).is_empty());
    }
}
