//! Native pieces for verified unsigned integer casts between private homes.
use super::*;
use crate::mir65816::emit::state::Width;

#[cfg(test)]
#[path = "integer_cast_tests.rs"]
mod tests;

#[derive(Clone, Copy)]
struct Piece {
    offset: u8,
    width: Width,
    copy: bool,
}

impl Builder<'_> {
    pub(super) fn integer_cast(&mut self, op: &Mir65816Op) -> Result<bool, String> {
        let Mir65816Op::Cast {
            dest,
            from,
            to,
            from_signed: false,
            kind: NirCastKind::Integer,
            value: Mir65816Value::Temp(source, actual),
        } = op
        else {
            return Ok(false);
        };
        if !(2..=4).contains(&from.get()) || !(2..=4).contains(&to.get()) {
            return Ok(false);
        }
        if actual != from {
            return Err("integer cast source width mismatch".into());
        }
        if !self.routine.temps.iter().any(|(id, ty)| {
            id == source
                && ty.width == Some(*from)
                && ty
                    .kind
                    .integer()
                    .is_some_and(|i| !i.signed && u32::from(i.bits) == from.get() * 8)
        }) {
            return Ok(false);
        }
        let (source_home, destination_home) = (self.temp(*source)?, self.temp(*dest)?);
        if source_home.slot().width != from.get() as u8
            || destination_home.slot().width != to.get() as u8
        {
            return Err("integer cast requires complete homes".into());
        }
        let (Location::Stack(_), Location::Stack(destination)) = (source_home, destination_home)
        else {
            return Ok(false);
        };
        let Some(Memory::Stack(source)) =
            self.value_memory(&Mir65816Value::Temp(*source, *from))?
        else {
            return Ok(false);
        };
        let destination = u32::from(destination.offset);
        // Check every owned byte, including discarded source bytes and new zeros.
        for (home, size) in [(source, from.get()), (destination, to.get())] {
            self.displacement(home, 0)?;
            self.displacement(home, size - 1)?;
        }
        if source != destination
            && source < destination + to.get()
            && destination < source + from.get()
        {
            return Ok(false);
        }
        let copied = from.get().min(to.get()) as u8;
        let mut pieces = Vec::new();
        if source != destination {
            pieces.push(Piece {
                offset: 0,
                width: Width::Word,
                copy: true,
            });
            if copied > 2 {
                // Private, non-overlapping homes permit the shared middle byte.
                // A three-byte copy reads words at 0 and 1, never a fourth byte.
                pieces.push(Piece {
                    offset: copied - 2,
                    width: Width::Word,
                    copy: true,
                });
            }
        }
        if to > from {
            pieces.push(Piece {
                offset: from.get() as u8,
                width: if to.get() - from.get() == 2 {
                    Width::Word
                } else {
                    Width::Byte
                },
                copy: false,
            });
        }
        let fallback = self.code.mode_cost(Width::Byte)
            + 4 * usize::from(copied)
            + if to > from {
                2 + 2 * (to.get() - from.get()) as usize
            } else {
                0
            };
        let mut previous = None;
        let mut native = 0;
        for piece in &pieces {
            native += previous.map_or_else(
                || self.code.mode_cost(piece.width),
                |w| if w == piece.width { 0 } else { 2 },
            );
            native += if piece.copy {
                4
            } else if piece.width == Width::Word {
                5
            } else {
                4
            };
            previous = Some(piece.width);
        }
        // Match the byte fallback's exit contract; no speculative downstream credit.
        native += previous.map_or_else(
            || self.code.mode_cost(Width::Byte),
            |w| if w == Width::Byte { 0 } else { 2 },
        );
        if native >= fallback {
            return Ok(false);
        }
        self.code.barrier();
        for piece in pieces {
            if piece.width == Width::Word {
                self.code.a16();
            } else {
                self.code.a8();
            }
            if piece.copy {
                self.load_memory(Memory::Stack(source), piece.offset.into())?;
            } else if piece.width == Width::Word {
                self.code.word(WordOp::LdaImm, 0);
            } else {
                self.code.byte(ByteOp::LdaImm, 0);
            }
            self.store_memory(Memory::Stack(destination), piece.offset.into())?;
        }
        self.code.a8();
        Ok(true)
    }
}
