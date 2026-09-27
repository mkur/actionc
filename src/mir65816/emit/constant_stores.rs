//! Exact-width immediate stores. Address preparation remains authoritative.
use super::*;

#[cfg(test)]
#[path = "constant_stores_tests.rs"]
mod tests;

impl Builder<'_> {
    pub(super) fn constant_store(
        &mut self,
        memory: Memory,
        value: &Mir65816Value,
        bytes: u8,
        volatile: bool,
    ) -> Result<bool, String> {
        if volatile || !(2..=4).contains(&bytes) {
            return Ok(false);
        }
        let immediate = match value {
            Mir65816Value::U8(v) => u32::from(*v),
            Mir65816Value::U16(v) => u32::from(*v),
            Mir65816Value::U24(v) | Mir65816Value::U32(v) => *v,
            Mir65816Value::Null(_) => 0,
            Mir65816Value::Address(v, _) => v.value as u32,
            _ => return Ok(false), // Symbolic values keep their byte fixups.
        };
        // Match value_byte's truncation and zero extension, including typed
        // addresses. Signed widening is an explicit Cast, never inferred here.
        let source_bytes = self.value_width(value)?;
        let immediate = immediate & (u32::MAX >> (8 * (4 - source_bytes)));
        // Preflight the complete destination before any mode or store prefix.
        self.check_transfer(memory, memory, bytes)?;
        let last = u32::from(bytes - 1);
        match memory {
            Memory::Stack(offset) => {
                self.displacement(offset, 0)?;
            }
            Memory::DirectPage(offset) if u32::from(offset) + last > u32::from(u8::MAX) => {
                return Err("direct-page offset overflow".into());
            }
            Memory::Symbol(_, offset) if offset.checked_add(last).is_none() => {
                return Err("address offset overflow".into());
            }
            Memory::Pointer { offset, .. } if u32::from(offset) + last > u32::from(u16::MAX) => {
                return Err("indirect displacement exceeds Y".into());
            }
            _ => {}
        }
        let low = immediate as u16;
        let high = (immediate >> 16) as u16;
        // A three-byte stack/DP store with a distinct bank byte costs 13 bytes
        // from A8 versus 12 for byte stores. Both paths end in A8. All other
        // admitted shapes are smaller locally, even with the entry REP.
        if bytes == 3
            && matches!(memory, Memory::Stack(_) | Memory::DirectPage(_))
            && self.code.boundary().env.m == state::Width::Byte
            && high as u8 != low as u8
        {
            return Ok(false);
        }
        self.code.barrier();
        self.code.a16();
        self.code.word(WordOp::LdaImm, low);
        self.store_memory(memory, 0)?;
        if bytes == 4 {
            // STA and destination LDY preserve A. Reuse only within this
            // operation, never across stores/calls or from an ambient A fact.
            if high != low {
                self.code.word(WordOp::LdaImm, high);
            }
            if !self.next_pointer_piece(memory, ByteOp::StaIndirectY) {
                self.store_memory(memory, 2)?;
            }
        } else if bytes == 3 {
            self.code.a8();
            if high as u8 != low as u8 {
                self.code.byte(ByteOp::LdaImm, high as u8);
            }
            // No overlap and no fourth byte, including NULL pointer fields.
            if !self.next_pointer_piece(memory, ByteOp::StaIndirectY) {
                self.store_memory(memory, 2)?;
            }
        }
        Ok(true)
    }
}
