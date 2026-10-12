//! Complete address values and exact, retained source-pointer stores.
use super::context::{MirProof, PreHomeRewriteContext};
use super::plan::{MirChangeSet, MirEffectDelta, MirRemovedDefinition, MirRewritePlan};
use crate::mir6502::analysis::address_values::{address_value, local_pointer_store};
use crate::mir6502::analysis::effects::classify_op;
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::*;
use crate::mir6502::materialize::MaterializeLayout;

/// Resolve only an explicit address value, never the contents of a descriptor.
/// The returned site makes the proof independently checkable over a transaction.
pub(in crate::mir6502) fn known_storage(
    block: &MirBlock,
    value: &MirValue,
    end: usize,
) -> Option<(MirMem, usize)> {
    fn trace(
        block: &MirBlock,
        value: &MirValue,
        end: usize,
        depth: usize,
    ) -> Option<(MirMem, usize)> {
        if depth == 0 {
            return None;
        }
        match value {
            MirValue::ConstU16(value) => Some((MirMem::Absolute(*value), end)),
            MirValue::Word { lo, hi } => match (lo.as_ref(), hi.as_ref()) {
                (
                    MirValue::StorageAddrByte { mem: a, byte: 0 },
                    MirValue::StorageAddrByte { mem: b, byte: 1 },
                ) if a == b => Some((a.clone(), end)),
                (MirValue::ConstU8(lo), MirValue::ConstU8(hi)) => {
                    Some((MirMem::Absolute(u16::from_le_bytes([*lo, *hi])), end))
                }
                _ => None,
            },
            MirValue::Def(MirDef::VTemp(temp)) => {
                let (index, op) = block.ops[..end].iter().enumerate().rev().find(|(_, op)| {
                    classify_op(op)
                        .logical
                        .temp_defs
                        .iter()
                        .any(|def| def.temp() == *temp)
                })?;
                match op {
                    MirOp::LeaAddr {
                        dst: MirDef::VTemp(id),
                        target,
                        width: MirWidth::Word,
                    } if id == temp => Some((target.clone(), index)),
                    MirOp::Move {
                        dst: MirDef::VTemp(id),
                        src,
                        width: MirWidth::Word,
                    } if id == temp => trace(block, src, index, depth - 1),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    trace(block, value, end, 32)
}

pub(in crate::mir6502) fn offset_known_storage(mem: &MirMem, offset: u16) -> Option<MirMem> {
    Some(match mem {
        MirMem::Absolute(base) => MirMem::Absolute(base.checked_add(offset)?),
        MirMem::Global { id, offset: base } => MirMem::Global {
            id: *id,
            offset: base.checked_add(offset)?,
        },
        MirMem::Static { id, offset: base } => MirMem::Static {
            id: *id,
            offset: base.checked_add(offset)?,
        },
        MirMem::Local { id, offset: base } => MirMem::Local {
            id: *id,
            offset: base.checked_add(offset)?,
        },
        _ => return None,
    })
}

fn ordinary_pointer_local(routine: &MirRoutine, cell: &MirMem) -> bool {
    let MirMem::Local { id, offset: 0 } = cell else {
        return false;
    };
    routine.frame.locals.iter().any(|slot| {
        matches!(slot.base,MirStorageBase::Local(local) if local==*id)
            && slot.scalar_width == Some(MirWidth::Word)
            && slot.storage_size >= 2
            && !matches!(slot.init, Some(MirStorageInit::Descriptor { .. }))
    })
}

pub(in crate::mir6502) fn discover_known_pointer_locals(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    layout: &MaterializeLayout,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (end, op) in block.ops.iter().enumerate() {
            let (cell, offset, width) = match op {
                MirOp::Load {
                    src: MirAddr::PointerCell { ptr, offset },
                    width,
                    ..
                }
                | MirOp::Store {
                    dst: MirAddr::PointerCell { ptr, offset },
                    width,
                    ..
                } => (ptr, *offset, *width),
                _ => continue,
            };
            if !ordinary_pointer_local(routine, cell) {
                continue;
            }
            let Ok((store, src)) = local_pointer_store(block, end, cell) else {
                continue;
            };
            let Some((base, start)) = known_storage(block, &src, store) else {
                continue;
            };
            if layout.is_descriptor_storage(routine.id, &base) {
                continue;
            }
            let Some(target) = offset_known_storage(&base, offset) else {
                continue;
            };
            // Direct word expansion must have a valid high lane. If the final
            // backing can wrap, the full-address fallback retains that behavior.
            if layout
                .mem_address(routine.id, &target)
                .and_then(|a| a.checked_add(u16::from(width == MirWidth::Word)))
                .is_none()
                || layout
                    .mem_address(routine.id, cell)
                    .is_none_or(|a| a < 0x100)
            {
                continue;
            }
            let site = MirSite::Op {
                block: block.id,
                op_index: end,
            };
            if !context.flags_dead_after(classify_op(op).machine.flag_writes, site) {
                continue;
            }
            let mut replacement = block.ops[start..end + 1].to_vec();
            match replacement.last_mut().unwrap() {
                MirOp::Load { src, .. } => *src = MirAddr::Direct(target),
                MirOp::Store { dst, .. } => *dst = MirAddr::Direct(target),
                _ => unreachable!(),
            }
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: start..end + 1,
                replacement,
                removed_defs: vec![],
                exit_effect_delta: MirEffectDelta::ForwardedKnownPointerLocal,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: "known-pointer-local",
                observations: vec![],
                family_priority: 5,
                estimated_byte_saving: 0,
                estimated_cycle_saving: 0,
            });
        }
    }
    plans
}

fn captured_value_available(
    value: &MirValue,
    original: MirSite,
    end: MirSite,
    context: &PreHomeRewriteContext<'_, '_>,
) -> bool {
    match value {
        MirValue::Def(MirDef::VTempByte { id: temp, byte }) => {
            let lane = crate::mir6502::analysis::use_def::MirTempLane {
                temp: *temp,
                byte: *byte,
            };
            context.uses_at(*temp, original).into_iter().any(|usage| {
                matches!(context.unique_reaching_definition(usage,lane),MirProof::Proven(definition)
                    if context.value_available_at(definition,context.point(end)).is_proven())
            })
        }
        MirValue::Def(MirDef::VTemp(temp)) => {
            let uses = context.uses_at(*temp, original);
            !uses.is_empty() && uses.into_iter().all(|usage| {
                [0, 1].into_iter().filter(|byte| usage.requirement.requires(crate::mir6502::analysis::use_def::MirTempLane {temp:*temp,byte:*byte})).all(|byte| {
                    let lane=crate::mir6502::analysis::use_def::MirTempLane {temp:*temp,byte};
                    matches!(context.unique_reaching_definition(usage,lane),MirProof::Proven(definition)
                        if context.value_available_at(definition,context.point(end)).is_proven())
                })
            })
        }
        MirValue::Word { lo, hi } => {
            captured_value_available(lo, original, end, context)
                && captured_value_available(hi, original, end, context)
        }
        MirValue::Def(MirDef::Reg(_)) | MirValue::PointerCell(_) => false,
        _ => true,
    }
}

pub(in crate::mir6502) fn discover_zero_index_address_values(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (end, op) in block.ops.iter().enumerate() {
            let MirOp::Store {
                dst: MirAddr::Direct(_),
                src,
                width: MirWidth::Word,
            } = op
            else {
                continue;
            };
            let site = MirSite::Op {
                block: block.id,
                op_index: end,
            };
            let Some(fact) = address_value(block, src, site, context) else {
                continue;
            };
            if fact.index.is_some()
                || !fact.producers.iter().any(|i| {
                    matches!(
                        block.ops[*i],
                        MirOp::Binary {
                            op: MirBinaryOp::Mul,
                            ..
                        }
                    )
                })
            {
                continue;
            }
            let Some(&start) = fact.producers.first() else {
                continue;
            };
            if !safe_to_move_address(block, start, end, &fact.inputs, context) {
                continue;
            }
            let MirValue::Def(dst @ MirDef::VTemp(_)) = src else {
                continue;
            };
            let mut replacement: Vec<_> = (start..end)
                .filter(|i| !fact.producers.contains(i))
                .map(|i| block.ops[i].clone())
                .collect();
            replacement.push(MirOp::ComputeAddress {
                dst: dst.clone(),
                base: fact.base,
                index: None,
                elem_size: fact.elem_size,
                offset: fact.offset,
            });
            replacement.push(op.clone());
            let Some(definitions) = super::pilots::prove_removed_window_definitions(
                block.id,
                &block.ops,
                start,
                end + 1,
                &replacement,
                context,
            ) else {
                continue;
            };
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: start..end + 1,
                replacement,
                removed_defs: definitions
                    .into_iter()
                    .map(|definition| MirRemovedDefinition { definition })
                    .collect(),
                exit_effect_delta: MirEffectDelta::Unchanged,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: "zero-index-address-value",
                observations: vec![],
                family_priority: 10,
                estimated_byte_saving: 0,
                estimated_cycle_saving: 0,
            });
        }
    }
    plans
}

