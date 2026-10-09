//! One unsigned constant-index interpretation for planning and selection.
use super::*;

pub(super) fn numeric(value: &Mir65816Value) -> Option<u64> {
    match value {
        Mir65816Value::U8(v) => Some(u64::from(*v)),
        Mir65816Value::U16(v) => Some(u64::from(*v)),
        Mir65816Value::U24(v) | Mir65816Value::U32(v) => Some(u64::from(*v)),
        _ => None,
    }
}

pub(super) fn constant(address: &Mir65816Address) -> Option<u16> {
    let mut offset = u64::from(address.displacement.get());
    if let Some(index) = &address.index {
        let stride = index.stride.get();
        if stride == 0 || stride >= 0x1000000 {
            return None;
        }
        offset = offset.checked_add(numeric(&index.value)?.checked_mul(u64::from(stride))?)?;
    }
    u16::try_from(offset).ok()
}
