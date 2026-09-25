//! Bounded native computation on complete captured three-byte values.
use super::*;

#[cfg(test)]
#[path = "pointer_value_tests.rs"]
mod tests;

impl Builder<'_> {
    pub(super) fn reload_pointer_base(&mut self, op: &Mir65816Op) -> Result<bool, String> {
        if !self.frame.pointer_reload(self.routine, op)? {
            return Ok(false);
        }
        if self.loop_x.is_some() || self.code.delta() != 0 {
            return Err("pointer reload conflicts with register/stack contract".into());
        }
        let Mir65816Op::Load { dest, address, .. } = op else {
            unreachable!()
        };
        let Location::DirectPage(slot) = self.temp(*dest)? else {
            unreachable!()
        };
        let source = Memory::Pointer {
            slot: slot.offset as u8,
            offset: address.displacement.get() as u16,
        };
        let destination = Memory::DirectPage(slot.offset);
        self.check_transfer(source, destination, 3)?;
        self.code.barrier();
        // The original full base remains intact through the final external read.
        // X16 owns the low word only inside this window; A8 then captures the bank.
        // Both stores are private. No external byte is repeated or reordered.
        self.code.a16();
        self.load_memory(source, 0)?;
        self.code.op(Implied::Tax);
        self.code.a8();
        self.load_memory(source, 2)?;
        self.store_memory(destination, 2)?;
        self.code.a16();
        self.code.op(Implied::Txa);
        self.store_memory(destination, 0)?;
        Ok(true)
    }
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
            self.pointer_constant_arithmetic(source, destination, offset, false)?;
        }
        Ok(true)
    }
    /// Caller preflights both complete homes and their geometry. The low-word
    /// store and mode change preserve carry into the bank-byte operation.
    fn pointer_constant_arithmetic(
        &mut self,
        source: Memory,
        destination: Memory,
        offset: u16,
        subtract: bool,
    ) -> Result<(), String> {
        self.code.a16();
        self.load_memory(source, 0)?;
        self.code
            .op(if subtract { Implied::Sec } else { Implied::Clc });
        self.code.word(
            if subtract {
                WordOp::SbcImm
            } else {
                WordOp::AdcImm
            },
            offset,
        );
        self.store_memory(destination, 0)?;
        self.code.a8();
        self.load_memory(source, 2)?;
        self.code.byte(
            if subtract {
                ByteOp::SbcImm
            } else {
                ByteOp::AdcImm
            },
            0,
        );
        self.store_memory(destination, 2)?;
        Ok(())
    }
    pub(super) fn captured_pointer_step(
        &mut self,
        dest: TempId,
        bytes: u8,
        operation: NirBinaryOp,
        left: &Mir65816Value,
        right: &Mir65816Value,
    ) -> Result<bool, String> {
        if bytes != 3 {
            return Ok(false);
        }
        let one = |v: &Mir65816Value| {
            matches!(
                v,
                Mir65816Value::U8(1)
                    | Mir65816Value::U16(1)
                    | Mir65816Value::U24(1)
                    | Mir65816Value::U32(1)
            )
        };
        let value = match operation {
            NirBinaryOp::Add | NirBinaryOp::Sub if one(right) => left,
            NirBinaryOp::Add if one(left) => right,
            _ => return Ok(false),
        };
        let Some((source, destination)) = self.pointer_copy_homes(dest, value)? else {
            return Ok(false);
        };
        self.code.barrier();
        self.pointer_constant_arithmetic(source, destination, 1, operation == NirBinaryOp::Sub)?;
        Ok(true)
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
        if matches!((source, destination), (Memory::Stack(a), Memory::Stack(b)) if a == b) {
            // The allocated home already contains the complete result. Keep
            // the current M/A/flags facts; no imaginary transfer took place.
            return Ok(true);
        }
        self.transfer(source, destination, 3, true)?;
        Ok(true)
    }
}

impl Builder<'_> {
    pub(super) fn pointer_edge(
        &mut self,
        edge: &Mir65816Edge,
        fallthrough: bool,
    ) -> Result<bool, String> {
        use super::super::pointer_copies::PointerHome;
        let Some(plan) = self
            .frame
            .pointer_copies(self.routine, edge, self.code.delta())?
        else {
            return Ok(false);
        };
        let stage = self.frame.pointer_staging(&plan, self.code.delta())?;
        let target = *self
            .blocks
            .get(&edge.target)
            .ok_or("missing pointer edge target")?;
        let load = |b: &mut Self, home, byte| match home {
            PointerHome::Stack(at) => b.code.byte(ByteOp::LdaStack, at + byte),
            PointerHome::DirectPage(at) => b.code.byte(ByteOp::LdaDp, at + byte),
        };
        self.code.barrier();
        self.code.a16();
        if let Some(stage) = stage {
            self.code.byte(ByteOp::StaStack, stage.a);
            for (source, destination) in plan.scheduled(stage) {
                for byte in [0, 1] {
                    load(self, source, byte);
                    match destination {
                        PointerHome::Stack(at) => self.code.byte(ByteOp::StaStack, at + byte),
                        PointerHome::DirectPage(at) => self.code.byte(ByteOp::StaDp, at + byte),
                    }
                }
            }
            self.code.byte(ByteOp::LdaStack, stage.a);
        }
        // Match the byte fallback's complete A and N/Z, including hidden B.
        // C/V, X/Y and all environment state are preserved by these forms.
        self.code.a8();
        load(self, plan.final_destination(), 2);
        self.code.a16();
        self.finish_edge(target, fallthrough);
        Ok(true)
    }
}