fn safe_to_move_address(
    block: &MirBlock,
    start: usize,
    end: usize,
    inputs: &[(MirValue, MirSite)],
    context: &PreHomeRewriteContext<'_, '_>,
) -> bool {
    let end_site = MirSite::Op {
        block: block.id,
        op_index: end,
    };
    if !inputs
        .iter()
        .all(|(v, s)| captured_value_available(v, *s, end_site, context))
    {
        return false;
    }
    (start..end).all(|i| {
        let e = classify_op(&block.ops[i]);
        context.flags_dead_after(
            e.machine.flag_writes,
            MirSite::Op {
                block: block.id,
                op_index: i,
            },
        ) && !e.memory.may_write_any
            && !e.memory.has_unknown_effects
            && e.memory.direct_writes.is_empty()
            && matches!(e.memory.structured_writes, MirMemoryEffect::None)
            && !e.machine.register_reads.a
            && !e.machine.register_reads.x
            && !e.machine.register_reads.y
            && !e.machine.flag_reads.any()
            && !matches!(
                block.ops[i],
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            )
    })
}

pub(in crate::mir6502) fn address_value_rank(routine: &MirRoutine) -> usize {
    routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .map(|op| {
            classify_op(op).logical.temp_defs.len() * 4
                + usize::from(matches!(
                    op,
                    MirOp::Load {
                        src: MirAddr::PointerCell { .. },
                        ..
                    } | MirOp::Store {
                        dst: MirAddr::PointerCell { .. },
                        ..
                    }
                )) * 2
                + usize::from(matches!(op, MirOp::Binary { .. }))
        })
        .sum()
}

