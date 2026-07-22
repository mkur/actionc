use super::*;

pub(super) fn cleanup_pre_materialization_temp_artifacts(
    routine: &mut super::super::ir::MirRoutine,
    layout: &MaterializeLayout,
) {
    cleanup_pre_materialization_temp_artifacts_inner(routine, layout, None);
}

pub(super) fn cleanup_pre_materialization_temp_artifacts_with_liveness(
    routine: &mut super::super::ir::MirRoutine,
    layout: &MaterializeLayout,
    liveness: &super::temp_liveness::MirTempLiveness,
) {
    cleanup_pre_materialization_temp_artifacts_inner(routine, layout, Some(liveness));
}

fn cleanup_pre_materialization_temp_artifacts_inner(
    routine: &mut super::super::ir::MirRoutine,
    layout: &MaterializeLayout,
    liveness: Option<&super::temp_liveness::MirTempLiveness>,
) {
    for (block_index, block) in routine.blocks.iter_mut().enumerate() {
        if matches!(
            block.terminator,
            MirTerminator::Branch {
                cond: MirCond::FusedCompare { .. },
                ..
            }
        ) {
            continue;
        }
        let live_out = liveness.map(|liveness| {
            liveness
                .live_out(block_index)
                .expect("block liveness exists")
        });
        let mut ops = std::mem::take(&mut block.ops);
        loop {
            let (cleaned, changed) = cleanup_pre_materialization_block_temps(
                ops,
                &block.terminator,
                live_out,
                routine.id,
                layout,
            );
            ops = cleaned;
            if !changed {
                break;
            }
        }
        block.ops = ops;
    }
}

