use super::defs::split_def_as_temp;
use super::pointers::pointer_value_from_mem;
use super::values::offset_mem;
use crate::mir6502::ir::{
    MirAddr, MirAddressConsumer, MirDef, MirMem, MirOp, MirReg, MirTempId, MirValue, MirWidth,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WordConsumerSource {
    Values { lo: MirValue, hi: MirValue },
    Indirect { pointer: MirValue, offset: u16 },
}

pub(super) fn word_consumer_load_source(op: &MirOp) -> Option<(MirTempId, WordConsumerSource)> {
    let MirOp::Load {
        dst,
        src,
        width: MirWidth::Word,
    } = op
    else {
        return None;
    };
    let temp = split_def_as_temp(dst)?;
    let source = match src {
        MirAddr::Direct(mem) if word_consumer_direct_mem_is_safe(mem) => {
            let (lo, hi) = (
                MirValue::PointerCell(mem.clone()),
                MirValue::PointerCell(offset_mem(mem, 1)),
            );
            WordConsumerSource::Values { lo, hi }
        }
        MirAddr::PointerCell { ptr, offset }
            if word_consumer_pointer_mem_is_safe(ptr) && *offset < u16::from(u8::MAX) =>
        {
            WordConsumerSource::Indirect {
                pointer: pointer_value_from_mem(ptr),
                offset: *offset,
            }
        }
        _ => return None,
    };
    Some((temp, source))
}

pub(super) fn resolve_word_consumer_source(
    value: &MirValue,
    sources: &BTreeMap<MirTempId, WordConsumerSource>,
) -> Option<WordConsumerSource> {
    match value {
        MirValue::Def(MirDef::VTemp(temp)) => sources.get(temp).cloned(),
        MirValue::ConstU8(value) => Some(WordConsumerSource::Values {
            lo: MirValue::ConstU8(*value),
            hi: MirValue::ConstU8(0),
        }),
        MirValue::ConstU16(value) => Some(WordConsumerSource::Values {
            lo: MirValue::ConstU8(*value as u8),
            hi: MirValue::ConstU8((value >> 8) as u8),
        }),
        MirValue::Word { lo, hi }
            if word_consumer_byte_value_is_safe(lo) && word_consumer_byte_value_is_safe(hi) =>
        {
            Some(WordConsumerSource::Values {
                lo: lo.as_ref().clone(),
                hi: hi.as_ref().clone(),
            })
        }
        MirValue::PointerCell(mem) if word_consumer_direct_mem_is_safe(mem) => {
            Some(WordConsumerSource::Values {
                lo: MirValue::PointerCell(mem.clone()),
                hi: MirValue::PointerCell(offset_mem(mem, 1)),
            })
        }
        _ => None,
    }
}

fn word_consumer_direct_mem_is_safe(mem: &MirMem) -> bool {
    matches!(
        mem,
        MirMem::Param { .. } | MirMem::Local { .. } | MirMem::Global { .. } | MirMem::Spill { .. }
    )
}

pub(super) fn word_consumer_pointer_mem_is_safe(mem: &MirMem) -> bool {
    matches!(
        mem,
        MirMem::Param { .. }
            | MirMem::Local { .. }
            | MirMem::Global { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_)
    )
}

pub(super) fn word_consumer_byte_value_is_safe(value: &MirValue) -> bool {
    matches!(value, MirValue::ConstU8(_) | MirValue::ConstU16(_))
        || matches!(value, MirValue::PointerCell(mem) if word_consumer_direct_mem_is_safe(mem))
}

pub(super) fn push_word_consumer_source_load(
    ops: &mut Vec<MirOp>,
    source: &WordConsumerSource,
    byte: u8,
    pointer_consumer: MirAddressConsumer,
) {
    match source {
        WordConsumerSource::Values { lo, hi } => ops.push(MirOp::Move {
            dst: MirDef::Reg(MirReg::A),
            src: if byte == 0 { lo.clone() } else { hi.clone() },
            width: MirWidth::Byte,
        }),
        WordConsumerSource::Indirect { offset, .. } => ops.push(MirOp::LoadIndirect {
            consumer: pointer_consumer,
            dst: MirDef::Reg(MirReg::A),
            offset: offset.saturating_add(u16::from(byte)),
        }),
    }
}
