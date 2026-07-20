#![allow(dead_code)] // Production migration begins in the next slice.

use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirDef, MirOp, MirRoutine, MirTempId, MirValue, MirWidth};
use crate::mir6502::rewrite::context::{MirProof, PreHomeRewriteContext};
use crate::mir6502::rewrite::plan::{
    MirChangeSet, MirEffectDelta, MirRemovedDefinition, MirRewritePlan,
};

pub(in crate::mir6502) fn discover_prehome_pilots(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = unused_lea_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
            if let Some(plan) = literal_compare_producer_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

fn unused_lea_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let MirOp::LeaAddr {
        dst: MirDef::VTemp(temp),
        ..
    } = ops.get(index)?
    else {
        return None;
    };
    let site = MirSite::Op {
        block,
        op_index: index,
    };
    let definitions = context.definitions_at(*temp, site);
    if definitions.is_empty()
        || definitions.iter().any(|definition| {
            !context
                .temp_definition_dead_after(*definition, context.point(site))
                .is_proven()
        })
    {
        return None;
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 1,
        replacement: Vec::new(),
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "analyzed-unused-lea-addr",
        family_priority: 10,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn literal_compare_producer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let MirOp::LoadImm {
        dst: MirDef::VTemp(temp),
        value,
        width,
    } = ops.get(index)?
    else {
        return None;
    };
    let MirOp::Compare {
        dst,
        op,
        left,
        right,
        width: compare_width,
        signed,
    } = ops.get(index + 1)?
    else {
        return None;
    };
    let replacement_value = match width {
        MirWidth::Byte => MirValue::ConstU8(*value as u8),
        MirWidth::Word => MirValue::ConstU16(*value),
    };
    let (left, left_uses) = replace_temp(left, *temp, &replacement_value);
    let (right, right_uses) = replace_temp(right, *temp, &replacement_value);
    if left_uses + right_uses != 1 {
        return None;
    }
    let producer_site = MirSite::Op {
        block,
        op_index: index,
    };
    let compare_site = MirSite::Op {
        block,
        op_index: index + 1,
    };
    let definitions = context.definitions_at(*temp, producer_site);
    let uses = context.uses_at(*temp, compare_site);
    if definitions.is_empty() || uses.len() != 1 {
        return None;
    }
    for definition in &definitions {
        if uses[0].requirement.requires(definition.lane)
            && !matches!(
                context.unique_reaching_definition(uses[0], definition.lane),
                MirProof::Proven(reaching) if reaching == *definition
            )
        {
            return None;
        }
        if !context
            .temp_definition_dead_after(*definition, context.point(compare_site))
            .is_proven()
        {
            return None;
        }
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 2,
        replacement: vec![MirOp::Compare {
            dst: dst.clone(),
            op: *op,
            left,
            right,
            width: *compare_width,
            signed: *signed,
        }],
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "analyzed-literal-compare-producer",
        family_priority: 20,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn replace_temp(value: &MirValue, temp: MirTempId, replacement: &MirValue) -> (MirValue, usize) {
    match value {
        MirValue::Def(MirDef::VTemp(id)) if *id == temp => (replacement.clone(), 1),
        MirValue::Def(MirDef::VTempByte { id, byte }) if *id == temp => {
            let replacement = match (replacement, byte) {
                (MirValue::ConstU16(value), 0) => MirValue::ConstU8(*value as u8),
                (MirValue::ConstU16(value), 1) => MirValue::ConstU8((value >> 8) as u8),
                (value, 0) => value.clone(),
                _ => return (value.clone(), 0),
            };
            (replacement, 1)
        }
        MirValue::Word { lo, hi } => {
            let (lo, lo_uses) = replace_temp(lo, temp, replacement);
            let (hi, hi_uses) = replace_temp(hi, temp, replacement);
            (
                MirValue::Word {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                },
                lo_uses + hi_uses,
            )
        }
        _ => (value.clone(), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    use crate::mir6502::ir::{
        MirAddr, MirBlock, MirCompareOp, MirCondDest, MirEdge, MirEdgeArg, MirEffects, MirFrame,
        MirMem, MirRoutineAbi, MirTerminator, RoutineId,
    };
    use crate::mir6502::rewrite::driver::MirPreHomeRewriteDriver;

    fn routine(blocks: Vec<MirBlock>) -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "pilot".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks,
            effects: MirEffects::default(),
        }
    }

    fn block(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirBlock {
        MirBlock {
            id: crate::mir6502::ir::MirBlockId(id),
            label: format!("b{id}"),
            params: Vec::new(),
            ops,
            terminator,
        }
    }

    fn lea(temp: u32) -> MirOp {
        MirOp::LeaAddr {
            dst: MirDef::VTemp(MirTempId(temp)),
            target: MirMem::Absolute(0x4000),
            width: MirWidth::Word,
        }
    }

    #[test]
    fn unused_lea_folds_but_terminator_and_successor_uses_block_it() {
        let mut local = routine(vec![block(0, vec![lea(1)], MirTerminator::Return)]);
        let mut driver = MirPreHomeRewriteDriver::default();
        let result = driver
            .run_fixed_point(&mut local, discover_prehome_pilots)
            .unwrap();
        assert!(local.blocks[0].ops.is_empty());
        assert_eq!(result.applied, 1);
        assert!(result.converged);
        let second = driver
            .run_fixed_point(&mut local, discover_prehome_pilots)
            .unwrap();
        assert_eq!((second.applied, second.rounds), (0, 1));

        let mut terminator_use = routine(vec![
            block(
                0,
                vec![lea(1)],
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Word,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        let blocked = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut terminator_use, discover_prehome_pilots)
            .unwrap();
        assert_eq!(
            (blocked.applied, terminator_use.blocks[0].ops.len()),
            (0, 1)
        );

        let mut successor_use = routine(vec![
            block(
                0,
                vec![lea(1)],
                MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
            ),
            block(
                1,
                vec![MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    width: MirWidth::Word,
                }],
                MirTerminator::Return,
            ),
        ]);
        let blocked = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut successor_use, discover_prehome_pilots)
            .unwrap();
        assert_eq!((blocked.applied, successor_use.blocks[0].ops.len()), (0, 1));
    }

    #[test]
    fn literal_compare_producer_folds_with_definition_identity_proof() {
        let mut routine = routine(vec![block(
            0,
            vec![
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(1)),
                    value: 7,
                    width: MirWidth::Byte,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU8(9),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ],
            MirTerminator::Return,
        )]);
        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut routine, discover_prehome_pilots)
            .unwrap();
        assert_eq!(result.applied, 1);
        assert!(matches!(
            &routine.blocks[0].ops[..],
            [MirOp::Compare {
                left: MirValue::ConstU8(7),
                ..
            }]
        ));
    }

    #[test]
    fn later_generation_rejects_a_stale_plan() {
        let mut routine = routine(vec![block(0, vec![lea(1)], MirTerminator::Return)]);
        let snapshot = crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot::new(
            &routine,
            MirRoutineGeneration::initial(),
        )
        .unwrap();
        let plans = discover_prehome_pilots(&routine, &PreHomeRewriteContext::new(&snapshot));
        drop(snapshot);
        let mut driver = MirPreHomeRewriteDriver::default();
        driver.apply_batch(&mut routine, plans.clone()).unwrap();
        assert!(matches!(
            driver.apply_batch(&mut routine, plans),
            Err(crate::mir6502::rewrite::driver::MirRewriteError::StalePlan { .. })
        ));
    }
}
