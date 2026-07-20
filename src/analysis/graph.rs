use std::collections::BTreeSet;

/// Immutable directed graph facts consumed by routine-level analyses.
///
/// Nodes and adjacency sets must have stable identities and deterministic
/// ordering for the lifetime of the graph snapshot. The shared analysis layer
/// deliberately knows nothing about NIR, MIR, temps, storage, or machines.
pub(crate) trait DataflowGraph {
    type Node: Copy + Ord;

    fn entry(&self) -> Option<Self::Node>;
    fn nodes(&self) -> &BTreeSet<Self::Node>;
    fn predecessors(&self, node: Self::Node) -> &BTreeSet<Self::Node>;
    fn successors(&self, node: Self::Node) -> &BTreeSet<Self::Node>;
    fn reachable(&self) -> &BTreeSet<Self::Node>;
    fn postorder(&self) -> &[Self::Node];
    fn reverse_postorder(&self) -> &[Self::Node];
}
