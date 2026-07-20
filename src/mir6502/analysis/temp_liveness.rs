#![allow(dead_code)] // Typed rewrite contexts consume block-ID queries later.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::{MirTempAccess, classify_op, classify_terminator};
use crate::mir6502::ir::{MirBlock, MirBlockId, MirOp, MirRoutine, MirTempId, MirWidth};

/// Backward may-liveness. A full-temp requirement is retained until both byte
/// lanes are defined; defining one lane narrows it to the missing lane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirTempLiveSet {
    exact: BTreeSet<(MirTempId, u8)>,
    full: BTreeSet<MirTempId>,
}

impl MirTempLiveSet {
    #[cfg(test)]
    pub(in crate::mir6502) fn with_exact_lane(id: MirTempId, byte: u8) -> Self {
        let mut set = Self::default();
        set.insert_exact(id, byte);
        set
    }

    pub(in crate::mir6502) fn exact_lane_live(&self, id: MirTempId, byte: u8) -> bool {
        self.exact.contains(&(id, byte))
    }

    pub(in crate::mir6502) fn full_temp_live(&self, id: MirTempId) -> bool {
        self.full.contains(&id)
    }

    pub(in crate::mir6502) fn exact_lanes(&self) -> impl Iterator<Item = (MirTempId, u8)> + '_ {
        self.exact.iter().copied()
    }

    pub(in crate::mir6502) fn full_temps(&self) -> impl Iterator<Item = MirTempId> + '_ {
        self.full.iter().copied()
    }

    pub(in crate::mir6502) fn exact_len(&self) -> usize {
        self.exact.len()
    }

    pub(in crate::mir6502) fn full_len(&self) -> usize {
        self.full.len()
    }

    fn defines_lane(&self, id: MirTempId, byte: u8) -> bool {
        self.full_temp_live(id) || self.exact_lane_live(id, byte)
    }

    fn defines_word(&self, id: MirTempId) -> bool {
        self.full_temp_live(id) || self.exact_lane_live(id, 0) && self.exact_lane_live(id, 1)
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

    fn insert_exact(&mut self, id: MirTempId, byte: u8) {
        self.exact.insert((id, byte));
    }

    fn insert_full(&mut self, id: MirTempId) {
        self.full.insert(id);
    }

    fn insert_full_definition(&mut self, id: MirTempId) {
        self.insert_full(id);
        self.insert_exact(id, 0);
        self.insert_exact(id, 1);
    }

    fn union_with(&mut self, other: &Self) {
        self.exact.extend(other.exact.iter().copied());
        self.full.extend(other.full.iter().copied());
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirTempBlockLiveness {
    pub uses: MirTempLiveSet,
    pub defs: MirTempLiveSet,
    pub live_in: MirTempLiveSet,
    pub live_out: MirTempLiveSet,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirTempLiveness {
    blocks: Vec<MirTempBlockLiveness>,
    block_indices: BTreeMap<MirBlockId, usize>,
    evaluations: usize,
}

impl MirTempLiveness {
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine, cfg: &MirCfg) -> Self {
        let facts = routine
            .blocks
            .iter()
            .map(|block| (block.id, temp_block_uses_and_defs(block)))
            .collect::<BTreeMap<_, _>>();
        let result = solve_dataflow(cfg, &TempLivenessProblem { facts: &facts });
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
                MirTempBlockLiveness {
                    uses: facts.uses.clone(),
                    defs: facts.defs.clone(),
                    live_in: result
                        .in_state(block.id)
                        .cloned()
                        .unwrap_or_else(|| facts.uses.clone()),
                    live_out: result.out_state(block.id).cloned().unwrap_or_default(),
                }
            })
            .collect();
        Self {
            blocks,
            block_indices,
            evaluations: result.evaluations(),
        }
    }

    pub(in crate::mir6502) fn block(&self, index: usize) -> Option<&MirTempBlockLiveness> {
        self.blocks.get(index)
    }

    pub(in crate::mir6502) fn block_by_id(
        &self,
        block: MirBlockId,
    ) -> Option<&MirTempBlockLiveness> {
        self.block_indices
            .get(&block)
            .and_then(|index| self.blocks.get(*index))
    }

    pub(in crate::mir6502) fn live_out(&self, index: usize) -> Option<&MirTempLiveSet> {
        self.block(index).map(|block| &block.live_out)
    }

    pub(in crate::mir6502) fn live_in(&self, index: usize) -> Option<&MirTempLiveSet> {
        self.block(index).map(|block| &block.live_in)
    }

    pub(in crate::mir6502) fn blocks(&self) -> &[MirTempBlockLiveness] {
        &self.blocks
    }

    pub(in crate::mir6502) fn evaluations(&self) -> usize {
        self.evaluations
    }
}

