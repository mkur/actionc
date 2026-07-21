use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::mir6502::analysis::effects::{MirHomeByte, classify_op};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirBlockId, MirOp, MirRoutine};
use crate::mir6502::rewrite::context::{MirExitStateChange, MirProof, PostHomeRewriteContext};
use crate::mir6502::rewrite::plan::{
    MirChangeSet, MirPostHomeRewritePlan, MirRemovedHomeDefinition,
};

/// Constructs a post-home plan only when every concrete home definition that
/// disappears is dead outside the proposed transaction. Replacement writes
/// preserve the latest original definitions for the same home; any excess
/// earlier writes are transient definitions and must be proven dead.
pub(in crate::mir6502) fn structural_plan(
    routine: &MirRoutine,
    context: &PostHomeRewriteContext<'_, '_>,
    block: MirBlockId,
    range: Range<usize>,
    replacement: Vec<MirOp>,
    exit_state_change: MirExitStateChange,
    stat: &'static str,
    family_priority: u16,
) -> Option<MirPostHomeRewritePlan> {
    let block_ops = &routine
        .blocks
        .iter()
        .find(|candidate| candidate.id == block)?
        .ops;
    if range.start >= range.end || range.end > block_ops.len() {
        return None;
    }
    let removed_homes = removed_home_definitions(block, &range, block_ops, &replacement);
    let end = context.point(MirSite::Op {
        block,
        op_index: range.end - 1,
    });
    if removed_homes.iter().any(|removed| {
        !matches!(
            context.home_definition_dead_after(removed.home, context.point(removed.store), end,),
            MirProof::Proven(())
        )
    }) || !matches!(
        context.exit_state_change_is_unobservable(&exit_state_change, end),
        MirProof::Proven(())
    ) {
        return None;
    }

    Some(MirPostHomeRewritePlan {
        generation: context.generation(),
        block,
        range,
        replacement,
        removed_homes,
        exit_state_change,
        change_set: MirChangeSet::posthome_operation_change(),
        stat,
        observations: Vec::new(),
        family_priority,
        estimated_byte_saving: 0,
        estimated_cycle_saving: 0,
    })
}

fn removed_home_definitions(
    block: MirBlockId,
    range: &Range<usize>,
    block_ops: &[MirOp],
    replacement: &[MirOp],
) -> Vec<MirRemovedHomeDefinition> {
    let mut replacement_writes = BTreeMap::<MirHomeByte, usize>::new();
    for op in replacement {
        for home in written_homes(op) {
            *replacement_writes.entry(home).or_default() += 1;
        }
    }

    let mut preserved = BTreeMap::<MirHomeByte, usize>::new();
    let mut removed = Vec::new();
    for op_index in range.clone().rev() {
        for home in written_homes(&block_ops[op_index]) {
            let keep = replacement_writes.get(&home).copied().unwrap_or(0);
            let already_preserved = preserved.entry(home).or_default();
            if *already_preserved < keep {
                *already_preserved += 1;
            } else {
                removed.push(MirRemovedHomeDefinition {
                    home,
                    store: MirSite::Op { block, op_index },
                });
            }
        }
    }
    removed.sort();
    removed
}

fn written_homes(op: &MirOp) -> BTreeSet<MirHomeByte> {
    let effects = classify_op(op);
    effects
        .homes
        .writes
        .into_iter()
        .chain(effects.addresses.pair_writes)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::posthome::PostHomeAnalysisSnapshot;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    use crate::mir6502::ir::{
        MirAddr, MirBlock, MirDef, MirEffects, MirFrame, MirMem, MirReg, MirRoutineAbi, MirSpillId,
        MirTerminator, MirValue, MirWidth, RoutineId,
    };

    fn store(id: u32) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Spill {
                id: MirSpillId(id),
                offset: 0,
            }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
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

    fn routine(ops: Vec<MirOp>, terminator: MirTerminator) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "posthome".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops,
                terminator,
            }],
            effects: MirEffects::default(),
        }
    }

    #[test]
    fn structural_plan_blocks_a_removed_home_definition_used_after_window() {
        let routine = routine(vec![store(0), load(0)], MirTerminator::Return);
        let snapshot =
            PostHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let context = PostHomeRewriteContext::new(&snapshot);
        assert!(
            structural_plan(
                &routine,
                &context,
                MirBlockId(0),
                0..1,
                Vec::new(),
                MirExitStateChange::default(),
                "remove-store",
                0,
            )
            .is_none()
        );
    }

    #[test]
    fn structural_plan_accepts_a_transient_home_definition_overwritten_later() {
        let routine = routine(vec![store(0), store(0)], MirTerminator::Return);
        let snapshot =
            PostHomeAnalysisSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let context = PostHomeRewriteContext::new(&snapshot);
        let plan = structural_plan(
            &routine,
            &context,
            MirBlockId(0),
            0..1,
            Vec::new(),
            MirExitStateChange::default(),
            "remove-store",
            0,
        )
        .unwrap();
        assert_eq!(plan.removed_homes.len(), 1);
    }
}
