//! Closed-operation interference for invocation-owned temporary stack homes.
use super::super::*;
use std::collections::{BTreeMap, BTreeSet};

type Live = BTreeSet<TempId>;
pub(super) type Interference = BTreeMap<TempId, Live>;

/// Temps whose only input occurrence is one Branch condition. This deliberately
/// includes unreachable blocks and distinguishes edge arguments from conditions.
/// Adjacency and checked machine operands are separate selector requirements.
pub(super) fn sole_branch_conditions(routine: &Mir65816Routine) -> Live {
    let mut conditions = BTreeMap::<TempId, usize>::new();
    let mut other = Uses::default();
    for block in &routine.blocks {
        for op in &block.ops {
            other.inputs.extend(Uses::operation(op).inputs);
        }
        let edges = match &block.terminator {
            Mir65816Terminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                if let Mir65816Value::Temp(id, _) = condition {
                    *conditions.entry(*id).or_default() += 1;
                }
                vec![then_edge, else_edge]
            }
            Mir65816Terminator::Goto(edge) => vec![edge],
            Mir65816Terminator::Return { value, .. } => {
                if let Some(value) = value {
                    other.value(value);
                }
                vec![]
            }
            Mir65816Terminator::Fallthrough
            | Mir65816Terminator::Exit
            | Mir65816Terminator::ArithmeticFault => vec![],
        };
        for edge in edges {
            for value in &edge.args {
                other.value(value);
            }
        }
    }
    conditions
        .into_iter()
        .filter_map(|(id, count)| (count == 1 && !other.inputs.contains(&id)).then_some(id))
        .collect()
}

#[cfg(test)]
#[path = "branch_use_tests.rs"]
mod branch_use_tests;

#[derive(Default)]
struct Uses {
    inputs: Live,
    occurrences: Vec<TempId>,
    output: Option<TempId>,
    pointer_copy: Option<(TempId, TempId)>,
}

impl Uses {
    fn value(&mut self, value: &Mir65816Value) {
        if let Mir65816Value::Temp(id, _) = value {
            self.inputs.insert(*id);
            self.occurrences.push(*id);
        }
    }

    fn address(&mut self, address: &Mir65816Address) {
        if let Mir65816AddressBase::Indirect(value) = &address.base {
            self.value(value);
        }
        if let Some(index) = &address.index {
            self.value(&index.value);
        }
    }

    fn operation(op: &Mir65816Op) -> Self {
        let mut uses = Self::default();
        // Exhaustive: a new operation must describe its inputs and definition
        // before it can participate in stack reuse.
        match op {
            Mir65816Op::Load { dest, address, .. }
            | Mir65816Op::AddressOf { dest, address, .. } => {
                uses.output = Some(*dest);
                uses.address(address);
            }
            Mir65816Op::Store { address, value, .. } => {
                uses.address(address);
                uses.value(value);
            }
            Mir65816Op::Copy {
                destination,
                source,
                ..
            } => {
                uses.address(destination);
                uses.address(source);
            }
            Mir65816Op::Unary { dest, value, .. } | Mir65816Op::Cast { dest, value, .. } => {
                uses.output = Some(*dest);
                uses.value(value);
            }
            Mir65816Op::PointerOffset {
                dest, base, offset, ..
            } => {
                uses.output = Some(*dest);
                uses.value(base);
                uses.value(offset);
            }
            Mir65816Op::Binary {
                dest, left, right, ..
            }
            | Mir65816Op::Compare {
                dest, left, right, ..
            } => {
                uses.output = Some(*dest);
                uses.value(left);
                uses.value(right);
            }
            Mir65816Op::Call {
                target,
                args,
                result,
                ..
            } => {
                match target {
                    Mir65816CallTarget::Indirect(value, _) => uses.value(value),
                    Mir65816CallTarget::Direct(_)
                    | Mir65816CallTarget::Helper(_)
                    | Mir65816CallTarget::Builtin(_)
                    | Mir65816CallTarget::Runtime(_) => {}
                }
                for value in args {
                    uses.value(value);
                }
                uses.output = result.map(|(id, _)| id);
            }
        }
        uses.pointer_copy = pointer_copy(op);
        uses
    }
}

/// Exhaustive typed operands shared with bounded private-source selection.
pub(super) fn operation_inputs(op: &Mir65816Op) -> Vec<TempId> {
    Uses::operation(op).occurrences
}

/// Every operand occurrence, including repeated address/value operands. Shared
/// with address selection so new MIR operations cannot hide a live producer.
pub(super) fn input_counts(routine: &Mir65816Routine) -> BTreeMap<TempId, usize> {
    let mut all = Vec::new();
    for block in &routine.blocks {
        for op in &block.ops {
            all.extend(Uses::operation(op).occurrences);
        }
        let mut term = Uses::default();
        let edges = match &block.terminator {
            Mir65816Terminator::Goto(edge) => vec![edge],
            Mir65816Terminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                term.value(condition);
                vec![then_edge, else_edge]
            }
            Mir65816Terminator::Return { value, .. } => {
                if let Some(value) = value {
                    term.value(value);
                }
                vec![]
            }
            Mir65816Terminator::Fallthrough
            | Mir65816Terminator::Exit
            | Mir65816Terminator::ArithmeticFault => vec![],
        };
        for edge in edges {
            for value in &edge.args {
                term.value(value);
            }
        }
        all.extend(term.occurrences);
    }
    let mut counts = BTreeMap::new();
    for id in all {
        *counts.entry(id).or_default() += 1;
    }
    counts
}

