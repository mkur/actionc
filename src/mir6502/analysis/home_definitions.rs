//! Forward reaching-definition analysis for post-home MIR.
//!
//! Home liveness deliberately answers whether a physical byte home is live.
//! This analysis keeps that policy unchanged and additionally attributes every
//! observable read to the concrete store definitions that can reach it.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::dataflow::{
    DataflowDirection, DataflowProblem, DataflowResult, solve_dataflow,
};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::MirHomeByte;
use crate::mir6502::analysis::home_liveness::{
    MirHomeBlockTransfers, action_return_home_uses, collect_home_transfers, collect_home_universe,
};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirBlockId, MirRoutine, MirTerminator};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MirHomeDefinition {
    home: MirHomeByte,
    store: MirSite,
}

/// Forward may-fact. Each home maps to all concrete store definitions that can
/// provide its current value on at least one structurally reachable path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MirHomeDefinitionState {
    definitions: BTreeMap<MirHomeByte, BTreeSet<MirHomeDefinition>>,
}

impl MirHomeDefinitionState {
    fn definitions(&self, home: MirHomeByte) -> impl Iterator<Item = MirHomeDefinition> + '_ {
        self.definitions
            .get(&home)
            .into_iter()
            .flat_map(|definitions| definitions.iter().copied())
    }

    fn join(&mut self, other: &Self) {
        for (home, definitions) in &other.definitions {
            self.definitions
                .entry(*home)
                .or_default()
                .extend(definitions);
        }
    }

    fn define(&mut self, home: MirHomeByte, store: MirSite) {
        self.definitions
            .insert(home, BTreeSet::from([MirHomeDefinition { home, store }]));
    }
}

struct HomeDefinitionProblem<'a> {
    transfers: &'a BTreeMap<MirBlockId, MirHomeBlockTransfers>,
}

impl DataflowProblem<MirCfg> for HomeDefinitionProblem<'_> {
    type State = MirHomeDefinitionState;

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
        let Some(transfers) = self.transfers.get(&node) else {
            return state;
        };
        for (op_index, transfer) in transfers.ops.iter().enumerate() {
            let site = MirSite::Op {
                block: node,
                op_index,
            };
            apply_writes(&mut state, &transfer.writes, site);
        }
        apply_writes(
            &mut state,
            &transfers.terminator.writes,
            MirSite::Terminator { block: node },
        );
        state
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirHomeDefinitionError {
    UnknownBlock(MirBlockId),
    UnreachableBlock(MirBlockId),
    OpOutOfBounds {
        block: MirBlockId,
        op_index: usize,
        op_count: usize,
    },
    StoreSiteIsNotOperation(MirSite),
    InvalidWindow {
        store: MirSite,
        end: MirSite,
    },
    HomeNotWrittenAtStore {
        home: MirHomeByte,
        store: MirSite,
    },
}

/// Concrete uses reached by each post-home store definition.
///
/// Unknown reads are expanded through the same conservative home universe as
/// whole-home liveness. Unknown writes remain may-writes and therefore do not
/// kill a reaching definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct MirHomeDefinitions {
    result: DataflowResult<MirBlockId, MirHomeDefinitionState>,
    transfers: BTreeMap<MirBlockId, MirHomeBlockTransfers>,
    uses: BTreeMap<MirHomeDefinition, BTreeSet<MirSite>>,
    reachable: BTreeSet<MirBlockId>,
}

impl MirHomeDefinitions {
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine, cfg: &MirCfg) -> Self {
        let universe = collect_home_universe(routine);
        let transfers = collect_home_transfers(routine, &universe);
        let result = solve_dataflow(
            cfg,
            &HomeDefinitionProblem {
                transfers: &transfers,
            },
        );
        let uses = collect_definition_uses(routine, cfg, &result, &transfers);
        Self {
            result,
            transfers,
            uses,
            reachable: cfg.reachable().clone(),
        }
    }

    /// Proves that the exact definition at `store_site` has no use outside the
    /// rewrite window. Reads inside the window are ignored because the rewrite
    /// driver validates that transaction separately.
    pub(in crate::mir6502) fn definition_dead_after(
        &self,
        home: MirHomeByte,
        store_site: MirSite,
        window_end: MirSite,
    ) -> Result<bool, MirHomeDefinitionError> {
        let MirSite::Op {
            block,
            op_index: store_index,
        } = store_site
        else {
            return Err(MirHomeDefinitionError::StoreSiteIsNotOperation(store_site));
        };
        let transfers = self
            .transfers
            .get(&block)
            .ok_or(MirHomeDefinitionError::UnknownBlock(block))?;
        if !self.reachable.contains(&block) || self.result.in_state(block).is_none() {
            return Err(MirHomeDefinitionError::UnreachableBlock(block));
        }
        let Some(store_transfer) = transfers.ops.get(store_index) else {
            return Err(MirHomeDefinitionError::OpOutOfBounds {
                block,
                op_index: store_index,
                op_count: transfers.ops.len(),
            });
        };
        if !store_transfer.writes.contains(&home) {
            return Err(MirHomeDefinitionError::HomeNotWrittenAtStore {
                home,
                store: store_site,
            });
        }
        if !valid_window(store_site, window_end, transfers.ops.len()) {
            return Err(MirHomeDefinitionError::InvalidWindow {
                store: store_site,
                end: window_end,
            });
        }

        let definition = MirHomeDefinition {
            home,
            store: store_site,
        };
        Ok(self.uses.get(&definition).is_none_or(|uses| {
            uses.iter()
                .all(|usage| site_inside_window(*usage, store_site, window_end))
        }))
    }
}

