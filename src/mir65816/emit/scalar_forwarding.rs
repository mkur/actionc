//! Single-use scalar reads from authoritative private homes, after allocation.
use super::*;

#[cfg(test)]
#[path = "scalar_forwarding_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceKind {
    Parameter(ParamId),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Source {
    kind: SourceKind,
    pub(super) home: Slot,
}

#[derive(Clone, Debug)]
struct Binding {
    temp: TempId,
    source: Source,
    definition: (BlockId, usize),
    consumer: usize,
}

#[derive(Default)]
pub(in crate::mir65816::emit) struct Plan {
    bindings: Vec<Binding>,
}

fn canonical(address: &Mir65816Address) -> bool {
    address.index.is_none() && address.displacement.get() == 0
}

fn incoming(
    r: &Mir65816Routine,
    frame: &AllocatedFrame,
    address: &Mir65816Address,
    bytes: u8,
) -> Result<Option<Source>, String> {
    let Mir65816AddressBase::Parameter(id) = address.base else {
        return Ok(None);
    };
    let p = r
        .frame
        .parameters
        .iter()
        .find(|p| p.param == id)
        .ok_or("unknown scalar parameter")?;
    if !canonical(address)
        || p.frame_object.is_some()
        || !matches!(p.incoming, Mir65816AbiHome::StackArgument {size, ..} if size.get() == u32::from(bytes))
        || r.frame
            .objects
            .iter()
            .any(|o| o.owner == Mir65816FrameObjectOwner::Param(id))
    {
        return Ok(None);
    }
    let refers = |a: &Mir65816Address| a.base == address.base;
    if r.blocks.iter().flat_map(|b| &b.ops).any(|op| match op {
        Mir65816Op::Load {
            address,
            width,
            volatile,
            ..
        } => {
            refers(address) && (*volatile || width.get() != u32::from(bytes) || !canonical(address))
        }
        Mir65816Op::Store { address, .. } | Mir65816Op::AddressOf { address, .. } => {
            refers(address)
        }
        Mir65816Op::Copy {
            source,
            destination,
            ..
        } => refers(source) || refers(destination),
        _ => false,
    }) {
        return Ok(None);
    }
    let offset = frame.incoming_home(r, id)?;
    abi::stack::access_displacement(
        ByteOffset::new(offset),
        ByteSize::new(bytes.into()),
        ByteSize::ZERO,
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(Source {
        kind: SourceKind::Parameter(id),
        home: Slot {
            offset: offset as u16,
            width: bytes,
        },
    }))
}

fn operand(
    r: &Mir65816Routine,
    frame: &AllocatedFrame,
    value: &Mir65816Value,
    arithmetic: bool,
) -> bool {
    match value {
        Mir65816Value::U32(_) => true,
        Mir65816Value::U8(_) | Mir65816Value::U16(_) | Mir65816Value::U24(_) => arithmetic,
        Mir65816Value::Temp(id, w) => {
            w.get() == 4 && matches!(frame.temps.get(id), Some(Location::Stack(s)) if s.width == 4)
        }
        Mir65816Value::Param(id) => matches!(frame.parameter_home(r, *id), Ok((_, 4))),
        _ => false,
    }
}

fn supported(r: &Mir65816Routine, frame: &AllocatedFrame, op: &Mir65816Op) -> bool {
    match op {
        Mir65816Op::Compare {
            width, left, right, ..
        } => width.get() == 4 && operand(r, frame, left, false) && operand(r, frame, right, false),
        Mir65816Op::Binary {
            width,
            operation,
            left,
            right,
            ..
        } => {
            width.get() == 4
                && matches!(
                    operation,
                    NirBinaryOp::Add
                        | NirBinaryOp::Sub
                        | NirBinaryOp::And
                        | NirBinaryOp::Or
                        | NirBinaryOp::Xor
                )
                && operand(r, frame, left, true)
                && operand(r, frame, right, true)
        }
        _ => false,
    }
}

impl Plan {
    pub(in crate::mir65816::emit) fn new(
        r: &Mir65816Routine,
        frame: &AllocatedFrame,
    ) -> Result<Self, String> {
        let counts = liveness::input_counts(r);
        let definitions = liveness::definition_counts(r);
        let mut plan = Self::default();
        for block in &r.blocks {
            for (index, op) in block.ops.iter().enumerate() {
                let Mir65816Op::Load {
                    dest,
                    width,
                    address,
                    volatile: false,
                } = op
                else {
                    continue;
                };
                if width.get() != 4
                    || counts.get(dest) != Some(&1)
                    || definitions.get(dest) != Some(&1)
                    || !r.temps.iter().any(|(id, ty)| {
                        id == dest && !ty.pointer && ty.kind.integer().is_some_and(|i| i.bits == 32)
                    })
                {
                    continue;
                }
                // Omitted accumulator homes and closed DP/X allocations retain
                // their existing producers/consumers and fact witnesses.
                let Some(Location::Stack(capture)) = frame.temps.get(dest) else {
                    continue;
                };
                if capture.width != 4 {
                    return Err("scalar capture width mismatch".into());
                }
                let Some(consumer) = block.ops.get(index + 1) else {
                    continue;
                };
                if liveness::operation_inputs(consumer)
                    .iter()
                    .filter(|id| *id == dest)
                    .count()
                    != 1
                    || !supported(r, frame, consumer)
                {
                    continue;
                }
                let Some(source) = incoming(r, frame, address, 4)? else {
                    continue;
                };
                if frame
                    .temps
                    .values()
                    .any(|home| home.overlaps(Location::Stack(source.home)))
                {
                    return Err("temporary overlaps authoritative scalar source".into());
                }
                plan.bindings.push(Binding {
                    temp: *dest,
                    source,
                    definition: (block.id, index),
                    consumer: index + 1,
                });
            }
        }
        Ok(plan)
    }

    #[cfg(feature = "native65816-state-proof")]
    pub(in crate::mir65816::emit) fn observations(&self) -> Vec<super::super::proof::ScalarRead> {
        self.bindings
            .iter()
            .map(|b| super::super::proof::ScalarRead {
                temp: b.temp,
                block: b.definition.0,
                producer: b.definition.1,
                consumer: b.consumer,
                source: match b.source.kind {
                    SourceKind::Parameter(id) => super::super::proof::HomeOwner::Incoming(id),
                },
                offset: b.source.home.offset,
                bytes: b.source.home.width,
            })
            .collect()
    }

    pub(super) fn enter(&self, b: &mut Builder<'_>, block: BlockId, index: usize) -> bool {
        b.scalar_borrowed.clear();
        for binding in &self.bindings {
            if binding.definition.0 == block && binding.consumer == index {
                match binding.source.kind {
                    SourceKind::Parameter(id) => debug_assert_eq!(
                        b.frame.incoming_home(b.routine, id).unwrap(),
                        u32::from(binding.source.home.offset)
                    ),
                }
                b.scalar_borrowed.insert(binding.temp, binding.source);
            }
        }
        let omitted = self.bindings.iter().any(|b| b.definition == (block, index));
        if omitted {
            b.code.barrier();
        }
        omitted
    }
}
