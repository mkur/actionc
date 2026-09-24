//! Prepare the native A/X result directly from constants and private homes.
use super::*;

#[cfg(test)]
#[path = "wide_return_tests.rs"]
mod tests;

impl Builder<'_> {
    pub(super) fn wide_return(&mut self, value: &Mir65816Value) -> Result<bool, String> {
        let bytes = match self.routine.result_home {
            Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X8ZeroExtended)) => 3,
            Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X16)) => 4,
            _ => return Ok(false),
        };
        let source_bytes = self.value_width(value)?;
        let memory = self.value_memory(value)?; // also checks temp annotations
        if source_bytes != bytes {
            return Ok(false); // Preserve explicit casts and the byte fallback.
        }
        if let Some(memory) = memory {
            // Validate the entire private source before changing mode or code.
            // No fourth byte is read from a three-byte home.
            match memory {
                Memory::Stack(offset) if offset != 0 => {
                    abi::stack::access_displacement(
                        ByteOffset::new(offset),
                        ByteSize::new(bytes.into()),
                        ByteSize::new(self.code.delta()),
                    )
                    .map_err(|e| e.to_string())?;
                }
                Memory::DirectPage(offset)
                    if u32::from(offset) + u32::from(bytes) <= abi::generated::DP_SCRATCH_SIZE => {}
                _ => return Err("wide return requires a complete private home".into()),
            }
            self.code.a16();
            self.load_memory(memory, u32::from(bytes - 2))?;
            if bytes == 3 {
                // Overlap the private low word by one byte to isolate the bank
                // in A16. This clears X.high without changing index width.
                self.code.op(Implied::Xba);
                self.code.word(WordOp::AndImm, 0x00ff);
            }
            self.code.op(Implied::Tax);
            self.load_memory(memory, 0)?;
        } else {
            let bits = match value {
                Mir65816Value::U24(v) | Mir65816Value::U32(v) => *v,
                Mir65816Value::Null(_) => 0,
                Mir65816Value::Address(v, _) => v.value as u32,
                _ => return Ok(false), // Symbolic addresses retain byte fixups.
            };
            let high = (bits >> 16) as u16 & if bytes == 3 { 0xff } else { 0xffff };
            self.code.a16();
            self.code.word(WordOp::LdxImm, high);
            self.code.word(WordOp::LdaImm, bits as u16);
        }
        // Shared frame teardown preserves both A and X; RTL retains ABI widths.
        Ok(true)
    }
}
