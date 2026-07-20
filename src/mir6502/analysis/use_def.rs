#![allow(dead_code)] // Rewrite contexts consume the query surface in later slices.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::{
    MirTempAccess, MirTempUseKind, classify_op, classify_terminator,
};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirBlockId, MirRoutine, MirTempId, MirWidth};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) struct MirTempLane {
    pub temp: MirTempId,
    pub byte: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) enum MirTempRequirement {
    Exact(MirTempLane),
    Full(MirTempId),
}

impl MirTempRequirement {
    pub(in crate::mir6502) fn temp(self) -> MirTempId {
        match self {
            Self::Exact(lane) => lane.temp,
            Self::Full(temp) => temp,
        }
    }

    pub(in crate::mir6502) fn requires(self, lane: MirTempLane) -> bool {
        match self {
            Self::Exact(required) => required == lane,
            Self::Full(temp) => temp == lane.temp,
        }
    }
}

impl From<MirTempAccess> for MirTempRequirement {
    fn from(access: MirTempAccess) -> Self {
        match access {
            MirTempAccess::Full(temp) => Self::Full(temp),
            MirTempAccess::Exact { temp, byte } => Self::Exact(MirTempLane { temp, byte }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) struct MirDefSite {
    pub site: MirSite,
    pub lane: MirTempLane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) struct MirUseSite {
    pub site: MirSite,
    pub requirement: MirTempRequirement,
    pub kind: MirTempUseKind,
}

/// Immutable routine-wide index. This records occurrences and deliberately
/// does not claim which definition reaches a use; reaching-definition analysis
/// owns that proof.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirTempUseDefIndex {
    definitions: BTreeMap<MirTempLane, Vec<MirDefSite>>,
    uses: BTreeMap<MirTempId, Vec<MirUseSite>>,
}

impl MirTempUseDefIndex {
    pub(in crate::mir6502) fn from_routine(routine: &MirRoutine) -> Self {
        let mut index = Self::default();
        for block in &routine.blocks {
            let entry = MirSite::BlockEntry { block: block.id };
            for param in &block.params {
                index.record_definition(
                    entry,
                    MirTempLane {
                        temp: param.dest,
                        byte: 0,
                    },
                );
                if param.width == MirWidth::Word {
                    index.record_definition(
                        entry,
                        MirTempLane {
                            temp: param.dest,
                            byte: 1,
                        },
                    );
                }
            }

            for (op_index, op) in block.ops.iter().enumerate() {
                let site = MirSite::Op {
                    block: block.id,
                    op_index,
                };
                let effects = classify_op(op);
                for definition in effects.logical.temp_defs {
                    match MirTempRequirement::from(definition) {
                        MirTempRequirement::Exact(lane) => index.record_definition(site, lane),
                        MirTempRequirement::Full(temp) => {
                            index.record_definition(site, MirTempLane { temp, byte: 0 });
                            index.record_definition(site, MirTempLane { temp, byte: 1 });
                        }
                    }
                }
                for use_effect in effects.logical.classified_temp_uses {
                    index.record_use(MirUseSite {
                        site,
                        requirement: use_effect.access.into(),
                        kind: use_effect.kind,
                    });
                }
            }

            let site = MirSite::Terminator { block: block.id };
            for use_effect in classify_terminator(&block.terminator)
                .logical
                .classified_temp_uses
            {
                index.record_use(MirUseSite {
                    site,
                    requirement: use_effect.access.into(),
                    kind: use_effect.kind,
                });
            }
        }
        index
    }

    pub(in crate::mir6502) fn definitions_of_lane(&self, lane: MirTempLane) -> &[MirDefSite] {
        self.definitions.get(&lane).map_or(&[], Vec::as_slice)
    }

    pub(in crate::mir6502) fn definitions_of_temp(
        &self,
        temp: MirTempId,
    ) -> impl Iterator<Item = &MirDefSite> {
        self.definitions
            .range(
                MirTempLane { temp, byte: 0 }..=MirTempLane {
                    temp,
                    byte: u8::MAX,
                },
            )
            .flat_map(|(_, definitions)| definitions)
    }

    pub(in crate::mir6502) fn uses_of_temp(&self, temp: MirTempId) -> &[MirUseSite] {
        self.uses.get(&temp).map_or(&[], Vec::as_slice)
    }

    pub(in crate::mir6502) fn uses_of_lane(
        &self,
        lane: MirTempLane,
    ) -> impl Iterator<Item = &MirUseSite> {
        self.uses_of_temp(lane.temp)
            .iter()
            .filter(move |usage| usage.requirement.requires(lane))
    }

    pub(in crate::mir6502) fn unique_definition(&self, lane: MirTempLane) -> Option<MirDefSite> {
        let definitions = self.definitions_of_lane(lane);
        (definitions.len() == 1).then(|| definitions[0])
    }

    pub(in crate::mir6502) fn unique_use(&self, lane: MirTempLane) -> Option<MirUseSite> {
        let mut uses = self.uses_of_lane(lane);
        let first = *uses.next()?;
        uses.next().is_none().then_some(first)
    }

    pub(in crate::mir6502) fn uses_in_window(
        &self,
        lane: MirTempLane,
        block: MirBlockId,
        window: Range<usize>,
    ) -> impl Iterator<Item = &MirUseSite> {
        self.uses_of_lane(lane).filter(move |usage| {
            matches!(
                usage.site,
                MirSite::Op {
                    block: use_block,
                    op_index,
                } if use_block == block && window.contains(&op_index)
            )
        })
    }

    pub(in crate::mir6502) fn has_terminator_use(
        &self,
        lane: MirTempLane,
        block: MirBlockId,
    ) -> bool {
        self.uses_of_lane(lane)
            .any(|usage| usage.site == MirSite::Terminator { block })
    }

    pub(in crate::mir6502) fn has_successor_use(
        &self,
        cfg: &MirCfg,
        lane: MirTempLane,
        block: MirBlockId,
    ) -> bool {
        self.uses_of_lane(lane)
            .any(|usage| cfg.successors(block).contains(&usage.site.block()))
    }

    pub(in crate::mir6502) fn definition_has_uses_outside(
        &self,
        definition: MirDefSite,
        block: MirBlockId,
        window: Range<usize>,
    ) -> bool {
        self.uses_of_lane(definition.lane).any(|usage| {
            !matches!(
                usage.site,
                MirSite::Op {
                    block: use_block,
                    op_index,
                } if use_block == block && window.contains(&op_index)
            )
        })
    }

    fn record_definition(&mut self, site: MirSite, lane: MirTempLane) {
        self.definitions
            .entry(lane)
            .or_default()
            .push(MirDefSite { site, lane });
    }

    fn record_use(&mut self, usage: MirUseSite) {
        self.uses
            .entry(usage.requirement.temp())
            .or_default()
            .push(usage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirAddr, MirBinaryOp, MirBlock, MirBlockParam, MirCarryIn, MirCarryOut, MirCond, MirDef,
        MirEdge, MirEdgeArg, MirEffects, MirFrame, MirOp, MirRoutineAbi, MirTerminator, MirValue,
        RoutineId,
    };

    fn routine() -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "uses".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![
                MirBlock {
                    id: MirBlockId(10),
                    label: "entry".to_string(),
                    params: vec![MirBlockParam {
                        dest: MirTempId(1),
                        width: MirWidth::Word,
                    }],
                    ops: vec![
                        MirOp::Load {
                            dst: MirDef::VTemp(MirTempId(2)),
                            src: MirAddr::Deref {
                                ptr: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                                offset: 0,
                            },
                            width: MirWidth::Byte,
                        },
                        MirOp::Binary {
                            op: MirBinaryOp::Add,
                            dst: MirDef::VTemp(MirTempId(3)),
                            left: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                            right: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                            width: MirWidth::Byte,
                            carry_in: Some(MirCarryIn::Clear),
                            carry_out: MirCarryOut::Ignore,
                        },
                    ],
                    terminator: MirTerminator::Jump(MirEdge {
                        target: MirBlockId(20),
                        args: vec![MirEdgeArg {
                            value: MirValue::Def(MirDef::VTemp(MirTempId(3))),
                            width: MirWidth::Byte,
                        }],
                    }),
                },
                MirBlock {
                    id: MirBlockId(20),
                    label: "exit".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Branch {
                        cond: MirCond::BoolValue(MirValue::Def(MirDef::VTemp(MirTempId(3)))),
                        then_edge: MirEdge::plain(MirBlockId(30)),
                        else_edge: MirEdge::plain(MirBlockId(30)),
                    },
                },
                MirBlock {
                    id: MirBlockId(30),
                    label: "return".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: MirTerminator::Return,
                },
            ],
            effects: MirEffects::default(),
        }
    }

    #[test]
    fn indexes_exact_definitions_full_uses_and_occurrences() {
        let index = MirTempUseDefIndex::from_routine(&routine());
        let low = MirTempLane {
            temp: MirTempId(1),
            byte: 0,
        };
        let high = MirTempLane {
            temp: MirTempId(1),
            byte: 1,
        };
        assert_eq!(index.definitions_of_lane(low).len(), 1);
        assert_eq!(index.definitions_of_lane(high).len(), 1);
        assert_eq!(index.definitions_of_temp(MirTempId(1)).count(), 2);
        assert!(matches!(
            index.unique_use(low),
            Some(MirUseSite {
                requirement: MirTempRequirement::Full(MirTempId(1)),
                kind: MirTempUseKind::Address,
                ..
            })
        ));

        let repeated = MirTempLane {
            temp: MirTempId(2),
            byte: 0,
        };
        assert_eq!(index.uses_of_lane(repeated).count(), 2);
        assert_eq!(index.unique_use(repeated), None);
    }

    #[test]
    fn queries_windows_terminators_and_immediate_successors() {
        let routine = routine();
        let cfg = MirCfg::from_routine(&routine).unwrap();
        let index = MirTempUseDefIndex::from_routine(&routine);
        let lane = MirTempLane {
            temp: MirTempId(3),
            byte: 0,
        };
        let definition = index.unique_definition(lane).unwrap();

        assert_eq!(index.uses_in_window(lane, MirBlockId(10), 0..2).count(), 0);
        assert!(index.has_terminator_use(lane, MirBlockId(10)));
        assert!(index.has_successor_use(&cfg, lane, MirBlockId(10)));
        assert!(index.definition_has_uses_outside(definition, MirBlockId(10), 0..2));

        let uses = index.uses_of_temp(MirTempId(3));
        assert_eq!(uses.len(), 2);
        assert_eq!(uses[0].kind, MirTempUseKind::EdgeArgument);
        assert_eq!(uses[1].kind, MirTempUseKind::BranchCondition);
    }
}
