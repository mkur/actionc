use std::collections::BTreeMap;

use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{
    MirAddr, MirBlockId, MirCond, MirDef, MirMem, MirMemoryEffect, MirOp, MirReg, MirRoutine,
    MirTerminator, MirValue, MirWidth,
};

/// A value known to occupy a physical 6502 register. The domain starts with
/// accumulator facts and is intentionally target-specific; X/Y can join it
/// when a rewrite has a measured use for those facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirMachineValue {
    ConstU8(u8),
    DirectMem(MirMem),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MirMachineValueState {
    reachable: bool,
    a: Option<MirMachineValue>,
}

impl MirMachineValueState {
    fn reachable_unknown() -> Self {
        Self {
            reachable: true,
            a: None,
        }
    }

    fn meet_with(&mut self, other: &Self) {
        if !other.reachable {
            return;
        }
        if !self.reachable {
            *self = other.clone();
            return;
        }
        if self.a != other.a {
            self.a = None;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMachineValueBlock {
    pub accumulator_in: Option<MirMachineValue>,
    pub accumulator_out: Option<MirMachineValue>,
    pub reachable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirMachineValueError {
    UnknownBlock(MirBlockId),
    OpOutOfBounds {
        block: MirBlockId,
        op_index: usize,
        op_count: usize,
    },
}

/// Forward, must-agree value facts for physical machine registers.
///
/// A block-entry fact exists only when every reachable predecessor supplies
/// the same value. Transfer is conservative around calls, opaque operations,
/// memory writes, edge arguments, and terminators that may require late
/// accumulator materialization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMachineValueAvailability {
    blocks: Vec<MirMachineValueBlock>,
    block_indices: BTreeMap<MirBlockId, usize>,
    ops: BTreeMap<MirBlockId, Vec<MirOp>>,
    evaluations: usize,
}

impl MirMachineValueAvailability {
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine, cfg: &MirCfg) -> Self {
        let entry = cfg.entry();
        let result = solve_dataflow(cfg, &MachineValueProblem { routine, entry });
        let block_indices = routine
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id, index))
            .collect();
        let blocks = routine
            .blocks
            .iter()
            .map(|block| {
                let input = result.in_state(block.id).cloned().unwrap_or_default();
                let output = result.out_state(block.id).cloned().unwrap_or_default();
                MirMachineValueBlock {
                    accumulator_in: input.a,
                    accumulator_out: output.a,
                    reachable: input.reachable,
                }
            })
            .collect();
        let ops = routine
            .blocks
            .iter()
            .map(|block| (block.id, block.ops.clone()))
            .collect();
        Self {
            blocks,
            block_indices,
            ops,
            evaluations: result.evaluations(),
        }
    }

    pub(in crate::mir6502) fn block_by_id(
        &self,
        block: MirBlockId,
    ) -> Option<&MirMachineValueBlock> {
        self.block_indices
            .get(&block)
            .and_then(|index| self.blocks.get(*index))
    }

    pub(in crate::mir6502) fn accumulator_at(
        &self,
        site: MirSite,
    ) -> Result<Option<MirMachineValue>, MirMachineValueError> {
        let block = site.block();
        let facts = self
            .block_by_id(block)
            .ok_or(MirMachineValueError::UnknownBlock(block))?;
        let ops = self
            .ops
            .get(&block)
            .ok_or(MirMachineValueError::UnknownBlock(block))?;
        let limit = match site {
            MirSite::BlockEntry { .. } => 0,
            MirSite::Op { op_index, .. } => {
                if op_index >= ops.len() {
                    return Err(MirMachineValueError::OpOutOfBounds {
                        block,
                        op_index,
                        op_count: ops.len(),
                    });
                }
                op_index
            }
            MirSite::Terminator { .. } => ops.len(),
        };
        let mut state = MirMachineValueState {
            reachable: facts.reachable,
            a: facts.accumulator_in.clone(),
        };
        for op in &ops[..limit] {
            apply_op(&mut state, op);
        }
        Ok(state.a)
    }

    pub(in crate::mir6502) fn evaluations(&self) -> usize {
        self.evaluations
    }
}

