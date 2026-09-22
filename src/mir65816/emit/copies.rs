//! Shared edge selection and staging requirements. No instruction emission.
use super::{allocation::width, *};
use std::collections::BTreeSet;

/// Fully checked operands for one native word operation. These never refer to
/// external memory or carry a value across MIR operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WordOperand {
    Immediate(u16),
    Stack(u8),
}

pub(super) struct WordCopies {
    pub moves: Vec<(WordOperand, Option<u8>, u8)>,
    pub order: Option<Vec<usize>>,
}

/// Private word homes are disjoint destinations. Capture each source before
/// another assignment overwrites it; retain every assignment, including self-copies.
/// A cycle or partial byte overlap keeps the complete staged path.
pub(super) fn acyclic_word_order<T>(moves: &[(WordOperand, T, u8)]) -> Option<Vec<usize>> {
    for (i, &(source, _, destination)) in moves.iter().enumerate() {
        if moves[..i]
            .iter()
            .any(|&(_, _, d)| destination.abs_diff(d) < 2)
        {
            return None;
        }
        if let WordOperand::Stack(s) = source {
            if moves.iter().any(|&(_, _, d)| s.abs_diff(d) == 1) {
                return None;
            }
        }
    }
    let mut pending: BTreeSet<_> = (0..moves.len()).collect();
    let mut order = Vec::with_capacity(moves.len());
    while !pending.is_empty() {
        let ready = |i: usize| {
            pending.iter().all(|&j| {
                i == j || !matches!(moves[j].0, WordOperand::Stack(s) if s.abs_diff(moves[i].2) < 2)
            })
        };
        let i = pending
            .iter()
            .copied()
            .find(|&i| i + 1 != moves.len() && ready(i))
            .or_else(|| pending.iter().copied().find(|&i| ready(i)))?;
        pending.remove(&i);
        order.push(i);
    }
    Some(order)
}

pub(super) fn word_displacement(offset: u32, delta: u32) -> Result<u8, String> {
    abi::stack::access_displacement(
        ByteOffset::new(offset),
        ByteSize::new(2),
        ByteSize::new(delta),
    )
    .map(|d| d.get() as u8)
    .map_err(|e| e.to_string())
}

impl AllocatedFrame {
    pub(super) fn incoming_home(
        &self,
        routine: &Mir65816Routine,
        id: ParamId,
    ) -> Result<u32, String> {
        let param = routine
            .frame
            .parameters
            .iter()
            .find(|p| p.param == id)
            .ok_or("unknown parameter")?;
        let Mir65816AbiHome::StackArgument { offset, size, .. } = param.incoming else {
            return Err("invalid parameter home".into());
        };
        abi::stack::incoming_displacement(ByteSize::new(self.extent.into()), offset, size)
            .map(|d| d.get())
            .map_err(|e| e.to_string())
    }
    pub(super) fn parameter_home(
        &self,
        routine: &Mir65816Routine,
        id: ParamId,
    ) -> Result<(u32, u8), String> {
        let param = routine
            .frame
            .parameters
            .iter()
            .find(|p| p.param == id)
            .ok_or("unknown parameter")?;
        let Mir65816AbiHome::StackArgument { size, .. } = param.incoming else {
            return Err("invalid parameter home".into());
        };
        Ok((
            if let Some(object) = param.frame_object {
                routine
                    .frame
                    .objects
                    .iter()
                    .find(|o| o.id == object)
                    .ok_or("unknown frame object")?
                    .stack_offset
                    .get()
            } else {
                self.incoming_home(routine, id)?
            },
            width(size)?,
        ))
    }
    pub(super) fn word_operand(
        &self,
        routine: &Mir65816Routine,
        delta: u32,
        value: &Mir65816Value,
    ) -> Result<Option<WordOperand>, String> {
        let offset = match value {
            Mir65816Value::U8(value) => {
                return Ok(Some(WordOperand::Immediate(u16::from(*value))));
            }
            Mir65816Value::U16(value) => return Ok(Some(WordOperand::Immediate(*value))),
            Mir65816Value::Temp(id, bytes) => {
                let location = self
                    .temps
                    .get(id)
                    .copied()
                    .ok_or_else(|| format!("undefined temporary t{}", id.0))?;
                if location.slot().width != width(*bytes)? {
                    return Err("temporary width mismatch".into());
                }
                match location {
                    Location::Stack(slot) if slot.width == 2 => u32::from(slot.offset),
                    _ => return Ok(None),
                }
            }
            Mir65816Value::Param(id) => {
                let (offset, bytes) = self.parameter_home(routine, *id)?;
                if bytes != 2 {
                    return Ok(None);
                }
                offset
            }
            _ => return Ok(None),
        };
        Ok(Some(WordOperand::Stack(word_displacement(offset, delta)?)))
    }
    pub(super) fn value_width(
        &self,
        routine: &Mir65816Routine,
        value: &Mir65816Value,
    ) -> Result<u8, String> {
        Ok(match value {
            Mir65816Value::U8(_) => 1,
            Mir65816Value::U16(_) => 2,
            Mir65816Value::U24(_) => 3,
            Mir65816Value::U32(_) => 4,
            Mir65816Value::Param(id) => self.parameter_home(routine, *id)?.1,
            Mir65816Value::Null(w)
            | Mir65816Value::Address(_, w)
            | Mir65816Value::StaticAddress(_, w)
            | Mir65816Value::Temp(_, w)
            | Mir65816Value::GlobalAddress(_, w)
            | Mir65816Value::RoutineAddress(_, w) => width(*w)?,
        })
    }

