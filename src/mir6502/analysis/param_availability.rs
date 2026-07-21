#![allow(dead_code)] // Rewrite-family migration consumes these facts in Slice 8.

use std::collections::BTreeMap;

use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::{MirMemoryRange, MirOpEffectSummary, classify_op};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{
    MirAddr, MirBlockId, MirMem, MirMemoryEffect, MirMemoryRegionKind, MirOp, MirReg,
    MirRegisterSet, MirRoutine, MirValue, MirWidth,
};
use crate::nir::ParamId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) struct MirParamHomeByte {
    pub param: ParamId,
    pub offset: u16,
}

impl MirParamHomeByte {
    pub(in crate::mir6502) fn from_mem(mem: &MirMem) -> Option<Self> {
        match mem {
            MirMem::Param { id, offset } => Some(Self {
                param: *id,
                offset: *offset,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirParamRegisterSet {
    homes: BTreeMap<MirParamHomeByte, MirReg>,
}

impl MirParamRegisterSet {
    pub(in crate::mir6502) fn register_for(&self, home: MirParamHomeByte) -> Option<MirReg> {
        self.homes.get(&home).copied()
    }

    pub(in crate::mir6502) fn iter(&self) -> impl Iterator<Item = (MirParamHomeByte, MirReg)> + '_ {
        self.homes.iter().map(|(home, reg)| (*home, *reg))
    }

    pub(in crate::mir6502) fn len(&self) -> usize {
        self.homes.len()
    }

    fn retain_identical(&mut self, other: &Self) {
        self.homes
            .retain(|home, reg| other.homes.get(home) == Some(reg));
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MirParamAvailabilityState {
    reachable: bool,
    available: MirParamRegisterSet,
}

impl MirParamAvailabilityState {
    fn reachable_empty() -> Self {
        Self {
            reachable: true,
            available: MirParamRegisterSet::default(),
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
        self.available.retain_identical(&other.available);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirParamAvailabilityBlock {
    pub available_in: MirParamRegisterSet,
    pub available_out: MirParamRegisterSet,
    pub reachable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirParamAvailabilityError {
    UnknownBlock(MirBlockId),
    OpOutOfBounds {
        block: MirBlockId,
        op_index: usize,
        op_count: usize,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirParamRegisterAvailability {
    blocks: Vec<MirParamAvailabilityBlock>,
    block_indices: BTreeMap<MirBlockId, usize>,
    ops: BTreeMap<MirBlockId, Vec<MirOp>>,
    entry: Option<MirBlockId>,
    evaluations: usize,
}

impl MirParamRegisterAvailability {
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine, cfg: &MirCfg) -> Self {
        let entry = cfg.entry();
        let result = solve_dataflow(cfg, &ParamAvailabilityProblem { routine, entry });
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
                MirParamAvailabilityBlock {
                    available_in: input.available,
                    available_out: output.available,
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
            entry,
            evaluations: result.evaluations(),
        }
    }

    pub(in crate::mir6502) fn block_by_id(
        &self,
        block: MirBlockId,
    ) -> Option<&MirParamAvailabilityBlock> {
        self.block_indices
            .get(&block)
            .and_then(|index| self.blocks.get(*index))
    }

    pub(in crate::mir6502) fn available_at(
        &self,
        site: MirSite,
    ) -> Result<MirParamRegisterSet, MirParamAvailabilityError> {
        let block = site.block();
        let facts = self
            .block_by_id(block)
            .ok_or(MirParamAvailabilityError::UnknownBlock(block))?;
        let ops = self
            .ops
            .get(&block)
            .ok_or(MirParamAvailabilityError::UnknownBlock(block))?;
        let limit = match site {
            MirSite::BlockEntry { .. } => 0,
            MirSite::Op { op_index, .. } => {
                if op_index >= ops.len() {
                    return Err(MirParamAvailabilityError::OpOutOfBounds {
                        block,
                        op_index,
                        op_count: ops.len(),
                    });
                }
                op_index
            }
            MirSite::Terminator { .. } => ops.len(),
        };
        let mut state = MirParamAvailabilityState {
            reachable: facts.reachable,
            available: facts.available_in.clone(),
        };
        for op in &ops[..limit] {
            apply_op(&mut state, op);
        }
        Ok(state.available)
    }

    pub(in crate::mir6502) fn register_at(
        &self,
        home: MirParamHomeByte,
        site: MirSite,
    ) -> Result<Option<MirReg>, MirParamAvailabilityError> {
        self.available_at(site)
            .map(|available| available.register_for(home))
    }

    pub(in crate::mir6502) fn entry(&self) -> Option<MirBlockId> {
        self.entry
    }

    pub(in crate::mir6502) fn evaluations(&self) -> usize {
        self.evaluations
    }
}

struct ParamAvailabilityProblem<'a> {
    routine: &'a MirRoutine,
    entry: Option<MirBlockId>,
}

impl DataflowProblem<MirCfg> for ParamAvailabilityProblem<'_> {
    type State = MirParamAvailabilityState;

    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Forward
    }

    fn bottom(&self) -> Self::State {
        Self::State::default()
    }

    fn boundary(&self, node: MirBlockId) -> Option<Self::State> {
        (Some(node) == self.entry).then(MirParamAvailabilityState::reachable_empty)
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
        output
    }
}

fn apply_op(state: &mut MirParamAvailabilityState, op: &MirOp) {
    if !state.reachable {
        return;
    }
    let effects = classify_op(op);
    state.available.homes.retain(|home, reg| {
        !register_invalidated(&effects, *reg) && !param_home_may_be_written(&effects, *home)
    });

    if let MirOp::Store {
        dst: MirAddr::Direct(dst),
        src: MirValue::Def(crate::mir6502::ir::MirDef::Reg(reg)),
        width: MirWidth::Byte,
    } = op
        && let Some(home) = MirParamHomeByte::from_mem(dst)
    {
        state.available.homes.insert(home, *reg);
    }
}

fn register_invalidated(effects: &MirOpEffectSummary, reg: MirReg) -> bool {
    register_is_set(effects.machine.register_writes, reg)
        || register_is_set(effects.machine.register_clobbers, reg)
        || register_is_set(effects.machine.conservative_register_clobbers, reg)
}

fn param_home_may_be_written(effects: &MirOpEffectSummary, home: MirParamHomeByte) -> bool {
    effects.memory.opaque
        || effects.memory.indirect_writes
        || effects
            .memory
            .direct_writes
            .iter()
            .any(|range| direct_range_contains_param(range, home))
        || structured_effect_may_write_param(&effects.memory.structured_writes, home)
}

fn direct_range_contains_param(range: &MirMemoryRange, home: MirParamHomeByte) -> bool {
    let MirMem::Param { id, offset } = range.base else {
        return false;
    };
    id == home.param && home.offset >= offset && home.offset < offset.saturating_add(range.bytes)
}

fn structured_effect_may_write_param(effect: &MirMemoryEffect, home: MirParamHomeByte) -> bool {
    match effect {
        MirMemoryEffect::None => false,
        MirMemoryEffect::Unknown | MirMemoryEffect::All => true,
        MirMemoryEffect::Regions(regions) => regions.iter().any(|region| match region.kind {
            MirMemoryRegionKind::Param(id) if id == home.param => {
                home.offset >= region.offset
                    && home.offset < region.offset.saturating_add(region.size)
            }
            MirMemoryRegionKind::AbsoluteRange | MirMemoryRegionKind::ZeroPage => true,
            MirMemoryRegionKind::Param(_)
            | MirMemoryRegionKind::Local(_)
            | MirMemoryRegionKind::Global(_)
            | MirMemoryRegionKind::Static(_)
            | MirMemoryRegionKind::Stack => false,
        }),
    }
}

fn register_is_set(registers: MirRegisterSet, reg: MirReg) -> bool {
    match reg {
        MirReg::A => registers.a,
        MirReg::X => registers.x,
        MirReg::Y => registers.y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirCallAbi, MirCallTarget, MirDef, MirEdge, MirEffects, MirFrame,
        MirMemoryRegion, MirRoutineAbi, MirTerminator, RoutineId,
    };

    fn param(param: u32, offset: u16) -> MirMem {
        MirMem::Param {
            id: ParamId(param),
            offset,
        }
    }

    fn home(param: u32, offset: u16) -> MirParamHomeByte {
        MirParamHomeByte {
            param: ParamId(param),
            offset,
        }
    }

    fn capture(param_id: u32, offset: u16, reg: MirReg) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(param(param_id, offset)),
            src: MirValue::Def(MirDef::Reg(reg)),
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
            name: "ParamAvailability".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn analyze(routine: &MirRoutine) -> MirParamRegisterAvailability {
        let cfg = MirCfg::from_routine(routine).unwrap();
        MirParamRegisterAvailability::analyze(routine, &cfg)
    }

    #[test]
    fn availability_flows_to_successors_and_register_writes_kill_it() {
        let routine = routine(vec![
            block(
                0,
                vec![capture(0, 0, MirReg::A)],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(
                1,
                vec![MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 7,
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            ),
        ]);
        let availability = analyze(&routine);
        assert_eq!(
            availability.register_at(
                home(0, 0),
                MirSite::BlockEntry {
                    block: MirBlockId(1)
                }
            ),
            Ok(Some(MirReg::A))
        );
        assert_eq!(
            availability.register_at(
                home(0, 0),
                MirSite::Terminator {
                    block: MirBlockId(1)
                }
            ),
            Ok(None)
        );
    }

    #[test]
    fn joins_retain_only_identical_register_facts() {
        let build = |right_reg| {
            routine(vec![
                block(
                    0,
                    Vec::new(),
                    MirTerminator::Branch {
                        cond: crate::mir6502::ir::MirCond::BoolValue(MirValue::ConstU8(1)),
                        then_edge: MirEdge::plain(MirBlockId(1)),
                        else_edge: MirEdge::plain(MirBlockId(2)),
                    },
                ),
                block(
                    1,
                    vec![capture(0, 0, MirReg::A)],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
                block(
                    2,
                    vec![capture(0, 0, right_reg)],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
                block(3, Vec::new(), MirTerminator::Return),
            ])
        };
        let same = analyze(&build(MirReg::A));
        assert_eq!(
            same.register_at(
                home(0, 0),
                MirSite::BlockEntry {
                    block: MirBlockId(3)
                }
            ),
            Ok(Some(MirReg::A))
        );
        let different = analyze(&build(MirReg::X));
        assert_eq!(
            different.register_at(
                home(0, 0),
                MirSite::BlockEntry {
                    block: MirBlockId(3)
                }
            ),
            Ok(None)
        );
    }

    #[test]
    fn structured_global_writes_preserve_param_facts_but_param_writes_kill() {
        let call = |memory_writes| MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(1)),
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet {
                    x: true,
                    y: true,
                    flags: true,
                    ..MirRegisterSet::default()
                },
                preserves: MirRegisterSet {
                    a: true,
                    ..MirRegisterSet::default()
                },
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects {
                memory_writes,
                clobbers: MirRegisterSet {
                    x: true,
                    y: true,
                    flags: true,
                    ..MirRegisterSet::default()
                },
                preserves: MirRegisterSet {
                    a: true,
                    ..MirRegisterSet::default()
                },
                ..MirEffects::default()
            },
        };
        let global_write = MirMemoryEffect::Regions(vec![MirMemoryRegion {
            kind: MirMemoryRegionKind::Global(crate::nir::SymbolId(0)),
            offset: 0,
            size: 1,
        }]);
        let param_write = MirMemoryEffect::Regions(vec![MirMemoryRegion {
            kind: MirMemoryRegionKind::Param(ParamId(0)),
            offset: 0,
            size: 1,
        }]);
        let routine = routine(vec![block(
            0,
            vec![
                capture(0, 0, MirReg::A),
                call(global_write),
                call(param_write),
            ],
            MirTerminator::Return,
        )]);
        let availability = analyze(&routine);
        assert_eq!(
            availability.register_at(
                home(0, 0),
                MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 2
                }
            ),
            Ok(Some(MirReg::A))
        );
        assert_eq!(
            availability.register_at(
                home(0, 0),
                MirSite::Terminator {
                    block: MirBlockId(0)
                }
            ),
            Ok(None)
        );
    }

    #[test]
    fn call_register_clobbers_invalidate_captured_param_values() {
        let clobbers = MirRegisterSet {
            a: true,
            x: true,
            y: true,
            flags: true,
            ..MirRegisterSet::default()
        };
        let routine = routine(vec![block(
            0,
            vec![
                capture(0, 0, MirReg::A),
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: MirCallAbi {
                        params: Vec::new(),
                        result: None,
                        clobbers,
                        preserves: MirRegisterSet::default(),
                    },
                    args: Vec::new(),
                    result: None,
                    effects: MirEffects {
                        clobbers,
                        ..MirEffects::default()
                    },
                },
            ],
            MirTerminator::Return,
        )]);
        let availability = analyze(&routine);
        assert_eq!(
            availability.register_at(
                home(0, 0),
                MirSite::Terminator {
                    block: MirBlockId(0)
                }
            ),
            Ok(None)
        );
    }

    #[test]
    fn loops_converge_without_inventing_availability() {
        let routine = routine(vec![
            block(
                0,
                vec![capture(0, 0, MirReg::A)],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(
                1,
                Vec::new(),
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
        ]);
        let availability = analyze(&routine);
        assert_eq!(
            availability.register_at(
                home(0, 0),
                MirSite::BlockEntry {
                    block: MirBlockId(1)
                }
            ),
            Ok(Some(MirReg::A))
        );
        assert!(availability.evaluations() >= 2);
    }
}