struct Block {
    params: Live,
    ops: Vec<Uses>,
    terminator: Uses,
    successors: Vec<usize>,
}

pub(super) fn interference(routine: &Mir65816Routine) -> Result<Interference, String> {
    interference_inner(routine, false)
}

/// A bit-preserving cast of a complete captured three-byte value. Parameters
/// and source memory are deliberately excluded from this storage affinity.
pub(super) fn pointer_copy(op: &Mir65816Op) -> Option<(TempId, TempId)> {
    match op {
        Mir65816Op::Cast {
            dest,
            from,
            to,
            value: Mir65816Value::Temp(source, width),
            ..
        } if from.get() == 3 && to.get() == 3 && width.get() == 3 => Some((*source, *dest)),
        _ => None,
    }
}

/// Stack-only exception: a dying identity-cast input need not coexist with its
/// output. Every other operation and every third-party overlap stays closed.
pub(super) fn pointer_copy_interference(routine: &Mir65816Routine) -> Result<Interference, String> {
    interference_inner(routine, true)
}

fn interference_inner(
    routine: &Mir65816Routine,
    pointer_copies: bool,
) -> Result<Interference, String> {
    let indices: BTreeMap<_, _> = routine
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let mut blocks = Vec::new();
    for (index, block) in routine.blocks.iter().enumerate() {
        let mut terminator = Uses::default();
        let mut successors = Vec::new();
        let edges = match &block.terminator {
            Mir65816Terminator::Goto(edge) => vec![edge],
            Mir65816Terminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                terminator.value(condition);
                vec![then_edge, else_edge]
            }
            Mir65816Terminator::Return { value, .. } => {
                if let Some(value) = value {
                    terminator.value(value);
                }
                vec![]
            }
            Mir65816Terminator::Fallthrough => {
                let next = routine
                    .blocks
                    .get(index + 1)
                    .ok_or("unresolved terminal fallthrough")?;
                if !next.params.is_empty() {
                    return Err("fallthrough cannot supply block parameters".into());
                }
                successors.push(index + 1);
                vec![]
            }
            Mir65816Terminator::Exit | Mir65816Terminator::ArithmeticFault => vec![],
        };
        for edge in edges {
            successors.push(
                *indices
                    .get(&edge.target)
                    .ok_or("unknown allocation edge target")?,
            );
            for value in &edge.args {
                terminator.value(value);
            }
        }
        blocks.push(Block {
            params: block.params.iter().map(|(id, _)| *id).collect(),
            ops: block.ops.iter().map(Uses::operation).collect(),
            terminator,
            successors,
        });
    }
    let mut graph: Interference = routine
        .temps
        .iter()
        .map(|(id, _)| (*id, Live::new()))
        .collect();
    let mut definitions = Live::new();
    for block in &blocks {
        for id in block
            .params
            .iter()
            .copied()
            .chain(block.ops.iter().filter_map(|op| op.output))
        {
            if !graph.contains_key(&id) || !definitions.insert(id) {
                return Err("invalid temporary definition in stack allocation".into());
            }
        }
        for op in block.ops.iter().chain([&block.terminator]) {
            if op.inputs.iter().any(|id| !graph.contains_key(id)) {
                return Err("unknown temporary use in stack allocation".into());
            }
        }
    }
    if definitions.len() != routine.temps.len() {
        return Err("missing temporary definition in stack allocation".into());
    }

    // Entry sets exclude block parameters, which are defined by incoming edge
    // copies. Outgoing arguments are uses in the predecessor. This also covers
    // values carried across backedges without being passed as parameters.
    let mut entries = vec![Live::new(); blocks.len()];
    loop {
        let mut changed = false;
        for (index, block) in blocks.iter().enumerate().rev() {
            let mut live = exit_live(block, &entries);
            for op in block.ops.iter().rev() {
                if let Some(id) = op.output {
                    live.remove(&id);
                }
                live.extend(&op.inputs);
            }
            live.retain(|id| !block.params.contains(id));
            if live != entries[index] {
                entries[index] = live;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for block in &blocks {
        let mut live = exit_live(block, &entries);
        add_clique(&mut graph, &live);
        for op in block.ops.iter().rev() {
            // Reserve even dead outputs, conservatively including fused Booleans.
            // Omit only this site's dying identity-copy pair. An interference
            // established at another site must never be removed from the graph.
            let exception = op
                .pointer_copy
                .filter(|(source, _)| pointer_copies && !live.contains(source));
            live.extend(&op.inputs);
            live.extend(op.output);
            for &id in &live {
                graph
                    .get_mut(&id)
                    .expect("checked temporary identity")
                    .extend(live.iter().copied().filter(|&other| {
                        other != id
                            && !exception.is_some_and(|(a, b)| {
                                (id == a && other == b) || (id == b && other == a)
                            })
                    }));
            }
            if let Some(id) = op.output {
                live.remove(&id);
            }
        }
        // All edge destinations are written, including unused parameters.
        // They must be disjoint from each other and all successor live-ins.
        live.extend(&block.params);
        add_clique(&mut graph, &live);
    }
    Ok(graph)
}

fn exit_live(block: &Block, entries: &[Live]) -> Live {
    let mut live = block.terminator.inputs.clone();
    for &successor in &block.successors {
        live.extend(&entries[successor]);
    }
    live
}

fn add_clique(graph: &mut Interference, live: &Live) {
    for &id in live {
        graph
            .get_mut(&id)
            .expect("checked temporary identity")
            .extend(live.iter().copied().filter(|other| *other != id));
    }
}
