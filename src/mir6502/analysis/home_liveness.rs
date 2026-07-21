#![allow(dead_code)] // Post-home rewrite consumers arrive in later workflow slices.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::{
    MirHomeByte, MirHomeEffects, MirOpEffectSummary, classify_op, classify_terminator,
};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirBlockId, MirFixedZpSlot, MirRoutine, MirTerminator};

const ACTION_RETURN_HOME_BASE: u8 = 0xA0;
const ACTION_RETURN_HOME_BYTES: u8 = 2;

/// Backward may-liveness for compiler-managed byte homes after temp
/// materialization. A byte is live when some path can read its current value
/// before a definite overwrite.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirHomeLiveSet {
    homes: BTreeSet<MirHomeByte>,
}

impl MirHomeLiveSet {
    pub(in crate::mir6502) fn contains(&self, home: MirHomeByte) -> bool {
        self.homes.contains(&home)
    }

    pub(in crate::mir6502) fn iter(&self) -> impl Iterator<Item = MirHomeByte> + '_ {
        self.homes.iter().copied()
    }

    pub(in crate::mir6502) fn len(&self) -> usize {
        self.homes.len()
    }

    fn insert(&mut self, home: MirHomeByte) {
        self.homes.insert(home);
    }

    fn extend(&mut self, homes: impl IntoIterator<Item = MirHomeByte>) {
        self.homes.extend(homes);
    }

    fn union_with(&mut self, other: &Self) {
        self.homes.extend(other.homes.iter().copied());
    }

    fn subtract(&self, defs: &BTreeSet<MirHomeByte>) -> Self {
        Self {
            homes: self.homes.difference(defs).copied().collect(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirHomeBlockLiveness {
    pub uses: MirHomeLiveSet,
    pub defs: MirHomeLiveSet,
    pub live_in: MirHomeLiveSet,
    pub live_out: MirHomeLiveSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirHomeLivenessError {
    UnknownBlock(MirBlockId),
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MirHomeTransfer {
    reads: BTreeSet<MirHomeByte>,
    writes: BTreeSet<MirHomeByte>,
}

impl MirHomeTransfer {
    fn apply(&self, live_after: &MirHomeLiveSet) -> MirHomeLiveSet {
        let mut live_before = live_after.subtract(&self.writes);
        live_before.extend(self.reads.iter().copied());
        live_before
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MirHomeBlockTransfers {
    ops: Vec<MirHomeTransfer>,
    terminator: MirHomeTransfer,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirHomeLiveness {
    blocks: Vec<MirHomeBlockLiveness>,
    block_indices: BTreeMap<MirBlockId, usize>,
    transfers: BTreeMap<MirBlockId, MirHomeBlockTransfers>,
    universe: MirHomeLiveSet,
    evaluations: usize,
}

impl MirHomeLiveness {
    pub(in crate::mir6502) fn analyze(routine: &MirRoutine, cfg: &MirCfg) -> Self {
        let universe = collect_home_universe(routine);
        let transfers = routine
            .blocks
            .iter()
            .map(|block| {
                let ops = block
                    .ops
                    .iter()
                    .map(|op| op_transfer(&classify_op(op), &universe))
                    .collect();
                let terminator = home_transfer(
                    &classify_terminator(&block.terminator).homes,
                    BTreeSet::new(),
                    BTreeSet::new(),
                    &universe,
                );
                (block.id, MirHomeBlockTransfers { ops, terminator })
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
            .map(|block| (block.id, action_return_home_uses()))
            .collect::<BTreeMap<_, _>>();
        let result = solve_dataflow(
            cfg,
            &HomeLivenessProblem {
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
                MirHomeBlockLiveness {
                    uses: facts.uses.clone(),
                    defs: MirHomeLiveSet {
                        homes: facts.defs.clone(),
                    },
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
            transfers,
            universe,
            evaluations: result.evaluations(),
        }
    }

    pub(in crate::mir6502) fn block_by_id(
        &self,
        block: MirBlockId,
    ) -> Option<&MirHomeBlockLiveness> {
        self.block_indices
            .get(&block)
            .and_then(|index| self.blocks.get(*index))
    }

    pub(in crate::mir6502) fn live_in(&self, block: MirBlockId) -> Option<&MirHomeLiveSet> {
        self.block_by_id(block).map(|facts| &facts.live_in)
    }

    pub(in crate::mir6502) fn live_out(&self, block: MirBlockId) -> Option<&MirHomeLiveSet> {
        self.block_by_id(block).map(|facts| &facts.live_out)
    }

    pub(in crate::mir6502) fn universe(&self) -> &MirHomeLiveSet {
        &self.universe
    }

    pub(in crate::mir6502) fn evaluations(&self) -> usize {
        self.evaluations
    }

    /// Whether a home value is live immediately after `site`.
    pub(in crate::mir6502) fn live_after(
        &self,
        home: MirHomeByte,
        site: MirSite,
    ) -> Result<bool, MirHomeLivenessError> {
        let transfers = self.block_transfers(site.block())?;
        let live_out = self
            .live_out(site.block())
            .ok_or(MirHomeLivenessError::UnknownBlock(site.block()))?;
        let live_after = match site {
            MirSite::BlockEntry { .. } => {
                return Ok(self
                    .live_in(site.block())
                    .is_some_and(|live| live.contains(home)));
            }
            MirSite::Terminator { .. } => live_out.clone(),
            MirSite::Op { op_index, block } => {
                self.validate_op_index(block, op_index, transfers.ops.len())?;
                let mut live = live_out.clone();
                live = transfers.terminator.apply(&live);
                for transfer in transfers.ops[op_index + 1..].iter().rev() {
                    live = transfer.apply(&live);
                }
                live
            }
        };
        Ok(live_after.contains(home))
    }

    /// Proves that the value defined by `store_site` cannot escape the rewrite
    /// window and be read before a definite overwrite. Reads inside the window
    /// are intentionally ignored because transactional validation compares the
    /// original and replacement window effects separately.
    pub(in crate::mir6502) fn home_definition_dead_after(
        &self,
        home: MirHomeByte,
        store_site: MirSite,
        window_end: MirSite,
    ) -> Result<bool, MirHomeLivenessError> {
        let MirSite::Op {
            block,
            op_index: store_index,
        } = store_site
        else {
            return Err(MirHomeLivenessError::StoreSiteIsNotOperation(store_site));
        };
        let transfers = self.block_transfers(block)?;
        self.validate_op_index(block, store_index, transfers.ops.len())?;
        if !transfers.ops[store_index].writes.contains(&home) {
            return Err(MirHomeLivenessError::HomeNotWrittenAtStore {
                home,
                store: store_site,
            });
        }
        let end_index = window_end_index(window_end, block, transfers.ops.len()).ok_or(
            MirHomeLivenessError::InvalidWindow {
                store: store_site,
                end: window_end,
            },
        )?;
        if end_index < store_index {
            return Err(MirHomeLivenessError::InvalidWindow {
                store: store_site,
                end: window_end,
            });
        }

        if store_index < end_index
            && transfers.ops[store_index + 1..=end_index]
                .iter()
                .any(|transfer| transfer.writes.contains(&home))
        {
            return Ok(true);
        }
        self.live_after(home, window_end).map(|live| !live)
    }

    fn block_transfers(
        &self,
        block: MirBlockId,
    ) -> Result<&MirHomeBlockTransfers, MirHomeLivenessError> {
        self.transfers
            .get(&block)
            .ok_or(MirHomeLivenessError::UnknownBlock(block))
    }

    fn validate_op_index(
        &self,
        block: MirBlockId,
        op_index: usize,
        op_count: usize,
    ) -> Result<(), MirHomeLivenessError> {
        if op_index < op_count {
            Ok(())
        } else {
            Err(MirHomeLivenessError::OpOutOfBounds {
                block,
                op_index,
                op_count,
            })
        }
    }
}

#[derive(Debug, Clone, Default)]
struct MirHomeBlockFacts {
    uses: MirHomeLiveSet,
    defs: BTreeSet<MirHomeByte>,
}

struct HomeLivenessProblem<'a> {
    facts: &'a BTreeMap<MirBlockId, MirHomeBlockFacts>,
    boundaries: &'a BTreeMap<MirBlockId, MirHomeLiveSet>,
}

impl DataflowProblem<MirCfg> for HomeLivenessProblem<'_> {
    type State = MirHomeLiveSet;

    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Backward
    }

    fn bottom(&self) -> Self::State {
        Self::State::default()
    }

    fn boundary(&self, node: MirBlockId) -> Option<Self::State> {
        self.boundaries.get(&node).cloned()
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.union_with(other);
    }

    fn transfer(&self, node: MirBlockId, live_out: &Self::State) -> Self::State {
        let facts = &self.facts[&node];
        let mut live_in = facts.uses.clone();
        live_in.union_with(&live_out.subtract(&facts.defs));
        live_in
    }
}

fn collect_home_universe(routine: &MirRoutine) -> MirHomeLiveSet {
    let mut universe = MirHomeLiveSet::default();
    for block in &routine.blocks {
        for op in &block.ops {
            let effects = classify_op(op);
            universe.extend(effects.homes.reads);
            universe.extend(effects.homes.writes);
            universe.extend(effects.addresses.pair_reads);
            universe.extend(effects.addresses.pair_writes);
        }
        let effects = classify_terminator(&block.terminator);
        universe.extend(effects.homes.reads);
        universe.extend(effects.homes.writes);
    }
    if routine.blocks.iter().any(|block| {
        matches!(
            block.terminator,
            MirTerminator::Return | MirTerminator::Exit
        )
    }) {
        universe.union_with(&action_return_home_uses());
    }
    universe
}

fn action_return_home_uses() -> MirHomeLiveSet {
    let mut uses = MirHomeLiveSet::default();
    for offset in 0..ACTION_RETURN_HOME_BYTES {
        uses.insert(MirHomeByte::FixedZeroPage(MirFixedZpSlot(
            ACTION_RETURN_HOME_BASE.saturating_add(offset),
        )));
    }
    uses
}

fn op_transfer(effects: &MirOpEffectSummary, universe: &MirHomeLiveSet) -> MirHomeTransfer {
    home_transfer(
        &effects.homes,
        effects.addresses.pair_reads.clone(),
        effects.addresses.pair_writes.clone(),
        universe,
    )
}

fn home_transfer(
    effects: &MirHomeEffects,
    pair_reads: BTreeSet<MirHomeByte>,
    pair_writes: BTreeSet<MirHomeByte>,
    universe: &MirHomeLiveSet,
) -> MirHomeTransfer {
    let mut reads = effects.reads.clone();
    reads.extend(pair_reads);
    if effects.unknown_reads {
        reads.extend(universe.iter());
    }
    let mut writes = effects.writes.clone();
    writes.extend(pair_writes);
    // `unknown_writes` is a may-write, not a definite kill. The old value can
    // survive it on some path and must remain live when a later read exists.
    MirHomeTransfer { reads, writes }
}

fn block_uses_and_defs(transfers: &MirHomeBlockTransfers) -> MirHomeBlockFacts {
    let mut facts = MirHomeBlockFacts::default();
    for transfer in transfers
        .ops
        .iter()
        .chain(std::iter::once(&transfers.terminator))
    {
        for read in &transfer.reads {
            if !facts.defs.contains(read) {
                facts.uses.insert(*read);
            }
        }
        facts.defs.extend(transfer.writes.iter().copied());
    }
    facts
}

fn window_end_index(end: MirSite, block: MirBlockId, op_count: usize) -> Option<usize> {
    match end {
        MirSite::Op {
            block: end_block,
            op_index,
        } if end_block == block && op_index < op_count => Some(op_index),
        MirSite::Terminator { block: end_block } if end_block == block => op_count.checked_sub(1),
        MirSite::BlockEntry { .. } | MirSite::Op { .. } | MirSite::Terminator { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirAddr, MirAddressConsumer, MirBlock, MirDef, MirEdge, MirEffects, MirFixedZpSlot,
        MirFrame, MirMem, MirMemoryEffect, MirOp, MirPointerPair, MirReg, MirRoutineAbi,
        MirSpillId, MirTerminator, MirValue, MirWidth, RoutineId,
    };

    fn spill(id: u32) -> MirHomeByte {
        MirHomeByte::Spill {
            id: MirSpillId(id),
            offset: 0,
        }
    }

    fn spill_mem(id: u32) -> MirMem {
        MirMem::Spill {
            id: MirSpillId(id),
            offset: 0,
        }
    }

    fn store(id: u32, value: u8) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(spill_mem(id)),
            src: MirValue::ConstU8(value),
            width: MirWidth::Byte,
        }
    }

    fn load(id: u32) -> MirOp {
        MirOp::Load {
            dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(spill_mem(id)),
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
            name: "HomeLiveness".to_string(),
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

    fn analyze(routine: &MirRoutine) -> MirHomeLiveness {
        let cfg = MirCfg::from_routine(routine).unwrap();
        MirHomeLiveness::analyze(routine, &cfg)
    }

    fn op_site(block: u32, op_index: usize) -> MirSite {
        MirSite::Op {
            block: MirBlockId(block),
            op_index,
        }
    }

    #[test]
    fn successor_read_keeps_store_live_and_successor_overwrite_kills_it() {
        let read_successor = routine(vec![
            block(
                0,
                vec![store(0, 1)],
                MirTerminator::Jump(MirEdge {
                    target: MirBlockId(1),
                    args: Vec::new(),
                }),
            ),
            block(1, vec![load(0)], MirTerminator::Return),
        ]);
        let liveness = analyze(&read_successor);
        assert!(liveness.live_out(MirBlockId(0)).unwrap().contains(spill(0)));
        assert_eq!(
            liveness.home_definition_dead_after(spill(0), op_site(0, 0), op_site(0, 0)),
            Ok(false)
        );

        let overwrite_successor = routine(vec![
            block(
                0,
                vec![store(0, 1)],
                MirTerminator::Jump(MirEdge {
                    target: MirBlockId(1),
                    args: Vec::new(),
                }),
            ),
            block(1, vec![store(0, 2), load(0)], MirTerminator::Return),
        ]);
        let liveness = analyze(&overwrite_successor);
        assert!(!liveness.live_out(MirBlockId(0)).unwrap().contains(spill(0)));
        assert_eq!(
            liveness.home_definition_dead_after(spill(0), op_site(0, 0), op_site(0, 0)),
            Ok(true)
        );
    }

    #[test]
    fn join_and_loop_follow_every_structural_path() {
        let routine = routine(vec![
            block(
                0,
                vec![store(0, 1)],
                MirTerminator::Branch {
                    cond: crate::mir6502::ir::MirCond::BoolValue(MirValue::ConstU8(1)),
                    then_edge: MirEdge {
                        target: MirBlockId(1),
                        args: Vec::new(),
                    },
                    else_edge: MirEdge {
                        target: MirBlockId(2),
                        args: Vec::new(),
                    },
                },
            ),
            block(
                1,
                vec![store(0, 2)],
                MirTerminator::Jump(MirEdge {
                    target: MirBlockId(3),
                    args: Vec::new(),
                }),
            ),
            block(
                2,
                Vec::new(),
                MirTerminator::Jump(MirEdge {
                    target: MirBlockId(3),
                    args: Vec::new(),
                }),
            ),
            block(
                3,
                vec![load(0)],
                MirTerminator::Jump(MirEdge {
                    target: MirBlockId(3),
                    args: Vec::new(),
                }),
            ),
        ]);
        let liveness = analyze(&routine);
        assert!(liveness.live_out(MirBlockId(0)).unwrap().contains(spill(0)));
        assert!(liveness.evaluations() > routine.blocks.len());
        assert_eq!(
            liveness.home_definition_dead_after(spill(0), op_site(0, 0), op_site(0, 0)),
            Ok(false)
        );
    }

    #[test]
    fn local_overwrite_kills_only_the_older_definition() {
        let routine = routine(vec![block(
            0,
            vec![store(0, 1), store(0, 2), load(0)],
            MirTerminator::Return,
        )]);
        let liveness = analyze(&routine);
        assert_eq!(
            liveness.home_definition_dead_after(spill(0), op_site(0, 0), op_site(0, 0)),
            Ok(true)
        );
        assert_eq!(
            liveness.home_definition_dead_after(spill(0), op_site(0, 1), op_site(0, 1)),
            Ok(false)
        );
        assert_eq!(
            liveness.home_definition_dead_after(
                spill(0),
                op_site(0, 1),
                MirSite::Terminator {
                    block: MirBlockId(0)
                }
            ),
            Ok(true)
        );
    }

    #[test]
    fn address_consumer_reads_both_pointer_home_bytes() {
        let lo = MirHomeByte::FixedZeroPage(MirFixedZpSlot(0xAC));
        let hi = MirHomeByte::FixedZeroPage(MirFixedZpSlot(0xAD));
        let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(0xAC),
        });
        let routine = routine(vec![block(
            0,
            vec![MirOp::LoadIndirect {
                consumer,
                dst: MirDef::Reg(MirReg::A),
                offset: 0,
            }],
            MirTerminator::Return,
        )]);
        let liveness = analyze(&routine);
        let live_in = liveness.live_in(MirBlockId(0)).unwrap();
        assert!(live_in.contains(lo));
        assert!(live_in.contains(hi));
    }

    #[test]
    fn action_return_slot_bytes_are_abi_boundary_uses() {
        let lo = MirHomeByte::FixedZeroPage(MirFixedZpSlot(0xA0));
        let hi = MirHomeByte::FixedZeroPage(MirFixedZpSlot(0xA1));
        let routine = routine(vec![block(0, Vec::new(), MirTerminator::Return)]);
        let liveness = analyze(&routine);
        let live_out = liveness.live_out(MirBlockId(0)).unwrap();
        assert!(live_out.contains(lo));
        assert!(live_out.contains(hi));
    }

    #[test]
    fn unknown_reads_expose_all_homes_and_unknown_writes_do_not_kill() {
        let opaque_read = MirOp::Barrier {
            effects: MirEffects {
                memory_reads: MirMemoryEffect::Unknown,
                ..MirEffects::default()
            },
        };
        let unknown_write = MirOp::Barrier {
            effects: MirEffects {
                memory_writes: MirMemoryEffect::Unknown,
                ..MirEffects::default()
            },
        };
        let routine = routine(vec![block(
            0,
            vec![
                store(0, 1),
                unknown_write,
                load(0),
                store(1, 2),
                opaque_read,
            ],
            MirTerminator::Return,
        )]);
        let liveness = analyze(&routine);
        assert_eq!(
            liveness.home_definition_dead_after(spill(0), op_site(0, 0), op_site(0, 0)),
            Ok(false)
        );
        assert_eq!(
            liveness.home_definition_dead_after(spill(1), op_site(0, 3), op_site(0, 3)),
            Ok(false)
        );
        assert!(liveness.universe().contains(spill(0)));
        assert!(liveness.universe().contains(spill(1)));
    }

    #[test]
    fn query_rejects_non_store_and_invalid_windows() {
        let routine = routine(vec![block(0, vec![store(0, 1)], MirTerminator::Return)]);
        let liveness = analyze(&routine);
        assert_eq!(
            liveness.home_definition_dead_after(
                spill(0),
                MirSite::BlockEntry {
                    block: MirBlockId(0)
                },
                op_site(0, 0)
            ),
            Err(MirHomeLivenessError::StoreSiteIsNotOperation(
                MirSite::BlockEntry {
                    block: MirBlockId(0)
                }
            ))
        );
        assert!(matches!(
            liveness.home_definition_dead_after(spill(1), op_site(0, 0), op_site(0, 0)),
            Err(MirHomeLivenessError::HomeNotWrittenAtStore { .. })
        ));
        assert!(matches!(
            liveness.home_definition_dead_after(spill(0), op_site(0, 0), op_site(1, 0)),
            Err(MirHomeLivenessError::InvalidWindow { .. })
        ));
    }
}
