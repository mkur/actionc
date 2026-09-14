//! Bounded allocation of values that cross blocks or calls. Scratch registers
//! and source-visible storage retain their existing materialization contracts.
use super::{
    analysis::{branch_compare, edges, op_values, term_values, use_counts},
    *,
};
use crate::analysis::{
    dataflow::{DataflowDirection, DataflowProblem, solve_dataflow},
    dominance::Dominance,
    graph::DataflowGraph,
};
use std::collections::{BTreeMap, BTreeSet};

const POOL: [u8; 4] = [4, 5, 6, 7];
type Temps = BTreeSet<TempId>;

#[derive(Debug, Default)]
pub(super) struct Allocation {
    pub registers: BTreeMap<TempId, u8>,
    pub discarded: Temps,
}

pub(super) fn allocate(routine: &Mir68kRoutine, control_flow: bool) -> Result<Allocation, String> {
    let cfg = Cfg::new(routine)?;
    let dominance = Dominance::from_graph(&cfg);
    verify_uses(&cfg, &dominance)?;
    let live = solve_dataflow(&cfg, &Liveness { cfg: &cfg });
    let counts = use_counts(routine);
    let mut discarded: Temps = routine
        .temps
        .iter()
        .filter(|(id, _)| !counts.contains_key(id))
        .map(|(id, _)| *id)
        .collect();
    if control_flow {
        for block in &routine.blocks {
            if let Some(Mir68kOp::Compare { dest, .. }) = branch_compare(block, &counts) {
                discarded.insert(*dest);
            }
        }
    }
    let weights = loop_weights(&cfg, &dominance);
    let mut scores: BTreeMap<TempId, u64> = BTreeMap::new();
    let mut across = Temps::new();
    let mut conflicts: BTreeMap<TempId, Temps> = routine
        .temps
        .iter()
        .map(|(id, _)| (*id, Temps::new()))
        .collect();
    let mut affinity: BTreeMap<TempId, Temps> = BTreeMap::new();
    for &id in &cfg.reachable {
        let block = cfg.blocks[&id];
        let weight = weights.get(&id).copied().unwrap_or(1);
        across.extend(live.in_state(id).into_iter().flatten().copied());
        for value in block
            .ops
            .iter()
            .flat_map(op_values)
            .chain(term_values(&block.terminator))
        {
            if let Mir68kValue::Temp(temp, _) = value {
                *scores.entry(*temp).or_default() += weight;
            }
        }
        for edge in edges(&block.terminator) {
            for ((dest, _), value) in cfg.blocks[&edge.target].params.iter().zip(&edge.args) {
                across.insert(*dest);
                if let Mir68kValue::Temp(source, _) = value {
                    across.insert(*source);
                    affinity.entry(*dest).or_default().insert(*source);
                    affinity.entry(*source).or_default().insert(*dest);
                }
            }
        }
        let mut active = live.out_state(id).cloned().unwrap_or_default();
        add_uses(&mut active, term_values(&block.terminator));
        for op in block.ops.iter().rev() {
            if let Some((dest, _)) = verify::op_result(op) {
                for &other in &active {
                    interfere(&mut conflicts, dest, other);
                }
                active.remove(&dest);
            }
            if matches!(op, Mir68kOp::Call { .. }) {
                across.extend(&active);
            }
            add_uses(&mut active, op_values(op));
        }
        // Block parameters are simultaneous definitions. In particular they
        // must not overwrite external values live through the incoming edge.
        for (dest, _) in &block.params {
            for &other in &active {
                interfere(&mut conflicts, *dest, other);
            }
        }
    }
    let mut candidates: Vec<_> = across
        .into_iter()
        .filter(|id| !discarded.contains(id) && scores.get(id).copied().unwrap_or(0) >= 2)
        .collect();
    candidates.sort_by_key(|id| (std::cmp::Reverse(scores[id]), *id));
    let mut allocation = Allocation {
        registers: BTreeMap::new(),
        discarded,
    };
    for id in candidates {
        let forbidden: BTreeSet<_> = conflicts[&id]
            .iter()
            .filter_map(|other| allocation.registers.get(other).copied())
            .collect();
        let preferred = affinity
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|other| allocation.registers.get(other).copied());
        if let Some(register) = preferred
            .chain(POOL)
            .find(|register| !forbidden.contains(register))
        {
            allocation.registers.insert(id, register);
        }
    }
    verify_allocation(&allocation, &conflicts)?;
    Ok(allocation)
}

