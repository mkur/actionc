//! Reaching stored definitions and ordered read attribution. Logical temp names
//! never participate in identity: one definition is (physical byte, write site).
use super::super::effects::Access;
use super::homes::{HomeAccess, HomeByte, Homes};
use super::sites::Node;
use crate::analysis::{
    dataflow::{DataflowDirection, DataflowProblem, DataflowResult, solve_dataflow},
    graph::DataflowGraph,
};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Definition {
    pub home: HomeByte,
    pub store: Node,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::mir65816::emit) struct ReadUse {
    pub node: Node,
    pub access_index: usize,
    /// Either the target is unresolved or a may-write can have changed its value.
    pub uncertain: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::mir65816::emit) struct UndefinedRead {
    pub home: HomeByte,
    pub usage: ReadUse,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    definitions: BTreeMap<HomeByte, BTreeSet<Definition>>,
    possibly_undefined: BTreeSet<HomeByte>,
    uncertain: BTreeSet<HomeByte>,
}
impl State {
    /// Compiler events and pure reads share immutable facts within one solver
    /// run. Copy on write prevents a transfer from changing predecessor facts.
    fn transfer(state: &mut Rc<Self>, node: Node, effect: &HomeAccess) {
        if effect.access != Access::Read && !effect.homes.is_empty() {
            Rc::make_mut(state).apply(node, effect);
        }
    }
    fn join(&mut self, other: &Self) {
        for (&home, definitions) in &other.definitions {
            self.definitions
                .entry(home)
                .or_default()
                .extend(definitions);
        }
        self.possibly_undefined.extend(&other.possibly_undefined);
        self.uncertain.extend(&other.uncertain);
    }
    fn apply(&mut self, node: Node, effect: &HomeAccess) {
        match effect.access {
            Access::Read => {}
            Access::Write if !effect.uncertain => {
                for &home in &effect.homes {
                    self.definitions
                        .insert(home, [Definition { home, store: node }].into());
                    self.possibly_undefined.remove(&home);
                    self.uncertain.remove(&home);
                }
            }
            Access::Write | Access::MayWrite => {
                // Preserve candidate definitions and undefined paths. A may-write
                // is not proof that any particular private byte was initialized.
                self.uncertain.extend(&effect.homes);
            }
        }
    }
}
struct Problem<'a> {
    homes: &'a Homes,
    entry: Option<Node>,
}
impl<G: DataflowGraph<Node = Node>> DataflowProblem<G> for Problem<'_> {
    type State = Option<Rc<State>>;
    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Forward
    }
    fn bottom(&self) -> Self::State {
        None
    }
    fn boundary(&self, node: Node) -> Option<Self::State> {
        (Some(node) == self.entry).then(|| {
            Some(Rc::new(State {
                // ABI-defined inputs start outside possibly_undefined. They
                // are not store sites; no per-site copy of their byte set is
                // needed to answer reaching-definition/undefined-read queries.
                possibly_undefined: self
                    .homes
                    .info
                    .iter()
                    .filter_map(|(&h, i)| (i.private && !i.entry_defined).then_some(h))
                    .collect(),
                ..State::default()
            }))
        })
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        if let Some(other) = other {
            if let Some(into) = into {
                if !Rc::ptr_eq(into, other) && into.as_ref() != other.as_ref() {
                    Rc::make_mut(into).join(other);
                }
            } else {
                *into = Some(other.clone());
            }
        }
    }
    fn transfer(&self, node: Node, state: &Self::State) -> Self::State {
        let mut state = state.clone()?;
        for effect in &self.homes.accesses[&node] {
            State::transfer(&mut state, node, effect);
        }
        Some(state)
    }
}