struct MachineValueProblem<'a> {
    routine: &'a MirRoutine,
    entry: Option<MirBlockId>,
}

impl DataflowProblem<MirCfg> for MachineValueProblem<'_> {
    type State = MirMachineValueState;

    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Forward
    }

    fn bottom(&self) -> Self::State {
        Self::State::default()
    }

    fn boundary(&self, node: MirBlockId) -> Option<Self::State> {
        (Some(node) == self.entry).then(MirMachineValueState::reachable_unknown)
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.meet_with(other);
    }

    fn transfer(&self, node: MirBlockId, input: &Self::State) -> Self::State {
        if !input.reachable {
            return input.clone();
        }
        let Some(block) = self.routine.blocks.iter().find(|block| block.id == node) else {
            return input.clone();
        };
        let mut output = input.clone();
        for op in &block.ops {
            apply_op(&mut output, op);
        }
        if !terminator_preserves_accumulator(&block.terminator) {
            output.a = None;
        }
        output
    }
}

fn apply_op(state: &mut MirMachineValueState, op: &MirOp) {
    if !state.reachable {
        return;
    }

    let effects = classify_op(op);
    let writes_memory = !effects.memory.direct_writes.is_empty()
        || effects.memory.indirect_writes
        || effects.memory.opaque
        || effects.memory.may_write_any
        || effects.memory.has_unknown_effects
        || !matches!(&effects.memory.structured_writes, MirMemoryEffect::None);
    if writes_memory && matches!(state.a, Some(MirMachineValue::DirectMem(_))) {
        state.a = None;
    }

    if let Some(value) = explicit_accumulator_result(op) {
        state.a = Some(value);
    } else if effects.may_clobber_reg_compat(MirReg::A) {
        state.a = None;
    }

    if let MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    } = op
    {
        state.a = Some(MirMachineValue::DirectMem(mem.clone()));
    }
}

fn explicit_accumulator_result(op: &MirOp) -> Option<MirMachineValue> {
    match op {
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value,
            width: MirWidth::Byte,
        } => u8::try_from(*value).ok().map(MirMachineValue::ConstU8),
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some(MirMachineValue::DirectMem(mem.clone())),
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::ConstU8(value),
            width: MirWidth::Byte,
        } => Some(MirMachineValue::ConstU8(*value)),
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::ConstU16(value),
            width: MirWidth::Byte,
        } => u8::try_from(*value).ok().map(MirMachineValue::ConstU8),
        MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: MirValue::PointerCell(mem),
            width: MirWidth::Byte,
        } => Some(MirMachineValue::DirectMem(mem.clone())),
        _ => None,
    }
}