fn collect_definition_uses(
    routine: &MirRoutine,
    cfg: &MirCfg,
    result: &DataflowResult<MirBlockId, MirHomeDefinitionState>,
    transfers: &BTreeMap<MirBlockId, MirHomeBlockTransfers>,
) -> BTreeMap<MirHomeDefinition, BTreeSet<MirSite>> {
    let mut uses = BTreeMap::<MirHomeDefinition, BTreeSet<MirSite>>::new();
    for block in routine
        .blocks
        .iter()
        .filter(|block| cfg.reachable().contains(&block.id))
    {
        let Some(mut state) = result.in_state(block.id).cloned() else {
            continue;
        };
        let block_transfers = &transfers[&block.id];
        for (op_index, transfer) in block_transfers.ops.iter().enumerate() {
            let site = MirSite::Op {
                block: block.id,
                op_index,
            };
            record_reads(&state, transfer.reads.iter().copied(), site, &mut uses);
            apply_writes(&mut state, &transfer.writes, site);
        }
        let terminator_site = MirSite::Terminator { block: block.id };
        record_reads(
            &state,
            block_transfers.terminator.reads.iter().copied(),
            terminator_site,
            &mut uses,
        );
        if matches!(
            block.terminator,
            MirTerminator::Return | MirTerminator::Exit
        ) {
            record_reads(
                &state,
                action_return_home_uses().iter(),
                terminator_site,
                &mut uses,
            );
        }
    }
    uses
}

fn record_reads(
    state: &MirHomeDefinitionState,
    reads: impl IntoIterator<Item = MirHomeByte>,
    site: MirSite,
    uses: &mut BTreeMap<MirHomeDefinition, BTreeSet<MirSite>>,
) {
    for home in reads {
        for definition in state.definitions(home) {
            uses.entry(definition).or_default().insert(site);
        }
    }
}

fn apply_writes(state: &mut MirHomeDefinitionState, writes: &BTreeSet<MirHomeByte>, site: MirSite) {
    for home in writes {
        state.define(*home, site);
    }
}

fn valid_window(store: MirSite, end: MirSite, op_count: usize) -> bool {
    let MirSite::Op {
        block,
        op_index: store_index,
    } = store
    else {
        return false;
    };
    match end {
        MirSite::Op {
            block: end_block,
            op_index: end_index,
        } => end_block == block && end_index >= store_index && end_index < op_count,
        MirSite::Terminator { block: end_block } => end_block == block,
        MirSite::BlockEntry { .. } => false,
    }
}

