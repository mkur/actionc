//! Bounded native computation on complete captured three-byte values.
use super::*;

#[cfg(test)]
#[path = "pointer_value_tests.rs"]
mod tests;

impl Builder<'_> {
    pub(super) fn pointer_address(
        &mut self,
        dest: TempId,
        address: &Mir65816Address,
    ) -> Result<bool, String> {
        let Mir65816AddressBase::Indirect(value) = &address.base else {
            return Ok(false);
        };
        if address.index.is_some() || address.displacement.get() > u32::from(u16::MAX) {
            return Ok(false);
        }
        let Some((source, destination)) = self.pointer_copy_homes(dest, value)? else {
            return Ok(false);
        };
        // Start with disjoint complete homes. Address formation does not read
        // through the captured value, even when it is null or crosses a bank.
        if matches!((source, destination), (Memory::Stack(a), Memory::Stack(b)) if a == b)
            || matches!((source, destination), (Memory::DirectPage(a), Memory::DirectPage(b)) if a == b)
        {
            return Ok(false);
        }
        self.code.barrier();
        let offset = address.displacement.get() as u16;
        if offset == 0 {
            self.transfer(source, destination, 3, true)?;
        } else {
            self.pointer_add_constant(source, destination, offset)?;
        }
        Ok(true)
    }
    /// Caller preflights both complete homes and their geometry. The low-word
    /// store and mode change preserve carry into the bank-byte operation.
    fn pointer_add_constant(
        &mut self,
        source: Memory,
        destination: Memory,
        offset: u16,
    ) -> Result<(), String> {
        self.code.a16();
        self.load_memory(source, 0)?;
        self.code.op(Implied::Clc);
        self.code.word(WordOp::AdcImm, offset);
        self.store_memory(destination, 0)?;
        self.code.a8();
        self.load_memory(source, 2)?;
        self.code.byte(ByteOp::AdcImm, 0);
        self.store_memory(destination, 2)?;
        Ok(())
    }
    /// Shared with transfer(): overlapping word pieces are safe only for a
    /// whole identity or disjoint private homes, never a partial overlap.
    pub(super) fn private_pointer_geometry(source: Memory, destination: Memory) -> bool {
        match (source, destination) {
            (Memory::Stack(a), Memory::Stack(b)) => a == b || a.abs_diff(b) >= 3,
            (Memory::DirectPage(a), Memory::DirectPage(b)) => a == b || a.abs_diff(b) >= 3,
            (Memory::Stack(_), Memory::DirectPage(_))
            | (Memory::DirectPage(_), Memory::Stack(_)) => true,
            _ => false,
        }
    }
    fn check_private_pointer(&self, memory: Memory) -> Result<(), String> {
        match memory {
            Memory::Stack(offset) if offset != 0 => {
                abi::stack::access_displacement(
                    ByteOffset::new(offset),
                    ByteSize::new(3),
                    ByteSize::new(self.code.delta()),
                )
                .map_err(|e| e.to_string())?;
            }
            Memory::DirectPage(offset)
                if u32::from(offset) + 3 <= abi::generated::DP_SCRATCH_SIZE => {}
            _ => return Err("pointer value requires a complete private home".into()),
        }
        Ok(())
    }
    fn pointer_copy_homes(
        &self,
        dest: TempId,
        value: &Mir65816Value,
    ) -> Result<Option<(Memory, Memory)>, String> {
        let bytes = self.value_width(value)?;
        let source = self.value_memory(value)?;
        let destination = self.temp(dest)?;
        if destination.slot().width != 3 {
            return Err("pointer copy destination width mismatch".into());
        }
        let Some(source) = source else {
            return Ok(None);
        };
        if bytes != 3 {
            return Ok(None);
        }
        let destination = destination.into();
        self.check_private_pointer(source)?;
        self.check_private_pointer(destination)?;
        Ok(Self::private_pointer_geometry(source, destination).then_some((source, destination)))
    }
    pub(super) fn pointer_cast(
        &mut self,
        dest: TempId,
        value: &Mir65816Value,
    ) -> Result<bool, String> {
        let Some((source, destination)) = self.pointer_copy_homes(dest, value)? else {
            return Ok(false);
        };
        self.code.barrier();
        // All current same-width three-byte cast kinds preserve representation.
        // Keep the semantic cast and its result home; only select its transfer.
        self.transfer(source, destination, 3, true)?;
        Ok(true)
    }
}