fn terminator_preserves_accumulator(terminator: &MirTerminator) -> bool {
    match terminator {
        MirTerminator::Jump(edge) => edge.args.is_empty(),
        MirTerminator::Branch {
            cond,
            then_edge,
            else_edge,
        } => {
            then_edge.args.is_empty()
                && else_edge.args.is_empty()
                && matches!(
                    cond,
                    MirCond::FlagTest(_) | MirCond::AnyFlagTest(_) | MirCond::FusedCompare { .. }
                )
        }
        MirTerminator::Return | MirTerminator::Exit | MirTerminator::Unreachable => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirCallAbi, MirCallTarget, MirEdge, MirEffects, MirFlagTest, MirFrame,
        MirRegisterSet, MirRoutineAbi, MirSpillId, RoutineId,
    };

    fn spill(id: u32) -> MirMem {
        MirMem::Spill {
            id: MirSpillId(id),
            offset: 0,
        }
    }

    fn load_a(mem: MirMem) -> MirOp {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }
    }

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("bb{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn routine(blocks: Vec<MirBlock>) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "MachineValues".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn branch(then_block: u32, else_block: u32) -> MirTerminator {
        MirTerminator::Branch {
            cond: MirCond::FlagTest(MirFlagTest::ZSet),
            then_edge: MirEdge::plain(MirBlockId(then_block)),
            else_edge: MirEdge::plain(MirBlockId(else_block)),
        }
    }

    fn analyze(routine: &MirRoutine) -> MirMachineValueAvailability {
        let cfg = MirCfg::from_routine(routine).unwrap();
        MirMachineValueAvailability::analyze(routine, &cfg)
    }

    #[test]
    fn accumulator_value_flows_across_both_conditional_edges() {
        let value = spill(1);
        let routine = routine(vec![
            block(0, vec![load_a(value.clone())], branch(1, 2)),
            block(1, Vec::new(), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
        ]);
        let values = analyze(&routine);

        for block in [MirBlockId(1), MirBlockId(2)] {
            assert_eq!(
                values.accumulator_at(MirSite::BlockEntry { block }),
                Ok(Some(MirMachineValue::DirectMem(value.clone())))
            );
        }
    }

    #[test]
    fn join_retains_only_identical_accumulator_values() {
        let value = spill(1);
        let build = |right| {
            routine(vec![
                block(0, Vec::new(), branch(1, 2)),
                block(
                    1,
                    vec![load_a(value.clone())],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
                block(
                    2,
                    vec![load_a(right)],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
                block(3, Vec::new(), MirTerminator::Return),
            ])
        };

        let same = analyze(&build(value.clone()));
        assert_eq!(
            same.accumulator_at(MirSite::BlockEntry {
                block: MirBlockId(3)
            }),
            Ok(Some(MirMachineValue::DirectMem(value.clone())))
        );

        let different = analyze(&build(spill(2)));
        assert_eq!(
            different.accumulator_at(MirSite::BlockEntry {
                block: MirBlockId(3)
            }),
            Ok(None)
        );
    }

    #[test]
    fn memory_writes_conservatively_kill_memory_backed_values() {
        let routine = routine(vec![
            block(
                0,
                vec![
                    load_a(spill(1)),
                    MirOp::Store {
                        dst: MirAddr::Direct(spill(2)),
                        src: MirValue::Def(MirDef::Reg(MirReg::X)),
                        width: MirWidth::Byte,
                    },
                ],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        let values = analyze(&routine);

        assert_eq!(
            values.accumulator_at(MirSite::BlockEntry {
                block: MirBlockId(1)
            }),
            Ok(None)
        );
    }

    #[test]
    fn calls_with_unknown_accumulator_effects_kill_values() {
        let routine = routine(vec![
            block(
                0,
                vec![
                    load_a(spill(1)),
                    MirOp::Call {
                        target: MirCallTarget::Routine(RoutineId(1)),
                        abi: MirCallAbi {
                            params: Vec::new(),
                            result: None,
                            clobbers: MirRegisterSet::default(),
                            preserves: MirRegisterSet::default(),
                        },
                        args: Vec::new(),
                        result: None,
                        effects: MirEffects::default(),
                    },
                ],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        let values = analyze(&routine);

        assert_eq!(
            values.accumulator_at(MirSite::BlockEntry {
                block: MirBlockId(1)
            }),
            Ok(None)
        );
    }

    #[test]
    fn non_flag_branch_conditions_do_not_export_accumulator_facts() {
        let routine = routine(vec![
            block(
                0,
                vec![load_a(spill(1))],
                MirTerminator::Branch {
                    cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                    then_edge: MirEdge::plain(MirBlockId(1)),
                    else_edge: MirEdge::plain(MirBlockId(2)),
                },
            ),
            block(1, Vec::new(), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
        ]);
        let values = analyze(&routine);

        assert_eq!(
            values.accumulator_at(MirSite::BlockEntry {
                block: MirBlockId(1)
            }),
            Ok(None)
        );
    }

    #[test]
    fn site_queries_replay_block_transfer_before_the_requested_op() {
        let value = spill(1);
        let routine = routine(vec![block(
            0,
            vec![
                load_a(value.clone()),
                MirOp::Compare {
                    dst: crate::mir6502::ir::MirCondDest::Flags,
                    op: crate::mir6502::ir::MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::Reg(MirReg::A)),
                    right: MirValue::ConstU8(7),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ],
            MirTerminator::Return,
        )]);
        let values = analyze(&routine);

        assert_eq!(
            values.accumulator_at(MirSite::Op {
                block: MirBlockId(0),
                op_index: 1,
            }),
            Ok(Some(MirMachineValue::DirectMem(value)))
        );
        assert!(values.evaluations() > 0);
    }
}
