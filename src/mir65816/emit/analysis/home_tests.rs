//! Hand-authored graphs and access sets: deliberately independent of instruction
//! classification, emission and the tracker. Shared by the home analyses.
use super::super::effects::Access;
use super::home_liveness::HomeLiveness;
use super::homes::{HomeAccess, HomeByte, Homes};
use super::sites::Node;
use crate::analysis::graph::DataflowGraph;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const LOW: HomeByte = HomeByte::Stack(-1);
pub(super) const HIGH: HomeByte = HomeByte::Stack(0);
pub(super) const DP: HomeByte = HomeByte::DirectPage(0);
pub(super) fn access(access: Access, homes: &[HomeByte]) -> HomeAccess {
    HomeAccess {
        access,
        homes: homes.iter().copied().collect(),
        uncertain: false,
    }
}
pub(super) fn homes(transfers: Vec<Vec<HomeAccess>>) -> Homes {
    Homes {
        info: BTreeMap::new(),
        accesses: transfers
            .into_iter()
            .enumerate()
            .map(|(i, a)| (Node(i), a))
            .collect(),
    }
}
pub(super) struct Graph {
    nodes: BTreeSet<Node>,
    pred: BTreeMap<Node, BTreeSet<Node>>,
    succ: BTreeMap<Node, BTreeSet<Node>>,
    reach: BTreeSet<Node>,
    post: Vec<Node>,
    reverse: Vec<Node>,
}
impl Graph {
    pub(super) fn new(count: usize, edges: &[(usize, usize)]) -> Self {
        let nodes: BTreeSet<_> = (0..count).map(Node).collect();
        let mut pred: BTreeMap<_, BTreeSet<_>> =
            nodes.iter().map(|&n| (n, BTreeSet::new())).collect();
        let mut succ = pred.clone();
        for &(a, b) in edges {
            succ.get_mut(&Node(a)).unwrap().insert(Node(b));
            pred.get_mut(&Node(b)).unwrap().insert(Node(a));
        }
        fn visit(
            n: Node,
            succ: &BTreeMap<Node, BTreeSet<Node>>,
            reach: &mut BTreeSet<Node>,
            post: &mut Vec<Node>,
        ) {
            if reach.insert(n) {
                for &s in &succ[&n] {
                    visit(s, succ, reach, post);
                }
                post.push(n);
            }
        }
        let mut reach = BTreeSet::new();
        let mut post = Vec::new();
        visit(Node(0), &succ, &mut reach, &mut post);
        let reverse = post.iter().rev().copied().collect();
        Self {
            nodes,
            pred,
            succ,
            reach,
            post,
            reverse,
        }
    }
}
impl DataflowGraph for Graph {
    type Node = Node;
    fn entry(&self) -> Option<Node> {
        Some(Node(0))
    }
    fn nodes(&self) -> &BTreeSet<Node> {
        &self.nodes
    }
    fn predecessors(&self, n: Node) -> &BTreeSet<Node> {
        &self.pred[&n]
    }
    fn successors(&self, n: Node) -> &BTreeSet<Node> {
        &self.succ[&n]
    }
    fn reachable(&self) -> &BTreeSet<Node> {
        &self.reach
    }
    fn postorder(&self) -> &[Node] {
        &self.post
    }
    fn reverse_postorder(&self) -> &[Node] {
        &self.reverse
    }
}
#[test]
fn diamond_read_on_one_branch_and_overwrites_on_every_branch() {
    let graph = Graph::new(4, &[(0, 1), (0, 2), (1, 3), (2, 3)]);
    for reads in [false, true] {
        let mut right = vec![access(Access::Write, &[LOW, HIGH])];
        if reads {
            right.insert(0, access(Access::Read, &[HIGH]));
        }
        let h = homes(vec![
            vec![],
            vec![access(Access::Write, &[LOW, HIGH])],
            right,
            vec![access(Access::Read, &[LOW, HIGH])],
        ]);
        let live = HomeLiveness::analyze(&graph, &h);
        assert_eq!(
            live.before(Node(0)).unwrap(),
            &if reads {
                [HIGH].into()
            } else {
                BTreeSet::new()
            }
        );
        assert_eq!(live.after(Node(1)).unwrap(), &[LOW, HIGH].into());
    }
}
#[test]
fn loop_reads_before_store_survive_backedge_and_unreachable_reads_do_not() {
    let graph = Graph::new(5, &[(0, 1), (1, 2), (2, 1), (2, 3)]);
    let h = homes(vec![
        vec![],
        vec![access(Access::Read, &[LOW])],
        vec![access(Access::Write, &[LOW])],
        vec![],
        vec![access(Access::Read, &[DP])],
    ]);
    let live = HomeLiveness::analyze(&graph, &h);
    assert_eq!(live.before(Node(0)).unwrap(), &[LOW].into());
    assert_eq!(live.after(Node(2)).unwrap(), &[LOW].into());
    assert!(live.before(Node(4)).is_err());
}
#[test]
fn partial_word_write_and_dp_are_distinct_physical_bytes() {
    let graph = Graph::new(2, &[(0, 1)]);
    let h = homes(vec![
        vec![access(Access::Write, &[LOW])],
        vec![access(Access::Read, &[LOW, HIGH, DP])],
    ]);
    assert_eq!(
        HomeLiveness::analyze(&graph, &h).before(Node(0)).unwrap(),
        &[HIGH, DP].into()
    );
}
#[test]
fn ordered_call_reads_arguments_before_clobbers_but_not_old_transfer_bytes() {
    let graph = Graph::new(1, &[]);
    let h = homes(vec![vec![
        access(Access::Write, &[LOW]), // JSL creates its return address.
        access(Access::Read, &[HIGH]), // Logical argument before any clobber.
        access(Access::MayWrite, &[HIGH, DP]),
        access(Access::Read, &[LOW]), // RTL reads the newly created bytes.
    ]]);
    assert_eq!(
        HomeLiveness::analyze(&graph, &h).before(Node(0)).unwrap(),
        &[HIGH].into()
    );
}
#[test]
fn possible_writes_never_kill_and_unknown_reads_expand_aliases() {
    let graph = Graph::new(2, &[(0, 1)]);
    let mut read = access(Access::Read, &[LOW, HIGH, DP]);
    read.uncertain = true;
    let mut write = access(Access::Write, &[LOW, HIGH, DP]);
    write.uncertain = true;
    let h = homes(vec![
        vec![write, access(Access::MayWrite, &[LOW])],
        vec![read],
    ]);
    assert_eq!(
        HomeLiveness::analyze(&graph, &h).before(Node(0)).unwrap(),
        &[LOW, HIGH, DP].into()
    );
}
#[test]
fn rmw_reads_old_value_even_when_later_overwritten() {
    let graph = Graph::new(1, &[]);
    let h = homes(vec![vec![
        access(Access::Read, &[DP]),
        access(Access::Write, &[DP]),
    ]]);
    assert_eq!(
        HomeLiveness::analyze(&graph, &h).before(Node(0)).unwrap(),
        &[DP].into()
    );
}
