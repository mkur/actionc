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
    byte: u8,
    width: u8,
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
        padding: &[u8],
        outgoing: u16,
    ) -> Option<Self> {
        // Exact scalar captures and numeric constants. The
        // existing complete reservation strategy handles all other calls.
        if arguments
            .iter()
            .any(|arg| matches!(arg.source, Source::Bytes))
        {
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
        // Modes: A8, A16, no permission after the guard join. The bottom of
        // the finished area must hand a direct JSL an explicit A16 state.
        let mut costs = vec![[0usize; 3]; extent + 1];
        let mut choices = vec![[1usize; 3]; extent + 1];
        costs[0] = [2, 0, 2];
        for remaining in 1..=extent {
            for mode in 0..3 {
                let mut best = usize::MAX;
                for width in 1..=2.min(remaining) {
                    let low = cells[remaining - width];
                    let high = cells[remaining - 1];
                    if width == 2
                        && (low.argument != high.argument
                            || low.argument.is_some() && low.byte + 1 != high.byte)
                    {
                        continue;
                    }
                    let load = if width == 2 && matches!(source(low), Source::Immediate(_)) {
                        3
                    } else {
                        2
                    };
                    let cost = usize::from(mode != width - 1) * 2
                        + load
                        + 1
                        + costs[remaining - width][width - 1];
                    if cost < best {
                        best = cost;
                        choices[remaining][mode] = width;
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
            let width = choices[remaining][mode];
            let cell = cells[remaining - width];
            chunks.push(Chunk {
                source: source(cell),
                byte: cell.byte,
                width: width as u8,
            });
            remaining -= width;
            mode = width - 1;
        }
        Some(Self { chunks, cost })
    }

    pub(in crate::mir65816::emit::select) fn emit(
        &self,
        b: &mut Builder<'_>,
    ) -> Result<(), String> {
        let start = b.code.position();
        for chunk in &self.chunks {
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
                Source::Bytes => unreachable!("unsupported push operand"),
            }
            b.code.instruction(Instruction::ArgumentPush)?;
        }
        b.code.a16();
        debug_assert_eq!(b.code.position() - start, self.cost);
        Ok(())
    }
}