fn cleanup_pre_materialization_block_temps(
    ops: Vec<MirOp>,
    terminator: &MirTerminator,
    live_out: Option<&MirTempLiveSet>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> (Vec<MirOp>, bool) {
    let mut out = Vec::with_capacity(ops.len());
    let mut changed = false;
    let mut index = 0usize;
    while index < ops.len() {
        if can_drop_unused_temp_def(&ops, index, terminator, live_out) {
            changed = true;
            index += 1;
            continue;
        }
        if let Some((consumer_index, replacement)) =
            single_use_temp_replacement(&ops, index, terminator, live_out, routine_id, layout)
        {
            out.extend(ops[index + 1..consumer_index].iter().cloned());
            let mut consumer = ops[consumer_index].clone();
            if let Some(temp) = split_def_as_temp(op_def(&ops[index]).expect("producer def"))
                && replace_op_temp_values(&mut consumer, temp, &replacement)
            {
                out.push(consumer);
                changed = true;
                index = consumer_index + 1;
                continue;
            }
        }
        if let Some(consumer_index) = single_use_temp_sink_index(&ops, index, terminator, live_out)
        {
            out.extend(ops[index + 1..consumer_index].iter().cloned());
            out.push(ops[index].clone());
            changed = true;
            index = consumer_index;
            continue;
        }
        out.push(ops[index].clone());
        index += 1;
    }
    (out, changed)
}

fn can_drop_unused_temp_def(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    live_out: Option<&MirTempLiveSet>,
) -> bool {
    let Some(def) = op_def(ops.get(index).expect("op index")) else {
        return false;
    };
    if !op_is_side_effect_free_temp_def(&ops[index]) {
        return false;
    }
    let Some(temp) = split_def_as_temp(def) else {
        return false;
    };
    if live_out.is_some_and(|live_out| temp_is_live_out(live_out, temp)) {
        return false;
    }
    !ops[index + 1..].iter().any(|op| op_uses_temp(op, temp))
        && !terminator_uses_temp(terminator, temp)
}

fn single_use_temp_replacement(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    live_out: Option<&MirTempLiveSet>,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<(usize, MirValue)> {
    let producer = ops.get(index)?;
    let def = op_def(producer)?;
    let temp = split_def_as_temp(def)?;
    if terminator_uses_temp(terminator, temp)
        || live_out.is_some_and(|live_out| temp_is_live_out(live_out, temp))
    {
        return None;
    }
    let replacement = temp_replacement_value(producer, routine_id, layout)?;
    let use_indices = ops[index + 1..]
        .iter()
        .enumerate()
        .filter_map(|(offset, op)| op_uses_temp(op, temp).then_some(index + 1 + offset))
        .collect::<Vec<_>>();
    let &[consumer_index] = use_indices.as_slice() else {
        return None;
    };
    if !temp_replacement_allowed_for_consumer(&replacement, ops.get(consumer_index)?, temp) {
        return None;
    }
    Some((consumer_index, replacement))
}

fn single_use_temp_sink_index(
    ops: &[MirOp],
    index: usize,
    terminator: &MirTerminator,
    live_out: Option<&MirTempLiveSet>,
) -> Option<usize> {
    let producer = ops.get(index)?;
    let def = op_def(producer)?;
    let temp = split_def_as_temp(def)?;
    if terminator_uses_temp(terminator, temp)
        || live_out.is_some_and(|live_out| temp_is_live_out(live_out, temp))
        || !op_is_sinkable_temp_producer(producer)
    {
        return None;
    }
    let use_indices = ops[index + 1..]
        .iter()
        .enumerate()
        .filter_map(|(offset, op)| op_uses_temp(op, temp).then_some(index + 1 + offset))
        .collect::<Vec<_>>();
    let &[consumer_index] = use_indices.as_slice() else {
        return None;
    };
    if consumer_index == index + 1 {
        return None;
    }
    if ops[index + 1..consumer_index]
        .iter()
        .any(|op| op_blocks_temp_producer_sink(producer, op))
    {
        return None;
    }
    Some(consumer_index)
}

fn temp_is_live_out(live_out: &MirTempLiveSet, temp: MirTempId) -> bool {
    live_out.full_temps().any(|candidate| candidate == temp)
        || live_out
            .exact_lanes()
            .any(|(candidate, _byte)| candidate == temp)
}

fn op_is_side_effect_free_temp_def(op: &MirOp) -> bool {
    matches!(
        op,
        MirOp::LoadImm {
            dst: MirDef::VTemp(_),
            ..
        } | MirOp::Move {
            dst: MirDef::VTemp(_),
            ..
        } | MirOp::LeaAddr {
            dst: MirDef::VTemp(_),
            ..
        } | MirOp::Extend {
            dst: MirDef::VTemp(_),
            ..
        } | MirOp::Truncate {
            dst: MirDef::VTemp(_),
            ..
        } | MirOp::Unary {
            dst: MirDef::VTemp(_),
            ..
        } | MirOp::Binary {
            dst: MirDef::VTemp(_),
            ..
        }
    )
}

fn op_is_sinkable_temp_producer(op: &MirOp) -> bool {
    if !op_is_side_effect_free_temp_def(op) {
        return false;
    }
    match op {
        MirOp::LoadImm { .. } => true,
        MirOp::Move { src, .. } => value_is_safe_temp_replacement(src),
        MirOp::Extend { src, .. } | MirOp::Truncate { src, .. } | MirOp::Unary { src, .. } => {
            value_is_safe_to_sink(src)
        }
        MirOp::Binary { left, right, .. } => {
            value_is_safe_to_sink(left) && value_is_safe_to_sink(right)
        }
        MirOp::Load { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::Call { .. }
        | MirOp::Store { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::AddByteToWordMem { .. }
        | MirOp::SubByteFromWordMem { .. }
        | MirOp::Compare { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::MaterializeAddress { .. }
        | MirOp::MaterializeIndexedAddress { .. }
        | MirOp::AdvanceAddress { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => false,
    }
}

fn temp_replacement_value(
    op: &MirOp,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> Option<MirValue> {
    match op {
        MirOp::LoadImm { value, width, .. } => Some(match width {
            MirWidth::Byte => MirValue::ConstU8(*value as u8),
            MirWidth::Word => MirValue::ConstU16(*value),
        }),
        MirOp::Move { src, .. } if value_is_safe_temp_replacement(src) => Some(src.clone()),
        MirOp::LeaAddr {
            target,
            width: MirWidth::Word,
            ..
        } => Some(if layout.is_descriptor_storage(routine_id, target) {
            pointer_value_from_mem(target)
        } else {
            storage_address_value(target)
        }),
        _ => None,
    }
}

fn temp_replacement_allowed_for_consumer(
    replacement: &MirValue,
    consumer: &MirOp,
    temp: MirTempId,
) -> bool {
    if value_contains_storage_address_byte(replacement) {
        return matches!(consumer, MirOp::Call { .. })
            || op_address_uses_temp(consumer, temp)
            || matches!(
                consumer,
                MirOp::MaterializeAddress { .. } | MirOp::MaterializeIndexedAddress { .. }
            );
    }
    true
}

fn op_address_uses_temp(op: &MirOp, temp: MirTempId) -> bool {
    match op {
        MirOp::Load { src, .. } => addr_uses_temp(src, temp),
        MirOp::Store { dst, .. } => addr_uses_temp(dst, temp),
        _ => false,
    }
}

fn addr_uses_temp(addr: &MirAddr, temp: MirTempId) -> bool {
    match addr {
        MirAddr::ComputedIndex { base, index, .. } => {
            value_uses_specific_temp(base, temp) || value_uses_specific_temp(index, temp)
        }
        MirAddr::PointerIndex { index, .. } => value_uses_specific_temp(index, temp),
        MirAddr::Deref { ptr, .. } => value_uses_specific_temp(ptr, temp),
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::PointerCell { .. } => false,
    }
}

fn value_uses_specific_temp(value: &MirValue, temp: MirTempId) -> bool {
    let mut uses = 0;
    count_value_temp_uses(value, temp, &mut uses);
    uses > 0
}

fn value_contains_storage_address_byte(value: &MirValue) -> bool {
    match value {
        MirValue::StorageAddrByte { .. } => true,
        MirValue::Word { lo, hi } => {
            value_contains_storage_address_byte(lo) || value_contains_storage_address_byte(hi)
        }
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::PointerCell(_) => false,
    }
}

fn value_is_safe_temp_replacement(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => true,
        MirValue::Word { lo, hi } => {
            value_is_safe_temp_replacement(lo) && value_is_safe_temp_replacement(hi)
        }
        MirValue::Def(_) => false,
    }
}

fn value_is_safe_to_sink(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => true,
        MirValue::Word { lo, hi } => value_is_safe_to_sink(lo) && value_is_safe_to_sink(hi),
        MirValue::Def(_) | MirValue::PointerCell(_) => false,
    }
}

fn op_blocks_temp_producer_sink(producer: &MirOp, op: &MirOp) -> bool {
    if !matches!(producer, MirOp::LoadImm { .. }) {
        return op_has_opaque_flag_or_a_effects(op);
    }
    false
}

fn replace_op_temp_values(op: &mut MirOp, temp: MirTempId, replacement: &MirValue) -> bool {
    let before = op.clone();
    match op {
        MirOp::Load { src, .. } => {
            *src = replace_temp_addr(src.clone(), temp, replacement);
        }
        MirOp::Store { dst, src, .. } => {
            *dst = replace_temp_addr(dst.clone(), temp, replacement);
            *src = replace_temp_value(src.clone(), temp, replacement);
        }
        MirOp::Move { src, .. }
        | MirOp::Extend { src, .. }
        | MirOp::Truncate { src, .. }
        | MirOp::Unary { src, .. }
        | MirOp::MaterializeAddress { value: src, .. }
        | MirOp::AdvanceAddress { index: src, .. }
        | MirOp::StoreIndirect { src, .. } => {
            *src = replace_temp_value(src.clone(), temp, replacement);
        }
        MirOp::MaterializeIndexedAddress { base, index, .. } => {
            *base = replace_temp_value(base.clone(), temp, replacement);
            *index = replace_temp_value(index.clone(), temp, replacement);
        }
        MirOp::Binary { left, right, .. } | MirOp::Compare { left, right, .. } => {
            *left = replace_temp_value(left.clone(), temp, replacement);
            *right = replace_temp_value(right.clone(), temp, replacement);
        }
        MirOp::CompareIndirectBytes { .. } => {}
        MirOp::AddByteToWordMem { value, .. } | MirOp::SubByteFromWordMem { value, .. } => {
            *value = replace_temp_value(value.clone(), temp, replacement);
        }
        MirOp::Call { target, args, .. } => {
            replace_call_target_temp_value(target, temp, replacement);
            for arg in args {
                arg.value = replace_temp_value(arg.value.clone(), temp, replacement);
            }
        }
        MirOp::LoadImm { .. }
        | MirOp::LeaAddr { .. }
        | MirOp::UpdateMem { .. }
        | MirOp::UpdateIndexedMem { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => {}
    }
    *op != before
}

fn replace_call_target_temp_value(
    target: &mut MirCallTarget,
    temp: MirTempId,
    replacement: &MirValue,
) {
    if let MirCallTarget::Indirect { target, .. } = target {
        *target = replace_temp_value(target.clone(), temp, replacement);
    }
}

pub(super) fn temp_is_used_after(ops: &[MirOp], start: usize, temp: MirTempId) -> bool {
    ops[start..].iter().any(|op| op_uses_temp(op, temp))
}

pub(super) fn def_is_used_after(ops: &[MirOp], start: usize, def: &MirDef) -> bool {
    split_def_as_temp(def).is_some_and(|temp| temp_is_used_after(ops, start, temp))
}

pub(super) fn materialize_temp_ops(ops: Vec<MirOp>, spills: &mut Vec<MirSpillId>) -> Vec<MirOp> {
    let temp_widths = collect_temp_widths(&ops);
    let mut out = Vec::new();
    let mut staged_address: Option<(MirAddressConsumer, MirValue)> = None;
    for op in ops {
        invalidate_staged_address_for_op(&mut staged_address, &op);
        match op {
            MirOp::LeaAddr {
                dst,
                target,
                width: MirWidth::Word,
            } if split_def_as_temp(&dst).is_some() => {
                let temp = split_def_as_temp(&dst).expect("lea temp");
                ensure_spill(spills, MirSpillId(temp.0.saturating_mul(2)));
                ensure_spill(
                    spills,
                    MirSpillId(temp.0.saturating_mul(2).saturating_add(1)),
                );
                out.push(MirOp::LeaAddr {
                    dst,
                    target,
                    width: MirWidth::Word,
                });
            }
            MirOp::MaterializeAddress {
                consumer,
                value: MirValue::Word { lo, hi },
            } if matches!(
                consumer,
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { .. })
            ) =>
            {
                let value = MirValue::Word {
                    lo: lo.clone(),
                    hi: hi.clone(),
                };
                if staged_address
                    .as_ref()
                    .is_some_and(|(staged_consumer, staged_value)| {
                        *staged_consumer == consumer && staged_value == &value
                    })
                {
                    continue;
                }
                let slot = match consumer {
                    MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed { lo }) => lo.0,
                    _ => unreachable!(),
                };
                let lo = materialize_value_to_a(&mut out, *lo, spills);
                out.push(MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(slot))),
                    src: lo,
                    width: MirWidth::Byte,
                });
                let hi = materialize_value_to_a(&mut out, *hi, spills);
                out.push(MirOp::Store {
                    dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(
                        slot.saturating_add(1),
                    ))),
                    src: hi,
                    width: MirWidth::Byte,
                });
                staged_address = Some((consumer, value));
            }
            MirOp::MaterializeAddress {
                consumer,
                value: value_non_word,
            } => {
                staged_address = None;
                out.push(MirOp::MaterializeAddress {
                    consumer,
                    value: value_non_word,
                });
            }
            MirOp::AdvanceAddress {
                consumer,
                index,
                scale,
            } => {
                let index = materialize_index_value(&mut out, index, spills, &temp_widths);
                out.push(MirOp::AdvanceAddress {
                    consumer,
                    index,
                    scale,
                });
                staged_address = None;
            }
            MirOp::MaterializeIndexedAddress {
                consumer,
                base,
                index,
                scale,
            } => {
                let base = materialize_address_word_value(base, spills);
                let index = materialize_index_value(&mut out, index, spills, &temp_widths);
                out.push(MirOp::MaterializeIndexedAddress {
                    consumer,
                    base,
                    index,
                    scale,
                });
                staged_address = None;
            }
            MirOp::LoadIndirect {
                consumer,
                dst,
                offset,
            } if temp_def_spill(&dst).is_some() => {
                let spill = temp_def_spill(&dst).expect("temp spill");
                ensure_spill(spills, spill);
                out.push(MirOp::LoadIndirect {
                    consumer,
                    dst: MirDef::Reg(MirReg::A),
                    offset,
                });
                store_a_to_spill(&mut out, spill);
            }
            MirOp::StoreIndirect {
                consumer,
                src,
                offset,
            } => {
                let src = materialize_value_to_a(&mut out, src, spills);
                out.push(MirOp::StoreIndirect {
                    consumer,
                    src,
                    offset,
                });
            }
            MirOp::LoadImm {
                dst,
                value,
                width: MirWidth::Byte,
            } if temp_def_spill(&dst).is_some() => {
                let spill = temp_def_spill(&dst).expect("temp spill");
                ensure_spill(spills, spill);
                out.push(MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value,
                    width: MirWidth::Byte,
                });
                store_a_to_spill(&mut out, spill);
            }
            MirOp::Load {
                dst,
                src,
                width: MirWidth::Byte,
            } if temp_def_spill(&dst).is_some() => {
                let spill = temp_def_spill(&dst).expect("temp spill");
                ensure_spill(spills, spill);
                out.push(MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src,
                    width: MirWidth::Byte,
                });
                store_a_to_spill(&mut out, spill);
            }
            MirOp::Move {
                dst,
                src,
                width: MirWidth::Byte,
            } if temp_def_spill(&dst).is_some() => {
                let spill = temp_def_spill(&dst).expect("temp spill");
                ensure_spill(spills, spill);
                if matches!(src, MirValue::Def(MirDef::Reg(MirReg::X | MirReg::Y))) {
                    out.push(MirOp::Store {
                        dst: MirAddr::Direct(MirMem::Spill {
                            id: spill,
                            offset: 0,
                        }),
                        src,
                        width: MirWidth::Byte,
                    });
                    continue;
                }
                let src = materialize_value_to_a(&mut out, src, spills);
                if !matches!(src, MirValue::Def(MirDef::Reg(MirReg::A))) {
                    out.push(MirOp::Move {
                        dst: MirDef::Reg(MirReg::A),
                        src,
                        width: MirWidth::Byte,
                    });
                }
                store_a_to_spill(&mut out, spill);
            }
            MirOp::Move {
                dst: MirDef::Reg(reg),
                src,
                width: MirWidth::Byte,
            } if materialize_value_to_reg(&mut out, src.clone(), reg, spills) => {}
            MirOp::Extend {
                dst,
                src,
                from_width: MirWidth::Byte,
                to_width: MirWidth::Word,
                signed,
            } if !signed && temp_def_spill(&dst).is_some() => {
                let lo_spill = temp_def_spill(&dst).expect("temp spill");
                let hi_spill = MirSpillId(lo_spill.0.saturating_add(1));
                ensure_spill(spills, lo_spill);
                ensure_spill(spills, hi_spill);
                let src = materialize_value_to_a(&mut out, src, spills);
                if !matches!(src, MirValue::Def(MirDef::Reg(MirReg::A))) {
                    out.push(MirOp::Move {
                        dst: MirDef::Reg(MirReg::A),
                        src,
                        width: MirWidth::Byte,
                    });
                }
                store_a_to_spill(&mut out, lo_spill);
                out.push(MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 0,
                    width: MirWidth::Byte,
                });
                store_a_to_spill(&mut out, hi_spill);
            }
            MirOp::Unary {
                op,
                dst,
                src,
                width: MirWidth::Byte,
            } if temp_def_spill(&dst).is_some() => {
                let spill = temp_def_spill(&dst).expect("temp spill");
                ensure_spill(spills, spill);
                let src = materialize_value_to_a(&mut out, src, spills);
                out.push(MirOp::Unary {
                    op,
                    dst: MirDef::Reg(MirReg::A),
                    src,
                    width: MirWidth::Byte,
                });
                store_a_to_spill(&mut out, spill);
            }
            MirOp::Binary {
                op,
                dst,
                left,
                right,
                width: MirWidth::Byte,
                carry_in,
                carry_out,
            } if temp_def_spill(&dst).is_some() => {
                let spill = temp_def_spill(&dst).expect("temp spill");
                ensure_spill(spills, spill);
                let left = materialize_value_to_a(&mut out, left, spills);
                let right = materialize_rhs_temp(right, spills);
                out.push(MirOp::Binary {
                    op,
                    dst: MirDef::Reg(MirReg::A),
                    left,
                    right,
                    width: MirWidth::Byte,
                    carry_in,
                    carry_out,
                });
                store_a_to_spill(&mut out, spill);
            }
            MirOp::Binary {
                op,
                dst: MirDef::Reg(MirReg::A),
                left,
                right,
                width: MirWidth::Byte,
                carry_in,
                carry_out,
            } if value_needs_accumulator_materialization(&left) || value_uses_temp(&right) => {
                let left = materialize_value_to_a(&mut out, left, spills);
                let right = materialize_rhs_temp(right, spills);
                out.push(MirOp::Binary {
                    op,
                    dst: MirDef::Reg(MirReg::A),
                    left,
                    right,
                    width: MirWidth::Byte,
                    carry_in,
                    carry_out,
                });
            }
            MirOp::Store {
                dst,
                src,
                width: MirWidth::Byte,
            } => {
                let src = materialize_value_to_a(&mut out, src, spills);
                out.push(MirOp::Store {
                    dst,
                    src,
                    width: MirWidth::Byte,
                });
            }
            MirOp::Compare {
                dst,
                op,
                left,
                right,
                width: MirWidth::Byte,
                signed,
            } => {
                let left = materialize_value_to_a(&mut out, left, spills);
                let right = materialize_compare_rhs(right, spills);
                out.push(MirOp::Compare {
                    dst,
                    op,
                    left,
                    right,
                    width: MirWidth::Byte,
                    signed,
                });
            }
            other => out.push(other),
        }
    }
    out
}

