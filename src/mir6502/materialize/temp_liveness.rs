use super::dead_spills::block_successor_indices;
use super::stats::MirPeepholeStats;
use crate::mir6502::ir::{
    MirAddr, MirBlock, MirCallTarget, MirDef, MirOp, MirRoutine, MirTempId, MirTerminator,
    MirValue, RoutineId,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MirTempLiveness {
    blocks: Vec<MirTempBlockLiveness>,
}

impl MirTempLiveness {
    #[cfg(test)]
    pub(super) fn block(&self, index: usize) -> Option<&MirTempBlockLiveness> {
        self.blocks.get(index)
    }

    pub(super) fn live_out(&self, index: usize) -> Option<&MirTempLiveSet> {
        self.blocks.get(index).map(|block| &block.live_out)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MirTempBlockLiveness {
    pub(super) uses: MirTempLiveSet,
    pub(super) defs: MirTempLiveSet,
    pub(super) live_in: MirTempLiveSet,
    pub(super) live_out: MirTempLiveSet,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MirTempLiveSet {
    exact: BTreeSet<(MirTempId, u8)>,
    full: BTreeSet<MirTempId>,
}

impl MirTempLiveSet {
    #[cfg(test)]
    pub(super) fn with_exact_lane(id: MirTempId, byte: u8) -> Self {
        let mut set = Self::default();
        set.insert_exact(id, byte);
        set
    }

    pub(super) fn exact_lane_live(&self, id: MirTempId, byte: u8) -> bool {
        self.contains_exact_lane(id, byte)
    }

    pub(super) fn full_temp_live(&self, id: MirTempId) -> bool {
        self.contains_full_temp(id)
    }

    fn contains_exact_lane(&self, id: MirTempId, byte: u8) -> bool {
        self.exact.contains(&(id, byte))
    }

    fn contains_full_temp(&self, id: MirTempId) -> bool {
        self.full.contains(&id)
    }

    fn defines_lane(&self, id: MirTempId, byte: u8) -> bool {
        self.contains_full_temp(id) || self.contains_exact_lane(id, byte)
    }

    fn defines_word(&self, id: MirTempId) -> bool {
        self.contains_full_temp(id)
            || self.contains_exact_lane(id, 0) && self.contains_exact_lane(id, 1)
    }

    fn insert_word_use_after_defs(&mut self, id: MirTempId, defs: &Self) {
        if defs.defines_word(id) {
            return;
        }
        if defs.defines_lane(id, 0) {
            self.insert_exact(id, 1);
        } else if defs.defines_lane(id, 1) {
            self.insert_exact(id, 0);
        } else {
            self.insert_full(id);
        }
    }

    fn exact_len(&self) -> usize {
        self.exact.len()
    }

    fn full_len(&self) -> usize {
        self.full.len()
    }

    pub(super) fn exact_lanes(&self) -> impl Iterator<Item = (MirTempId, u8)> + '_ {
        self.exact.iter().copied()
    }

    pub(super) fn full_temps(&self) -> impl Iterator<Item = MirTempId> + '_ {
        self.full.iter().copied()
    }

    fn insert_exact(&mut self, id: MirTempId, byte: u8) {
        self.exact.insert((id, byte));
    }

    fn insert_full(&mut self, id: MirTempId) {
        self.full.insert(id);
    }

    fn union_with(&mut self, other: &Self) -> bool {
        let before = self.clone();
        self.exact.extend(other.exact.iter().copied());
        self.full.extend(other.full.iter().copied());
        *self != before
    }

    fn subtract_defs(&self, defs: &Self) -> Self {
        let mut out = Self::default();
        for (id, byte) in &self.exact {
            if !defs.defines_lane(*id, *byte) {
                out.insert_exact(*id, *byte);
            }
        }
        for id in &self.full {
            let low_defined = defs.defines_lane(*id, 0);
            let high_defined = defs.defines_lane(*id, 1);
            match (low_defined, high_defined) {
                (false, false) => out.insert_full(*id),
                (true, false) => out.insert_exact(*id, 1),
                (false, true) => out.insert_exact(*id, 0),
                (true, true) => {}
            }
        }
        out
    }
}

pub(super) fn record_temp_liveness_observability(
    routine_id: RoutineId,
    liveness: &MirTempLiveness,
    peephole_stats: &mut MirPeepholeStats,
) {
    let live_in_lanes = liveness
        .blocks
        .iter()
        .map(|block| block.live_in.exact_len())
        .sum();
    let live_out_lanes = liveness
        .blocks
        .iter()
        .map(|block| block.live_out.exact_len())
        .sum();
    let live_in_full_temps = liveness
        .blocks
        .iter()
        .map(|block| block.live_in.full_len())
        .sum();
    let live_out_full_temps = liveness
        .blocks
        .iter()
        .map(|block| block.live_out.full_len())
        .sum();

    peephole_stats.record_many(routine_id, "temp-liveness-live-in-lanes", live_in_lanes);
    peephole_stats.record_many(routine_id, "temp-liveness-live-out-lanes", live_out_lanes);
    peephole_stats.record_many(
        routine_id,
        "temp-liveness-live-in-full-temps",
        live_in_full_temps,
    );
    peephole_stats.record_many(
        routine_id,
        "temp-liveness-live-out-full-temps",
        live_out_full_temps,
    );
}

pub(super) fn analyze_temp_liveness(routine: &MirRoutine) -> MirTempLiveness {
    let mut blocks = routine
        .blocks
        .iter()
        .map(temp_block_uses_and_defs)
        .collect::<Vec<_>>();

    loop {
        let mut changed = false;
        for block_index in (0..routine.blocks.len()).rev() {
            let mut live_out = MirTempLiveSet::default();
            for successor_index in
                block_successor_indices(routine, &routine.blocks[block_index].terminator)
            {
                live_out.union_with(&blocks[successor_index].live_in);
            }

            let mut live_in = blocks[block_index].uses.clone();
            live_in.union_with(&live_out.subtract_defs(&blocks[block_index].defs));

            changed |= blocks[block_index].live_out != live_out;
            changed |= blocks[block_index].live_in != live_in;
            blocks[block_index].live_out = live_out;
            blocks[block_index].live_in = live_in;
        }
        if !changed {
            break;
        }
    }

    MirTempLiveness { blocks }
}

fn temp_block_uses_and_defs(block: &MirBlock) -> MirTempBlockLiveness {
    let mut liveness = MirTempBlockLiveness::default();
    for op in &block.ops {
        observe_op_uses(op, &mut liveness.uses, &liveness.defs);
        observe_op_def(op, &mut liveness.defs);
    }
    observe_terminator_uses(&block.terminator, &mut liveness.uses, &liveness.defs);
    liveness.live_in = liveness.uses.clone();
    liveness
}

fn observe_op_def(op: &MirOp, defs: &mut MirTempLiveSet) {
    match op_def(op) {
        Some(MirDef::VTemp(id)) => {
            defs.insert_full(*id);
            defs.insert_exact(*id, 0);
            defs.insert_exact(*id, 1);
        }
        Some(MirDef::VTempByte { id, byte }) => defs.insert_exact(*id, *byte),
        Some(MirDef::Reg(_)) | None => {}
    }
}

fn op_def(op: &MirOp) -> Option<&MirDef> {
    match op {
        MirOp::LoadImm { dst, .. }
        | MirOp::Load { dst, .. }
        | MirOp::Move { dst, .. }
        | MirOp::LeaAddr { dst, .. }
        | MirOp::Extend { dst, .. }
        | MirOp::Truncate { dst, .. }
        | MirOp::Unary { dst, .. }
        | MirOp::Binary { dst, .. }
        | MirOp::LoadIndirect { dst, .. } => Some(dst),
        MirOp::Store { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::Compare { .. }
        | MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => None,
    }
}

fn observe_op_uses(op: &MirOp, uses: &mut MirTempLiveSet, defs: &MirTempLiveSet) {
    match op {
        MirOp::Load { src, .. } => observe_addr(src, uses, defs),
        MirOp::Store { dst, src, .. } => {
            observe_addr(dst, uses, defs);
            observe_value(src, uses, defs);
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::AddByteToWordMem { value: src, .. }
        | MirOp::SubByteFromWordMem { value: src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => observe_value(src, uses, defs),
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            observe_value(left, uses, defs);
            observe_value(right, uses, defs);
        }
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            observe_value(base, uses, defs);
            observe_value(index, uses, defs);
        }
        MirOp::Call { target, args, .. } => {
            if let MirCallTarget::Indirect { target, .. } = target {
                observe_value(target, uses, defs);
            }
            for arg in args {
                observe_value(&arg.value, uses, defs);
            }
        }
        MirOp::LoadImm { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::MachineBlock { .. } => {}
    }
}

fn observe_addr(addr: &MirAddr, uses: &mut MirTempLiveSet, defs: &MirTempLiveSet) {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            observe_value(base, uses, defs);
            observe_value(index, uses, defs);
        }
        MirAddr::PointerIndex { index, .. } => observe_value(index, uses, defs),
        MirAddr::Deref { ptr, .. } => observe_value(ptr, uses, defs),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::PointerCell { .. } => {}
    }
}

fn observe_terminator_uses(
    terminator: &MirTerminator,
    uses: &mut MirTempLiveSet,
    defs: &MirTempLiveSet,
) {
    if let MirTerminator::Branch {
        cond: crate::mir6502::ir::MirCond::BoolValue(value),
        ..
    } = terminator
    {
        observe_value(value, uses, defs);
    }
}

fn observe_value(value: &MirValue, uses: &mut MirTempLiveSet, defs: &MirTempLiveSet) {
    match value {
        MirValue::Def(MirDef::VTemp(id)) => {
            uses.insert_word_use_after_defs(*id, defs);
        }
        MirValue::Def(MirDef::VTempByte { id, byte }) => {
            if !defs.defines_lane(*id, *byte) {
                uses.insert_exact(*id, *byte);
            }
        }
        MirValue::Word { lo, hi } => {
            observe_value(lo, uses, defs);
            observe_value(hi, uses, defs);
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(MirDef::Reg(_))
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => {}
    }
}
