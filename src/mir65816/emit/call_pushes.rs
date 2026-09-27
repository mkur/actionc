//! Complete outgoing areas built from high addresses down, using exact PHA chunks.
use super::*;

#[cfg(test)]
#[path = "call_push_tests.rs"]
mod tests;

#[derive(Clone, Copy)]
struct Cell {
    argument: Option<usize>,
    byte: u8,
}

struct Chunk {
    source: Source,
    argument: Option<usize>,
    byte: u8,
    width: u8,
    immediate: Option<u16>,
}

pub(in crate::mir65816::emit::select) struct Plan {
    chunks: Vec<Chunk>,
    cost: usize,
}

/// Cost of the already selected reservation/stores, including padding and the
/// final A16 request. Only private/immediate byte operands reach this selector.
fn store_cost(arguments: &[Argument], padding: &[u8]) -> usize {
    let mut cost = 6;
    let mut mode = None;
    if !padding.is_empty() {
        cost += 4 + 2 * padding.len();
        mode = Some(1);
    }
    for arg in arguments {
        let width = if arg.copy == ArgumentCopy::Bytes {
            1
        } else {
            2
        };
        cost += usize::from(mode != Some(width)) * 2;
        mode = Some(width);
        match arg.copy {
            ArgumentCopy::Bytes => cost += 4 * usize::from(arg.bytes),
            ArgumentCopy::Native | ArgumentCopy::OverlappingWords => {
                let pair = if matches!(arg.source, Source::Immediate(_)) {
                    5
                } else {
                    4
                };
                cost += pair
                    * if arg.copy == ArgumentCopy::OverlappingWords {
                        2
                    } else {
                        usize::from(arg.bytes / 2)
                    };
                if arg.copy == ArgumentCopy::Native && arg.bytes % 2 != 0 {
                    cost += 6;
                    mode = Some(1);
                }
            }
        }
    }
    cost + usize::from(mode != Some(2)) * 2
}

impl Plan {
    pub(in crate::mir65816::emit::select) fn new(
        arguments: &[Argument],
        values: &[Mir65816Value],
        padding: &[u8],
        outgoing: u16,
    ) -> Option<Self> {
        // Relocatable operands retain their existing BYTE selectors. Width
        // extension and unsupported sources keep the complete store fallback.
        if arguments.iter().enumerate().any(|(i, arg)| {
            matches!(arg.source, Source::Bytes)
                && !matches!(values.get(i), Some(
                    Mir65816Value::StaticAddress(_, w)
                    | Mir65816Value::GlobalAddress(_, w)
                    | Mir65816Value::RoutineAddress(_, w)
                ) if w.get() == u32::from(arg.bytes))
        }) {
            return None;
        }
        let extent = usize::from(outgoing);
        let mut cells = vec![
            Cell {
                argument: None,
                byte: 0
            };
            extent
        ];
        for (i, arg) in arguments.iter().enumerate() {
            for byte in 0..arg.bytes {
                cells[usize::from(arg.displacement - 1 + byte)] = Cell {
                    argument: Some(i),
                    byte,
                };
            }
        }
        let source = |cell: Cell| {
            cell.argument
                .map_or(Source::Immediate(0), |i| arguments[i].source)
        };
        let immediate = |cell: Cell| match source(cell) {
            Source::Immediate(value) => Some((value >> (8 * cell.byte)) as u8),
            _ => None,
        };
        // Modes: A8, A16, no permission after the guard join. The bottom of
        // the finished area must hand a direct JSL an explicit A16 state.
        let mut costs = vec![[0usize; 3]; extent + 1];
        let mut choices = vec![[(1usize, false); 3]; extent + 1];
        costs[0] = [2, 0, 2];
        for remaining in 1..=extent {
            for mode in 0..3 {
                let mut best = usize::MAX;
                for width in 1..=2.min(remaining) {
                    let low = cells[remaining - width];
                    let high = cells[remaining - 1];
                    let pea = width == 2 && immediate(low).is_some() && immediate(high).is_some();
                    if width == 2
                        && !pea
                        && (matches!(source(low), Source::Bytes)
                            || low.argument != high.argument
                            || low.argument.is_some() && low.byte + 1 != high.byte)
                    {
                        continue;
                    }
                    let load = if width == 2 && matches!(source(low), Source::Immediate(_)) {
                        3
                    } else {
                        2
                    };
                    let cost = if pea {
                        // PEA preserves A and M, including unknown permission
                        // after the guard join. Both bytes are independently
                        // known, even across an argument/padding boundary.
                        3 + costs[remaining - width][mode]
                    } else {
                        usize::from(mode != width - 1) * 2
                            + load
                            + 1
                            + costs[remaining - width][width - 1]
                    };
                    if cost < best {
                        best = cost;
                        choices[remaining][mode] = (width, pea);
                    }
                }
                costs[remaining][mode] = best;
            }
        }
        let cost = costs[extent][2];
        if cost >= store_cost(arguments, padding) {
            return None;
        }
        let mut chunks = Vec::new();
        let (mut remaining, mut mode) = (extent, 2);
        while remaining > 0 {
            let (width, pea) = choices[remaining][mode];
            let cell = cells[remaining - width];
            chunks.push(Chunk {
                source: source(cell),
                argument: cell.argument,
                byte: cell.byte,
                width: width as u8,
                immediate: if pea {
                    Some(u16::from_le_bytes([
                        immediate(cell).unwrap(),
                        immediate(cells[remaining - 1]).unwrap(),
                    ]))
                } else {
                    None
                },
            });
            remaining -= width;
            if !pea {
                mode = width - 1;
            }
        }
        Some(Self { chunks, cost })
    }

    pub(in crate::mir65816::emit::select) fn emit(
        &self,
        b: &mut Builder<'_>,
        values: &[Mir65816Value],
    ) -> Result<(), String> {
        let start = b.code.position();
        for chunk in &self.chunks {
            if let Some(value) = chunk.immediate {
                b.code.instruction(Instruction::ArgumentPushWord(value))?;
                continue;
            }
            if chunk.width == 2 {
                b.code.a16();
            } else {
                b.code.a8();
            }
            match chunk.source {
                Source::Immediate(value) if chunk.width == 2 => b
                    .code
                    .word(WordOp::LdaImm, (value >> (8 * chunk.byte)) as u16),
                Source::Immediate(value) => b
                    .code
                    .byte(ByteOp::LdaImm, (value >> (8 * chunk.byte)) as u8),
                Source::Home(memory) => b.load_memory(memory, chunk.byte.into())?,
                Source::Bytes => b.value_byte(&values[chunk.argument.unwrap()], chunk.byte)?,
            }
            b.code.instruction(Instruction::ArgumentPush)?;
        }
        b.code.a16();
        debug_assert_eq!(b.code.position() - start, self.cost);
        Ok(())
    }
}
