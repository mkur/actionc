use std::collections::BTreeMap;

use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::known_callees::MirKnownCalleeSummaries;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{
    MirAddr, MirBlockId, MirCond, MirDef, MirFixedZpSlot, MirMem, MirMemoryEffect, MirOp, MirReg,
    MirRoutine, MirTerminator, MirValue, MirWidth,
};

/// A value known to occupy a physical 6502 register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirMachineValue {
    ConstU8(u8),
    DirectMem(MirMem),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MirMachineValueState {
    reachable: bool,
    a: Option<MirMachineValue>,
    x: Option<MirMachineValue>,
    y: Option<MirMachineValue>,
    /// The byte value whose most recent load/result established both Z and N.
    ///
    /// This is deliberately an exact provenance fact, not a general model of
    /// flag values.  Any operation that writes or clobbers either flag without
    /// a known byte result drops the fact.
    zn: Option<MirMachineValue>,
    /// Register whose current byte established Z/N, even when the byte itself
    /// has no stable memory or constant identity yet.
    zn_register: Option<MirReg>,
    fixed_zero_page: BTreeMap<MirFixedZpSlot, MirMachineValue>,
}

impl MirMachineValueState {
    fn reachable_unknown() -> Self {
        Self {
            reachable: true,
            a: None,
            x: None,
            y: None,
            zn: None,
            zn_register: None,
            fixed_zero_page: BTreeMap::new(),
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
        if self.x != other.x {
            self.x = None;
        }
        if self.y != other.y {
            self.y = None;
        }
        if self.zn != other.zn {
            self.zn = None;
        }
        if self.zn_register != other.zn_register {
            self.zn_register = None;
        }
        self.fixed_zero_page
            .retain(|slot, value| other.fixed_zero_page.get(slot) == Some(value));
    }

    fn register(&self, reg: MirReg) -> Option<&MirMachineValue> {
        match reg {
            MirReg::A => self.a.as_ref(),
            MirReg::X => self.x.as_ref(),
            MirReg::Y => self.y.as_ref(),
        }
    }

    fn set_register(&mut self, reg: MirReg, value: Option<MirMachineValue>) {
        match reg {
            MirReg::A => self.a = value,
            MirReg::X => self.x = value,
            MirReg::Y => self.y = value,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirMachineValueBlock {
    pub accumulator_in: Option<MirMachineValue>,
    pub accumulator_out: Option<MirMachineValue>,
    pub x_in: Option<MirMachineValue>,
    pub x_out: Option<MirMachineValue>,
    pub y_in: Option<MirMachineValue>,
    pub y_out: Option<MirMachineValue>,
    zn_in: Option<MirMachineValue>,
    zn_out: Option<MirMachineValue>,
    zn_register_in: Option<MirReg>,
    zn_register_out: Option<MirReg>,
    pub reachable: bool,
    fixed_zero_page_in: BTreeMap<MirFixedZpSlot, MirMachineValue>,
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
    known_callees: MirKnownCalleeSummaries,
    evaluations: usize,
}

impl MirMachineValueAvailability {
    #[allow(dead_code)] // Default-summary analysis remains useful to focused tests and callers.
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine, cfg: &MirCfg) -> Self {
        Self::analyze_with_known_callees(routine, cfg, &MirKnownCalleeSummaries::default())
    }

    pub(in crate::mir6502) fn analyze_with_known_callees(
        routine: &MirRoutine,
        cfg: &MirCfg,
        known_callees: &MirKnownCalleeSummaries,
    ) -> Self {
        let entry = cfg.entry();
        let result = solve_dataflow(
            cfg,
            &MachineValueProblem {
                routine,
                entry,
                known_callees,
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
                let input = result.in_state(block.id).cloned().unwrap_or_default();
                let output = result.out_state(block.id).cloned().unwrap_or_default();
                MirMachineValueBlock {
                    accumulator_in: input.a,
                    accumulator_out: output.a,
                    x_in: input.x,
                    x_out: output.x,
                    y_in: input.y,
                    y_out: output.y,
                    zn_in: input.zn,
                    zn_out: output.zn,
                    zn_register_in: input.zn_register,
                    zn_register_out: output.zn_register,
                    reachable: input.reachable,
                    fixed_zero_page_in: input.fixed_zero_page,
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
            known_callees: known_callees.clone(),
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
        self.register_at(site, MirReg::A)
    }

    pub(in crate::mir6502) fn register_at(
        &self,
        site: MirSite,
        reg: MirReg,
    ) -> Result<Option<MirMachineValue>, MirMachineValueError> {
        Ok(self.state_at(site)?.register(reg).cloned())
    }

    pub(in crate::mir6502) fn zn_value_at(
        &self,
        site: MirSite,
    ) -> Result<Option<MirMachineValue>, MirMachineValueError> {
        Ok(self.state_at(site)?.zn)
    }

    pub(in crate::mir6502) fn fixed_zero_page_value_at(
        &self,
        site: MirSite,
        slot: MirFixedZpSlot,
    ) -> Result<Option<MirMachineValue>, MirMachineValueError> {
        Ok(self.state_at(site)?.fixed_zero_page.get(&slot).cloned())
    }

    fn state_at(&self, site: MirSite) -> Result<MirMachineValueState, MirMachineValueError> {
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
            x: facts.x_in.clone(),
            y: facts.y_in.clone(),
            zn: facts.zn_in.clone(),
            zn_register: facts.zn_register_in,
            fixed_zero_page: facts.fixed_zero_page_in.clone(),
        };
        for op in &ops[..limit] {
            apply_op(&mut state, op, &self.known_callees);
        }
        Ok(state)
    }

    #[cfg(test)]
    pub(in crate::mir6502) fn evaluations(&self) -> usize {
        self.evaluations
    }
}

struct MachineValueProblem<'a> {
    routine: &'a MirRoutine,
    entry: Option<MirBlockId>,
    known_callees: &'a MirKnownCalleeSummaries,
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
            apply_op(&mut output, op, self.known_callees);
        }
        if !terminator_preserves_accumulator(&block.terminator) {
            output.a = None;
        }
        if !terminator_preserves_flags(&block.terminator) {
            output.zn = None;
            output.zn_register = None;
        }
        output
    }
}

fn apply_op(state: &mut MirMachineValueState, op: &MirOp, known_callees: &MirKnownCalleeSummaries) {
    if !state.reachable {
        return;
    }

    let before = state.clone();
    update_fixed_zero_page_values(state, op, &before, known_callees);

    let effects = classify_op(op);
    let writes_memory = !effects.memory.direct_writes.is_empty()
        || effects.memory.indirect_writes
        || effects.memory.opaque
        || effects.memory.may_write_any
        || effects.memory.has_unknown_effects
        || !matches!(&effects.memory.structured_writes, MirMemoryEffect::None);
    if writes_memory {
        for reg in [MirReg::A, MirReg::X, MirReg::Y] {
            if matches!(state.register(reg), Some(MirMachineValue::DirectMem(_))) {
                state.set_register(reg, None);
            }
        }
        if matches!(state.zn, Some(MirMachineValue::DirectMem(_))) {
            state.zn = None;
        }
    } else if let MirOp::Call { target, .. } = op {
        for reg in [MirReg::X, MirReg::Y] {
            let Some(MirMachineValue::DirectMem(mem)) = state.register(reg) else {
                continue;
            };
            let invalidated = known_callees
                .for_target(target)
                .is_none_or(|summary| summary.writes().may_write_mem(mem));
            if invalidated {
                state.set_register(reg, None);
            }
        }
    }

    if let Some(value) = explicit_accumulator_result(op, known_callees) {
        state.a = Some(value);
    } else if !operation_preserves_accumulator(op) {
        state.a = None;
    }

    for reg in [MirReg::X, MirReg::Y] {
        if let Some(value) = explicit_index_register_result(op, reg, &before) {
            state.set_register(reg, Some(value));
        } else if effects.may_clobber_reg_compat(reg)
            && !known_call_preserves_register(op, reg, known_callees)
        {
            state.set_register(reg, None);
        }
    }

    if state
        .zn_register
        .is_some_and(|reg| effects.may_clobber_reg_compat(reg))
    {
        state.zn_register = None;
    }
    let zn_register = explicit_zn_register_result(op, known_callees);
    if let Some(value) = explicit_zn_result(op, &before, known_callees) {
        state.zn = Some(value);
        state.zn_register = zn_register;
    } else if effects.machine.flag_writes.z
        || effects.machine.flag_writes.n
        || effects.machine.flag_clobbers.z
        || effects.machine.flag_clobbers.n
        || effects.machine.unknown_flag_or_a_effects
        || effects.machine.opaque_flag_or_a_effects
    {
        state.zn = None;
        state.zn_register = zn_register;
    }

    if let MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(reg)),
        width: MirWidth::Byte,
    } = op
    {
        state.set_register(*reg, Some(MirMachineValue::DirectMem(mem.clone())));
        if before.zn == before.register(*reg).cloned() || before.zn_register == Some(*reg) {
            state.zn = Some(MirMachineValue::DirectMem(mem.clone()));
            state.zn_register = Some(*reg);
        }
    }
}