#[derive(Debug, Clone, Default)]
struct MirTempBlockFacts {
    uses: MirTempLiveSet,
    defs: MirTempLiveSet,
}

struct TempLivenessProblem<'a> {
    facts: &'a BTreeMap<MirBlockId, MirTempBlockFacts>,
}

impl DataflowProblem<MirCfg> for TempLivenessProblem<'_> {
    type State = MirTempLiveSet;

    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Backward
    }

    fn bottom(&self) -> Self::State {
        Self::State::default()
    }

    fn boundary(&self, _node: MirBlockId) -> Option<Self::State> {
        None
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.union_with(other);
    }

    fn transfer(&self, node: MirBlockId, live_out: &Self::State) -> Self::State {
        let facts = &self.facts[&node];
        let mut live_in = facts.uses.clone();
        live_in.union_with(&live_out.subtract_defs(&facts.defs));
        live_in
    }
}

fn temp_block_uses_and_defs(block: &MirBlock) -> MirTempBlockFacts {
    let mut facts = MirTempBlockFacts::default();
    for param in &block.params {
        match param.width {
            MirWidth::Byte => facts.defs.insert_exact(param.dest, 0),
            MirWidth::Word => facts.defs.insert_full_definition(param.dest),
        }
    }
    for op in &block.ops {
        let effects = classify_op(op);
        for access in &effects.logical.temp_uses {
            observe_use(*access, &mut facts.uses, &facts.defs);
        }
        observe_compat_definitions(op, &effects.logical, &mut facts.defs);
    }
    for access in classify_terminator(&block.terminator).logical.temp_uses {
        observe_use(access, &mut facts.uses, &facts.defs);
    }
    facts
}

fn observe_use(access: MirTempAccess, uses: &mut MirTempLiveSet, defs: &MirTempLiveSet) {
    match access {
        MirTempAccess::Full(temp) => uses.insert_word_use_after_defs(temp, defs),
        MirTempAccess::Exact { temp, byte } if !defs.defines_lane(temp, byte) => {
            uses.insert_exact(temp, byte);
        }
        MirTempAccess::Exact { .. } => {}
    }
}

fn observe_compat_definitions(
    op: &MirOp,
    logical: &crate::mir6502::analysis::effects::MirLogicalEffects,
    defs: &mut MirTempLiveSet,
) {
    // Calls and compares did not participate in the existing block-local
    // liveness kill set. Preserve that behavior until consumers move to
    // definition-identity queries.
    if matches!(op, MirOp::Call { .. } | MirOp::Compare { .. }) {
        return;
    }
    for temp in &logical.full_temp_defs_compat {
        defs.insert_full_definition(*temp);
    }
    for access in &logical.temp_defs {
        if let MirTempAccess::Exact { temp, byte } = *access {
            defs.insert_exact(temp, byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirAddr, MirBlockParam, MirDef, MirEdge, MirEdgeArg, MirEffects, MirFrame, MirMem, MirOp,
        MirRoutineAbi, MirTemp, MirTerminator, MirValue, RoutineId,
    };

    #[test]
    fn edge_arguments_are_predecessor_uses_and_block_params_are_entry_defs() {
        let routine = MirRoutine {
            id: RoutineId(0),
            name: "Liveness".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: vec![MirTemp { id: MirTempId(0) }, MirTemp { id: MirTempId(1) }],
            blocks: vec![
                MirBlock {
                    id: MirBlockId(0),
                    label: "entry".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Jump(MirEdge {
                        target: MirBlockId(1),
                        args: vec![MirEdgeArg {
                            value: MirValue::Def(MirDef::VTemp(MirTempId(0))),
                            width: MirWidth::Byte,
                        }],
                    }),
                },
                MirBlock {
                    id: MirBlockId(1),
                    label: "join".to_string(),
                    params: vec![MirBlockParam {
                        dest: MirTempId(1),
                        width: MirWidth::Byte,
                    }],
                    ops: vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x4000)),
                        src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    }],
                    terminator: MirTerminator::Return,
                },
            ],
            effects: MirEffects::default(),
        };
        let cfg = MirCfg::from_routine(&routine).unwrap();
        let liveness = MirTempLiveness::analyze(&routine, &cfg);
        let entry = liveness.block_by_id(MirBlockId(0)).unwrap();
        assert!(entry.uses.exact_lane_live(MirTempId(0), 0));
        assert!(!entry.uses.full_temp_live(MirTempId(0)));
        let join = liveness.block_by_id(MirBlockId(1)).unwrap();
        assert!(!join.live_in.exact_lane_live(MirTempId(1), 0));
        assert!(!join.live_in.full_temp_live(MirTempId(1)));
        assert!(liveness.evaluations() >= 2);
    }
}
