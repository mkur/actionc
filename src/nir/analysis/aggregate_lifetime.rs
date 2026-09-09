//! Must-availability of one captured byte image. A copy establishes the fact;
//! writes/effects kill it, joins intersect it, and loop entries may reestablish
//! it. Only consumers require availability, not every path to routine exit.

use std::collections::BTreeMap;

use super::{
    cfg::NirCfg,
    dataflow::{NirDataflowDirection, NirDataflowProblem, solve_dataflow},
};
use crate::nir::*;

pub(in crate::nir) struct AggregateLifetime {
    available: BTreeMap<(BlockId, usize), bool>,
}

impl AggregateLifetime {
    pub(in crate::nir) fn analyze(
        routine: &NirRoutine,
        facts: &NirRoutineAggregateRegions<'_>,
        destination: &NirPlace,
        source: &NirPlace,
        size: ByteSize,
        initialized: NirAggregatePoint,
    ) -> Self {
        let cfg = NirCfg::from_routine(routine);
        let transfers = routine
            .blocks
            .iter()
            .map(|block| {
                let ops = block
                    .ops
                    .iter()
                    .enumerate()
                    .map(|(op_index, _)| {
                        let at = NirAggregatePoint {
                            block: block.id,
                            op_index,
                        };
                        if at == initialized {
                            Transfer::Define
                        } else {
                            let end = NirAggregatePoint {
                                op_index: op_index + 1,
                                ..at
                            };
                            if facts.unchanged_between(source, size, at, end).is_ok()
                                && facts.unchanged_between(destination, size, at, end).is_ok()
                            {
                                Transfer::Preserve
                            } else {
                                Transfer::Kill
                            }
                        }
                    })
                    .collect::<Vec<_>>();
                (block.id, ops)
            })
            .collect();
        let problem = Availability {
            entry: cfg.entry(),
            transfers,
        };
        let solution = solve_dataflow(&cfg, &problem);
        let mut available = BTreeMap::new();
        for block in &routine.blocks {
            let mut state = cfg.reachable().contains(&block.id)
                && solution.in_state(block.id).copied().unwrap_or(false);
            for (index, transfer) in problem.transfers[&block.id].iter().enumerate() {
                available.insert((block.id, index), state);
                state = transfer.apply(state);
            }
            available.insert((block.id, block.ops.len()), state);
        }
        Self { available }
    }

    pub(in crate::nir) fn at(&self, point: NirAggregatePoint) -> bool {
        self.available
            .get(&(point.block, point.op_index))
            .copied()
            .unwrap_or(false)
    }
}

enum Transfer {
    Define,
    Preserve,
    Kill,
}

impl Transfer {
    fn apply(&self, state: bool) -> bool {
        match self {
            Self::Define => true,
            Self::Preserve => state,
            Self::Kill => false,
        }
    }
}

struct Availability {
    entry: Option<BlockId>,
    transfers: BTreeMap<BlockId, Vec<Transfer>>,
}

impl NirDataflowProblem for Availability {
    type State = bool;
    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Forward
    }
    // Top for a must-analysis. Entry has no established snapshot; reachable
    // predecessor facts monotonically remove unsupported availability.
    fn bottom(&self) -> bool {
        true
    }
    fn boundary(&self, block: BlockId) -> Option<bool> {
        (Some(block) == self.entry).then_some(false)
    }
    fn join(&self, into: &mut bool, other: &bool) {
        *into &= *other;
    }
    fn transfer(&self, block: BlockId, state: &bool) -> bool {
        self.transfers[&block]
            .iter()
            .fold(*state, |state, transfer| transfer.apply(state))
    }
}