fn site_inside_window(site: MirSite, store: MirSite, end: MirSite) -> bool {
    let MirSite::Op {
        block,
        op_index: store_index,
    } = store
    else {
        return false;
    };
    match (site, end) {
        (
            MirSite::Op {
                block: use_block,
                op_index: use_index,
            },
            MirSite::Op {
                block: end_block,
                op_index: end_index,
            },
        ) => {
            use_block == block
                && end_block == block
                && use_index >= store_index
                && use_index <= end_index
        }
        (
            MirSite::Op {
                block: use_block,
                op_index: use_index,
            },
            MirSite::Terminator { block: end_block },
        ) => use_block == block && end_block == block && use_index >= store_index,
        (MirSite::Terminator { block: use_block }, MirSite::Terminator { block: end_block }) => {
            use_block == block && end_block == block
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirAddr, MirBlock, MirDef, MirEdge, MirEffects, MirFixedZpSlot, MirFrame, MirMem,
        MirMemoryEffect, MirOp, MirReg, MirRoutineAbi, MirSpillId, MirValue, MirWidth, RoutineId,
    };

    fn spill(id: u32) -> MirHomeByte {
        MirHomeByte::Spill {
            id: MirSpillId(id),
            offset: 0,
        }
    }

    fn store(id: u32, value: u8) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(id),
                offset: 0,
            }),
            src: MirValue::ConstU8(value),
            width: MirWidth::Byte,
        }
    }

    fn load(id: u32) -> MirOp {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(id),
                offset: 0,
            }),
            width: MirWidth::Byte,
        }
    }

    fn store_fixed(slot: u8) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
            src: MirValue::ConstU8(1),
            width: MirWidth::Byte,
        }
    }

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn routine(blocks: Vec<MirBlock>) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "definitions".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame {
                spills: vec![MirSpillId(0), MirSpillId(1)],
                ..MirFrame::default()
            },
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn analyze(routine: &MirRoutine) -> MirHomeDefinitions {
        let cfg = MirCfg::from_routine(routine).unwrap();
        MirHomeDefinitions::analyze(routine, &cfg)
    }

    fn op(block: u32, op_index: usize) -> MirSite {
        MirSite::Op {
            block: MirBlockId(block),
            op_index,
        }
    }

    #[test]
    fn attributes_a_read_only_to_the_definition_that_reaches_it() {
        let routine = routine(vec![block(
            0,
            vec![store(0, 1), store(0, 2), load(0)],
            MirTerminator::Return,
        )]);
        let definitions = analyze(&routine);

        assert_eq!(
            definitions.definition_dead_after(spill(0), op(0, 0), op(0, 0)),
            Ok(true)
        );
        assert_eq!(
            definitions.definition_dead_after(spill(0), op(0, 1), op(0, 1)),
            Ok(false)
        );
    }

    #[test]
    fn joins_reaching_definitions_across_cfg_edges() {
        let routine = routine(vec![
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
                vec![store(0, 1)],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            ),
            block(
                2,
                vec![store(0, 2)],
                MirTerminator::Jump(MirEdge::plain(MirBlockId(3))),
            ),
            block(3, vec![load(0)], MirTerminator::Return),
        ]);
        let definitions = analyze(&routine);

        assert_eq!(
            definitions.definition_dead_after(spill(0), op(1, 0), op(1, 0)),
            Ok(false)
        );
        assert_eq!(
            definitions.definition_dead_after(spill(0), op(2, 0), op(2, 0)),
            Ok(false)
        );
    }

    #[test]
    fn follows_a_definition_through_a_loop_backedge() {
        let routine = routine(vec![block(
            0,
            vec![load(0), store(0, 1)],
            MirTerminator::Jump(MirEdge::plain(MirBlockId(0))),
        )]);
        let definitions = analyze(&routine);

        assert_eq!(
            definitions.definition_dead_after(spill(0), op(0, 1), op(0, 1)),
            Ok(false)
        );
    }

    #[test]
    fn ignores_window_local_reads_but_not_later_reads() {
        let routine = routine(vec![block(
            0,
            vec![store(0, 1), load(0), load(0)],
            MirTerminator::Return,
        )]);
        let definitions = analyze(&routine);

        assert_eq!(
            definitions.definition_dead_after(spill(0), op(0, 0), op(0, 1)),
            Ok(false)
        );
        assert_eq!(
            definitions.definition_dead_after(spill(0), op(0, 0), op(0, 2)),
            Ok(true)
        );
    }

    #[test]
    fn opaque_reads_observe_reaching_definitions_and_unknown_writes_do_not_kill() {
        let routine = routine(vec![block(
            0,
            vec![
                store(0, 1),
                MirOp::Barrier {
                    effects: MirEffects {
                        memory_writes: MirMemoryEffect::Unknown,
                        ..MirEffects::default()
                    },
                },
                load(0),
                store(1, 2),
                MirOp::Barrier {
                    effects: MirEffects {
                        memory_reads: MirMemoryEffect::Unknown,
                        ..MirEffects::default()
                    },
                },
            ],
            MirTerminator::Return,
        )]);
        let definitions = analyze(&routine);

        assert_eq!(
            definitions.definition_dead_after(spill(0), op(0, 0), op(0, 0)),
            Ok(false)
        );
        assert_eq!(
            definitions.definition_dead_after(spill(1), op(0, 3), op(0, 3)),
            Ok(false)
        );
    }

    #[test]
    fn action_return_boundary_observes_public_return_slot_definitions() {
        let routine = routine(vec![block(
            0,
            vec![store_fixed(0xA0), store_fixed(0xA2)],
            MirTerminator::Return,
        )]);
        let definitions = analyze(&routine);

        assert_eq!(
            definitions.definition_dead_after(
                MirHomeByte::FixedZeroPage(MirFixedZpSlot(0xA0)),
                op(0, 0),
                op(0, 0),
            ),
            Ok(false)
        );
        assert_eq!(
            definitions.definition_dead_after(
                MirHomeByte::FixedZeroPage(MirFixedZpSlot(0xA2)),
                op(0, 1),
                op(0, 1),
            ),
            Ok(true)
        );
    }

    #[test]
    fn rejects_invalid_definition_queries() {
        let routine = routine(vec![block(0, vec![store(0, 1)], MirTerminator::Return)]);
        let definitions = analyze(&routine);
        assert!(matches!(
            definitions.definition_dead_after(
                spill(0),
                MirSite::BlockEntry {
                    block: MirBlockId(0)
                },
                op(0, 0),
            ),
            Err(MirHomeDefinitionError::StoreSiteIsNotOperation(_))
        ));
        assert!(matches!(
            definitions.definition_dead_after(spill(1), op(0, 0), op(0, 0)),
            Err(MirHomeDefinitionError::HomeNotWrittenAtStore { .. })
        ));
        assert!(matches!(
            definitions.definition_dead_after(spill(0), op(0, 0), op(1, 0)),
            Err(MirHomeDefinitionError::InvalidWindow { .. })
        ));
    }
}
