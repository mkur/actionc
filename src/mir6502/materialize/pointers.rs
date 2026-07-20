use super::*;

pub(super) fn select_pointer_temp_deref(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    require_local_deadness: bool,
) -> Option<PointerRewriteCandidate> {
    let Some(load) = ops.get(index) else {
        return None;
    };
    let MirOp::Load {
        dst,
        src: MirAddr::Direct(src_mem),
        width: MirWidth::Word,
    } = load
    else {
        return None;
    };

    let next = match ops.get(index + 1) {
        Some(op) => op,
        None => return None,
    };

    let Some(ptr_temp) = split_def_as_temp(dst) else {
        return None;
    };
    if require_local_deadness && temp_is_used_after(ops, index + 2, ptr_temp) {
        return None;
    }
    let producer_ptr = pointer_value_from_mem(src_mem);
    let mut replacement = Vec::new();

    match next {
        MirOp::Load {
            dst: use_dst,
            src:
                MirAddr::Deref {
                    ptr: MirValue::Def(MirDef::VTemp(use_temp)),
                    offset,
                },
            width: MirWidth::Word,
            ..
        } if *use_temp == ptr_temp => {
            let Some((lo_use, hi_use)) = split_def(use_dst.clone()) else {
                return None;
            };
            materialize_pointer_deref_read(
                lo_use,
                hi_use,
                producer_ptr.clone(),
                *offset,
                routine_id,
                layout,
                temp_widths,
                &mut replacement,
            );
            return Some(PointerRewriteCandidate {
                consumed: 2,
                replacement,
            });
        }
        MirOp::Store {
            dst:
                MirAddr::Deref {
                    ptr: MirValue::Def(MirDef::VTemp(use_temp)),
                    offset,
                },
            src,
            width: MirWidth::Word,
            ..
        } if *use_temp == ptr_temp => {
            materialize_pointer_deref_write(
                producer_ptr.clone(),
                *offset,
                src.clone(),
                routine_id,
                layout,
                temp_widths,
                &mut replacement,
            );
            return Some(PointerRewriteCandidate {
                consumed: 2,
                replacement,
            });
        }
        MirOp::Load {
            dst: use_dst,
            src:
                MirAddr::Deref {
                    ptr: MirValue::Def(MirDef::VTemp(use_temp)),
                    offset,
                },
            width: MirWidth::Byte,
            ..
        } if *use_temp == ptr_temp => {
            materialize_pointer_deref_read_byte(
                use_dst.clone(),
                producer_ptr.clone(),
                *offset,
                routine_id,
                layout,
                temp_widths,
                &mut replacement,
            );
            return Some(PointerRewriteCandidate {
                consumed: 2,
                replacement,
            });
        }
        MirOp::Store {
            dst:
                MirAddr::Deref {
                    ptr: MirValue::Def(MirDef::VTemp(use_temp)),
                    offset,
                },
            src,
            width: MirWidth::Byte,
            ..
        } if *use_temp == ptr_temp => {
            materialize_pointer_deref_write_byte(
                src.clone(),
                producer_ptr.clone(),
                *offset,
                routine_id,
                layout,
                temp_widths,
                &mut replacement,
            );
            return Some(PointerRewriteCandidate {
                consumed: 2,
                replacement,
            });
        }
        _ => {}
    }

    None
}

#[cfg(test)]
pub(super) fn try_fuse_pointer_temp_deref(
    ops: &[MirOp],
    index: usize,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) -> usize {
    let Some(candidate) =
        select_pointer_temp_deref(ops, index, routine_id, layout, temp_widths, true)
    else {
        return 0;
    };
    out.extend(candidate.replacement);
    candidate.consumed
}

#[cfg(test)]
pub(super) fn rematerialize_direct_pointer_temp_derefs(ops: Vec<MirOp>) -> Vec<MirOp> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if let Some(candidate) = select_direct_pointer_temp_rematerialization(&ops, index, true) {
            out.extend(candidate.replacement);
            index += candidate.consumed;
        } else {
            out.push(ops[index].clone());
            index += 1;
        }
    }
    out
}

pub(super) fn select_direct_pointer_temp_rematerialization(
    ops: &[MirOp],
    index: usize,
    require_local_deadness: bool,
) -> Option<PointerRewriteCandidate> {
    let Some(MirOp::Load {
        dst,
        src: MirAddr::Direct(ptr_mem),
        width: MirWidth::Word,
    }) = ops.get(index)
    else {
        return None;
    };
    let ptr_temp = split_def_as_temp(dst)?;

    for use_index in index + 1..ops.len() {
        let op = &ops[use_index];
        if !op_uses_temp(op, ptr_temp) {
            if op_def(op).is_some_and(|definition| match definition {
                MirDef::VTemp(temp) | MirDef::VTempByte { id: temp, .. } => *temp == ptr_temp,
                MirDef::Reg(_) => false,
            }) {
                return None;
            }
            if op_may_write_mem(op, ptr_mem) || op_may_write_mem(op, &offset_mem(ptr_mem, 1)) {
                return None;
            }
            continue;
        }
        if op_uses_temp_more_than_once(op, ptr_temp)
            || (require_local_deadness && temp_is_used_after(ops, use_index + 1, ptr_temp))
        {
            return None;
        }

        let rewritten =
            replace_deref_temp_pointer(op.clone(), ptr_temp, pointer_value_from_mem(ptr_mem))?;
        let mut replacement = ops[index + 1..use_index].to_vec();
        replacement.push(rewritten);
        return Some(PointerRewriteCandidate {
            consumed: use_index + 1 - index,
            replacement,
        });
    }

    None
}

