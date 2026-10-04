//! Bounded address formation from complete private three-byte values.
use super::*;

#[cfg(test)]
#[path = "indexed_address_tests.rs"]
mod tests;

fn numeric(value: &Mir65816Value) -> Option<u64> {
    match value {
        Mir65816Value::U8(v) => Some(u64::from(*v)),
        Mir65816Value::U16(v) => Some(u64::from(*v)),
        Mir65816Value::U24(v) | Mir65816Value::U32(v) => Some(u64::from(*v)),
        _ => None,
    }
}

impl Builder<'_> {
    pub(super) fn indexed_pointer_address(
        &mut self,
        dest: TempId,
        address: &Mir65816Address,
    ) -> Result<bool, String> {
        if !matches!(address.base, Mir65816AddressBase::Indirect(_)) {
            return Ok(false);
        }
        let Some(index) = &address.index else {
            return Ok(false);
        };
        let stride = index.stride.get();
        if stride == 0 || stride >= 0x1000000 {
            return Ok(false);
        }
        let Some(value) = numeric(&index.value) else {
            return Ok(false);
        };
        let offset = value * u64::from(stride) + u64::from(address.displacement.get());
        if offset > u64::from(u16::MAX) {
            return Ok(false);
        }
        let mut folded = address.clone();
        folded.index = None;
        folded.displacement = ByteOffset::new(offset as u32);
        // The unindexed selector checks both complete homes (including borrowed
        // reads), overlap and transient stack reach before changing state.
        self.pointer_address(dest, &folded)
    }
}
