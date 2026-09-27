//! Complete byte-fallback preflight; omit only isolated private stack identities.
use super::*;

struct MixedCopy {
    source: Option<Memory>,
    destination: Memory,
    staging: Memory,
    bytes: u8,
    identity: bool,
}

fn overlap(a: Memory, aw: u8, b: Memory, bw: u8) -> bool {
    let (a, b) = match (a, b) {
        (Memory::Stack(a), Memory::Stack(b)) => (a, b),
        (Memory::DirectPage(a), Memory::DirectPage(b)) => (a.into(), b.into()),
        _ => return false,
    };
    u64::from(a) < u64::from(b) + u64::from(bw) && u64::from(b) < u64::from(a) + u64::from(aw)
}

impl Builder<'_> {
    fn mixed_edge_plan(&self, edge: &Mir65816Edge) -> Result<Vec<MixedCopy>, String> {
        let widths = self.frame.edge_widths(self.routine, edge)?;
        let block = self
            .routine
            .blocks
            .iter()
            .find(|b| b.id == edge.target)
            .unwrap();
        let mut copies = Vec::new();
        for (i, ((value, &(dest, _)), bytes)) in
            edge.args.iter().zip(&block.params).zip(widths).enumerate()
        {
            self.staging(i, bytes)?;
            let staging = Memory::Stack(self.frame.edge_copies[i].offset.into());
            let source = self.value_memory(value)?;
            let destination = self.temp(dest)?.into();
            for memory in [Some(staging), Some(destination), source]
                .into_iter()
                .flatten()
            {
                match memory {
                    Memory::Stack(at) if at != 0 => {
                        self.displacement(at, u32::from(bytes) - 1)?;
                    }
                    Memory::DirectPage(at)
                        if u32::from(at) + u32::from(bytes) <= abi::generated::DP_SCRATCH_SIZE => {}
                    _ => return Err("mixed edge requires complete private homes".into()),
                }
            }
            copies.push(MixedCopy {
                source,
                destination,
                staging,
                bytes,
                identity: false,
            });
        }
        for (i, copy) in copies.iter().enumerate() {
            if copies.iter().enumerate().any(|(j, other)| {
                (i != j && overlap(copy.staging, copy.bytes, other.staging, other.bytes))
                    || overlap(copy.staging, copy.bytes, other.destination, other.bytes)
                    || other
                        .source
                        .is_some_and(|s| overlap(copy.staging, copy.bytes, s, other.bytes))
            }) {
                return Err("mixed edge staging overlaps a live home or staging slot".into());
            }
        }
        for i in 0..copies.len() {
            copies[i].identity = matches!((copies[i].source, copies[i].destination),
                (Some(Memory::Stack(a)), Memory::Stack(b)) if a == b)
                && copies.iter().enumerate().all(|(j, other)| {
                    i == j
                        || !overlap(
                            copies[i].destination,
                            copies[i].bytes,
                            other.destination,
                            other.bytes,
                        )
                });
        }
        Ok(copies)
    }

    fn native_mixed_profitable(&self, copies: &[MixedCopy]) -> bool {
        // RESULT is existing domain-local scratch, saved with every task. Never
        // borrow it when an edge operand occupies any of its first two bytes.
        let saved = Memory::DirectPage(RESULT.into());
        if copies.iter().any(|c| {
            !matches!(c.bytes, 1 | 2 | 4)
                || !matches!(c.destination, Memory::Stack(_))
                || c.source.is_some_and(|s| !matches!(s, Memory::Stack(_)))
                || overlap(saved, 2, c.destination, c.bytes)
                || c.source.is_some_and(|s| overlap(saved, 2, s, c.bytes))
                || (c.bytes > 1 && !c.identity && c.source.is_none())
        }) {
            return false;
        }
        // Conservative encoded-size/cycle model, starting and ending in A16.
        // A16 stack pairs cost 10 cycles versus 16 for two byte pairs; domain
        // scratch can be one cycle cheaper. Mode changes and A/B repair count.
        let cost = |wide: bool| {
            let (mut size, mut cycles, mut word) = (0, 0, true);
            let mut mode = |next: bool, size: &mut u32, cycles: &mut u32| {
                if next != word {
                    *size += 2;
                    *cycles += 3;
                    word = next;
                }
            };
            if wide {
                size += 2;
                cycles += 5;
            } // Save A; budget unaligned D.
            if !wide {
                mode(false, &mut size, &mut cycles);
            }
            for _ in 0..2 {
                for c in copies.iter().filter(|c| !c.identity) {
                    let w = wide && c.bytes > 1;
                    mode(w, &mut size, &mut cycles);
                    let pieces = u32::from(c.bytes) / if w { 2 } else { 1 };
                    size += 4 * pieces;
                    cycles += if w { 10 } else { 8 } * pieces;
                }
            }
            if wide {
                mode(true, &mut size, &mut cycles);
                size += 2;
                cycles += 5; // Restore A, including hidden B.
            }
            if wide || copies.last().is_some_and(|c| c.identity) {
                mode(false, &mut size, &mut cycles);
                size += 2;
                cycles += 4;
            }
            mode(true, &mut size, &mut cycles);
            (size, cycles)
        };
        let (old_size, old_cycles) = cost(false);
        let (new_size, new_cycles) = cost(true);
        new_size <= old_size && new_cycles < old_cycles
    }

    pub(super) fn emit_mixed_edge(&mut self, edge: &Mir65816Edge) -> Result<(), String> {
        let copies = self.mixed_edge_plan(edge)?;
        let wide = self.native_mixed_profitable(&copies);
        if wide {
            self.code.a16();
            self.code.byte(ByteOp::StaDp, RESULT);
        } else {
            self.code.a8();
        }
        // All remaining sources are captured before any destination changes.
        for (value, copy) in edge.args.iter().zip(&copies).filter(|(_, c)| !c.identity) {
            if wide && copy.bytes > 1 {
                self.transfer(copy.source.unwrap(), copy.staging, copy.bytes, true)?;
                continue;
            }
            self.code.a8();
            for byte in 0..copy.bytes {
                self.value_byte(value, byte)?;
                self.store_memory(copy.staging, byte.into())?;
            }
        }
        for copy in copies.iter().filter(|c| !c.identity) {
            if wide && copy.bytes > 1 {
                self.transfer(copy.staging, copy.destination, copy.bytes, true)?;
                continue;
            }
            self.code.a8();
            for byte in 0..copy.bytes {
                self.load_memory(copy.staging, byte.into())?;
                self.store_memory(copy.destination, byte.into())?;
            }
        }
        if wide {
            self.code.a16();
            self.code.byte(ByteOp::LdaDp, RESULT);
        }
        if let Some(last) = copies.last().filter(|c| wide || c.identity) {
            self.code.a8();
            // A8 keeps incoming hidden B; this restores the final byte and N/Z.
            self.load_memory(last.destination, u32::from(last.bytes) - 1)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "mixed_edge_tests.rs"]
mod tests;