fn replace_deref_temp_pointer(op: MirOp, temp: MirTempId, replacement: MirValue) -> Option<MirOp> {
    match op {
        MirOp::Load {
            dst,
            src:
                MirAddr::Deref {
                    ptr: MirValue::Def(MirDef::VTemp(id)),
                    offset,
                },
            width,
        } if id == temp => Some(MirOp::Load {
            dst,
            src: MirAddr::Deref {
                ptr: replacement,
                offset,
            },
            width,
        }),
        MirOp::Store {
            dst:
                MirAddr::Deref {
                    ptr: MirValue::Def(MirDef::VTemp(id)),
                    offset,
                },
            src,
            width,
        } if id == temp => Some(MirOp::Store {
            dst: MirAddr::Deref {
                ptr: replacement,
                offset,
            },
            src,
            width,
        }),
        _ => None,
    }
}

pub(super) fn pointer_value_from_mem(mem: &MirMem) -> MirValue {
    MirValue::Word {
        lo: Box::new(MirValue::PointerCell(mem.clone())),
        hi: Box::new(MirValue::PointerCell(offset_mem(mem, 1))),
    }
}

pub(super) fn word_value_splits_to_constants(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(_) | MirValue::ConstU16(_) => true,
        MirValue::Word { lo, hi } => {
            matches!(lo.as_ref(), MirValue::ConstU8(_))
                && matches!(hi.as_ref(), MirValue::ConstU8(_))
        }
        _ => false,
    }
}

pub(super) fn is_zero_word_value(value: &MirValue) -> bool {
    match value {
        MirValue::ConstU8(0) | MirValue::ConstU16(0) => true,
        MirValue::Word { lo, hi } => {
            matches!(lo.as_ref(), MirValue::ConstU8(0))
                && matches!(hi.as_ref(), MirValue::ConstU8(0))
        }
        _ => false,
    }
}

pub(super) fn materialize_pointer_deref_read(
    lo_dst: MirDef,
    hi_dst: MirDef,
    ptr: MirValue,
    offset: u16,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let consumer = pointer_consumer_for_value(&ptr, routine_id, layout);
    let (ptr_lo, ptr_hi) = split_value_with_temp_widths(ptr, layout, temp_widths);
    materialize_pointer_value_for_consumer(consumer, ptr_lo, ptr_hi, out);
    out.push(MirOp::LoadIndirect {
        consumer,
        dst: lo_dst,
        offset,
    });
    out.push(MirOp::LoadIndirect {
        consumer,
        dst: hi_dst,
        offset: offset.saturating_add(1),
    });
}

pub(super) fn materialize_pointer_deref_read_byte(
    dst: MirDef,
    ptr: MirValue,
    offset: u16,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let consumer = pointer_consumer_for_value(&ptr, routine_id, layout);
    let (ptr_lo, ptr_hi) = split_value_with_temp_widths(ptr, layout, temp_widths);
    materialize_pointer_value_for_consumer(consumer, ptr_lo, ptr_hi, out);
    out.push(MirOp::LoadIndirect {
        consumer,
        dst,
        offset,
    });
}

pub(super) fn materialize_pointer_deref_write(
    ptr: MirValue,
    offset: u16,
    src: MirValue,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let consumer = pointer_consumer_for_value(&ptr, routine_id, layout);
    let (ptr_lo, ptr_hi) = split_value_with_temp_widths(ptr, layout, temp_widths);
    let (src_lo, src_hi) = split_value_with_temp_widths(src, layout, temp_widths);
    materialize_pointer_value_for_consumer(consumer, ptr_lo, ptr_hi, out);
    out.push(MirOp::StoreIndirect {
        consumer,
        src: src_lo,
        offset,
    });
    out.push(MirOp::StoreIndirect {
        consumer,
        src: src_hi,
        offset: offset.saturating_add(1),
    });
}

pub(super) fn materialize_pointer_deref_write_byte(
    src: MirValue,
    ptr: MirValue,
    offset: u16,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let consumer = materialize_pointer_deref_address(ptr, routine_id, layout, temp_widths, out);
    out.push(MirOp::StoreIndirect {
        consumer,
        src,
        offset,
    });
}

pub(super) fn materialize_pointer_deref_address(
    ptr: MirValue,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) -> MirAddressConsumer {
    let consumer = pointer_consumer_for_value(&ptr, routine_id, layout);
    let (ptr_lo, ptr_hi) = split_value_with_temp_widths(ptr, layout, temp_widths);
    materialize_pointer_value_for_consumer(consumer, ptr_lo, ptr_hi, out);
    consumer
}

fn materialize_pointer_value_for_consumer(
    consumer: MirAddressConsumer,
    ptr_lo: MirValue,
    ptr_hi: MirValue,
    out: &mut Vec<MirOp>,
) {
    if consumer == DEFAULT_POINTER_PAIR {
        out.push(MirOp::MaterializeAddress {
            consumer,
            value: MirValue::Word {
                lo: Box::new(ptr_lo),
                hi: Box::new(ptr_hi),
            },
        });
    }
}

fn pointer_consumer_for_value(
    ptr: &MirValue,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
) -> MirAddressConsumer {
    let MirValue::Word { lo, hi } = ptr else {
        return DEFAULT_POINTER_PAIR;
    };
    let (MirValue::PointerCell(lo_mem), MirValue::PointerCell(hi_mem)) = (lo.as_ref(), hi.as_ref())
    else {
        return DEFAULT_POINTER_PAIR;
    };
    if offset_mem(lo_mem, 1) != *hi_mem {
        return DEFAULT_POINTER_PAIR;
    }
    let Some(address) = layout.mem_address(routine_id, lo_mem) else {
        return DEFAULT_POINTER_PAIR;
    };
    if address <= 0x00FE {
        MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(address as u8),
        })
    } else {
        DEFAULT_POINTER_PAIR
    }
}