    pub(super) fn edge_widths(
        &self,
        routine: &Mir65816Routine,
        edge: &Mir65816Edge,
    ) -> Result<Vec<u8>, String> {
        let block = routine
            .blocks
            .iter()
            .find(|b| b.id == edge.target)
            .ok_or("unknown branch target")?;
        if edge.args.len() != block.params.len() {
            return Err("edge argument count mismatch".into());
        }
        edge.args
            .iter()
            .zip(&block.params)
            .map(|(value, (dest, bytes))| {
                let bytes = width(*bytes)?;
                if self.value_width(routine, value)? != bytes {
                    return Err("edge argument width mismatch".into());
                }
                let home = self
                    .temps
                    .get(dest)
                    .ok_or("missing edge destination temporary")?;
                if home.slot().width != bytes {
                    return Err("edge destination temporary width mismatch".into());
                }
                Ok(bytes)
            })
            .collect()
    }

    pub(super) fn word_copies(
        &self,
        routine: &Mir65816Routine,
        edge: &Mir65816Edge,
        delta: u32,
    ) -> Result<Option<WordCopies>, String> {
        let widths = self.edge_widths(routine, edge)?;
        if widths.is_empty() || widths.iter().any(|&w| w != 2) {
            return Ok(None);
        }
        let block = routine.blocks.iter().find(|b| b.id == edge.target).unwrap();
        let mut moves = Vec::with_capacity(edge.args.len());
        let mut supported = true;
        for (value, (dest, _)) in edge.args.iter().zip(&block.params) {
            let source = self.word_operand(routine, delta, value)?;
            let destination = match self.temps[dest] {
                Location::Stack(slot) => Some(word_displacement(slot.offset.into(), delta)?),
                Location::DirectPage(_) => None,
            };
            if let (Some(source), Some(destination)) = (source, destination) {
                moves.push((source, None, destination));
            } else {
                // Unsupported entries must not hide a malformed later home.
                supported = false;
            }
        }
        if !supported {
            return Ok(None);
        }
        // A single load captures both bytes before any write, even for partial overlap.
        let order = if moves.len() == 1 {
            Some(vec![0])
        } else {
            acyclic_word_order(&moves)
        };
        Ok(Some(WordCopies { moves, order }))
    }

    /// Dense prefix of slots: every staged edge saves all its arguments. Only
    /// staged edges contribute; each index reserves its maximum actual width.
    pub(super) fn staging_widths(&self, routine: &Mir65816Routine) -> Result<Vec<u8>, String> {
        let mut required = Vec::<u8>::new();
        for block in &routine.blocks {
            let edges = match &block.terminator {
                Mir65816Terminator::Goto(e) => vec![e],
                Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
                Mir65816Terminator::Return { .. }
                | Mir65816Terminator::Fallthrough
                | Mir65816Terminator::Exit => vec![],
            };
            for edge in edges {
                if self
                    .word_copies(routine, edge, 0)?
                    .is_some_and(|p| p.order.is_some())
                {
                    continue;
                }
                for (i, w) in self.edge_widths(routine, edge)?.into_iter().enumerate() {
                    if i == required.len() {
                        required.push(w);
                    } else {
                        required[i] = required[i].max(w);
                    }
                }
            }
        }
        Ok(required)
    }
}
