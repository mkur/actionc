use super::indexes::{
    DelayedByteIndexPlan, indexed_addr_parts, materialize_indexed_address_for_consumer,
    materialize_indexed_write_from_value,
};
use super::layout::MaterializeLayout;
use super::values::{offset_mem, return_slot_mem, split_value_as_word};
use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirArgHome, MirCallAbi, MirCallArg, MirCallTarget, MirEffects,
    MirFixedZpSlot, MirMemoryEffect, MirMemoryRegionKind, MirOp, MirPointerPair, MirResultHome,
    MirTempId, MirValue, MirWidth, RoutineId,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PreparedStoreAddress {
    pub(super) consumer: MirAddressConsumer,
    pub(super) offset: u16,
}

pub(super) fn call_result_store_addr_supported(width: MirWidth, dst: &MirAddr) -> bool {
    matches!(
        (width, dst),
        (_, MirAddr::Direct(_))
            | (_, MirAddr::Deref { .. })
            | (_, MirAddr::PointerCell { .. })
            | (_, MirAddr::ComputedIndex { .. })
            | (_, MirAddr::PointerIndex { .. })
            | (
                MirWidth::Byte,
                MirAddr::AbsoluteIndexedX { .. } | MirAddr::AbsoluteIndexedY { .. }
            )
    )
}

pub(super) fn materialize_call_result_to_store_addr(
    width: MirWidth,
    home: MirResultHome,
    dst: MirAddr,
    routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    delayed_byte_indexes: &DelayedByteIndexPlan,
    out: &mut Vec<MirOp>,
) {
    let Some(src) = call_result_value(width, home) else {
        return;
    };
    match dst {
        MirAddr::Direct(dst) => match width {
            MirWidth::Byte => super::materialize_value_to_mem(src, dst, out),
            MirWidth::Word => {
                let (lo, hi) = split_value_as_word(src, layout);
                super::materialize_value_to_mem(lo, dst.clone(), out);
                super::materialize_value_to_mem(hi, offset_mem(&dst, 1), out);
            }
        },
        MirAddr::Deref { ptr, offset } => match width {
            MirWidth::Byte => super::materialize_pointer_deref_write_byte(
                src,
                ptr,
                offset,
                routine_id,
                layout,
                temp_widths,
                out,
            ),
            MirWidth::Word => super::materialize_pointer_deref_write(
                ptr,
                offset,
                src,
                routine_id,
                layout,
                temp_widths,
                out,
            ),
        },
        MirAddr::PointerCell { ptr, offset } => match width {
            MirWidth::Byte => super::materialize_pointer_deref_write_byte(
                src,
                super::pointer_value_from_mem(&ptr),
                offset,
                routine_id,
                layout,
                temp_widths,
                out,
            ),
            MirWidth::Word => super::materialize_pointer_deref_write(
                super::pointer_value_from_mem(&ptr),
                offset,
                src,
                routine_id,
                layout,
                temp_widths,
                out,
            ),
        },
        MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. } => {
            let parts = indexed_addr_parts(&dst).expect("indexed store target matched above");
            materialize_indexed_write_from_value(
                parts,
                src,
                width,
                layout,
                Some(delayed_byte_indexes),
                out,
            );
        }
        MirAddr::AbsoluteIndexedX { .. } | MirAddr::AbsoluteIndexedY { .. }
            if width == MirWidth::Byte =>
        {
            out.push(MirOp::Store { dst, src, width });
        }
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. } => {}
    }
}

pub(super) fn prepare_call_result_store_addr(
    width: MirWidth,
    dst: &MirAddr,
    _routine_id: RoutineId,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) -> Option<PreparedStoreAddress> {
    let consumer = super::DEST_POINTER_PAIR;
    match dst {
        MirAddr::Deref { ptr, offset } => {
            checked_indirect_offset(*offset, width)?;
            materialize_pointer_value_to_consumer(ptr.clone(), consumer, layout, temp_widths, out);
            Some(PreparedStoreAddress {
                consumer,
                offset: *offset,
            })
        }
        MirAddr::PointerCell { ptr, offset } => {
            checked_indirect_offset(*offset, width)?;
            materialize_pointer_value_to_consumer(
                super::pointer_value_from_mem(ptr),
                consumer,
                layout,
                temp_widths,
                out,
            );
            Some(PreparedStoreAddress {
                consumer,
                offset: *offset,
            })
        }
        MirAddr::ComputedIndex { .. } | MirAddr::PointerIndex { .. } => {
            let parts = indexed_addr_parts(dst)?;
            checked_indirect_offset(parts.offset, width)?;
            materialize_indexed_address_for_consumer(parts.clone(), consumer, layout, None, out);
            Some(PreparedStoreAddress {
                consumer,
                offset: parts.offset,
            })
        }
        MirAddr::Direct(_)
        | MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => None,
    }
}

