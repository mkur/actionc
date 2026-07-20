#![allow(dead_code)] // Rewrite contexts consume the query surface in later slices.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::dataflow::{
    DataflowDirection, DataflowProblem, DataflowResult, solve_dataflow,
};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::{MirTempAccess, classify_op};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::analysis::use_def::{MirDefSite, MirTempLane, MirUseSite};
use crate::mir6502::ir::{MirBlockId, MirRoutine, MirWidth};

/// Forward may-fact: every definition that can reach a lane on at least one
/// structurally reachable path is retained. Join is set union and defining a
/// lane replaces the reaching set for that lane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirReachingDefinitionState {
    definitions: BTreeMap<MirTempLane, BTreeSet<MirDefSite>>,
}

impl MirReachingDefinitionState {
    pub(in crate::mir6502) fn definitions(&self, lane: MirTempLane) -> &BTreeSet<MirDefSite> {
        self.definitions.get(&lane).unwrap_or(&EMPTY_DEFINITIONS)
    }

    fn join(&mut self, other: &Self) {
        for (lane, definitions) in &other.definitions {
            self.definitions
                .entry(*lane)
                .or_default()
                .extend(definitions);
        }
    }

    fn define(&mut self, definition: MirDefSite) {
        self.definitions
            .insert(definition.lane, BTreeSet::from([definition]));
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MirBlockDefinitions {
    entry: Vec<MirDefSite>,
    ops: Vec<Vec<MirDefSite>>,
}

#[derive(Debug)]
struct ReachingDefinitionProblem<'a> {
    blocks: &'a BTreeMap<MirBlockId, MirBlockDefinitions>,
}

impl DataflowProblem<MirCfg> for ReachingDefinitionProblem<'_> {
    type State = MirReachingDefinitionState;

    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Forward
    }

    fn bottom(&self) -> Self::State {
        Self::State::default()
    }

    fn boundary(&self, _node: MirBlockId) -> Option<Self::State> {
        None
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.join(other);
    }

    fn transfer(&self, node: MirBlockId, state: &Self::State) -> Self::State {
        let mut state = state.clone();
        if let Some(block) = self.blocks.get(&node) {
            apply_definitions(&mut state, &block.entry);
            for definitions in &block.ops {
                apply_definitions(&mut state, definitions);
            }
        }
        state
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirReachingDefinitionError {
    UnknownBlock(MirBlockId),
    UnreachableBlock(MirBlockId),
    OpOutOfBounds {
        block: MirBlockId,
        op_index: usize,
        op_count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirReachingDefinitions {
    result: DataflowResult<MirBlockId, MirReachingDefinitionState>,
    blocks: BTreeMap<MirBlockId, MirBlockDefinitions>,
}

impl MirReachingDefinitions {
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine, cfg: &MirCfg) -> Self {
        let blocks = routine
            .blocks
            .iter()
            .map(|block| {
                let entry_site = MirSite::BlockEntry { block: block.id };
                let mut entry = Vec::new();
                for param in &block.params {
                    entry.push(MirDefSite {
                        site: entry_site,
                        lane: MirTempLane {
                            temp: param.dest,
                            byte: 0,
                        },
                    });
                    if param.width == MirWidth::Word {
                        entry.push(MirDefSite {
                            site: entry_site,
                            lane: MirTempLane {
                                temp: param.dest,
                                byte: 1,
                            },
                        });
                    }
                }
                let ops = block
                    .ops
                    .iter()
                    .enumerate()
                    .map(|(op_index, op)| {
                        let site = MirSite::Op {
                            block: block.id,
                            op_index,
                        };
                        classify_op(op)
                            .logical
                            .temp_defs
                            .into_iter()
                            .flat_map(move |access| definition_sites(site, access))
                            .collect()
                    })
                    .collect();
                (block.id, MirBlockDefinitions { entry, ops })
            })
            .collect::<BTreeMap<_, _>>();
        let result = solve_dataflow(cfg, &ReachingDefinitionProblem { blocks: &blocks });
        Self { result, blocks }
    }

    pub(in crate::mir6502) fn block_in(
        &self,
        block: MirBlockId,
    ) -> Option<&MirReachingDefinitionState> {
        self.result.in_state(block)
    }

    pub(in crate::mir6502) fn block_out(
        &self,
        block: MirBlockId,
    ) -> Option<&MirReachingDefinitionState> {
        self.result.out_state(block)
    }

    pub(in crate::mir6502) fn state_before(
        &self,
        site: MirSite,
    ) -> Result<MirReachingDefinitionState, MirReachingDefinitionError> {
        let block = site.block();
        let Some(mut state) = self.result.in_state(block).cloned() else {
            if self.blocks.contains_key(&block) {
                return Err(MirReachingDefinitionError::UnreachableBlock(block));
            }
            return Err(MirReachingDefinitionError::UnknownBlock(block));
        };
        let block_definitions = self.blocks.get(&block).expect("known analysis block");
        if matches!(site, MirSite::BlockEntry { .. }) {
            return Ok(state);
        }
        apply_definitions(&mut state, &block_definitions.entry);
        let stop = match site {
            MirSite::BlockEntry { .. } => 0,
            MirSite::Op { op_index, .. } => {
                if op_index >= block_definitions.ops.len() {
                    return Err(MirReachingDefinitionError::OpOutOfBounds {
                        block,
                        op_index,
                        op_count: block_definitions.ops.len(),
                    });
                }
                op_index
            }
            MirSite::Terminator { .. } => block_definitions.ops.len(),
        };
        for definitions in &block_definitions.ops[..stop] {
            apply_definitions(&mut state, definitions);
        }
        Ok(state)
    }

    pub(in crate::mir6502) fn definitions_reaching_site(
        &self,
        site: MirSite,
        lane: MirTempLane,
    ) -> Result<BTreeSet<MirDefSite>, MirReachingDefinitionError> {
        Ok(self.state_before(site)?.definitions(lane).clone())
    }

    pub(in crate::mir6502) fn unique_reaching_definition(
        &self,
        usage: MirUseSite,
        lane: MirTempLane,
    ) -> Result<Option<MirDefSite>, MirReachingDefinitionError> {
        if !usage.requirement.requires(lane) {
            return Ok(None);
        }
        let definitions = self.definitions_reaching_site(usage.site, lane)?;
        Ok((definitions.len() == 1).then(|| *definitions.first().unwrap()))
    }

    pub(in crate::mir6502) fn definition_reaches_use(
        &self,
        definition: MirDefSite,
        usage: MirUseSite,
    ) -> Result<bool, MirReachingDefinitionError> {
        if !usage.requirement.requires(definition.lane) {
            return Ok(false);
        }
        Ok(self
            .definitions_reaching_site(usage.site, definition.lane)?
            .contains(&definition))
    }

    pub(in crate::mir6502) fn value_available_at(
        &self,
        definition: MirDefSite,
        point: MirSite,
    ) -> Result<bool, MirReachingDefinitionError> {
        let definitions = self.definitions_reaching_site(point, definition.lane)?;
        Ok(definitions == BTreeSet::from([definition]))
    }

    pub(in crate::mir6502) fn evaluations(&self) -> usize {
        self.result.evaluations()
    }
}

fn definition_sites(site: MirSite, access: MirTempAccess) -> Vec<MirDefSite> {
    match access {
        MirTempAccess::Exact { temp, byte } => vec![MirDefSite {
            site,
            lane: MirTempLane { temp, byte },
        }],
        MirTempAccess::Full(temp) => vec![
            MirDefSite {
                site,
                lane: MirTempLane { temp, byte: 0 },
            },
            MirDefSite {
                site,
                lane: MirTempLane { temp, byte: 1 },
            },
        ],
    }
}

fn apply_definitions(state: &mut MirReachingDefinitionState, definitions: &[MirDefSite]) {
    for definition in definitions {
        state.define(*definition);
    }
}

static EMPTY_DEFINITIONS: BTreeSet<MirDefSite> = BTreeSet::new();

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::effects::MirTempUseKind;
    use crate::mir6502::analysis::use_def::MirTempRequirement;
    use crate::mir6502::ir::{
        MirBlock, MirCond, MirDef, MirEdge, MirEffects, MirFrame, MirOp, MirRoutineAbi,
        MirTerminator, MirValue, RoutineId,
    };

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn definition(temp: u32, value: u16, width: MirWidth) -> MirOp {
        MirOp::LoadImm {
            dst: MirDef::VTemp(crate::mir6502::ir::MirTempId(temp)),
            value,
            width,
        }
    }

    fn routine(blocks: Vec<MirBlock>) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "reaching".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn usage(site: MirSite, lane: MirTempLane) -> MirUseSite {
        MirUseSite {
            site,
            requirement: MirTempRequirement::Exact(lane),
            kind: MirTempUseKind::Operand,
        }
    }

    #[test]
    fn union_join_retains_both_definitions_at_diamond_merge() {
        let routine = routine(vec![
            block(
                0,
                Vec::new(),
                MirTerminator::Branch {
                    cond: MirCond::BoolValue(MirValue::ConstU8(1)),
                    then_edge: MirEdge::plain(MirBlockId(1)),
                    else_edge: MirEdge::plain(MirBlockId(2)),
                },
            ),
            block(
                1,
                vec![definition(1, 10, MirWidth::Byte)],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            ),
            block(
                2,
                vec![definition(1, 20, MirWidth::Byte)],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            ),
            block(
                3,
                vec![MirOp::Move {
                    dst: MirDef::VTemp(crate::mir6502::ir::MirTempId(2)),
                    src: MirValue::Def(MirDef::VTemp(crate::mir6502::ir::MirTempId(1))),
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            ),
        ]);
        let cfg = MirCfg::from_routine(&routine).unwrap();
        let reaching = MirReachingDefinitions::analyze(&routine, &cfg);
        let lane = MirTempLane {
            temp: crate::mir6502::ir::MirTempId(1),
            byte: 0,
        };
        let use_site = usage(
            MirSite::Op {
                block: MirBlockId(3),
                op_index: 0,
            },
            lane,
        );

        let definitions = reaching
            .definitions_reaching_site(use_site.site, lane)
            .unwrap();
        assert_eq!(definitions.len(), 2);
        assert_eq!(
            reaching.unique_reaching_definition(use_site, lane),
            Ok(None)
        );
        for definition in definitions {
            assert_eq!(
                reaching.definition_reaches_use(definition, use_site),
                Ok(true)
            );
        }
    }

    #[test]
    fn lane_redefinition_kills_only_that_word_lane() {
        let routine = routine(vec![block(
            0,
            vec![
                definition(1, 0x1234, MirWidth::Word),
                definition(1, 0x56, MirWidth::Byte),
                MirOp::Move {
                    dst: MirDef::VTemp(crate::mir6502::ir::MirTempId(2)),
                    src: MirValue::Def(MirDef::VTemp(crate::mir6502::ir::MirTempId(1))),
                    width: MirWidth::Word,
                },
            ],
            MirTerminator::Return,
        )]);
        let cfg = MirCfg::from_routine(&routine).unwrap();
        let reaching = MirReachingDefinitions::analyze(&routine, &cfg);
        let point = MirSite::Op {
            block: MirBlockId(0),
            op_index: 2,
        };
        let low = MirTempLane {
            temp: crate::mir6502::ir::MirTempId(1),
            byte: 0,
        };
        let high = MirTempLane {
            temp: crate::mir6502::ir::MirTempId(1),
            byte: 1,
        };
        let low_definition = *reaching
            .definitions_reaching_site(point, low)
            .unwrap()
            .first()
            .unwrap();
        let high_definition = *reaching
            .definitions_reaching_site(point, high)
            .unwrap()
            .first()
            .unwrap();
        assert_eq!(
            low_definition.site,
            MirSite::Op {
                block: MirBlockId(0),
                op_index: 1,
            }
        );
        assert_eq!(
            high_definition.site,
            MirSite::Op {
                block: MirBlockId(0),
                op_index: 0,
            }
        );
        assert!(reaching.value_available_at(low_definition, point).unwrap());
        assert!(reaching.value_available_at(high_definition, point).unwrap());
    }

    #[test]
    fn unreachable_blocks_have_no_reaching_state() {
        let routine = routine(vec![
            block(0, Vec::new(), MirTerminator::Return),
            block(
                9,
                vec![definition(1, 1, MirWidth::Byte)],
                MirTerminator::Return,
            ),
        ]);
        let cfg = MirCfg::from_routine(&routine).unwrap();
        let reaching = MirReachingDefinitions::analyze(&routine, &cfg);
        assert_eq!(
            reaching.state_before(MirSite::Terminator {
                block: MirBlockId(9),
            }),
            Err(MirReachingDefinitionError::UnreachableBlock(MirBlockId(9)))
        );
    }
}