pub(super) struct HomeDefinitions {
    result: DataflowResult<Node, Option<Rc<State>>>,
    uses: BTreeMap<Definition, BTreeSet<ReadUse>>,
    undefined: Vec<UndefinedRead>,
}
impl HomeDefinitions {
    pub fn analyze(graph: &impl DataflowGraph<Node = Node>, homes: &Homes) -> Self {
        #[cfg(any(test, feature = "native65816-state-proof"))]
        crate::mir65816::emit::work::add("home_definitions", 1);
        let result = solve_dataflow(
            graph,
            &Problem {
                homes,
                entry: graph.entry(),
            },
        );
        let mut uses: BTreeMap<Definition, BTreeSet<ReadUse>> = BTreeMap::new();
        let mut undefined = Vec::new();
        for &node in graph.reachable() {
            let mut state = result
                .in_state(node)
                .and_then(Option::as_ref)
                .expect("reachable definition state")
                .clone();
            for (access_index, effect) in homes.accesses[&node].iter().enumerate() {
                if effect.access == Access::Read {
                    for &home in &effect.homes {
                        let usage = ReadUse {
                            node,
                            access_index,
                            uncertain: effect.uncertain || state.uncertain.contains(&home),
                        };
                        if let Some(definitions) = state.definitions.get(&home) {
                            for &definition in definitions {
                                uses.entry(definition).or_default().insert(usage);
                            }
                        }
                        if state.possibly_undefined.contains(&home) {
                            undefined.push(UndefinedRead { home, usage });
                        }
                    }
                }
                // Reads in an RMW/summary see the incoming definitions; later
                // internal reads see earlier writes in that same summary.
                State::transfer(&mut state, node, effect);
            }
        }
        Self {
            result,
            uses,
            undefined,
        }
    }
    fn write_index(&self, homes: &Homes, definition: Definition) -> Result<usize, String> {
        if self
            .result
            .in_state(definition.store)
            .and_then(Option::as_ref)
            .is_none()
        {
            return Err("invalid or unreachable definition site".into());
        }
        if !homes.info.contains_key(&definition.home) {
            return Err("unknown physical home".into());
        }
        let indices = homes.accesses[&definition.store]
            .iter()
            .enumerate()
            .filter_map(|(index, effect)| {
                (effect.access == Access::Write
                    && !effect.uncertain
                    && effect.homes.contains(&definition.home))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        match indices.as_slice() {
            [index] => Ok(*index),
            [] => Err("home has no definite write at this site".into()),
            _ => Err(
                "multiple writes to one byte at a selected site require finer definition identity"
                    .into(),
            ),
        }
    }
    pub fn uses_of_definition(
        &self,
        homes: &Homes,
        definition: Definition,
    ) -> Result<BTreeSet<ReadUse>, String> {
        self.write_index(homes, definition)?;
        Ok(self.uses.get(&definition).cloned().unwrap_or_default())
    }
    /// Window endpoints are inclusive selected sites. This is only an
    /// outside-use proof: a replacement must independently preserve local reads,
    /// definedness, observable effects and machine/environment state.
    pub fn definition_dead_outside_window(
        &self,
        graph: &impl DataflowGraph<Node = Node>,
        homes: &Homes,
        definition: Definition,
        end: Node,
    ) -> Result<bool, String> {
        let write_index = self.write_index(homes, definition)?;
        let start = definition.store;
        if end < start || !graph.reachable().contains(&end) {
            return Err("invalid definition window".into());
        }
        if !homes.info[&definition.home].private {
            return Err("protected home cannot authorize a private-store rewrite".into());
        }
        // Start with straight-line, single-entry windows. Branch/loop windows
        // need a future transaction contract; numeric site order is not a CFG.
        for index in start.0..=end.0 {
            let node = Node(index);
            if !graph.reachable().contains(&node)
                || (node != end && graph.successors(node) != &[Node(index + 1)].into())
                || (node != start
                    && graph
                        .predecessors(node)
                        .intersection(graph.reachable())
                        .copied()
                        .collect::<BTreeSet<_>>()
                        != [Node(index - 1)].into())
            {
                return Err("definition window is not a single-entry straight-line path".into());
            }
            if homes.accesses[&node]
                .iter()
                .any(|effect| effect.uncertain && effect.homes.contains(&definition.home))
            {
                return Err("incomplete alias effects in definition window".into());
            }
        }
        for usage in self.uses.get(&definition).into_iter().flatten() {
            if usage.uncertain {
                return Err("uncertain reaching definition or aliasing read".into());
            }
            if usage.node < start || usage.node > end {
                return Ok(false);
            }
            // A read before this write, attributed to this same static site,
            // consumes a previous loop iteration, not the proposed window's store.
            if usage.node == start && usage.access_index <= write_index {
                return Err("loop-carried read precedes this definition".into());
            }
        }
        Ok(true)
    }
    pub fn undefined_private_reads(&self) -> &[UndefinedRead] {
        &self.undefined
    }
}

#[cfg(test)]
#[path = "home_definition_tests.rs"]
mod tests;