fn value_needs_accumulator_materialization(value: &MirValue) -> bool {
    value_uses_temp(value) || matches!(value, MirValue::PointerCell(_))
}

fn invalidate_staged_address_for_op(
    staged: &mut Option<(MirAddressConsumer, MirValue)>,
    op: &MirOp,
) {
    let Some((consumer, value)) = staged.as_ref() else {
        return;
    };
    let invalidate = match op {
        MirOp::Store { dst, .. } => match dst {
            MirAddr::Direct(mem) => {
                direct_mem_writes_consumer(*consumer, mem) || value_depends_on_mem(value, mem)
            }
            _ => false,
        },
        MirOp::UpdateMem { mem, .. } => {
            direct_mem_writes_consumer(*consumer, mem) || value_depends_on_mem(value, mem)
        }
        MirOp::UpdateIndexedMem { .. } => true,
        MirOp::AddByteToWordMem { mem, .. } | MirOp::SubByteFromWordMem { mem, .. } => {
            direct_mem_writes_consumer(*consumer, mem)
                || direct_mem_writes_consumer(*consumer, &offset_mem(mem, 1))
                || value_depends_on_mem(value, mem)
                || value_depends_on_mem(value, &offset_mem(mem, 1))
        }
        MirOp::Move { dst, .. } | MirOp::LoadImm { dst, .. } | MirOp::Load { dst, .. } => {
            matches!(dst, MirDef::Reg(_))
        }
        MirOp::Binary { dst, .. }
        | MirOp::Unary { dst, .. }
        | MirOp::Extend { dst, .. }
        | MirOp::Truncate { dst, .. } => matches!(dst, MirDef::Reg(_)),
        MirOp::Compare { .. }
        | MirOp::CompareIndirectBytes { .. }
        | MirOp::LoadIndirect { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::LeaAddr { .. } => false,
        MirOp::AdvanceAddress {
            consumer: op_consumer,
            ..
        } => *op_consumer == *consumer,
        MirOp::MaterializeAddress {
            consumer: op_consumer,
            value: op_value,
        } => *op_consumer != *consumer || op_value != value,
        MirOp::MaterializeIndexedAddress {
            consumer: op_consumer,
            ..
        } => *op_consumer == *consumer,
        MirOp::Call { .. }
        | MirOp::RuntimeHelper { .. }
        | MirOp::Barrier { .. }
        | MirOp::MachineBlock { .. } => true,
    };
    if invalidate {
        *staged = None;
    }
}

fn direct_mem_writes_consumer(consumer: MirAddressConsumer, mem: &MirMem) -> bool {
    let MirPointerPair::Fixed { lo } = consumer.pointer_pair() else {
        return false;
    };
    matches!(
        mem,
        MirMem::FixedZeroPage(slot) if slot.0 == lo.0 || slot.0 == lo.0.saturating_add(1)
    )
}

fn value_depends_on_mem(value: &MirValue, mem: &MirMem) -> bool {
    match value {
        MirValue::PointerCell(source) => source == mem,
        MirValue::Word { lo, hi } => value_depends_on_mem(lo, mem) || value_depends_on_mem(hi, mem),
        MirValue::ConstU8(_)
        | MirValue::ConstU16(_)
        | MirValue::Def(_)
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. } => false,
    }
}