fn known_call_preserves_register(
    op: &MirOp,
    register: MirReg,
    known_callees: &MirKnownCalleeSummaries,
) -> bool {
    let MirOp::Call { target, .. } = op else {
        return false;
    };
    known_callees
        .for_target(target)
        .is_some_and(|summary| summary.preserves_register(register))
}

fn update_fixed_zero_page_values(
    state: &mut MirMachineValueState,
    op: &MirOp,
    before: &MirMachineValueState,
    known_callees: &MirKnownCalleeSummaries,
) {
    match op {
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            src,
            width: MirWidth::Byte,
        } => {
            invalidate_fixed_zero_page_value_dependencies(state, mem);
            let Some(slot) = fixed_zero_page_slot(mem) else {
                return;
            };
            state.fixed_zero_page.remove(&slot);
            if let Some(value) = stored_machine_value(src, before) {
                state.fixed_zero_page.insert(slot, value);
            }
        }
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            width: MirWidth::Word,
            ..
        }
        | MirOp::UpdateMem {
            mem,
            width: MirWidth::Word,
            ..
        }
        | MirOp::AddByteToWordMem { mem, .. }
        | MirOp::SubByteFromWordMem { mem, .. }
        | MirOp::OffsetPointerByIndirectByte { dst: mem, .. } => {
            invalidate_fixed_zero_page_value_dependencies(state, mem);
            invalidate_fixed_zero_page_value_dependencies(state, &offset_mem(mem, 1));
            remove_fixed_zero_page_slot(state, mem);
            remove_fixed_zero_page_slot(state, &offset_mem(mem, 1));
        }
        MirOp::UpdateMem { mem, .. } => {
            invalidate_fixed_zero_page_value_dependencies(state, mem);
            remove_fixed_zero_page_slot(state, mem);
        }
        MirOp::Store { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyDirectWordToIndirect { .. }
        | MirOp::CopyIndirectBytesToFixedZp { .. }
        | MirOp::AbsoluteWordSubToIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => state.fixed_zero_page.clear(),
        MirOp::Call { target, .. } => {
            let Some(summary) = known_callees.for_target(target) else {
                state.fixed_zero_page.clear();
                return;
            };
            state.fixed_zero_page.retain(|slot, value| {
                let destination = MirMem::FixedZeroPage(*slot);
                !summary.writes().may_write_mem(&destination)
                    && match value {
                        MirMachineValue::ConstU8(_) => true,
                        MirMachineValue::DirectMem(source) => {
                            !summary.writes().may_write_mem(source)
                        }
                    }
            });
        }
        MirOp::MaterializeAddress { consumer, .. }
        | MirOp::MaterializeIndexedAddress { consumer, .. }
        | MirOp::AdvanceAddress { consumer, .. } => {
            let crate::mir6502::ir::MirPointerPair::Fixed { lo } = consumer.pointer_pair() else {
                return;
            };
            state.fixed_zero_page.remove(&lo);
            state
                .fixed_zero_page
                .remove(&MirFixedZpSlot(lo.0.saturating_add(1)));
        }
        MirOp::LoadImm { .. }
        | MirOp::Load { .. }
        | MirOp::Move { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::Extend { .. }
        | MirOp::Truncate { .. }
        | MirOp::Unary { .. }
        | MirOp::Binary { .. }
        | MirOp::Compare { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::LoadIndirect { .. } => {}
    }
}

fn stored_machine_value(
    value: &MirValue,
    before: &MirMachineValueState,
) -> Option<MirMachineValue> {
    match value {
        MirValue::Def(MirDef::Reg(reg)) => before.register(*reg).cloned(),
        MirValue::ConstU8(value) => Some(MirMachineValue::ConstU8(*value)),
        MirValue::ConstU16(value) => u8::try_from(*value).ok().map(MirMachineValue::ConstU8),
        MirValue::PointerCell(mem) => Some(MirMachineValue::DirectMem(mem.clone())),
        _ => None,
    }
}

fn invalidate_fixed_zero_page_value_dependencies(
    state: &mut MirMachineValueState,
    written: &MirMem,
) {
    if !matches!(
        written,
        MirMem::FixedZeroPage(_) | MirMem::Spill { .. } | MirMem::ZeroPage(_)
    ) {
        // Without final layout, distinct named/absolute storage identities can
        // still be aliases. Preserve only facts independent of memory.
        state
            .fixed_zero_page
            .retain(|_, value| matches!(value, MirMachineValue::ConstU8(_)));
        return;
    }
    state.fixed_zero_page.retain(|_, value| {
        !matches!(value, MirMachineValue::DirectMem(source) if mems_may_be_same(source, written))
    });
}

fn remove_fixed_zero_page_slot(state: &mut MirMachineValueState, mem: &MirMem) {
    if let Some(slot) = fixed_zero_page_slot(mem) {
        state.fixed_zero_page.remove(&slot);
    }
}

fn fixed_zero_page_slot(mem: &MirMem) -> Option<MirFixedZpSlot> {
    match mem {
        MirMem::FixedZeroPage(slot) => Some(*slot),
        MirMem::Absolute(address) => u8::try_from(*address).ok().map(MirFixedZpSlot),
        _ => None,
    }
}

fn mems_may_be_same(left: &MirMem, right: &MirMem) -> bool {
    left == right
        || fixed_zero_page_slot(left)
            .zip(fixed_zero_page_slot(right))
            .is_some_and(|(left, right)| left == right)
}

fn offset_mem(mem: &MirMem, offset: u16) -> MirMem {
    match mem {
        MirMem::Absolute(address) => MirMem::Absolute(address.saturating_add(offset)),
        MirMem::Static {
            id,
            offset: current,
        } => MirMem::Static {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::Global {
            id,
            offset: current,
        } => MirMem::Global {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::Local {
            id,
            offset: current,
        } => MirMem::Local {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::Param {
            id,
            offset: current,
        } => MirMem::Param {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::Spill {
            id,
            offset: current,
        } => MirMem::Spill {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::ZeroPage(slot) => MirMem::ZeroPage(*slot),
        MirMem::FixedZeroPage(slot) => MirMem::FixedZeroPage(MirFixedZpSlot(
            slot.0
                .saturating_add(u8::try_from(offset).unwrap_or(u8::MAX)),
        )),
    }
}

fn operation_preserves_accumulator(op: &MirOp) -> bool {
    match op {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::X | MirReg::Y),
            width: MirWidth::Byte,
            ..
        }
        | MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X | MirReg::Y),
            width: MirWidth::Byte,
            ..
        }
        | MirOp::Store {
            dst: MirAddr::Direct(_),
            src: MirValue::Def(MirDef::Reg(MirReg::A | MirReg::X | MirReg::Y)),
            width: MirWidth::Byte,
        }
        | MirOp::UpdateMem {
            width: MirWidth::Byte,
            ..
        }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::Compare {
            left: MirValue::Def(MirDef::Reg(MirReg::A)),
            ..
        } => true,
        _ => false,
    }
}

fn explicit_accumulator_result(
    op: &MirOp,
    known_callees: &MirKnownCalleeSummaries,
) -> Option<MirMachineValue> {
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
        MirOp::Call { target, .. } => known_callees
            .for_target(target)
            .and_then(|summary| summary.accumulator().cloned()),
        _ => None,
    }
}

fn explicit_index_register_result(
    op: &MirOp,
    reg: MirReg,
    before: &MirMachineValueState,
) -> Option<MirMachineValue> {
    debug_assert!(matches!(reg, MirReg::X | MirReg::Y));
    match op {
        MirOp::LoadImm {
            dst: MirDef::Reg(dst),
            value,
            width: MirWidth::Byte,
        } if *dst == reg => u8::try_from(*value).ok().map(MirMachineValue::ConstU8),
        MirOp::Load {
            dst: MirDef::Reg(dst),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } if *dst == reg => Some(MirMachineValue::DirectMem(mem.clone())),
        MirOp::Move {
            dst: MirDef::Reg(dst),
            src,
            width: MirWidth::Byte,
        } if *dst == reg => machine_value_for_value(src, before),
        _ => None,
    }
}

fn explicit_zn_result(
    op: &MirOp,
    before: &MirMachineValueState,
    known_callees: &MirKnownCalleeSummaries,
) -> Option<MirMachineValue> {
    match op {
        MirOp::LoadImm {
            dst: MirDef::Reg(_),
            value,
            width: MirWidth::Byte,
        } => u8::try_from(*value).ok().map(MirMachineValue::ConstU8),
        MirOp::Load {
            dst: MirDef::Reg(_),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        } => Some(MirMachineValue::DirectMem(mem.clone())),
        MirOp::Move {
            dst: MirDef::Reg(_),
            src,
            width: MirWidth::Byte,
        } => machine_value_for_value(src, before),
        MirOp::Call { target, .. } => known_callees
            .for_target(target)
            .and_then(|summary| summary.zn().cloned()),
        _ => None,
    }
}

fn explicit_zn_register_result(
    op: &MirOp,
    known_callees: &MirKnownCalleeSummaries,
) -> Option<MirReg> {
    match op {
        MirOp::LoadImm {
            dst: MirDef::Reg(reg),
            width: MirWidth::Byte,
            ..
        }
        | MirOp::Load {
            dst: MirDef::Reg(reg),
            width: MirWidth::Byte,
            ..
        }
        | MirOp::Move {
            dst: MirDef::Reg(reg),
            width: MirWidth::Byte,
            ..
        }
        | MirOp::LoadIndirect {
            dst: MirDef::Reg(reg),
            ..
        } => Some(*reg),
        MirOp::Call { target, .. } => known_callees.for_target(target).and_then(|summary| {
            summary
                .zn()
                .filter(|zn| Some(*zn) == summary.accumulator())
                .map(|_| MirReg::A)
        }),
        _ => None,
    }
}

fn machine_value_for_value(
    value: &MirValue,
    before: &MirMachineValueState,
) -> Option<MirMachineValue> {
    match value {
        MirValue::Def(MirDef::Reg(reg)) => before.register(*reg).cloned(),
        MirValue::ConstU8(value) => Some(MirMachineValue::ConstU8(*value)),
        MirValue::ConstU16(value) => u8::try_from(*value).ok().map(MirMachineValue::ConstU8),
        MirValue::PointerCell(mem) => Some(MirMachineValue::DirectMem(mem.clone())),
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

fn terminator_preserves_flags(terminator: &MirTerminator) -> bool {
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
        MirProgram, MirRegisterSet, MirRoutineAbi, MirSpillId, RoutineId,
    };

    fn spill(id: u32) -> MirMem {
        MirMem::Spill {
            id: MirSpillId(id),
            offset: 0,
        }
    }

    fn load_a(mem: MirMem) -> MirOp {
        load_reg(MirReg::A, mem)
    }

    fn load_reg(reg: MirReg, mem: MirMem) -> MirOp {
        MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }
    }

    fn store_a(mem: MirMem) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(mem),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
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

    fn call(routine: u32) -> MirOp {
        MirOp::Call {
            target: MirCallTarget::Routine(RoutineId(routine)),
            abi: MirCallAbi {
                params: Vec::new(),
                result: None,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            },
            args: Vec::new(),
            result: None,
            effects: MirEffects::default(),
        }
    }

    fn analyze_caller_with_known_callee(
        caller: MirRoutine,
        callee: MirRoutine,
    ) -> MirMachineValueAvailability {
        let program = MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines: vec![caller, callee],
            machine_blocks: Vec::new(),
            runtime_helpers: Vec::new(),
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program);
        let caller = &program.routines[0];
        let cfg = MirCfg::from_routine(caller).unwrap();
        MirMachineValueAvailability::analyze_with_known_callees(caller, &cfg, &summaries)
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
    fn index_register_values_flow_across_diamonds_and_must_agree_at_joins() {
        let x_value = spill(1);
        let y_value = spill(2);
        let build = |right_x| {
            routine(vec![
                block(0, Vec::new(), branch(1, 2)),
                block(
                    1,
                    vec![
                        load_reg(MirReg::X, x_value.clone()),
                        load_reg(MirReg::Y, y_value.clone()),
                    ],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
                block(
                    2,
                    vec![
                        load_reg(MirReg::X, right_x),
                        load_reg(MirReg::Y, y_value.clone()),
                    ],
                    MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
                ),
                block(3, Vec::new(), MirTerminator::Return),
            ])
        };

        let same = analyze(&build(x_value.clone()));
        let join = MirSite::BlockEntry {
            block: MirBlockId(3),
        };
        assert_eq!(
            same.register_at(join, MirReg::X),
            Ok(Some(MirMachineValue::DirectMem(x_value.clone())))
        );
        assert_eq!(
            same.register_at(join, MirReg::Y),
            Ok(Some(MirMachineValue::DirectMem(y_value.clone())))
        );

        let different = analyze(&build(spill(3)));
        assert_eq!(different.register_at(join, MirReg::X), Ok(None));
        assert_eq!(
            different.register_at(join, MirReg::Y),
            Ok(Some(MirMachineValue::DirectMem(y_value)))
        );
    }

    #[test]
    fn index_register_values_converge_around_loops() {
        let value = spill(1);
        let routine = routine(vec![
            block(
                0,
                vec![load_reg(MirReg::X, value.clone())],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(1, Vec::new(), branch(1, 2)),
            block(2, Vec::new(), MirTerminator::Return),
        ]);
        let values = analyze(&routine);

        for block in [MirBlockId(1), MirBlockId(2)] {
            assert_eq!(
                values.register_at(MirSite::BlockEntry { block }, MirReg::X),
                Ok(Some(MirMachineValue::DirectMem(value.clone())))
            );
        }
        assert!(values.evaluations() > routine.blocks.len());
    }

    #[test]
    fn unknown_calls_kill_index_register_values() {
        let routine = routine(vec![block(
            0,
            vec![
                load_reg(MirReg::X, spill(1)),
                load_reg(MirReg::Y, spill(2)),
                call(1),
            ],
            MirTerminator::Return,
        )]);
        let values = analyze(&routine);
        let after_call = MirSite::Terminator {
            block: MirBlockId(0),
        };

        assert_eq!(values.register_at(after_call, MirReg::X), Ok(None));
        assert_eq!(values.register_at(after_call, MirReg::Y), Ok(None));
    }

    #[test]
    fn unknown_calls_kill_exact_zn_provenance() {
        let routine = routine(vec![block(
            0,
            vec![
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 7,
                    width: MirWidth::Byte,
                },
                call(1),
            ],
            MirTerminator::Return,
        )]);
        let values = analyze(&routine);
        assert_eq!(
            values.zn_value_at(MirSite::Terminator {
                block: MirBlockId(0)
            }),
            Ok(None)
        );
    }

    #[test]
    fn indirect_load_stored_to_memory_acquires_exact_zn_provenance() {
        let return_slot = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
        let routine = routine(vec![block(
            0,
            vec![
                MirOp::LoadIndirect {
                    consumer: crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
                        crate::mir6502::ir::MirPointerPair::Fixed {
                            lo: MirFixedZpSlot(0xAC),
                        },
                    ),
                    dst: MirDef::Reg(MirReg::A),
                    offset: 0,
                },
                store_a(return_slot.clone()),
            ],
            MirTerminator::Return,
        )]);
        let values = analyze(&routine);
        assert_eq!(
            values.zn_value_at(MirSite::Terminator {
                block: MirBlockId(0)
            }),
            Ok(Some(MirMachineValue::DirectMem(return_slot)))
        );
    }

    #[test]
    fn unknown_preserving_call_still_invalidates_index_register_memory_aliases() {
        let routine = routine(vec![block(
            0,
            vec![
                load_reg(MirReg::X, spill(1)),
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::Y),
                    value: 7,
                    width: MirWidth::Byte,
                },
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: MirCallAbi {
                        params: Vec::new(),
                        result: None,
                        clobbers: MirRegisterSet::default(),
                        preserves: MirRegisterSet {
                            x: true,
                            y: true,
                            ..MirRegisterSet::default()
                        },
                    },
                    args: Vec::new(),
                    result: None,
                    effects: MirEffects::default(),
                },
            ],
            MirTerminator::Return,
        )]);
        let values = analyze(&routine);
        let after_call = MirSite::Terminator {
            block: MirBlockId(0),
        };

        assert_eq!(values.register_at(after_call, MirReg::X), Ok(None));
        assert_eq!(
            values.register_at(after_call, MirReg::Y),
            Ok(Some(MirMachineValue::ConstU8(7)))
        );
    }

    #[test]
    fn direct_store_establishes_source_register_alias_and_invalidates_old_aliases() {
        let source = spill(1);
        let destination = spill(2);
        let routine = routine(vec![block(
            0,
            vec![
                load_reg(MirReg::X, source.clone()),
                load_reg(MirReg::Y, destination.clone()),
                MirOp::Store {
                    dst: MirAddr::Direct(destination.clone()),
                    src: MirValue::Def(MirDef::Reg(MirReg::X)),
                    width: MirWidth::Byte,
                },
                load_reg(MirReg::X, destination.clone()),
            ],
            MirTerminator::Return,
        )]);
        let values = analyze(&routine);
        let before_reload = MirSite::Op {
            block: MirBlockId(0),
            op_index: 3,
        };

        assert_eq!(
            values.register_at(before_reload, MirReg::X),
            Ok(Some(MirMachineValue::DirectMem(destination)))
        );
        assert_eq!(values.register_at(before_reload, MirReg::Y), Ok(None));
    }

    #[test]
    fn fixed_pointer_values_flow_across_conditional_edges() {
        let low_source = spill(1);
        let high_source = spill(2);
        let low_slot = MirFixedZpSlot(0xAC);
        let high_slot = MirFixedZpSlot(0xAD);
        let routine = routine(vec![
            block(
                0,
                vec![
                    load_a(low_source.clone()),
                    store_a(MirMem::FixedZeroPage(low_slot)),
                    load_a(high_source.clone()),
                    store_a(MirMem::FixedZeroPage(high_slot)),
                    MirOp::Compare {
                        dst: crate::mir6502::ir::MirCondDest::Flags,
                        op: crate::mir6502::ir::MirCompareOp::Eq,
                        left: MirValue::Def(MirDef::Reg(MirReg::A)),
                        right: MirValue::ConstU8(0),
                        width: MirWidth::Byte,
                        signed: false,
                    },
                ],
                branch(1, 2),
            ),
            block(1, Vec::new(), MirTerminator::Return),
            block(2, Vec::new(), MirTerminator::Return),
        ]);
        let values = analyze(&routine);

        for block in [MirBlockId(1), MirBlockId(2)] {
            assert_eq!(
                values.fixed_zero_page_value_at(MirSite::BlockEntry { block }, low_slot,),
                Ok(Some(MirMachineValue::DirectMem(low_source.clone())))
            );
            assert_eq!(
                values.fixed_zero_page_value_at(MirSite::BlockEntry { block }, high_slot,),
                Ok(Some(MirMachineValue::DirectMem(high_source.clone())))
            );
        }
    }

    #[test]
    fn possible_indirect_write_kills_fixed_pointer_values() {
        let low_slot = MirFixedZpSlot(0xAC);
        let pointer = crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
            crate::mir6502::ir::MirPointerPair::Fixed { lo: low_slot },
        );
        let routine = routine(vec![
            block(
                0,
                vec![
                    load_a(spill(1)),
                    store_a(MirMem::FixedZeroPage(low_slot)),
                    MirOp::StoreIndirect {
                        consumer: pointer,
                        src: MirValue::Def(MirDef::Reg(MirReg::A)),
                        offset: 0,
                    },
                ],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        let values = analyze(&routine);

        assert_eq!(
            values.fixed_zero_page_value_at(
                MirSite::BlockEntry {
                    block: MirBlockId(1),
                },
                low_slot,
            ),
            Ok(None)
        );
    }

    #[test]
    fn direct_source_write_invalidates_only_dependent_pointer_byte() {
        let low_source = spill(1);
        let high_source = spill(2);
        let low_slot = MirFixedZpSlot(0xAC);
        let high_slot = MirFixedZpSlot(0xAD);
        let routine = routine(vec![
            block(
                0,
                vec![
                    load_a(low_source.clone()),
                    store_a(MirMem::FixedZeroPage(low_slot)),
                    load_a(high_source.clone()),
                    store_a(MirMem::FixedZeroPage(high_slot)),
                    MirOp::LoadImm {
                        dst: MirDef::Reg(MirReg::A),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    store_a(low_source),
                ],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(1))),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        let values = analyze(&routine);

        assert_eq!(
            values.fixed_zero_page_value_at(
                MirSite::BlockEntry {
                    block: MirBlockId(1),
                },
                low_slot,
            ),
            Ok(None)
        );
        assert_eq!(
            values.fixed_zero_page_value_at(
                MirSite::BlockEntry {
                    block: MirBlockId(1),
                },
                high_slot,
            ),
            Ok(Some(MirMachineValue::DirectMem(high_source)))
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
    fn implicit_accumulator_materialization_does_not_preserve_old_facts() {
        let routine = routine(vec![
            block(
                0,
                vec![
                    load_a(spill(1)),
                    MirOp::Compare {
                        dst: crate::mir6502::ir::MirCondDest::Flags,
                        op: crate::mir6502::ir::MirCompareOp::Eq,
                        left: MirValue::PointerCell(spill(2)),
                        right: MirValue::ConstU8(7),
                        width: MirWidth::Byte,
                        signed: false,
                    },
                ],
                branch(1, 2),
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
    fn constant_store_materialization_does_not_preserve_old_constants() {
        let routine = routine(vec![
            block(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: MirDef::Reg(MirReg::A),
                        value: 1,
                        width: MirWidth::Byte,
                    },
                    MirOp::Store {
                        dst: MirAddr::Direct(spill(2)),
                        src: MirValue::ConstU8(2),
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
        let pointer_slot = MirFixedZpSlot(0xAC);
        let routine = routine(vec![
            block(
                0,
                vec![
                    load_a(spill(1)),
                    store_a(MirMem::FixedZeroPage(pointer_slot)),
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
        assert_eq!(
            values.fixed_zero_page_value_at(
                MirSite::BlockEntry {
                    block: MirBlockId(1)
                },
                pointer_slot,
            ),
            Ok(None)
        );
    }

    #[test]
    fn known_callee_preserves_unwritten_fixed_pointer_values_and_sources() {
        let low_source = spill(1);
        let high_source = spill(2);
        let low_slot = MirFixedZpSlot(0xAC);
        let high_slot = MirFixedZpSlot(0xAD);
        let caller = routine(vec![block(
            0,
            vec![
                load_a(low_source.clone()),
                store_a(MirMem::FixedZeroPage(low_slot)),
                load_a(high_source.clone()),
                store_a(MirMem::FixedZeroPage(high_slot)),
                call(1),
                load_a(low_source.clone()),
                store_a(MirMem::FixedZeroPage(low_slot)),
                load_a(high_source.clone()),
                store_a(MirMem::FixedZeroPage(high_slot)),
            ],
            MirTerminator::Return,
        )]);
        let mut callee = routine(vec![block(
            1,
            vec![
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 7,
                    width: MirWidth::Byte,
                },
                store_a(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
            ],
            MirTerminator::Return,
        )]);
        callee.id = RoutineId(1);
        callee.name = "KnownCallee".to_string();
        let values = analyze_caller_with_known_callee(caller, callee);

        assert_eq!(
            values.fixed_zero_page_value_at(
                MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 6,
                },
                low_slot,
            ),
            Ok(Some(MirMachineValue::DirectMem(low_source)))
        );
        assert_eq!(
            values.fixed_zero_page_value_at(
                MirSite::Op {
                    block: MirBlockId(0),
                    op_index: 8,
                },
                high_slot,
            ),
            Ok(Some(MirMachineValue::DirectMem(high_source)))
        );
    }

    #[test]
    fn known_callee_preserves_proven_index_register_values() {
        let caller = routine(vec![block(
            0,
            vec![
                load_reg(MirReg::X, spill(1)),
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::Y),
                    value: 7,
                    width: MirWidth::Byte,
                },
                call(1),
            ],
            MirTerminator::Return,
        )]);
        let mut callee = routine(vec![block(
            1,
            vec![MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::X),
                value: 9,
                width: MirWidth::Byte,
            }],
            MirTerminator::Return,
        )]);
        callee.id = RoutineId(1);
        callee.name = "PreservesY".to_string();

        let values = analyze_caller_with_known_callee(caller, callee);
        let after_call = MirSite::Terminator {
            block: MirBlockId(0),
        };
        assert_eq!(values.register_at(after_call, MirReg::X), Ok(None));
        assert_eq!(
            values.register_at(after_call, MirReg::Y),
            Ok(Some(MirMachineValue::ConstU8(7)))
        );
    }

    #[test]
    fn known_callee_establishes_its_exact_accumulator_and_zn_exit_values() {
        let return_slot = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
        let caller = routine(vec![block(0, vec![call(1)], MirTerminator::Return)]);
        let mut callee = routine(vec![block(
            1,
            vec![
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 1,
                    width: MirWidth::Byte,
                },
                store_a(return_slot.clone()),
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::X),
                    value: 9,
                    width: MirWidth::Byte,
                },
            ],
            MirTerminator::Return,
        )]);
        callee.id = RoutineId(1);
        callee.name = "ExactExit".to_string();

        let values = analyze_caller_with_known_callee(caller, callee);
        let after_call = MirSite::Terminator {
            block: MirBlockId(0),
        };
        assert_eq!(
            values.accumulator_at(after_call),
            Ok(Some(MirMachineValue::DirectMem(return_slot)))
        );
        assert_eq!(
            values.zn_value_at(after_call),
            Ok(Some(MirMachineValue::ConstU8(9)))
        );
    }

    #[test]
    fn known_callee_invalidates_only_pointer_facts_it_may_write() {
        let low_source = spill(1);
        let high_source = spill(2);
        let low_slot = MirFixedZpSlot(0xAC);
        let high_slot = MirFixedZpSlot(0xAD);
        let caller = routine(vec![block(
            0,
            vec![
                load_a(low_source),
                store_a(MirMem::FixedZeroPage(low_slot)),
                load_a(high_source.clone()),
                store_a(MirMem::FixedZeroPage(high_slot)),
                call(1),
            ],
            MirTerminator::Return,
        )]);
        let mut callee = routine(vec![block(
            1,
            vec![
                MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 7,
                    width: MirWidth::Byte,
                },
                store_a(MirMem::FixedZeroPage(low_slot)),
            ],
            MirTerminator::Return,
        )]);
        callee.id = RoutineId(1);
        callee.name = "PointerWriter".to_string();
        let values = analyze_caller_with_known_callee(caller, callee);
        let after_call = MirSite::Terminator {
            block: MirBlockId(0),
        };

        assert_eq!(
            values.fixed_zero_page_value_at(after_call, low_slot),
            Ok(None)
        );
        assert_eq!(
            values.fixed_zero_page_value_at(after_call, high_slot),
            Ok(Some(MirMachineValue::DirectMem(high_source)))
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
