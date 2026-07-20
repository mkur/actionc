#![allow(dead_code)] // Typed rewrite contexts consume these facts incrementally.

use std::collections::BTreeSet;

use crate::analysis::dominance::Dominance;
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::analysis::use_def::{MirDefSite, MirUseSite};
use crate::mir6502::ir::MirBlockId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirDominance {
    dominance: Dominance<MirBlockId>,
    reachable: BTreeSet<MirBlockId>,
}

impl MirDominance {
    pub(in crate::mir6502) fn from_cfg(cfg: &MirCfg) -> Self {
        Self {
            dominance: Dominance::from_graph(cfg),
            reachable: cfg.reachable().clone(),
        }
    }

    pub(in crate::mir6502) fn block_dominates(
        &self,
        dominator: MirBlockId,
        block: MirBlockId,
    ) -> bool {
        self.reachable.contains(&dominator)
            && self.reachable.contains(&block)
            && self.dominance.dominates(dominator, block)
    }

    pub(in crate::mir6502) fn site_dominates(&self, dominator: MirSite, site: MirSite) -> bool {
        if dominator.block() != site.block() {
            return self.block_dominates(dominator.block(), site.block());
        }
        if !self.reachable.contains(&site.block()) {
            return false;
        }
        match (dominator, site) {
            (MirSite::BlockEntry { .. }, _) => true,
            (
                MirSite::Op { op_index: left, .. },
                MirSite::Op {
                    op_index: right, ..
                },
            ) => left < right,
            (MirSite::Op { .. }, MirSite::Terminator { .. }) => true,
            (MirSite::Terminator { .. }, _) => false,
            (_, MirSite::BlockEntry { .. }) => false,
        }
    }

    pub(in crate::mir6502) fn definition_dominates_use(
        &self,
        definition: MirDefSite,
        usage: MirUseSite,
    ) -> bool {
        self.site_dominates(definition.site, usage.site)
    }

    pub(in crate::mir6502) fn immediate_dominator(&self, block: MirBlockId) -> Option<MirBlockId> {
        self.dominance.immediate_dominator(block)
    }

    pub(in crate::mir6502) fn dominance_frontier(
        &self,
        block: MirBlockId,
    ) -> &BTreeSet<MirBlockId> {
        self.dominance.dominance_frontier(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::effects::MirTempUseKind;
    use crate::mir6502::analysis::use_def::{MirTempLane, MirTempRequirement};
    use crate::mir6502::ir::{
        MirBlock, MirCond, MirEdge, MirEffects, MirFrame, MirRoutine, MirRoutineAbi, MirTerminator,
        MirValue, RoutineId,
    };

    fn block(id: u32, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops: Vec::new(),
            terminator,
        }
    }

    #[test]
    fn combines_block_dominance_with_intra_block_order() {
        let routine = MirRoutine {
            id: RoutineId(0),
            name: "dominance".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![
                block(
                    0,
                    MirTerminator::Branch {
                        cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                        then_edge: MirEdge::plain(MirBlockId(1)),
                        else_edge: MirEdge::plain(MirBlockId(2)),
                    },
                ),
                block(1, MirTerminator::Jump(MirEdge::plain(MirBlockId(3)))),
                block(2, MirTerminator::Jump(MirEdge::plain(MirBlockId(3)))),
                block(3, MirTerminator::Return),
                block(9, MirTerminator::Return),
            ],
            effects: MirEffects::default(),
        };
        let cfg = MirCfg::from_routine(&routine).unwrap();
        let dominance = MirDominance::from_cfg(&cfg);
        assert!(dominance.block_dominates(MirBlockId(0), MirBlockId(3)));
        assert!(!dominance.block_dominates(MirBlockId(1), MirBlockId(3)));
        assert!(!dominance.block_dominates(MirBlockId(9), MirBlockId(9)));

        let lane = MirTempLane {
            temp: crate::mir6502::ir::MirTempId(1),
            byte: 0,
        };
        let definition = MirDefSite {
            site: MirSite::Op {
                block: MirBlockId(0),
                op_index: 1,
            },
            lane,
        };
        let before = MirUseSite {
            site: MirSite::Op {
                block: MirBlockId(0),
                op_index: 0,
            },
            requirement: MirTempRequirement::Exact(lane),
            kind: MirTempUseKind::Operand,
        };
        let after = MirUseSite {
            site: MirSite::Op {
                block: MirBlockId(0),
                op_index: 2,
            },
            ..before
        };
        assert!(!dominance.definition_dominates_use(definition, before));
        assert!(dominance.definition_dominates_use(definition, after));
        assert!(dominance.site_dominates(
            MirSite::BlockEntry {
                block: MirBlockId(0),
            },
            before.site,
        ));
    }
}