fn materialize_compare_rhs(value: MirValue, spills: &mut Vec<MirSpillId>) -> MirValue {
    let MirValue::Def(def) = value else {
        return value;
    };
    let Some(spill) = temp_def_spill(&def) else {
        return MirValue::Def(def);
    };
    ensure_spill(spills, spill);
    MirValue::PointerCell(MirMem::Spill {
        id: spill,
        offset: 0,
    })
}

fn materialize_index_value(
    out: &mut Vec<MirOp>,
    value: MirValue,
    spills: &mut Vec<MirSpillId>,
    temp_widths: &std::collections::BTreeMap<MirTempId, MirWidth>,
) -> MirValue {
    match &value {
        MirValue::Def(MirDef::VTemp(id)) if temp_widths.get(id) == Some(&MirWidth::Word) => {
            materialize_address_word_value(value, spills)
        }
        MirValue::Word { .. } => materialize_address_word_value(value, spills),
        _ => materialize_value_to_a(out, value, spills),
    }
}

fn materialize_address_word_value(value: MirValue, spills: &mut Vec<MirSpillId>) -> MirValue {
    match value {
        MirValue::Word { lo, hi } => MirValue::Word {
            lo: Box::new(materialize_compare_rhs(*lo, spills)),
            hi: Box::new(materialize_compare_rhs(*hi, spills)),
        },
        MirValue::Def(MirDef::VTemp(id)) => MirValue::Word {
            lo: Box::new(materialize_compare_rhs(
                MirValue::Def(MirDef::VTempByte { id, byte: 0 }),
                spills,
            )),
            hi: Box::new(materialize_compare_rhs(
                MirValue::Def(MirDef::VTempByte { id, byte: 1 }),
                spills,
            )),
        },
        other => materialize_compare_rhs(other, spills),
    }
}

