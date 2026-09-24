//! Local width selection for already captured call operands and ABI result lanes.
use super::*;

#[cfg(test)]
#[path = "call_copies_tests.rs"]
mod tests;

#[derive(Clone, Copy)]
enum Source {
    Immediate(u32),
    Home(Memory),
    Bytes,
}

pub(super) struct Argument {
    source: Source,
    displacement: u8,
    bytes: u8,
    wide: bool,
}

// Padding leaves A8. Minimize encoded bytes through the argument sequence and
// the next required width, preserving argument order. Each state is A8 or A16;
// a three-byte native copy ends in A8 after its exact, non-overlapping tail.
fn select_widths(arguments: &mut [Argument], next_word: bool) {
    let mut suffix = if next_word { [2, 0] } else { [0, 2] };
    let mut choices = vec![[false; 2]; arguments.len()];
    for (arg, choices) in arguments.iter().zip(&mut choices).rev() {
        let mut costs = [0; 2];
        for state in 0..2 {
            costs[state] = 2 * state + 4 * usize::from(arg.bytes) + suffix[0];
            if arg.bytes >= 2 && !matches!(arg.source, Source::Bytes) {
                let pairs = usize::from(arg.bytes / 2);
                let tail = usize::from(arg.bytes % 2);
                let pair_cost = if matches!(arg.source, Source::Immediate(_)) {
                    5
                } else {
                    4
                };
                let native = 2 * (1 - state) + pairs * pair_cost + tail * 6 + suffix[1 - tail];
                if native < costs[state] {
                    costs[state] = native;
                    choices[state] = true;
                }
            }
        }
        suffix = costs;
    }
    let mut state = 0;
    for (arg, choices) in arguments.iter_mut().zip(choices) {
        arg.wide = choices[state];
        state = usize::from(arg.wide && arg.bytes % 2 == 0);
    }
}

impl Builder<'_> {
    fn check_call_home(memory: Memory, bytes: u8, delta: u32) -> Result<(), String> {
        match memory {
            Memory::Stack(offset) if offset != 0 => {
                abi::stack::access_displacement(
                    ByteOffset::new(offset),
                    ByteSize::new(bytes.into()),
                    ByteSize::new(delta),
                )
                .map_err(|e| e.to_string())?;
            }
            Memory::DirectPage(offset)
                if u32::from(offset) + u32::from(bytes) <= abi::generated::DP_SCRATCH_SIZE => {}
            _ => return Err("call copy requires a complete private home".into()),
        }
        Ok(())
    }

    pub(super) fn call_arguments(
        &self,
        args: &[Mir65816Value],
        plan: &Mir65816CallPlan,
        target: &Mir65816CallTarget,
    ) -> Result<Vec<Argument>, String> {
        if self.code.delta() != 0 || args.len() != plan.arguments.len() {
            return Err("invalid call argument count or stack phase".into());
        }
        let delta = plan.outgoing_bytes.get();
        let mut arguments = Vec::with_capacity(args.len());
        for (value, home) in args.iter().zip(&plan.arguments) {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = home else {
                return Err("invalid outgoing home".into());
            };
            let bytes = width(*size)?;
            let displacement = abi::stack::access_displacement(
                ByteOffset::new(
                    offset
                        .get()
                        .checked_add(1)
                        .ok_or("argument offset overflow")?,
                ),
                *size,
                ByteSize::ZERO,
            )
            .map_err(|e| e.to_string())?
            .get() as u8;
            let source_bytes = self.value_width(value)?;
            let memory = self.value_memory(value)?;
            if let Some(memory) = memory {
                Self::check_call_home(memory, source_bytes.min(bytes), delta)?;
            }
            let source = if source_bytes != bytes {
                Source::Bytes // Preserve the existing explicit/zero-extension path.
            } else if let Some(memory) = memory {
                Source::Home(memory)
            } else {
                match value {
                    Mir65816Value::U8(v) => Source::Immediate((*v).into()),
                    Mir65816Value::U16(v) => Source::Immediate((*v).into()),
                    Mir65816Value::U24(v) | Mir65816Value::U32(v) => Source::Immediate(*v),
                    Mir65816Value::Null(_) => Source::Immediate(0),
                    Mir65816Value::Address(v, _) => Source::Immediate(v.value as u32),
                    _ => Source::Bytes, // Keep symbolic byte fixups authoritative.
                }
            };
            arguments.push(Argument {
                source,
                displacement,
                bytes,
                wide: false,
            });
        }
        let next_word = if let Mir65816CallTarget::Indirect(value, bytes) = target {
            if bytes.get() != 3 || self.value_width(value)? != 3 {
                return Err("indirect call requires a full-width callable".into());
            }
            if let Some(memory) = self.value_memory(value)? {
                Self::check_call_home(memory, 3, delta)?;
                true // pointer_value begins with A16 for a captured callable.
            } else {
                false
            }
        } else {
            true
        };
        select_widths(&mut arguments, next_word);
        Ok(arguments)
    }

    pub(super) fn call_result(
        &self,
        result: Option<(TempId, ByteSize)>,
        plan: &Mir65816CallPlan,
    ) -> Result<Option<(Memory, u8)>, String> {
        let Some((id, size)) = result else {
            return Ok(None);
        };
        let bytes = width(size)?;
        let expected = match plan.result {
            Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A8ZeroExtended)) => 1,
            Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)) => 2,
            Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X8ZeroExtended)) => 3,
            Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X16)) => 4,
            _ => return Err("call result requires its declared native lanes".into()),
        };
        let home = self.temp(id)?;
        if bytes != expected || home.slot().width != bytes {
            return Err("call result width mismatch".into());
        }
        Self::check_call_home(home.into(), bytes, 0)?;
        Ok(Some((home.into(), bytes)))
    }

    pub(super) fn copy_call_argument(
        &mut self,
        value: &Mir65816Value,
        arg: &Argument,
    ) -> Result<(), String> {
        if !arg.wide {
            self.code.a8();
            for byte in 0..arg.bytes {
                self.value_byte(value, byte)?;
                self.code.byte(ByteOp::StaStack, arg.displacement + byte);
            }
            return Ok(());
        }
        self.code.a16();
        for byte in (0..arg.bytes - 1).step_by(2) {
            match arg.source {
                Source::Immediate(value) => {
                    self.code.word(WordOp::LdaImm, (value >> (8 * byte)) as u16)
                }
                Source::Home(memory) => self.load_memory(memory, byte.into())?,
                Source::Bytes => unreachable!("byte fallback is never selected wide"),
            }
            self.code.byte(ByteOp::StaStack, arg.displacement + byte);
        }
        if arg.bytes % 2 != 0 {
            self.code.a8();
            self.value_byte(value, arg.bytes - 1)?;
            self.code
                .byte(ByteOp::StaStack, arg.displacement + (arg.bytes - 1));
        }
        Ok(())
    }

    pub(super) fn capture_call_result(&mut self, home: Memory, bytes: u8) -> Result<(), String> {
        if bytes == 1 {
            self.code.a8();
        } else {
            self.code.a16();
        }
        self.store_memory(home, 0)?;
        if bytes > 2 {
            self.code.op(Implied::Txa);
            if bytes == 3 {
                self.code.a8();
            }
            self.store_memory(home, 2)?;
        }
        self.code.a16();
        Ok(())
    }
}