fn interfere(conflicts: &mut BTreeMap<TempId, Temps>, a: TempId, b: TempId) {
    if a != b {
        conflicts.entry(a).or_default().insert(b);
        conflicts.entry(b).or_default().insert(a);
    }
}

fn verify_allocation(
    allocation: &Allocation,
    conflicts: &BTreeMap<TempId, Temps>,
) -> Result<(), String> {
    for (id, register) in &allocation.registers {
        if !POOL.contains(register)
            || allocation.discarded.contains(id)
            || !conflicts.contains_key(id)
        {
            return Err("invalid native register assignment".into());
        }
        if conflicts[id]
            .iter()
            .any(|other| allocation.registers.get(other) == Some(register))
        {
            return Err("interfering native values share a register".into());
        }
    }
    Ok(())
}

fn add_uses<'a>(set: &mut Temps, values: impl IntoIterator<Item = &'a Mir68kValue>) {
    set.extend(values.into_iter().filter_map(|v| {
        if let Mir68kValue::Temp(id, _) = v {
            Some(*id)
        } else {
            None
        }
    }));
}

struct Cfg<'a> {
    entry: BlockId,
    blocks: BTreeMap<BlockId, &'a Mir68kBlock>,
    nodes: BTreeSet<BlockId>,
    predecessors: BTreeMap<BlockId, BTreeSet<BlockId>>,
    successors: BTreeMap<BlockId, BTreeSet<BlockId>>,
    reachable: BTreeSet<BlockId>,
    postorder: Vec<BlockId>,
    reverse_postorder: Vec<BlockId>,
}
impl<'a> Cfg<'a> {
    fn new(routine: &'a Mir68kRoutine) -> Result<Self, String> {
        let entry = routine.blocks.first().ok_or("routine has no entry")?.id;
        let blocks: BTreeMap<_, _> = routine.blocks.iter().map(|b| (b.id, b)).collect();
        let mut nodes: BTreeSet<_> = blocks.keys().copied().collect();
        let mut predecessors: BTreeMap<_, BTreeSet<_>> =
            nodes.iter().map(|id| (*id, BTreeSet::new())).collect();
        let mut successors: BTreeMap<_, BTreeSet<_>> = predecessors.clone();
        for block in &routine.blocks {
            for edge in edges(&block.terminator) {
                predecessors
                    .get_mut(&edge.target)
                    .ok_or("missing edge target")?
                    .insert(block.id);
                successors.get_mut(&block.id).unwrap().insert(edge.target);
            }
        }
        let mut reachable = BTreeSet::new();
        let mut postorder = Vec::new();
        let mut pending = vec![(entry, false)];
        while let Some((id, visited)) = pending.pop() {
            if visited {
                postorder.push(id);
            } else if reachable.insert(id) {
                pending.push((id, true));
                pending.extend(successors[&id].iter().rev().map(|id| (*id, false)));
            }
        }
        let reverse_postorder = postorder.iter().rev().copied().collect();
        // Unreachable predecessors cannot invalidate dominance or extend a
        // live range in the code that materialization actually emits.
        nodes.retain(|id| reachable.contains(id));
        predecessors.retain(|id, incoming| {
            incoming.retain(|from| reachable.contains(from));
            reachable.contains(id)
        });
        successors.retain(|id, _| reachable.contains(id));
        Ok(Self {
            entry,
            blocks,
            nodes,
            predecessors,
            successors,
            reachable,
            postorder,
            reverse_postorder,
        })
    }
}
impl DataflowGraph for Cfg<'_> {
    type Node = BlockId;
    fn entry(&self) -> Option<BlockId> {
        Some(self.entry)
    }
    fn nodes(&self) -> &BTreeSet<BlockId> {
        &self.nodes
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

struct Liveness<'a> {
    cfg: &'a Cfg<'a>,
}
impl DataflowProblem<Cfg<'_>> for Liveness<'_> {
    type State = Temps;
    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Backward
    }
    fn bottom(&self) -> Temps {
        Temps::new()
    }
    fn boundary(&self, _: BlockId) -> Option<Temps> {
        None
    }
    fn join(&self, into: &mut Temps, other: &Temps) {
        into.extend(other);
    }
    fn transfer(&self, id: BlockId, output: &Temps) -> Temps {
        let block = self.cfg.blocks[&id];
        let mut input = output.clone();
        add_uses(&mut input, term_values(&block.terminator));
        for op in block.ops.iter().rev() {
            if let Some((dest, _)) = verify::op_result(op) {
                input.remove(&dest);
            }
            add_uses(&mut input, op_values(op));
        }
        for (id, _) in &block.params {
            input.remove(id);
        }
        input
    }
}