fn materialize_value_to_a(
    out: &mut Vec<MirOp>,
    value: MirValue,
    spills: &mut Vec<MirSpillId>,
) -> MirValue {
    if materialize_value_to_reg(out, value.clone(), MirReg::A, spills) {
        return MirValue::Def(MirDef::Reg(MirReg::A));
    }
    value
}

fn materialize_rhs_temp(value: MirValue, spills: &mut Vec<MirSpillId>) -> MirValue {
    let MirValue::Def(def) = value else {
        return value;
    };
    let Some(spill) = temp_def_spill(&def) else {
        return MirValue::Def(def);
    };
    ensure_spill(spills, spill);
    MirValue::PointerCell(MirMem::Spill {
        id: spill,
        offset: 0,
    })
}

fn materialize_value_to_reg(
    out: &mut Vec<MirOp>,
    value: MirValue,
    reg: MirReg,
    spills: &mut Vec<MirSpillId>,
) -> bool {
    if matches!(value, MirValue::Def(MirDef::Reg(existing)) if existing == reg) {
        return true;
    }
    if let MirValue::ConstU8(value) = value {
        out.push(MirOp::LoadImm {
            dst: MirDef::Reg(reg),
            value: value as u16,
            width: MirWidth::Byte,
        });
        return true;
    }
    if let MirValue::PointerCell(mem) = value {
        out.push(MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        });
        return true;
    }
    if matches!(
        value,
        MirValue::StorageAddrByte { .. } | MirValue::RoutineAddrByte { .. }
    ) {
        out.push(MirOp::Move {
            dst: MirDef::Reg(reg),
            src: value,
            width: MirWidth::Byte,
        });
        return true;
    }
    let MirValue::Def(def) = value else {
        return false;
    };
    let Some(spill) = temp_def_spill(&def) else {
        return false;
    };
    ensure_spill(spills, spill);
    out.push(MirOp::Load {
        dst: MirDef::Reg(reg),
        src: MirAddr::Direct(MirMem::Spill {
            id: spill,
            offset: 0,
        }),
        width: MirWidth::Byte,
    });
    true
}

