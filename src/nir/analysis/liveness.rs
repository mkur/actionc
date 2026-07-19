use std::collections::{BTreeMap, BTreeSet};

use super::cfg::NirCfg;
use super::dataflow::{
    NirDataflowDirection, NirDataflowProblem, NirDataflowResult, solve_dataflow,
};
use super::use_def::{NirDefSite, NirUseDef, NirUseSite};
use crate::nir::{BlockId, NirRoutine, TempId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::nir) struct NirTempLiveness {
    result: NirDataflowResult<BTreeSet<TempId>>,
}

impl NirTempLiveness {
    pub(in crate::nir) fn analyze(routine: &NirRoutine, cfg: &NirCfg, use_def: &NirUseDef) -> Self {
        let problem = TempLivenessProblem::new(routine, cfg, use_def);
        Self {
            result: solve_dataflow(cfg, &problem),
        }
    }

    #[allow(dead_code)] // Part of the shared query surface; DCE currently needs live-out only.
    pub(in crate::nir) fn live_in(&self, block: BlockId) -> &BTreeSet<TempId> {
        self.result.in_state(block).unwrap_or(&EMPTY_TEMP_SET)
    }

    pub(in crate::nir) fn live_out(&self, block: BlockId) -> &BTreeSet<TempId> {
        self.result.out_state(block).unwrap_or(&EMPTY_TEMP_SET)
    }

    #[allow(dead_code)] // Retained for convergence diagnostics and focused solver tests.
    pub(in crate::nir) fn evaluations(&self) -> usize {
        self.result.evaluations()
    }
}

struct TempLivenessProblem {
    definitions: BTreeMap<BlockId, BTreeSet<TempId>>,
    upward_uses: BTreeMap<BlockId, BTreeSet<TempId>>,
}

