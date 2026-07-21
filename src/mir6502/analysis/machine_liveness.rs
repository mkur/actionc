#![allow(dead_code)] // Post-home rewrite consumers arrive in later workflow slices.

use std::collections::BTreeMap;

use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::{
    MirFlagSet, MirMachineEffects, MirOpEffectSummary, MirOpKind, MirTerminatorEffectSummary,
    classify_op, classify_terminator,
};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirBlockId, MirFlag, MirReg, MirRegisterSet, MirRoutine, MirTerminator};

/// Backward liveness for physical 6502 state represented by MIR. Flags remain
/// independent because a rewrite can preserve one condition code while
/// destroying another.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMachineLiveSet {
    registers: MirRegisterSet,
    flags: MirFlagSet,
}

impl MirMachineLiveSet {
    pub(in crate::mir6502) fn register_live(self, reg: MirReg) -> bool {
        register_is_set(self.registers, reg)
    }

    pub(in crate::mir6502) fn stack_pointer_live(self) -> bool {
        self.registers.sp
    }

    pub(in crate::mir6502) fn flag_live(self, flag: MirFlag) -> bool {
        self.flags.contains(flag)
    }

    pub(in crate::mir6502) fn flags_live(self, flags: MirFlagSet) -> bool {
        flags_intersect(self.flags, flags)
    }

    fn all() -> Self {
        Self {
            registers: MirRegisterSet {
                a: true,
                x: true,
                y: true,
                flags: false,
                sp: true,
            },
            flags: MirFlagSet::all(),
        }
    }

    fn union_with(&mut self, other: Self) {
        merge_registers(&mut self.registers, other.registers);
        merge_flags(&mut self.flags, other.flags);
    }