pub(super) fn store_a_to_spill(out: &mut Vec<MirOp>, spill: MirSpillId) {
    out.push(MirOp::Store {
        dst: MirAddr::Direct(MirMem::Spill {
            id: spill,
            offset: 0,
        }),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    });
}

pub(super) fn temp_def_spill(def: &MirDef) -> Option<MirSpillId> {
    match def {
        MirDef::VTemp(id) => Some(MirSpillId(id.0.saturating_mul(2))),
        MirDef::VTempByte { id, byte } if *byte <= 1 => Some(MirSpillId(
            id.0.saturating_mul(2).saturating_add(*byte as u32),
        )),
        MirDef::VTempByte { .. } | MirDef::Reg(_) => None,
    }
}

fn ensure_spill(spills: &mut Vec<MirSpillId>, spill: MirSpillId) {
    if !spills.contains(&spill) {
        spills.push(spill);
    }
}

pub(super) fn materialize_terminator(
    block_id: MirBlockId,
    terminator: &MirTerminator,
    ops: &[MirOp],
    config: &Mir6502Config,
) -> MirTerminator {
    if !config.enable_peepholes {
        return terminator.clone();
    }
    let MirTerminator::Branch {
        cond,
        then_edge,
        else_edge,
    } = terminator
    else {
        return terminator.clone();
    };

    match cond {
        MirCond::BoolValue(MirValue::ConstU8(0)) => MirTerminator::Jump(else_edge.clone()),
        MirCond::BoolValue(MirValue::ConstU8(_)) | MirCond::BoolValue(MirValue::ConstU16(_)) => {
            MirTerminator::Jump(then_edge.clone())
        }
        MirCond::BoolValue(MirValue::Def(MirDef::VTemp(id))) => {
            if let Some((op_index, op)) = ops.iter().enumerate().next_back()
                && let Some(flag_test) = compare_temp_flag_test(op, *id)
            {
                return MirTerminator::Branch {
                    cond: MirCond::FusedCompare {
                        producer: MirOpRef {
                            block: block_id,
                            op_index,
                        },
                        flag_test,
                    },
                    then_edge: then_edge.clone(),
                    else_edge: else_edge.clone(),
                };
            }
            terminator.clone()
        }
        _ => terminator.clone(),
    }
}

