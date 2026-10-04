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
        if let Some(value) = numeric(&index.value) {
            let offset = value * u64::from(stride) + u64::from(address.displacement.get());
            if offset > u64::from(u16::MAX) {
                return Ok(false);
            }
            let mut folded = address.clone();
            folded.index = None;
            folded.displacement = ByteOffset::new(offset as u32);
            // The unindexed selector checks both complete homes (including borrowed
            // reads), overlap and transient stack reach before changing state.
            return self.pointer_address(dest, &folded);
        }
        self.byte_indexed_pointer_address(dest, address)
    }

    fn byte_indexed_pointer_address(
        &mut self,
        dest: TempId,
        address: &Mir65816Address,
    ) -> Result<bool, String> {
        let Mir65816AddressBase::Indirect(base) = &address.base else {
            return Ok(false);
        };
        let Some(index) = &address.index else {
            return Ok(false);
        };
        let Mir65816Value::Temp(id, width) = index.value else {
            return Ok(false);
        };
        if width != ByteSize::ONE
            || !self.routine.temps.iter().any(|(temp, ty)| {
                *temp == id
                    && ty.width == Some(width)
                    && ty.kind.integer().is_some_and(|i| i.bits == 8 && !i.signed)
            })
        {
            return Ok(false);
        }
        let stride = index.stride.get();
        let displacement = address.displacement.get();
        if stride == 0 || 255 * u64::from(stride) + u64::from(displacement) > 65535 {
            return Ok(false);
        }
        // Even ignoring generic base preparation, its bytewise scale is larger.
        // Budget the whole native window: index extension (9), displacement (4),
        // low-word/bank addition (13), and the binary scale below.
        if !stride.is_power_of_two() {
            let shifts = 31 - stride.leading_zeros();
            let ones = stride.count_ones();
            if 26 + 2 + shifts + 3 * (ones - 1) >= 19 * ones + 6 * shifts {
                return Ok(false);
            }
        }
        let Location::Stack(index_home) = self.temp(id)? else {
            return Ok(false);
        };
        if index_home.width != 1 {
            return Err("indexed address requires an exact BYTE home".into());
        }
        let index_at = abi::stack::access_displacement(
            ByteOffset::new(u32::from(index_home.offset)),
            ByteSize::ONE,
            ByteSize::new(self.code.delta()),
        )
        .map_err(|e| e.to_string())?;
        let Some((Memory::Stack(source), Memory::Stack(destination))) =
            self.pointer_copy_homes(dest, base)?
        else {
            return Ok(false);
        };
        if source.abs_diff(destination) < 3 {
            return Ok(false);
        }
        let source_at = self.displacement(source, 0)?;
        self.code.barrier();
        self.code.a8();
        self.code.byte(ByteOp::LdaStack, index_at.get() as u8);
        self.code.a16();
        self.code.word(WordOp::AndImm, 255); // Clear hidden B after the exact byte read.
        if stride.is_power_of_two() {
            for _ in 0..stride.trailing_zeros() {
                self.code.op(Implied::AslA);
            }
        } else {
            // Every prefix coefficient is bounded by the admitted final stride.
            self.code.byte(ByteOp::StaDp, INDEX);
            for bit in (0..31 - stride.leading_zeros()).rev() {
                self.code.op(Implied::AslA);
                if stride & (1 << bit) != 0 {
                    self.code.op(Implied::Clc);
                    self.code.byte(ByteOp::AdcDp, INDEX);
                }
            }
        }
        if displacement != 0 {
            self.code.op(Implied::Clc);
            self.code.word(WordOp::AdcImm, displacement as u16);
        }
        self.code.op(Implied::Clc);
        self.code.byte(ByteOp::AdcStack, source_at);
        self.store_memory(Memory::Stack(destination), 0)?;
        // STA, SEP and LDA preserve carry from the low-word addition.
        self.code.a8();
        self.load_memory(Memory::Stack(source), 2)?;
        self.code.byte(ByteOp::AdcImm, 0);
        self.store_memory(Memory::Stack(destination), 2)?;
        Ok(true)
    }
}
