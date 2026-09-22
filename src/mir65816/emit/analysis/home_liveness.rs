//! Backward may-liveness on selected sites, using physical byte aliases.
use super::super::effects::Access;
use super::homes::{HomeAccess, HomeByte, Homes};
use super::sites::Node;
use crate::analysis::{
    dataflow::{DataflowDirection, DataflowProblem, DataflowResult, solve_dataflow},
    graph::DataflowGraph,
};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

pub(in crate::mir65816::emit) struct HomeLiveness {
    result: DataflowResult<Node, Rc<BTreeSet<HomeByte>>>,
}
impl HomeLiveness {
    pub fn analyze(graph: &impl DataflowGraph<Node = Node>, homes: &Homes) -> Self {
        Self {
            result: solve_dataflow(graph, &Problem(&homes.accesses)),
        }
    }
    pub fn before(&self, node: Node) -> Result<&BTreeSet<HomeByte>, String> {
        self.result
            .in_state(node)
            .map(Rc::as_ref)
            .ok_or_else(|| "unreachable home liveness site".into())
    }
    pub fn after(&self, node: Node) -> Result<&BTreeSet<HomeByte>, String> {
        self.result
            .out_state(node)
            .map(Rc::as_ref)
            .ok_or_else(|| "unreachable home liveness site".into())
    }
}
struct Problem<'a>(&'a BTreeMap<Node, Vec<HomeAccess>>);
impl<G: DataflowGraph<Node = Node>> DataflowProblem<G> for Problem<'_> {
    type State = Rc<BTreeSet<HomeByte>>;
    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Backward
    }
    fn bottom(&self) -> Self::State {
        Rc::new(BTreeSet::new())
    }
    fn boundary(&self, _: Node) -> Option<Self::State> {
        None
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        if into.is_empty() {
            *into = other.clone();
        } else if !other.is_subset(into) {
            Rc::make_mut(into).extend(other.iter().copied());
        }
    }
    fn transfer(&self, node: Node, state: &Self::State) -> Self::State {
        let mut live = state.clone();
        // Reverse sub-effect order matters: a transfer-frame write followed by
        // its internal read does not read the caller's old bytes at node entry.
        for effect in self.0[&node].iter().rev() {
            match effect.access {
                Access::Read if !effect.homes.is_subset(&live) => {
                    Rc::make_mut(&mut live).extend(&effect.homes)
                }
                Access::Read => {}
                Access::Write if !effect.uncertain => {
                    if !live.is_disjoint(&effect.homes) {
                        Rc::make_mut(&mut live).retain(|home| !effect.homes.contains(home));
                    }
                }
                Access::Write | Access::MayWrite => {}
            }
        }
        live
    }
}