pub(super) fn materialize_fused_compare_dest(
    block_id: MirBlockId,
    terminator: &MirTerminator,
    ops: &mut [MirOp],
) {
    let MirTerminator::Branch {
        cond: MirCond::FusedCompare {
            producer,
            flag_test: _,
        },
        ..
    } = terminator
    else {
        return;
    };
    if producer.block != block_id {
        return;
    }
    if let Some(MirOp::Compare {
        dst,
        op,
        right,
        width: MirWidth::Byte,
        signed: false,
        ..
    }) = ops.get_mut(producer.op_index)
    {
        if let Some((_flag_test, Some((rewritten_op, rewritten_right)))) =
            compare_branch_plan(*op, right)
        {
            *op = rewritten_op;
            *right = rewritten_right;
        }
        *dst = MirCondDest::Flags;
    } else if let Some(MirOp::CompareIndirectBytes {
        dst,
        op,
        signed: false,
        ..
    }) = ops.get_mut(producer.op_index)
        && indirect_compare_flag_test(*op).is_some()
    {
        *dst = MirCondDest::Flags;
    }
}

fn compare_temp_flag_test(op: &MirOp, expected: MirTempId) -> Option<MirFlagTest> {
    match op {
        MirOp::Compare {
            dst: MirCondDest::Temp(actual),
            op,
            right,
            width: MirWidth::Byte,
            signed: false,
            ..
        } if *actual == expected => compare_branch_plan(*op, right).map(|(test, _)| test),
        MirOp::CompareIndirectBytes {
            dst: MirCondDest::Temp(actual),
            op,
            signed: false,
            ..
        } if *actual == expected => indirect_compare_flag_test(*op),
        _ => None,
    }
}

fn indirect_compare_flag_test(op: MirCompareOp) -> Option<MirFlagTest> {
    match op {
        MirCompareOp::Eq => Some(MirFlagTest::ZSet),
        MirCompareOp::Ne => Some(MirFlagTest::ZClear),
        MirCompareOp::Lt => Some(MirFlagTest::CClear),
        MirCompareOp::Ge => Some(MirFlagTest::CSet),
        MirCompareOp::Le | MirCompareOp::Gt => None,
    }
}