    fn subtract(self, defs: Self) -> Self {
        Self {
            registers: subtract_registers(self.registers, defs.registers),
            flags: subtract_flags(self.flags, defs.flags),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMachineBlockLiveness {
    pub uses: MirMachineLiveSet,
    pub defs: MirMachineLiveSet,
    pub live_in: MirMachineLiveSet,
    pub live_out: MirMachineLiveSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirMachineLivenessError {
    UnknownBlock(MirBlockId),
    OpOutOfBounds {
        block: MirBlockId,
        op_index: usize,
        op_count: usize,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MirMachineTransfer {
    reads: MirMachineLiveSet,
    writes: MirMachineLiveSet,
}

impl MirMachineTransfer {
    fn apply(self, live_after: MirMachineLiveSet) -> MirMachineLiveSet {
        let mut live_before = live_after.subtract(self.writes);
        live_before.union_with(self.reads);
        live_before
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MirMachineBlockTransfers {
    ops: Vec<MirMachineTransfer>,
    terminator: MirMachineTransfer,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMachineLiveness {
    blocks: Vec<MirMachineBlockLiveness>,
    block_indices: BTreeMap<MirBlockId, usize>,
    transfers: BTreeMap<MirBlockId, MirMachineBlockTransfers>,
    evaluations: usize,
}

impl MirMachineLiveness {
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine, cfg: &MirCfg) -> Self {
        let transfers = routine
            .blocks
            .iter()
            .map(|block| {
                let ops = block
                    .ops
                    .iter()
                    .map(|op| op_transfer(&classify_op(op)))
                    .collect();
                let terminator = terminator_transfer(&classify_terminator(&block.terminator));
                (block.id, MirMachineBlockTransfers { ops, terminator })
            })
            .collect::<BTreeMap<_, _>>();
        let facts = transfers
            .iter()
            .map(|(block, transfers)| (*block, block_uses_and_defs(transfers)))
            .collect::<BTreeMap<_, _>>();
        let boundaries = routine
            .blocks
            .iter()
            .filter(|block| {
                matches!(
                    block.terminator,
                    MirTerminator::Return | MirTerminator::Exit
                )
            })
            .map(|block| (block.id, action_return_machine_uses()))
            .collect::<BTreeMap<_, _>>();
        let result = solve_dataflow(
            cfg,
            &MachineLivenessProblem {
                facts: &facts,
                boundaries: &boundaries,
            },
        );
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
                let facts = &facts[&block.id];
                MirMachineBlockLiveness {
                    uses: facts.uses,
                    defs: facts.defs,
                    live_in: result.in_state(block.id).copied().unwrap_or(facts.uses),
                    live_out: result.out_state(block.id).copied().unwrap_or_default(),
                }
            })
            .collect();
        Self {
            blocks,
            block_indices,
            transfers,
            evaluations: result.evaluations(),
        }
    }

    pub(in crate::mir6502) fn block_by_id(
        &self,
        block: MirBlockId,
    ) -> Option<&MirMachineBlockLiveness> {
        self.block_indices
            .get(&block)
            .and_then(|index| self.blocks.get(*index))
    }

    pub(in crate::mir6502) fn live_in(&self, block: MirBlockId) -> Option<MirMachineLiveSet> {
        self.block_by_id(block).map(|facts| facts.live_in)
    }

    pub(in crate::mir6502) fn live_out(&self, block: MirBlockId) -> Option<MirMachineLiveSet> {
        self.block_by_id(block).map(|facts| facts.live_out)
    }

    pub(in crate::mir6502) fn evaluations(&self) -> usize {
        self.evaluations
    }

    pub(in crate::mir6502) fn live_after(
        &self,
        site: MirSite,
    ) -> Result<MirMachineLiveSet, MirMachineLivenessError> {
        let transfers = self.block_transfers(site.block())?;
        let live_out = self
            .live_out(site.block())
            .ok_or(MirMachineLivenessError::UnknownBlock(site.block()))?;
        match site {
            MirSite::BlockEntry { .. } => self
                .live_in(site.block())
                .ok_or(MirMachineLivenessError::UnknownBlock(site.block())),
            MirSite::Terminator { .. } => Ok(live_out),
            MirSite::Op { block, op_index } => {
                if op_index >= transfers.ops.len() {
                    return Err(MirMachineLivenessError::OpOutOfBounds {
                        block,
                        op_index,
                        op_count: transfers.ops.len(),
                    });
                }
                let mut live = transfers.terminator.apply(live_out);
                for transfer in transfers.ops[op_index + 1..].iter().rev() {
                    live = transfer.apply(live);
                }
                Ok(live)
            }
        }
    }

    pub(in crate::mir6502) fn register_dead_after(
        &self,
        reg: MirReg,
        site: MirSite,
    ) -> Result<bool, MirMachineLivenessError> {
        self.live_after(site).map(|live| !live.register_live(reg))
    }

    pub(in crate::mir6502) fn flags_dead_after(
        &self,
        flags: MirFlagSet,
        site: MirSite,
    ) -> Result<bool, MirMachineLivenessError> {
        self.live_after(site).map(|live| !live.flags_live(flags))
    }

    pub(in crate::mir6502) fn stack_pointer_dead_after(
        &self,
        site: MirSite,
    ) -> Result<bool, MirMachineLivenessError> {
        self.live_after(site).map(|live| !live.stack_pointer_live())
    }

    fn block_transfers(
        &self,
        block: MirBlockId,
    ) -> Result<&MirMachineBlockTransfers, MirMachineLivenessError> {
        self.transfers
            .get(&block)
            .ok_or(MirMachineLivenessError::UnknownBlock(block))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct MirMachineBlockFacts {
    uses: MirMachineLiveSet,
    defs: MirMachineLiveSet,
}

struct MachineLivenessProblem<'a> {
    facts: &'a BTreeMap<MirBlockId, MirMachineBlockFacts>,
    boundaries: &'a BTreeMap<MirBlockId, MirMachineLiveSet>,
}

impl DataflowProblem<MirCfg> for MachineLivenessProblem<'_> {
    type State = MirMachineLiveSet;

    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Backward
    }

    fn bottom(&self) -> Self::State {
        Self::State::default()
    }

    fn boundary(&self, node: MirBlockId) -> Option<Self::State> {
        self.boundaries.get(&node).copied()
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.union_with(*other);
    }

    fn transfer(&self, node: MirBlockId, live_out: &Self::State) -> Self::State {
        let facts = self.facts[&node];
        let mut live_in = live_out.subtract(facts.defs);
        live_in.union_with(facts.uses);
        live_in
    }
}

fn op_transfer(effects: &MirOpEffectSummary) -> MirMachineTransfer {
    let mut reads = machine_reads(&effects.machine);
    let writes = machine_writes(&effects.machine);

    // Conservative clobbers are may-defs: they cannot kill a value that is
    // live after the operation, but they also do not make an otherwise-dead
    // incoming value observable. Keeping them out of both `reads` and the
    // definite `writes` set gives exactly that transfer behavior.
    if effects.machine.uses_previous_carry {
        reads.flags.c = true;
    }
    if matches!(effects.kind, MirOpKind::Call | MirOpKind::RuntimeHelper) {
        reads.registers.sp = true;
    }
    if effects.memory.opaque || matches!(effects.kind, MirOpKind::MachineBlock) {
        reads.union_with(MirMachineLiveSet::all());
    }
    MirMachineTransfer { reads, writes }
}

fn terminator_transfer(effects: &MirTerminatorEffectSummary) -> MirMachineTransfer {
    MirMachineTransfer {
        reads: machine_reads(&effects.machine),
        writes: machine_writes(&effects.machine),
    }
}

fn machine_reads(effects: &MirMachineEffects) -> MirMachineLiveSet {
    MirMachineLiveSet {
        registers: normalized_registers(effects.register_reads),
        flags: effects.flag_reads,
    }
}

fn machine_writes(effects: &MirMachineEffects) -> MirMachineLiveSet {
    let mut registers = normalized_registers(effects.register_writes);
    merge_registers(&mut registers, effects.register_clobbers);
    let mut flags = effects.flag_writes;
    merge_flags(&mut flags, effects.flag_clobbers);
    MirMachineLiveSet { registers, flags }
}

fn block_uses_and_defs(transfers: &MirMachineBlockTransfers) -> MirMachineBlockFacts {
    let mut facts = MirMachineBlockFacts::default();
    for transfer in transfers
        .ops
        .iter()
        .chain(std::iter::once(&transfers.terminator))
    {
        let mut exposed = transfer.reads.subtract(facts.defs);
        facts.uses.union_with(exposed);
        exposed = transfer.writes;
        facts.defs.union_with(exposed);
    }
    facts
}

fn action_return_machine_uses() -> MirMachineLiveSet {
    let mut uses = MirMachineLiveSet::default();
    uses.registers.sp = true;
    uses
}

fn normalized_registers(mut registers: MirRegisterSet) -> MirRegisterSet {
    registers.flags = false;
    registers
}

fn register_is_set(registers: MirRegisterSet, reg: MirReg) -> bool {
    match reg {
        MirReg::A => registers.a,
        MirReg::X => registers.x,
        MirReg::Y => registers.y,
    }
}

fn merge_registers(into: &mut MirRegisterSet, other: MirRegisterSet) {
    into.a |= other.a;
    into.x |= other.x;
    into.y |= other.y;
    into.sp |= other.sp;
    into.flags = false;
}

fn subtract_registers(mut live: MirRegisterSet, defs: MirRegisterSet) -> MirRegisterSet {
    live.a &= !defs.a;
    live.x &= !defs.x;
    live.y &= !defs.y;
    live.sp &= !defs.sp;
    live.flags = false;
    live
}

fn merge_flags(into: &mut MirFlagSet, other: MirFlagSet) {
    into.c |= other.c;
    into.z |= other.z;
    into.n |= other.n;
    into.v |= other.v;
}

fn subtract_flags(mut live: MirFlagSet, defs: MirFlagSet) -> MirFlagSet {
    live.c &= !defs.c;
    live.z &= !defs.z;
    live.n &= !defs.n;
    live.v &= !defs.v;
    live
}

fn flags_intersect(left: MirFlagSet, right: MirFlagSet) -> bool {
    left.c && right.c || left.z && right.z || left.n && right.n || left.v && right.v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirCallAbi, MirCallTarget, MirCond, MirDef, MirEdge, MirEffects, MirFlagTest,
        MirFrame, MirMachineBlockId, MirOp, MirRoutineAbi, MirValue, MirWidth, RoutineId,
    };

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
            name: "MachineLiveness".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn analyze(routine: &MirRoutine) -> MirMachineLiveness {
        let cfg = MirCfg::from_routine(routine).unwrap();
        MirMachineLiveness::analyze(routine, &cfg)
    }

    fn op_site(block: u32, op_index: usize) -> MirSite {
        MirSite::Op {
            block: MirBlockId(block),
            op_index,
        }
    }

    fn load_imm(reg: MirReg, value: u16) -> MirOp {
        MirOp::LoadImm {
            dst: MirDef::Reg(reg),
            value,
            width: MirWidth::Byte,
        }
    }

    fn branch(test: MirFlagTest, then_block: u32, else_block: u32) -> MirTerminator {
        MirTerminator::Branch {
            cond: MirCond::FlagTest(test),
            then_edge: MirEdge::plain(MirBlockId(then_block)),
            else_edge: MirEdge::plain(MirBlockId(else_block)),
        }
    }

    #[test]
    fn flag_tests_keep_only_their_individual_condition_code_live() {
        let routine = routine(vec![
            block(
                0,
                vec![load_imm(MirReg::A, 1)],
                branch(MirFlagTest::ZSet, 1, 2),
            ),
            block(1, Vec::new(), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
        ]);
        let liveness = analyze(&routine);
        let live = liveness.live_after(op_site(0, 0)).unwrap();
        assert!(live.flag_live(MirFlag::Z));
        assert!(!live.flag_live(MirFlag::N));
        assert!(!live.flag_live(MirFlag::C));
        assert!(!live.flag_live(MirFlag::V));
    }

    #[test]
    fn carry_overflow_and_negative_tests_do_not_collapse_together() {
        for (test, expected) in [
            (MirFlagTest::CSet, MirFlag::C),
            (MirFlagTest::VSet, MirFlag::V),
            (MirFlagTest::NSet, MirFlag::N),
        ] {
            let routine = routine(vec![
                block(0, Vec::new(), branch(test, 1, 2)),
                block(1, Vec::new(), MirTerminator::Return),
                block(2, Vec::new(), MirTerminator::Return),
            ]);
            let liveness = analyze(&routine);
            let live = liveness.live_in(MirBlockId(0)).unwrap();
            assert!(live.flag_live(expected));
            for other in [MirFlag::C, MirFlag::Z, MirFlag::N, MirFlag::V] {
                assert_eq!(live.flag_live(other), other == expected);
            }
        }
    }

    #[test]
    fn successor_read_keeps_register_live_and_definite_overwrite_kills_it() {
        let read = routine(vec![
            block(
                0,
                vec![load_imm(MirReg::A, 1)],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(
                1,
                vec![MirOp::Move {
                    dst: MirDef::Reg(MirReg::X),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            ),
        ]);
        let liveness = analyze(&read);
        assert_eq!(
            liveness.register_dead_after(MirReg::A, op_site(0, 0)),
            Ok(false)
        );

        let overwritten = routine(vec![
            block(
                0,
                vec![load_imm(MirReg::A, 1)],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(
                1,
                vec![
                    load_imm(MirReg::A, 2),
                    MirOp::Move {
                        dst: MirDef::Reg(MirReg::X),
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        width: MirWidth::Byte,
                    },
                ],
                MirTerminator::Return,
            ),
        ]);
        let liveness = analyze(&overwritten);
        assert_eq!(
            liveness.register_dead_after(MirReg::A, op_site(0, 0)),
            Ok(true)
        );
    }

    #[test]
    fn incomplete_call_clobbers_are_may_defs_not_unconditional_uses() {
        let call = || MirOp::Call {
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
        };
        let dead_after_call = routine(vec![block(
            0,
            vec![load_imm(MirReg::A, 1), call()],
            MirTerminator::Return,
        )]);
        assert_eq!(
            analyze(&dead_after_call).register_dead_after(MirReg::A, op_site(0, 0)),
            Ok(true)
        );

        let live_through_call = routine(vec![block(
            0,
            vec![
                load_imm(MirReg::A, 1),
                call(),
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::X),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ],
            MirTerminator::Return,
        )]);
        assert_eq!(
            analyze(&live_through_call).register_dead_after(MirReg::A, op_site(0, 0)),
            Ok(false)
        );
    }

    #[test]
    fn opaque_machine_blocks_observe_all_incoming_machine_state() {
        let routine = routine(vec![block(
            0,
            vec![
                load_imm(MirReg::A, 1),
                MirOp::MachineBlock {
                    id: MirMachineBlockId(0),
                    effects: MirEffects {
                        opaque: true,
                        ..MirEffects::default()
                    },
                },
            ],
            MirTerminator::Return,
        )]);
        let liveness = analyze(&routine);
        let live = liveness.live_after(op_site(0, 0)).unwrap();
        assert!(live.register_live(MirReg::A));
        assert!(live.register_live(MirReg::X));
        assert!(live.register_live(MirReg::Y));
        assert!(live.stack_pointer_live());
        assert!(live.flags_live(MirFlagSet::all()));
    }

    #[test]
    fn return_boundary_keeps_stack_pointer_but_not_volatile_registers_live() {
        let routine = routine(vec![block(0, Vec::new(), MirTerminator::Return)]);
        let liveness = analyze(&routine);
        let live_out = liveness.live_out(MirBlockId(0)).unwrap();
        assert!(live_out.stack_pointer_live());
        assert!(!live_out.register_live(MirReg::A));
        assert!(!live_out.register_live(MirReg::X));
        assert!(!live_out.register_live(MirReg::Y));
        assert!(!live_out.flags_live(MirFlagSet::all()));
    }
}