impl TempLivenessProblem {
    fn new(routine: &NirRoutine, cfg: &NirCfg, use_def: &NirUseDef) -> Self {
        let mut definitions = cfg
            .reachable()
            .iter()
            .copied()
            .map(|block| (block, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        let mut upward_uses = definitions.clone();

        for (temp, sites) in use_def.all_definitions() {
            for site in sites {
                if cfg.reachable().contains(&site.block) {
                    definitions.entry(site.block).or_default().insert(*temp);
                }
            }
        }
        for (temp, sites) in use_def.all_uses() {
            for site in sites {
                if !cfg.reachable().contains(&site.block()) {
                    continue;
                }
                if !use_def
                    .definitions(*temp)
                    .iter()
                    .any(|definition| definition_precedes_use(*definition, *site))
                {
                    upward_uses.entry(site.block()).or_default().insert(*temp);
                }
            }
        }

        debug_assert!(
            routine
                .blocks
                .iter()
                .filter(|block| cfg.reachable().contains(&block.id))
                .all(|block| definitions.contains_key(&block.id)
                    && upward_uses.contains_key(&block.id))
        );
        Self {
            definitions,
            upward_uses,
        }
    }
}

impl NirDataflowProblem for TempLivenessProblem {
    type State = BTreeSet<TempId>;

    fn direction(&self) -> NirDataflowDirection {
        NirDataflowDirection::Backward
    }

    fn bottom(&self) -> Self::State {
        BTreeSet::new()
    }

    fn boundary(&self, _block: BlockId) -> Option<Self::State> {
        None
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.extend(other);
    }

    fn transfer(&self, block: BlockId, live_out: &Self::State) -> Self::State {
        let mut live_in = live_out.clone();
        if let Some(definitions) = self.definitions.get(&block) {
            live_in.retain(|temp| !definitions.contains(temp));
        }
        if let Some(uses) = self.upward_uses.get(&block) {
            live_in.extend(uses);
        }
        live_in
    }
}

fn definition_precedes_use(definition: NirDefSite, usage: NirUseSite) -> bool {
    if definition.block != usage.block() {
        return false;
    }
    match usage.op_index() {
        Some(use_index) => definition.op_index < use_index,
        None => true,
    }
}

static EMPTY_TEMP_SET: BTreeSet<TempId> = BTreeSet::new();

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{NirBinaryOp, NirBlock, NirOp, NirTerminator, NirType, NirTypeKind, NirValue};

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "BYTE".to_string(),
            width: Some(1),
            pointer: false,
        }
    }

    fn temp(id: u32) -> NirValue {
        NirValue::Temp {
            id: TempId(id),
            ty: byte_type(),
        }
    }

    fn binary(dest: u32, left: NirValue) -> NirOp {
        NirOp::Binary {
            dest: TempId(dest),
            ty: byte_type(),
            op: NirBinaryOp::Add,
            left,
            right: NirValue::ConstU8(1),
        }
    }

    fn block(id: u32, label: &str, ops: Vec<NirOp>, terminator: NirTerminator) -> NirBlock {
        NirBlock {
            id: BlockId(id),
            label: label.to_string(),
            ops,
            terminator,
        }
    }

    fn routine(blocks: Vec<NirBlock>) -> NirRoutine {
        NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks,
        }
    }

    #[test]
    fn computes_diamond_liveness_and_ignores_unreachable_blocks() {
        let routine = routine(vec![
            block(
                0,
                "entry",
                vec![binary(0, NirValue::ConstU8(1))],
                NirTerminator::Branch {
                    condition: NirValue::ConstU8(1),
                    then_label: "left".to_string(),
                    else_label: "right".to_string(),
                },
            ),
            block(
                1,
                "left",
                vec![binary(1, temp(0))],
                NirTerminator::Goto("join".to_string()),
            ),
            block(
                2,
                "right",
                Vec::new(),
                NirTerminator::Goto("join".to_string()),
            ),
            block(3, "join", Vec::new(), NirTerminator::Return(Some(temp(0)))),
            block(9, "dead", Vec::new(), NirTerminator::Return(Some(temp(99)))),
        ]);
        let cfg = NirCfg::from_routine(&routine);
        let use_def = NirUseDef::from_routine(&routine);
        let liveness = NirTempLiveness::analyze(&routine, &cfg, &use_def);

        assert!(liveness.live_in(BlockId(0)).is_empty());
        assert_eq!(liveness.live_out(BlockId(0)), &BTreeSet::from([TempId(0)]));
        assert_eq!(liveness.live_in(BlockId(1)), &BTreeSet::from([TempId(0)]));
        assert_eq!(liveness.live_out(BlockId(1)), &BTreeSet::from([TempId(0)]));
        assert_eq!(liveness.live_in(BlockId(2)), &BTreeSet::from([TempId(0)]));
        assert_eq!(liveness.live_in(BlockId(3)), &BTreeSet::from([TempId(0)]));
        assert!(liveness.live_out(BlockId(3)).is_empty());
        assert!(liveness.live_in(BlockId(9)).is_empty());
        assert!(!liveness.live_in(BlockId(1)).contains(&TempId(1)));
    }

    #[test]
    fn converges_across_a_loop_backedge() {
        let routine = routine(vec![
            block(
                0,
                "entry",
                vec![binary(0, NirValue::ConstU8(1))],
                NirTerminator::Goto("header".to_string()),
            ),
            block(
                1,
                "header",
                vec![binary(1, temp(0))],
                NirTerminator::Branch {
                    condition: temp(1),
                    then_label: "body".to_string(),
                    else_label: "exit".to_string(),
                },
            ),
            block(
                2,
                "body",
                vec![binary(2, temp(0))],
                NirTerminator::Goto("header".to_string()),
            ),
            block(3, "exit", Vec::new(), NirTerminator::Return(None)),
        ]);
        let cfg = NirCfg::from_routine(&routine);
        let use_def = NirUseDef::from_routine(&routine);
        let liveness = NirTempLiveness::analyze(&routine, &cfg, &use_def);

        assert!(liveness.live_in(BlockId(0)).is_empty());
        assert_eq!(liveness.live_out(BlockId(0)), &BTreeSet::from([TempId(0)]));
        assert_eq!(liveness.live_in(BlockId(1)), &BTreeSet::from([TempId(0)]));
        assert_eq!(liveness.live_out(BlockId(2)), &BTreeSet::from([TempId(0)]));
        assert!(!liveness.live_in(BlockId(1)).contains(&TempId(1)));
        assert!(!liveness.live_out(BlockId(2)).contains(&TempId(2)));
        assert!(liveness.evaluations() > routine.blocks.len());
    }
}
