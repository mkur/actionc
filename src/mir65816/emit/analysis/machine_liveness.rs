//! Backward physical register/flag liveness. Environment effects are observed,
//! but never grant permission to delete protected environment operations.
use super::super::effects::{Control, InstructionEffects, NZCV, Registers, env};
use super::super::selected::{Action, SelectedRoutine};
use super::sites::Node;
use crate::analysis::{
    dataflow::{DataflowDirection, DataflowProblem, DataflowResult, solve_dataflow},
    graph::DataflowGraph,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterLane {
    ALow,
    AHigh,
    XLow,
    XHigh,
    YLow,
    YHigh,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionFlag {
    N,
    Z,
    C,
    V,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MachineLive {
    pub registers: Registers,
    pub flags: u8,
    /// Dataflow observations only. All environment effects remain protected.
    pub environment: u16,
}
impl MachineLive {
    pub fn register_live(self, lane: RegisterLane) -> bool {
        let (value, mask) = match lane {
            RegisterLane::ALow => (self.registers.a, 0xff),
            RegisterLane::AHigh => (self.registers.a, 0xff00),
            RegisterLane::XLow => (self.registers.x, 0xff),
            RegisterLane::XHigh => (self.registers.x, 0xff00),
            RegisterLane::YLow => (self.registers.y, 0xff),
            RegisterLane::YHigh => (self.registers.y, 0xff00),
        };
        value & mask != 0
    }
    pub fn flag_live(self, flag: ConditionFlag) -> bool {
        use super::super::effects::{C, N, V, Z};
        self.flags
            & match flag {
                ConditionFlag::N => N,
                ConditionFlag::Z => Z,
                ConditionFlag::C => C,
                ConditionFlag::V => V,
            }
            != 0
    }
    fn union(&mut self, other: Self) {
        self.registers.a |= other.registers.a;
        self.registers.x |= other.registers.x;
        self.registers.y |= other.registers.y;
        self.flags |= other.flags;
        self.environment |= other.environment;
    }
    fn before(mut self, effects: &InstructionEffects) -> Self {
        // A call's definite clobbers kill the old register value too; its inputs
        // are added afterward. This never claims unspecified results are defined.
        self.registers.a =
            (self.registers.a & !(effects.writes.a | effects.clobbers.a)) | effects.reads.a;
        self.registers.x =
            (self.registers.x & !(effects.writes.x | effects.clobbers.x)) | effects.reads.x;
        self.registers.y =
            (self.registers.y & !(effects.writes.y | effects.clobbers.y)) | effects.reads.y;
        self.flags =
            (self.flags & !(effects.flag_writes | effects.flag_clobbers)) | effects.flag_reads;
        self.environment =
            (self.environment & !effects.environment_writes) | effects.environment_reads;
        self
    }
}
pub(super) struct MachineLiveness {
    result: DataflowResult<Node, MachineLive>,
}
impl MachineLiveness {
    pub fn analyze(selected: &SelectedRoutine) -> Self {
        #[cfg(any(test, feature = "native65816-state-proof"))]
        crate::mir65816::emit::work::add("machine_liveness", 1);
        let mut effects = BTreeMap::new();
        let mut result = MachineLive {
            environment: env::ALL,
            ..MachineLive::default()
        };
        for (index, record) in selected.records().iter().enumerate() {
            if let Action::Instruction { effects: e, .. } = &record.action {
                if e.control == Control::Return && selected.cfg().reachable().contains(&Node(index))
                {
                    // Central native-return effects use ResultLocation, including
                    // the ABI's byte/three-byte zero-extension requirements.
                    result.union(MachineLive {
                        registers: e.reads,
                        ..MachineLive::default()
                    });
                }
                effects.insert(Node(index), e.clone());
            }
        }
        let boundaries = selected
            .records()
            .iter()
            .enumerate()
            .filter_map(|(index, record)| match record.action {
                Action::ReturnExit => Some((Node(index), result)),
                Action::FaultExit => Some((
                    Node(index),
                    MachineLive {
                        registers: Registers::ALL,
                        flags: NZCV,
                        environment: env::ALL,
                    },
                )),
                _ => None,
            })
            .collect();
        Self::solve(selected.cfg(), &effects, &boundaries)
    }
    fn solve(
        graph: &impl DataflowGraph<Node = Node>,
        effects: &BTreeMap<Node, InstructionEffects>,
        boundaries: &BTreeMap<Node, MachineLive>,
    ) -> Self {
        Self {
            result: solve_dataflow(
                graph,
                &Problem {
                    effects,
                    boundaries,
                },
            ),
        }
    }
    pub fn before(&self, node: Node) -> Result<MachineLive, String> {
        self.result
            .in_state(node)
            .copied()
            .ok_or_else(|| "unreachable machine-liveness site".into())
    }
    pub fn after(&self, node: Node) -> Result<MachineLive, String> {
        self.result
            .out_state(node)
            .copied()
            .ok_or_else(|| "unreachable machine-liveness site".into())
    }
}
struct Problem<'a> {
    effects: &'a BTreeMap<Node, InstructionEffects>,
    boundaries: &'a BTreeMap<Node, MachineLive>,
}
impl<G: DataflowGraph<Node = Node>> DataflowProblem<G> for Problem<'_> {
    type State = MachineLive;
    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Backward
    }
    fn bottom(&self) -> Self::State {
        MachineLive::default()
    }
    fn boundary(&self, node: Node) -> Option<Self::State> {
        self.boundaries.get(&node).copied()
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.union(*other);
    }
    fn transfer(&self, node: Node, state: &Self::State) -> Self::State {
        self.effects.get(&node).map_or(*state, |e| state.before(e))
    }
}

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;
