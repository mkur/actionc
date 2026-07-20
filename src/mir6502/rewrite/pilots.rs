#![allow(dead_code)] // Families become live incrementally during Slice 6.

use crate::mir6502::analysis::effects::{MirTempAccess, classify_op};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{MirDef, MirOp, MirRoutine};
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
            if let Some(plan) = compare_operand_producer_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_compare_producers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = compare_operand_producer_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_compare_narrowing(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = compare_narrowing_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn compare_narrowing_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .map(|block| {
            (0..block.ops.len())
                .filter(|index| {
                    crate::mir6502::materialize::analyzed_compare_narrowing_candidate(
                        &block.ops, *index,
                    )
                    .is_some()
                })
                .count()
        })
        .sum()
}

pub(in crate::mir6502) fn discover_byte_binary_compare_consumers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) =
                byte_binary_compare_consumer_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn byte_binary_compare_consumer_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .map(|block| {
            (0..block.ops.len())
                .filter(|index| {
                    crate::mir6502::materialize::analyzed_byte_binary_compare_candidate(
                        &block.ops, *index,
                    )
                    .is_some()
                })
                .count()
        })
        .sum()
}

pub(in crate::mir6502) fn discover_call_arg_producers(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = call_arg_producer_plan(block.id, &block.ops, index, context) {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn discover_return_slot_call_arg_forwards(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) =
                return_slot_call_arg_forward_plan(block.id, &block.ops, index, context)
            {
                plans.push(plan);
            }
        }
    }
    plans
}

pub(in crate::mir6502) fn return_slot_call_arg_forward_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .map(|block| {
            (0..block.ops.len())
                .filter(|index| {
                    crate::mir6502::materialize::analyzed_return_slot_call_arg_candidate(
                        &block.ops, *index,
                    )
                    .is_some_and(|candidate| !candidate.blocked_home_overlap)
                })
                .count()
        })
        .sum()
}

