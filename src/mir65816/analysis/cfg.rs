use super::operands;
use crate::analysis::graph::DataflowGraph;
use crate::mir65816::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub(super) struct Cfg {
    entry: BlockId,
    pub indices: BTreeMap<BlockId, usize>,
    predecessors: BTreeMap<BlockId, BTreeSet<BlockId>>,
    successors: BTreeMap<BlockId, BTreeSet<BlockId>>,
    reachable: BTreeSet<BlockId>,
    postorder: Vec<BlockId>,
    reverse_postorder: Vec<BlockId>,
    pub cyclic: BTreeSet<BlockId>,
}

impl Cfg {
    pub fn build(routine: &Mir65816Routine) -> Result<Self, String> {
        let entry = routine
            .blocks
            .first()
            .ok_or("logical analysis requires a body")?
            .id;
        let indices: BTreeMap<_, _> = routine
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id, i))
            .collect();
        if indices.len() != routine.blocks.len() {
            return Err("duplicate block identity".into());
        }
        if !routine.blocks[0].params.is_empty() {
            return Err("routine entry cannot require edge arguments".into());
        }
        let nodes: BTreeSet<_> = indices.keys().copied().collect();
        let mut successors: BTreeMap<_, BTreeSet<_>> =
            nodes.iter().map(|&id| (id, BTreeSet::new())).collect();
        let mut predecessors = successors.clone();
        for (index, block) in routine.blocks.iter().enumerate() {
            for target in operands::successors(routine, index)? {
                predecessors
                    .get_mut(&target)
                    .ok_or("unknown edge target")?
                    .insert(block.id);
                successors.get_mut(&block.id).unwrap().insert(target);
            }
        }
        let mut reachable = BTreeSet::new();
        let mut postorder = Vec::new();
        visit(entry, &successors, &mut reachable, &mut postorder);
        let reverse_postorder = postorder.iter().rev().copied().collect();

        // SCCs include irreducible cycles and unreachable cycles. A static write
        // site inside any cycle cannot name one dynamic contents version.
        let mut visited = BTreeSet::new();
        let mut order = Vec::new();
        for &node in &nodes {
            visit(node, &successors, &mut visited, &mut order);
        }
        visited.clear();
        let mut cyclic = BTreeSet::new();
        for node in order.into_iter().rev() {
            if visited.contains(&node) {
                continue;
            }
            let mut component = Vec::new();
            visit(node, &predecessors, &mut visited, &mut component);
            if component.len() > 1 || successors[&node].contains(&node) {
                cyclic.extend(component);
            }
        }
        for adjacent in predecessors.values_mut() {
            adjacent.retain(|id| reachable.contains(id));
        }
        Ok(Self {
            entry,
            indices,
            predecessors,
            successors,
            reachable,
            postorder,
            reverse_postorder,
            cyclic,
        })
    }
}

fn visit(
    start: BlockId,
    edges: &BTreeMap<BlockId, BTreeSet<BlockId>>,
    visited: &mut BTreeSet<BlockId>,
    order: &mut Vec<BlockId>,
) {
    let mut pending = vec![(start, false)];
    while let Some((node, finished)) = pending.pop() {
        if finished {
            order.push(node);
        } else if visited.insert(node) {
            pending.push((node, true));
            pending.extend(edges[&node].iter().rev().map(|&n| (n, false)));
        }
    }
}

impl DataflowGraph for Cfg {
    type Node = BlockId;
    fn entry(&self) -> Option<BlockId> {
        Some(self.entry)
    }
    fn nodes(&self) -> &BTreeSet<BlockId> {
        &self.reachable
    }
    fn predecessors(&self, node: BlockId) -> &BTreeSet<BlockId> {
        &self.predecessors[&node]
    }
    fn successors(&self, node: BlockId) -> &BTreeSet<BlockId> {
        &self.successors[&node]
    }
    fn reachable(&self) -> &BTreeSet<BlockId> {
        &self.reachable
    }
    fn postorder(&self) -> &[BlockId] {
        &self.postorder
    }
    fn reverse_postorder(&self) -> &[BlockId] {
        &self.reverse_postorder
    }
}
