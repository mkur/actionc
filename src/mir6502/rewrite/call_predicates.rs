//! Exact transactions for private snapshots crossing a pure predicate.
use super::context::PreHomeRewriteContext;
use super::plan::{MirChangeSet, MirEffectDelta, MirRemovedDefinition, MirRewritePlan};
use crate::mir6502::analysis::call_predicates::predicate_call;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::*;
use crate::mir6502::materialize::MaterializeLayout;

pub(in crate::mir6502) fn source_value(source: &MirMem, width: MirWidth) -> Option<MirValue> {
    let low = MirValue::PointerCell(source.clone());
    if width == MirWidth::Byte {
        return Some(low);
    }
    let high = match source {
        MirMem::Param { id, offset } => MirMem::Param {
            id: *id,
            offset: offset.checked_add(1)?,
        },
        MirMem::Local { id, offset } => MirMem::Local {
            id: *id,
            offset: offset.checked_add(1)?,
        },
        _ => return None,
    };
    Some(MirValue::Word {
        lo: Box::new(low),
        hi: Box::new(MirValue::PointerCell(high)),
    })
}

pub(in crate::mir6502) fn discover_argument_snapshots(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (index, op) in block.ops.iter().enumerate() {
            let site = MirSite::Op {
                block: block.id,
                op_index: index,
            };
            let Some((_, predicate)) = predicate_call(routine, context, site) else {
                continue;
            };
            let facts = (0..predicate)
                .filter_map(|argument| {
                    context
                        .argument_snapshot(routine, context.point(site), argument, layout)
                        .ok()
                })
                .collect::<Vec<_>>();
            let Some(start) = facts
                .iter()
                .filter_map(|fact| match fact.definitions[0].site {
                    MirSite::Op { op_index, .. } => Some(op_index),
                    _ => None,
                })
                .min()
            else {
                continue;
            };
            let mut call = op.clone();
            let MirOp::Call { args, .. } = &mut call else {
                unreachable!()
            };
            for fact in &facts {
                debug_assert_eq!(fact.call, context.point(site));
                args[fact.argument].value = source_value(&fact.source, fact.width).unwrap();
            }
            let removed = facts
                .iter()
                .flat_map(|fact| &fact.definitions)
                .copied()
                .collect::<Vec<_>>();
            let mut replacement = block.ops[start..index]
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    !removed.iter().any(|d| {
                        d.site
                            == (MirSite::Op {
                                block: block.id,
                                op_index: start + i,
                            })
                    })
                })
                .map(|(_, op)| op.clone())
                .collect::<Vec<_>>();
            replacement.push(call);
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: start..index + 1,
                replacement,
                removed_defs: removed
                    .into_iter()
                    .map(|definition| MirRemovedDefinition { definition })
                    .collect(),
                exit_effect_delta: MirEffectDelta::DeferredPredicateArguments,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: "call-predicate-snapshots",
                observations: vec![],
                family_priority: 5,
                estimated_byte_saving: 0,
                estimated_cycle_saving: 0,
            });
        }
    }
    plans
}

/// Independent effect-delta check: reconstruct the entire replacement from
/// exact removed loads and changed argument lanes. No call effects or memory
/// reads are projected away. The predicate and every retained operation match.
pub(in crate::mir6502) fn snapshot_delta_is_justified(
    original: &[MirOp],
    replacement: &[MirOp],
) -> bool {
    let Some(MirOp::Call { args: old_args, .. }) = original.last() else {
        return false;
    };
    let Some(MirOp::Call { args: new_args, .. }) = replacement.last() else {
        return false;
    };
    if old_args.len() != new_args.len() {
        return false;
    }
    if !matches!(
        original.get(original.len().saturating_sub(2)),
        Some(MirOp::Compare {
            dst: MirCondDest::Temp(_),
            op: MirCompareOp::Eq | MirCompareOp::Ne,
            right: MirValue::ConstU8(0),
            width: MirWidth::Byte,
            ..
        })
    ) {
        return false;
    }
    let mut removed = std::collections::BTreeSet::new();
    let mut expected_call = original.last().unwrap().clone();
    let MirOp::Call { args, .. } = &mut expected_call else {
        unreachable!()
    };
    for (i, (old, new)) in old_args.iter().zip(new_args).enumerate() {
        if old == new {
            continue;
        }
        if old.width != new.width || old.home != new.home {
            return false;
        }
        let MirValue::Def(MirDef::VTemp(temp)) = old.value else {
            return false;
        };
        let loads = original[..original.len() - 2]
            .iter()
            .enumerate()
            .filter_map(|(j, op)| match op {
                MirOp::Load {
                    dst: MirDef::VTemp(id),
                    src: MirAddr::Direct(source @ (MirMem::Param { .. } | MirMem::Local { .. })),
                    width,
                } if *id == temp && *width == old.width => Some((j, source)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let [(j, source)] = loads.as_slice() else {
            return false;
        };
        if source_value(source, old.width).as_ref() != Some(&new.value) || !removed.insert(*j) {
            return false;
        }
        args[i].value = new.value.clone();
    }
    if removed.is_empty() || replacement.last() != Some(&expected_call) {
        return false;
    }
    // Deferral crosses no writes, calls, machine/stack effects or indirect and
    // absolute reads. Storage backing and complete exclusive flow are checked
    // by the immutable context and again at the final placement gate.
    if !original[..original.len() - 1].iter().all(|op| {
        matches!(
            op,
            MirOp::Load {
                dst: MirDef::VTemp(_),
                src: MirAddr::Direct(MirMem::Param { .. } | MirMem::Local { .. }),
                ..
            } | MirOp::LoadImm {
                dst: MirDef::VTemp(_),
                ..
            } | MirOp::Compare {
                dst: MirCondDest::Temp(_),
                ..
            }
        )
    }) {
        return false;
    }
    let expected = original[..original.len() - 1]
        .iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .map(|(_, op)| op.clone())
        .chain(std::iter::once(expected_call))
        .collect::<Vec<_>>();
    expected == replacement
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    #[test]
    fn call_predicate_delta_preserves_exact_sources_predicate_and_call() {
        let p = crate::mir6502::analysis::call_predicates::tests::program();
        let r = &p.routines[1];
        let s = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let context = PreHomeRewriteContext::new(&s);
        let plans = discover_argument_snapshots(r, &context, &MaterializeLayout::new(&p, 0x3000));
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];
        let original = &r.blocks[0].ops[plan.range.clone()];
        assert!(snapshot_delta_is_justified(original, &plan.replacement));
        let mut wrong = plan.replacement.clone();
        let MirOp::Call { args, .. } = wrong.last_mut().unwrap() else {
            unreachable!()
        };
        let MirValue::Word { hi, .. } = &mut args[1].value else {
            unreachable!()
        };
        *hi = Box::new(MirValue::ConstU8(0));
        assert!(!snapshot_delta_is_justified(original, &wrong));
        let mut missing = plan.replacement.clone();
        missing.remove(0);
        assert!(!snapshot_delta_is_justified(original, &missing));
    }
}