pub(super) fn materialize_call_result_to_prepared_store_addr(
    width: MirWidth,
    home: MirResultHome,
    prepared: PreparedStoreAddress,
    layout: &MaterializeLayout,
    out: &mut Vec<MirOp>,
) {
    let Some(src) = call_result_value(width, home) else {
        return;
    };
    match width {
        MirWidth::Byte => out.push(MirOp::StoreIndirect {
            consumer: prepared.consumer,
            src,
            offset: prepared.offset,
        }),
        MirWidth::Word => {
            let (lo, hi) = split_value_as_word(src, layout);
            out.push(MirOp::StoreIndirect {
                consumer: prepared.consumer,
                src: lo,
                offset: prepared.offset,
            });
            out.push(MirOp::StoreIndirect {
                consumer: prepared.consumer,
                src: hi,
                offset: prepared.offset.saturating_add(1),
            });
        }
    }
}

pub(super) fn call_preserves_prepared_store_addr(
    target: &MirCallTarget,
    abi: &MirCallAbi,
    args: &[MirCallArg],
    effects: &MirEffects,
    prepared: PreparedStoreAddress,
) -> bool {
    if !call_target_has_trustworthy_local_effects(target) {
        return false;
    }
    let Some(lo) = fixed_consumer_lo(prepared.consumer) else {
        return false;
    };
    !effects_may_write_fixed_pair(effects, lo)
        && !abi_homes_may_write_fixed_pair(&abi.params, lo)
        && !args
            .iter()
            .any(|arg| arg_home_may_write_fixed_pair(&arg.home, lo))
}

fn call_target_has_trustworthy_local_effects(target: &MirCallTarget) -> bool {
    matches!(
        target,
        MirCallTarget::Builtin { .. } | MirCallTarget::Runtime { .. }
    )
}

fn materialize_pointer_value_to_consumer(
    ptr: MirValue,
    consumer: MirAddressConsumer,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let (lo, hi) = super::split_value_with_temp_widths(ptr, layout, temp_widths);
    out.push(MirOp::MaterializeAddress {
        consumer,
        value: MirValue::Word {
            lo: Box::new(lo),
            hi: Box::new(hi),
        },
    });
}

fn checked_indirect_offset(offset: u16, width: MirWidth) -> Option<u16> {
    let last_byte = offset.checked_add(super::width_bytes(width).saturating_sub(1))?;
    (last_byte <= u8::MAX as u16).then_some(offset)
}

fn fixed_consumer_lo(consumer: MirAddressConsumer) -> Option<MirFixedZpSlot> {
    match consumer.pointer_pair() {
        MirPointerPair::Fixed { lo } => Some(lo),
        MirPointerPair::Virtual(_) => None,
    }
}

fn effects_may_write_fixed_pair(effects: &MirEffects, lo: MirFixedZpSlot) -> bool {
    effects.opaque || memory_effect_may_write_fixed_pair(&effects.memory_writes, lo)
}

fn memory_effect_may_write_fixed_pair(effect: &MirMemoryEffect, lo: MirFixedZpSlot) -> bool {
    match effect {
        MirMemoryEffect::None => false,
        MirMemoryEffect::Unknown | MirMemoryEffect::All => true,
        MirMemoryEffect::Regions(regions) => regions.iter().any(|region| {
            matches!(
                region.kind,
                MirMemoryRegionKind::ZeroPage | MirMemoryRegionKind::AbsoluteRange
            ) && ranges_overlap(region.offset, region.size, u16::from(lo.0), 2)
        }),
    }
}

fn abi_homes_may_write_fixed_pair(homes: &[MirArgHome], lo: MirFixedZpSlot) -> bool {
    homes
        .iter()
        .any(|home| arg_home_may_write_fixed_pair(home, lo))
}

fn arg_home_may_write_fixed_pair(home: &MirArgHome, lo: MirFixedZpSlot) -> bool {
    match home {
        MirArgHome::FixedZeroPage(slot) => ranges_overlap(u16::from(slot.0), 1, u16::from(lo.0), 2),
        MirArgHome::Absolute(address) => ranges_overlap(*address, 1, u16::from(lo.0), 2),
        MirArgHome::BytePair { lo: low, hi } => {
            arg_home_may_write_fixed_pair(low, lo) || arg_home_may_write_fixed_pair(hi, lo)
        }
        MirArgHome::Reg(_)
        | MirArgHome::RegisterPair { .. }
        | MirArgHome::ZeroPage(_)
        | MirArgHome::StackFrame { .. } => false,
    }
}

fn ranges_overlap(left_start: u16, left_len: u16, right_start: u16, right_len: u16) -> bool {
    let left_end = left_start.saturating_add(left_len);
    let right_end = right_start.saturating_add(right_len);
    left_start < right_end && right_start < left_end
}

pub(super) fn call_result_value(width: MirWidth, home: MirResultHome) -> Option<MirValue> {
    let MirResultHome::ReturnSlot { offset } = home else {
        return None;
    };
    Some(match width {
        MirWidth::Byte => MirValue::PointerCell(return_slot_mem(offset)),
        MirWidth::Word => MirValue::Word {
            lo: Box::new(MirValue::PointerCell(return_slot_mem(offset))),
            hi: Box::new(MirValue::PointerCell(return_slot_mem(
                offset.saturating_add(1),
            ))),
        },
    })
}