/// Pure multiply/add chains feeding a complete word value, before ABI lowering.
pub(in crate::mir6502) fn discover_indexed_address_values(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
) -> Vec<MirRewritePlan> {
    let mut plans = Vec::new();
    for block in &routine.blocks {
        for (end, op) in block.ops.iter().enumerate() {
            let MirOp::Store {
                dst: MirAddr::Direct(_),
                src,
                width: MirWidth::Word,
            } = op
            else {
                continue;
            };
            let site = MirSite::Op {
                block: block.id,
                op_index: end,
            };
            let Some(mut fact) = address_value(block, src, site, context) else {
                continue;
            };
            let Some(index) = fact.index.clone() else {
                continue;
            };
            if !(1..=255).contains(&fact.elem_size)
                || !fact.producers.iter().any(|i| {
                    matches!(
                        block.ops[*i],
                        MirOp::Binary {
                            op: MirBinaryOp::Mul,
                            ..
                        }
                    )
                })
            {
                continue;
            }
            // A proven unsigned byte extension is a full word whose high lane
            // is zero. Capture its original byte, retaining the originating load.
            if let MirValue::Def(MirDef::VTemp(temp)) = &index {
                let Some((_, mul_site)) = fact.inputs.iter().find(|(v, _)| v == &index) else {
                    continue;
                };
                if let MirProof::Proven(defs) =
                    context.exclusive_word_temp_definition(*temp, *mul_site)
                    && let MirSite::Op {
                        block: id,
                        op_index: extension,
                    } = defs[0].site
                    && id == block.id
                    && let MirOp::Extend {
                        src,
                        from_width: MirWidth::Byte,
                        to_width: MirWidth::Word,
                        signed: false,
                        ..
                    } = &block.ops[extension]
                {
                    let byte = match src {
                        MirValue::Def(MirDef::VTemp(id)) => {
                            MirValue::Def(MirDef::VTempByte { id: *id, byte: 0 })
                        }
                        MirValue::Def(MirDef::VTempByte { .. }) | MirValue::ConstU8(_) => {
                            src.clone()
                        }
                        MirValue::ConstU16(v) => MirValue::ConstU8(*v as u8),
                        _ => continue,
                    };
                    fact.index = Some(MirValue::Word {
                        lo: Box::new(byte.clone()),
                        hi: Box::new(MirValue::ConstU8(0)),
                    });
                    fact.inputs.retain(|(v, _)| v != &index);
                    fact.inputs.push((
                        byte,
                        MirSite::Op {
                            block: block.id,
                            op_index: extension,
                        },
                    ));
                    fact.producers.insert(extension);
                }
            }
            let Some(&start) = fact.producers.first() else {
                continue;
            };
            if !safe_to_move_address(block, start, end, &fact.inputs, context) {
                continue;
            }
            let MirValue::Def(dst @ MirDef::VTemp(_)) = src else {
                continue;
            };
            let mut replacement: Vec<_> = (start..end)
                .filter(|i| !fact.producers.contains(i))
                .map(|i| block.ops[i].clone())
                .collect();
            replacement.push(MirOp::ComputeAddress {
                dst: dst.clone(),
                base: fact.base,
                index: fact.index,
                elem_size: fact.elem_size,
                offset: fact.offset,
            });
            replacement.push(op.clone());
            let Some(definitions) = super::pilots::prove_removed_window_definitions(
                block.id,
                &block.ops,
                start,
                end + 1,
                &replacement,
                context,
            ) else {
                continue;
            };
            plans.push(MirRewritePlan {
                generation: context.generation(),
                block: block.id,
                range: start..end + 1,
                replacement,
                removed_defs: definitions
                    .into_iter()
                    .map(|definition| MirRemovedDefinition { definition })
                    .collect(),
                exit_effect_delta: MirEffectDelta::Unchanged,
                change_set: MirChangeSet::prehome_operation_change(),
                stat: "indexed-address-value",
                observations: vec![],
                family_priority: 10,
                estimated_byte_saving: 0,
                estimated_cycle_saving: 0,
            });
        }
    }
    plans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::prehome::PreHomeAnalysisSnapshot;
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    fn lower(source: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        crate::mir6502::lower_program(&nir).unwrap()
    }
    #[test]
    fn indexed_address_values_preserve_byte_extension_and_full_card_indexes() {
        for ty in ["BYTE", "CARD"] {
            let p = lower(&format!(
                "TYPE E=[CARD value BYTE ARRAY pad(18)] TYPE T=[BYTE ARRAY pad(38) E ARRAY rows(2)] CARD FUNC Address(T POINTER p {ty} index) RETURN(@p.rows(index)) PROC Main() RETURN"
            ));
            let r = &p.routines[0];
            let snapshot =
                PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
            let plans = discover_indexed_address_values(r, &PreHomeRewriteContext::new(&snapshot));
            assert_eq!(plans.len(), 1, "{ty}");
            let compute = plans[0]
                .replacement
                .iter()
                .find(|op| matches!(op, MirOp::ComputeAddress { .. }))
                .unwrap();
            let MirOp::ComputeAddress {
                index: Some(index),
                elem_size,
                offset,
                ..
            } = compute
            else {
                unreachable!()
            };
            assert_eq!((*elem_size, *offset), (20, 38));
            assert_eq!(
                matches!(index,MirValue::Word {hi,..} if **hi==MirValue::ConstU8(0)),
                ty == "BYTE"
            );
            let mut trial = r.clone();
            super::super::driver::MirPreHomeRewriteDriver::default()
                .apply_batch(&mut trial, plans)
                .unwrap();
        }
    }
    #[test]
    fn known_pointer_local_rewrite_keeps_the_source_store_and_typed_field() {
        for backing in ["=$6000", ""] {
            let p = lower(&format!(
                "TYPE Pair=[BYTE tag CARD value] Pair object{backing} CARD result PROC Main() Pair POINTER p p=@object result=p.value RETURN"
            ));
            let r = &p.routines[0];
            let layout = MaterializeLayout::new(&p, 0x3000);
            let snapshot =
                PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
            let ctx = PreHomeRewriteContext::new(&snapshot);
            let plans = discover_known_pointer_locals(r, &ctx, &layout);
            assert_eq!(
                plans.len(),
                1,
                "{backing}: {}",
                crate::mir6502::format_program(&p)
            );
            let plan = &plans[0];
            assert_eq!(
                &r.blocks[0].ops[plan.range.start..plan.range.end - 1],
                &plan.replacement[..plan.replacement.len() - 1]
            );
            assert!(matches!(
                plan.replacement.last(),
                Some(MirOp::Load {
                    src: MirAddr::Direct(MirMem::Global { offset: 1, .. }),
                    width: MirWidth::Word,
                    ..
                })
            ));
            let mut trial = r.clone();
            super::super::driver::MirPreHomeRewriteDriver::default()
                .apply_batch(&mut trial, plans)
                .unwrap();
        }
    }
    #[test]
    fn known_pointer_local_rewrite_rejects_calls_lane_mutation_and_fixed_cells() {
        for change in [
            "Touch()",
            "raw=BYTE POINTER(@p) raw^=$00",
            "raw=BYTE POINTER(@p) raw==+1 raw^=$00",
        ] {
            let p = lower(&format!(
                "TYPE Pair=[BYTE tag CARD value] Pair object=$6000 CARD result PROC Touch() RETURN PROC Main() Pair POINTER p BYTE POINTER raw p=@object {change} result=p.value RETURN"
            ));
            let r = p.routines.iter().find(|r| r.name == "Main").unwrap();
            let layout = MaterializeLayout::new(&p, 0x3000);
            let snapshot =
                PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
            assert!(
                discover_known_pointer_locals(r, &PreHomeRewriteContext::new(&snapshot), &layout)
                    .is_empty(),
                "{change}"
            );
        }
        let mut p = lower(
            "TYPE Pair=[BYTE tag CARD value] Pair object=$6000 CARD result PROC Main() Pair POINTER p p=@object result=p.value RETURN",
        );
        p.routines[0].frame.locals[0].base = MirStorageBase::Absolute(0xD400);
        let r = &p.routines[0];
        let layout = MaterializeLayout::new(&p, 0x3000);
        let snapshot = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        assert!(
            discover_known_pointer_locals(r, &PreHomeRewriteContext::new(&snapshot), &layout)
                .is_empty()
        );
    }
    #[test]
    fn zero_index_address_rewrite_keeps_base_evaluation_and_both_public_lanes() {
        let p = lower(
            "TYPE E=[CARD value BYTE ARRAY pad(18)] TYPE T=[BYTE ARRAY pad(38) E ARRAY rows(2)] CARD FUNC Address(T POINTER p) RETURN(@p.rows(0)) PROC Main() RETURN",
        );
        let r = &p.routines[0];
        let snapshot = PreHomeAnalysisSnapshot::new(r, MirRoutineGeneration::initial()).unwrap();
        let plans = discover_zero_index_address_values(r, &PreHomeRewriteContext::new(&snapshot));
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];
        assert!(plan.replacement.iter().any(|op| matches!(
            op,
            MirOp::ComputeAddress {
                index: None,
                offset: 38,
                ..
            }
        )));
        assert!(matches!(
            plan.replacement.last(),
            Some(MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                width: MirWidth::Word,
                ..
            })
        ));
        let mut trial = r.clone();
        super::super::driver::MirPreHomeRewriteDriver::default()
            .apply_batch(&mut trial, plans)
            .unwrap();
        assert!(trial.blocks[0].ops.iter().any(|op| matches!(
            op,
            MirOp::Load {
                src: MirAddr::Direct(MirMem::Param { .. }),
                width: MirWidth::Word,
                ..
            }
        )));
    }
}