fn verify_uses(cfg: &Cfg<'_>, dominance: &Dominance<BlockId>) -> Result<(), String> {
    let mut definitions = BTreeMap::new();
    for block in cfg.blocks.values() {
        for (id, _) in &block.params {
            definitions.insert(*id, (block.id, None));
        }
        for (index, op) in block.ops.iter().enumerate() {
            if let Some((dest, _)) = verify::op_result(op) {
                definitions.insert(dest, (block.id, Some(index)));
            }
        }
    }
    for id in &cfg.reachable {
        let block = cfg.blocks[id];
        for (index, values) in block
            .ops
            .iter()
            .map(op_values)
            .chain([term_values(&block.terminator)])
            .enumerate()
        {
            for value in values {
                if let Mir68kValue::Temp(temp, _) = value {
                    let (owner, position) =
                        definitions.get(temp).ok_or("undefined native value")?;
                    if !dominance.dominates(*owner, *id)
                        || (*owner == *id && position.is_some_and(|p| p >= index))
                    {
                        return Err(format!("native value {temp:?} does not dominate its use"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn loop_weights(cfg: &Cfg<'_>, dominance: &Dominance<BlockId>) -> BTreeMap<BlockId, u64> {
    let mut headers: BTreeMap<BlockId, BTreeSet<BlockId>> = BTreeMap::new();
    for &tail in &cfg.reachable {
        for &header in &cfg.successors[&tail] {
            if !dominance.dominates(header, tail) {
                continue;
            }
            let members = headers.entry(header).or_default();
            members.insert(header);
            let mut pending = vec![tail];
            while let Some(id) = pending.pop() {
                if members.insert(id) {
                    pending.extend(cfg.predecessors[&id].intersection(&cfg.reachable).copied());
                }
            }
        }
    }
    let mut weights = BTreeMap::new();
    for members in headers.values() {
        for &id in members {
            *weights.entry(id).or_insert(1) += 16;
        }
    }
    weights
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_verifier_rejects_scratch_registers_and_interference() {
        let conflicts = BTreeMap::from([
            (TempId(0), Temps::from([TempId(1)])),
            (TempId(1), Temps::from([TempId(0)])),
        ]);
        for registers in [
            BTreeMap::from([(TempId(0), 2)]),
            BTreeMap::from([(TempId(0), 4), (TempId(1), 4)]),
        ] {
            assert!(
                verify_allocation(
                    &Allocation {
                        registers,
                        ..Default::default()
                    },
                    &conflicts
                )
                .is_err()
            );
        }
        assert!(
            verify_allocation(
                &Allocation {
                    registers: BTreeMap::from([(TempId(0), 4), (TempId(1), 5)]),
                    ..Default::default()
                },
                &conflicts
            )
            .is_ok()
        );
    }
}