pub(in crate::mir6502) fn discover_unused_lea_addrs(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for index in 0..block.ops.len() {
            if let Some(plan) = unused_lea_plan(block.id, &block.ops, index, context) {
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

fn compare_operand_producer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_compare_operand_rewrite_candidate(ops, index)?;
    let compare_index = index + candidate.consumed - 1;
    let compare_site = MirSite::Op {
        block,
        op_index: compare_index,
    };
    let mut definitions = Vec::new();
    for producer_index in index..compare_index {
        let site = MirSite::Op {
            block,
            op_index: producer_index,
        };
        for access in classify_op(&ops[producer_index]).logical.temp_defs {
            let temp = match access {
                MirTempAccess::Full(temp) | MirTempAccess::Exact { temp, .. } => temp,
            };
            definitions.extend(context.definitions_at(temp, site));
        }
    }
    definitions.sort_unstable();
    definitions.dedup();
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let temp = definition.lane.temp;
        for usage_index in index + 1..=compare_index {
            for usage in context.uses_at(
                temp,
                MirSite::Op {
                    block,
                    op_index: usage_index,
                },
            ) {
                if usage.requirement.requires(definition.lane)
                    && !matches!(
                        context.unique_reaching_definition(usage, definition.lane),
                        MirProof::Proven(reaching) if reaching == *definition
                    )
                {
                    return None;
                }
            }
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
        range: index..index + candidate.consumed,
        replacement: vec![candidate.replacement],
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "compare-operand-consumer-prebranch",
        family_priority: 20,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn compare_narrowing_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate = crate::mir6502::materialize::analyzed_compare_narrowing_candidate(ops, index)?;
    let producer_site = MirSite::Op {
        block,
        op_index: index,
    };
    let compare_site = MirSite::Op {
        block,
        op_index: index + 1,
    };
    let definition = context
        .definitions_at(candidate.temp, producer_site)
        .into_iter()
        .find(|definition| definition.lane.byte == 1)?;
    let high_uses = context
        .uses_at(candidate.temp, compare_site)
        .into_iter()
        .filter(|usage| usage.requirement.requires(definition.lane))
        .collect::<Vec<_>>();
    if high_uses.is_empty()
        || high_uses.iter().any(|usage| {
            !matches!(
                context.unique_reaching_definition(*usage, definition.lane),
                MirProof::Proven(reaching) if reaching == definition
            )
        })
        || !context
            .temp_definition_dead_after(definition, context.point(compare_site))
            .is_proven()
    {
        return None;
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 2,
        replacement: candidate.replacement.into_iter().collect(),
        removed_defs: vec![MirRemovedDefinition { definition }],
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "byte-derived-word-bitwise-zero-compare-narrowed",
        family_priority: 30,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn byte_binary_compare_consumer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_byte_binary_compare_candidate(ops, index)?;
    let producer_site = MirSite::Op {
        block,
        op_index: index,
    };
    let compare_site = MirSite::Op {
        block,
        op_index: index + 1,
    };
    let definitions = context.definitions_at(candidate.temp, producer_site);
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let uses = context
            .uses_at(candidate.temp, compare_site)
            .into_iter()
            .filter(|usage| usage.requirement.requires(definition.lane))
            .collect::<Vec<_>>();
        if uses.is_empty()
            || uses.iter().any(|usage| {
                !matches!(
                    context.unique_reaching_definition(*usage, definition.lane),
                    MirProof::Proven(reaching) if reaching == *definition
                )
            })
            || !context
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
        replacement: candidate.replacement.into_iter().collect(),
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::SelectedResultRegister(crate::mir6502::ir::MirReg::A),
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "byte-binary-compare-consumer",
        family_priority: 40,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn call_arg_producer_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate = crate::mir6502::materialize::analyzed_call_arg_producer_candidate(ops, index)?;
    let call_index = index + candidate.consumed - 1;
    let call_site = MirSite::Op {
        block,
        op_index: call_index,
    };
    let mut definitions = Vec::new();
    for producer_index in index..call_index {
        let site = MirSite::Op {
            block,
            op_index: producer_index,
        };
        for access in classify_op(&ops[producer_index]).logical.temp_defs {
            let temp = match access {
                MirTempAccess::Full(temp) | MirTempAccess::Exact { temp, .. } => temp,
            };
            if candidate.temps.contains(&temp) {
                definitions.extend(context.definitions_at(temp, site));
            }
        }
    }
    definitions.sort_unstable();
    definitions.dedup();
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let temp = definition.lane.temp;
        for usage_index in index + 1..=call_index {
            for usage in context.uses_at(
                temp,
                MirSite::Op {
                    block,
                    op_index: usage_index,
                },
            ) {
                if usage.requirement.requires(definition.lane)
                    && !matches!(
                        context.unique_reaching_definition(usage, definition.lane),
                        MirProof::Proven(reaching) if reaching == *definition
                    )
                {
                    return None;
                }
            }
        }
        if !context
            .temp_definition_dead_after(*definition, context.point(call_site))
            .is_proven()
        {
            return None;
        }
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + candidate.consumed,
        replacement: vec![candidate.replacement],
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::Unchanged,
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "call-arg-producer",
        family_priority: 50,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

fn return_slot_call_arg_forward_plan(
    block: crate::mir6502::ir::MirBlockId,
    ops: &[MirOp],
    index: usize,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Option<MirRewritePlan> {
    let candidate =
        crate::mir6502::materialize::analyzed_return_slot_call_arg_candidate(ops, index)?;
    if candidate.blocked_home_overlap {
        return None;
    }
    let producer_site = MirSite::Op {
        block,
        op_index: index,
    };
    let consumer_site = MirSite::Op {
        block,
        op_index: index + 1,
    };
    let definitions = context.definitions_at(candidate.temp, producer_site);
    if definitions.is_empty() {
        return None;
    }
    for definition in &definitions {
        let uses = context
            .uses_at(candidate.temp, consumer_site)
            .into_iter()
            .filter(|usage| usage.requirement.requires(definition.lane))
            .collect::<Vec<_>>();
        if uses.is_empty()
            || uses.iter().any(|usage| {
                !matches!(
                    context.unique_reaching_definition(*usage, definition.lane),
                    MirProof::Proven(reaching) if reaching == *definition
                )
            })
            || !context
                .temp_definition_dead_after(*definition, context.point(consumer_site))
                .is_proven()
        {
            return None;
        }
    }
    Some(MirRewritePlan {
        generation: context.generation(),
        block,
        range: index..index + 2,
        replacement: candidate.replacement.into_iter().collect(),
        removed_defs: definitions
            .into_iter()
            .map(|definition| MirRemovedDefinition { definition })
            .collect(),
        exit_effect_delta: MirEffectDelta::ForwardedReturnSlot {
            base: candidate.return_slot,
            width: candidate.result_width,
        },
        change_set: MirChangeSet::prehome_operation_change(),
        stat: "return-slot-call-arg-forwards",
        family_priority: 60,
        estimated_byte_saving: 1,
        estimated_cycle_saving: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    use crate::mir6502::ir::{
        MirAddr, MirArgHome, MirBlock, MirCallAbi, MirCallArg, MirCallResult, MirCallTarget,
        MirCompareOp, MirCondDest, MirEdge, MirEdgeArg, MirEffects, MirFrame, MirMem,
        MirRegisterSet, MirResultHome, MirRoutineAbi, MirTempId, MirTerminator, MirValue, MirWidth,
        RoutineId,
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
    fn compare_operand_producer_folds_with_definition_identity_proof() {
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
    fn compare_operand_producer_subsumes_two_loaded_byte_consumer() {
        let mut routine = routine(vec![block(
            0,
            vec![
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(1)),
                    src: MirAddr::Direct(MirMem::Absolute(0x4000)),
                    width: MirWidth::Byte,
                },
                MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(2)),
                    src: MirAddr::Direct(MirMem::Absolute(0x4001)),
                    width: MirWidth::Byte,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ],
            MirTerminator::Return,
        )]);
        let result = MirPreHomeRewriteDriver::default()
            .run_fixed_point(&mut routine, discover_compare_producers)
            .unwrap();
        assert_eq!(result.applied, 1);
        assert!(matches!(
            &routine.blocks[0].ops[..],
            [MirOp::Compare {
                left: MirValue::PointerCell(MirMem::Absolute(0x4000)),
                right: MirValue::PointerCell(MirMem::Absolute(0x4001)),
                ..
            }]
        ));
    }

    #[test]
    fn compare_operand_producer_preserves_later_terminator_and_successor_uses() {
        fn word_compare_ops() -> Vec<MirOp> {
            vec![
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(1)),
                    value: 7,
                    width: MirWidth::Word,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU16(9),
                    width: MirWidth::Word,
                    signed: false,
                },
            ]
        }

        fn assert_blocked(mut candidate: MirRoutine) {
            let original_ops = candidate.blocks[0].ops.clone();
            let result = MirPreHomeRewriteDriver::default()
                .run_fixed_point(&mut candidate, discover_compare_producers)
                .unwrap();
            assert_eq!(result.applied, 0);
            assert_eq!(candidate.blocks[0].ops, original_ops);
        }

        let mut local_ops = word_compare_ops();
        local_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 1,
            }),
            width: MirWidth::Byte,
        });
        assert_blocked(routine(vec![block(0, local_ops, MirTerminator::Return)]));

        assert_blocked(routine(vec![
            block(
                0,
                word_compare_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Word,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]));

        for (src, width) in [
            (
                MirValue::Def(MirDef::VTempByte {
                    id: MirTempId(1),
                    byte: 1,
                }),
                MirWidth::Byte,
            ),
            (MirValue::Def(MirDef::VTemp(MirTempId(1))), MirWidth::Word),
        ] {
            assert_blocked(routine(vec![
                block(
                    0,
                    word_compare_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width,
                    }],
                    MirTerminator::Return,
                ),
            ]));
        }
    }

    #[test]
    fn compare_narrowing_uses_lane_aware_routine_deadness() {
        fn narrowing_ops() -> Vec<MirOp> {
            vec![
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(2)),
                    value: 7,
                    width: MirWidth::Byte,
                },
                MirOp::Binary {
                    op: crate::mir6502::ir::MirBinaryOp::And,
                    dst: MirDef::VTemp(MirTempId(1)),
                    left: MirValue::Def(MirDef::VTemp(MirTempId(2))),
                    right: MirValue::ConstU16(3),
                    width: MirWidth::Word,
                    carry_in: None,
                    carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU16(0),
                    width: MirWidth::Word,
                    signed: false,
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    discover_compare_narrowing,
                    compare_narrowing_rank,
                )
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(0, narrowing_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[1..=2],
            [
                MirOp::Binary {
                    width: MirWidth::Byte,
                    ..
                },
                MirOp::Compare {
                    width: MirWidth::Byte,
                    ..
                }
            ]
        ));

        let mut local_high_use = narrowing_ops();
        local_high_use.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 1,
            }),
            width: MirWidth::Byte,
        });
        let mut local_high_use = routine(vec![block(0, local_high_use, MirTerminator::Return)]);
        assert_eq!(run(&mut local_high_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                narrowing_ops(),
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
        assert_eq!(run(&mut terminator_use), 0);

        for (src, width) in [
            (
                MirValue::Def(MirDef::VTempByte {
                    id: MirTempId(1),
                    byte: 1,
                }),
                MirWidth::Byte,
            ),
            (MirValue::Def(MirDef::VTemp(MirTempId(1))), MirWidth::Word),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    narrowing_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }
    }

    #[test]
    fn byte_binary_compare_selection_uses_routine_deadness() {
        fn compare_ops() -> Vec<MirOp> {
            vec![
                MirOp::Binary {
                    op: crate::mir6502::ir::MirBinaryOp::Xor,
                    dst: MirDef::VTemp(MirTempId(1)),
                    left: MirValue::ConstU8(0xaa),
                    right: MirValue::ConstU8(0x55),
                    width: MirWidth::Byte,
                    carry_in: None,
                    carry_out: crate::mir6502::ir::MirCarryOut::Ignore,
                },
                MirOp::Compare {
                    dst: MirCondDest::Flags,
                    op: MirCompareOp::Eq,
                    left: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    right: MirValue::ConstU8(0),
                    width: MirWidth::Byte,
                    signed: false,
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    discover_byte_binary_compare_consumers,
                    byte_binary_compare_consumer_rank,
                )
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(0, compare_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::Binary {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    ..
                },
                MirOp::Compare {
                    left: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    ..
                }
            ]
        ));

        let mut local_use_ops = compare_ops();
        local_use_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut local_use = routine(vec![block(0, local_use_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut local_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                compare_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for src in [
            MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 0,
            }),
            MirValue::Def(MirDef::VTemp(MirTempId(1))),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    compare_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }
    }

    #[test]
    fn call_arg_producer_uses_routine_definition_identity_and_deadness() {
        fn call_ops() -> Vec<MirOp> {
            vec![
                MirOp::LoadImm {
                    dst: MirDef::VTemp(MirTempId(1)),
                    value: 7,
                    width: MirWidth::Byte,
                },
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: MirCallAbi {
                        params: vec![MirArgHome::Reg(crate::mir6502::ir::MirReg::A)],
                        result: None,
                        clobbers: MirRegisterSet::default(),
                        preserves: MirRegisterSet::default(),
                    },
                    args: vec![MirCallArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                        home: MirArgHome::Reg(crate::mir6502::ir::MirReg::A),
                    }],
                    result: None,
                    effects: MirEffects::default(),
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point(candidate, discover_call_arg_producers)
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(0, call_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [MirOp::Call {
                args,
                ..
            }] if matches!(args[0].value, MirValue::ConstU8(7))
        ));

        let mut local_use_ops = call_ops();
        local_use_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut local_use = routine(vec![block(0, local_use_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut local_use), 0);

        let mut terminator_use = routine(vec![
            block(
                0,
                call_ops(),
                MirTerminator::Jump(MirEdge {
                    target: crate::mir6502::ir::MirBlockId(1),
                    args: vec![MirEdgeArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                    }],
                }),
            ),
            block(1, Vec::new(), MirTerminator::Return),
        ]);
        assert_eq!(run(&mut terminator_use), 0);

        for src in [
            MirValue::Def(MirDef::VTempByte {
                id: MirTempId(1),
                byte: 0,
            }),
            MirValue::Def(MirDef::VTemp(MirTempId(1))),
        ] {
            let mut successor_use = routine(vec![
                block(
                    0,
                    call_ops(),
                    MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
                ),
                block(
                    1,
                    vec![MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                        src,
                        width: MirWidth::Byte,
                    }],
                    MirTerminator::Return,
                ),
            ]);
            assert_eq!(run(&mut successor_use), 0);
        }
    }

    #[test]
    fn return_slot_call_arg_forward_uses_routine_deadness() {
        fn call_abi(result: Option<MirResultHome>, params: Vec<MirArgHome>) -> MirCallAbi {
            MirCallAbi {
                params,
                result,
                clobbers: MirRegisterSet::default(),
                preserves: MirRegisterSet::default(),
            }
        }

        fn call_ops() -> Vec<MirOp> {
            vec![
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(1)),
                    abi: call_abi(Some(MirResultHome::ReturnSlot { offset: 0 }), Vec::new()),
                    args: Vec::new(),
                    result: Some(MirCallResult {
                        dst: MirDef::VTemp(MirTempId(1)),
                        width: MirWidth::Byte,
                        home: MirResultHome::ReturnSlot { offset: 0 },
                    }),
                    effects: MirEffects::default(),
                },
                MirOp::Call {
                    target: MirCallTarget::Routine(RoutineId(2)),
                    abi: call_abi(None, vec![MirArgHome::Reg(crate::mir6502::ir::MirReg::A)]),
                    args: vec![MirCallArg {
                        value: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                        width: MirWidth::Byte,
                        home: MirArgHome::Reg(crate::mir6502::ir::MirReg::A),
                    }],
                    result: None,
                    effects: MirEffects::default(),
                },
            ]
        }

        fn run(candidate: &mut MirRoutine) -> usize {
            MirPreHomeRewriteDriver::default()
                .run_fixed_point_by_key(
                    candidate,
                    discover_return_slot_call_arg_forwards,
                    return_slot_call_arg_forward_rank,
                )
                .unwrap()
                .applied
        }

        let mut local = routine(vec![block(0, call_ops(), MirTerminator::Return)]);
        assert_eq!(run(&mut local), 1);
        assert!(matches!(
            &local.blocks[0].ops[..],
            [
                MirOp::Call { result: None, .. },
                MirOp::Call { args, .. }
            ] if matches!(args[0].value, MirValue::PointerCell(_))
        ));

        let mut later_ops = call_ops();
        later_ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
            src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
            width: MirWidth::Byte,
        });
        let mut later_use = routine(vec![block(0, later_ops, MirTerminator::Return)]);
        assert_eq!(run(&mut later_use), 0);

        let mut successor_use = routine(vec![
            block(
                0,
                call_ops(),
                MirTerminator::Jump(MirEdge::plain(crate::mir6502::ir::MirBlockId(1))),
            ),
            block(
                1,
                vec![MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Absolute(0x5000)),
                    src: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            ),
        ]);
        assert_eq!(run(&mut successor_use), 0);
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
