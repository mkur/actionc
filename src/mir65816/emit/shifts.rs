//! Constant counts use byte moves and a bounded residual shift in domain scratch.
use super::*;

#[cfg(test)]
#[path = "shift_tests.rs"]
mod tests;

fn offset(memory: Memory, byte: u8) -> Memory {
    match memory {
        Memory::Stack(at) => Memory::Stack(at + u32::from(byte)),
        Memory::DirectPage(at) => Memory::DirectPage(at + u16::from(byte)),
        _ => unreachable!("preflight requires private homes"),
    }
}

impl Builder<'_> {
    fn check_shift_home(&self, home: Memory, bytes: u8) -> Result<(), String> {
        match home {
            Memory::Stack(at) if at != 0 => {
                self.displacement(at, u32::from(bytes) - 1)?;
            }
            Memory::DirectPage(at)
                if u32::from(at) + u32::from(bytes) <= abi::generated::DP_SCRATCH_SIZE
                    && (at + u16::from(bytes) <= u16::from(RESULT)
                        || at >= u16::from(RESULT + 4)) => {}
            _ => {
                return Err("shift requires a complete private home outside result scratch".into());
            }
        }
        Ok(())
    }

    pub(super) fn constant_shift(
        &mut self,
        dest: TempId,
        bytes: u8,
        operation: NirBinaryOp,
        value: &Mir65816Value,
        count: &Mir65816Value,
    ) -> Result<bool, String> {
        let left = match operation {
            NirBinaryOp::Lsh => true,
            NirBinaryOp::Rsh => false,
            _ => return Ok(false),
        };
        // Inspect the entire constant, including the high bytes. Signed counts
        // are bit patterns here; a negative pattern exceeds the scalar width.
        let count = match count {
            Mir65816Value::U8(v) => u32::from(*v),
            Mir65816Value::U16(v) => u32::from(*v),
            Mir65816Value::U24(v) | Mir65816Value::U32(v) => *v,
            _ => return Ok(false),
        };
        let destination = self.temp(dest)?;
        if !(1..=4).contains(&bytes) || destination.slot().width != bytes {
            return Err("constant shift width mismatch".into());
        }
        let destination = destination.into();
        let source_bytes = self.value_width(value)?;
        let source = self.value_memory(value)?;
        self.check_shift_home(destination, bytes)?;
        if let Some(source) = source {
            self.check_shift_home(source, source_bytes)?;
        }
        // All validation precedes emission, including the zero-result case.
        self.code.barrier();
        if count >= u32::from(bytes) * 8 {
            self.shift_zero(destination, 0, bytes)?;
            return Ok(true);
        }
        let whole = (count / 8) as u8;
        let residual = (count % 8) as u8;
        let active = bytes - whole;
        let input = if left { 0 } else { whole };
        let output = if left { whole } else { 0 };
        let scratch = Memory::DirectPage(u16::from(RESULT));
        // Capture before writing any destination byte: allocated source and
        // destination homes may coincide or partially overlap.
        if let Some(source) = source.filter(|_| input + active <= source_bytes) {
            self.transfer(offset(source, input), offset(scratch, output), active, true)?;
        } else {
            self.code.a8();
            for i in 0..active {
                self.value_byte(value, input + i)?;
                self.code.byte(ByteOp::StaDp, RESULT + output + i);
            }
        }
        if residual != 0 {
            self.shift_residual(RESULT + output, active, residual, left);
        }
        if whole != 0 {
            self.shift_zero(scratch, if left { 0 } else { active }, whole)?;
        }
        self.transfer(scratch, destination, bytes, true)?;
        Ok(true)
    }

    fn shift_zero(&mut self, home: Memory, start: u8, bytes: u8) -> Result<(), String> {
        if bytes >= 2 {
            self.code.a16();
            self.code.word(WordOp::LdaImm, 0);
            // Overlapping private stores cover three bytes without a mode
            // change or a fourth-byte access.
            self.store_memory(home, start.into())?;
            if bytes > 2 {
                self.store_memory(home, u32::from(start + bytes - 2))?;
            }
        } else {
            self.code.a8();
            self.code.byte(ByteOp::LdaImm, 0);
            self.store_memory(home, start.into())?;
        }
        Ok(())
    }

    fn shift_residual(&mut self, start: u8, bytes: u8, count: u8, left: bool) {
        let wide = bytes % 2 == 0;
        let step = if wide { 2 } else { 1 };
        // A fixed X16 counter costs four bytes, at most two width changes for
        // A8, and three bytes for DEX/BNE. For the three-byte chain both entry
        // widths choose the loop from count three; the byte chain starts A8.
        let body = 2 * (bytes / step);
        let looped = body * count > body + if wide { 7 } else { 11 };
        let label = if looped {
            self.code.a16();
            self.code.word(WordOp::LdaImm, count.into());
            self.code.op(Implied::Tax);
            if !wide {
                self.code.a8();
            }
            let label = self.code.label();
            self.code.mark(label);
            Some(label)
        } else {
            if wide {
                self.code.a16();
            } else {
                self.code.a8();
            }
            None
        };
        for _ in 0..if looped { 1 } else { count } {
            if left {
                self.code.byte(ByteOp::AslDp, start);
                for byte in (step..bytes).step_by(usize::from(step)) {
                    self.code.byte(ByteOp::RolDp, start + byte);
                }
            } else {
                self.code.byte(ByteOp::LsrDp, start + bytes - step);
                for byte in (0..bytes - step).step_by(usize::from(step)).rev() {
                    self.code.byte(ByteOp::RorDp, start + byte);
                }
            }
        }
        if let Some(label) = label {
            self.code.op(Implied::Dex);
            self.code.branch(Branch::NotEqual, label);
        }
    }
}
