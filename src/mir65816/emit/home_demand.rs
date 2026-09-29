//! Storage decisions made before allocation, from typed producer/consumer uses.
use super::*;
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "home_demand_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MemoryReason {
    UnsupportedProducer,
    MultipleDefinitions,
    NotSingleUse,
    VolatileOrAddress,
    UnsupportedWidth,
    UnsupportedConsumer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Accumulator {
    pub block: BlockId,
    pub producer: usize,
    pub consumer: usize,
    pub bytes: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Memory(MemoryReason),
    Accumulator(Accumulator),
}

pub(super) struct Plan {
    pub decisions: BTreeMap<TempId, Decision>,
    producers: BTreeMap<(BlockId, usize), TempId>,
    consumers: BTreeMap<(BlockId, usize), TempId>,
}

fn unsigned(r: &Mir65816Routine, id: TempId, bytes: u32) -> bool {
    r.temps.iter().any(|(t, ty)| {
        *t == id
            && ty.width == Some(ByteSize::new(bytes))
            && !ty.pointer
            && ty
                .kind
                .integer()
                .is_some_and(|i| !i.signed && u32::from(i.bits) == bytes * 8)
    })
}

fn word_value(r: &Mir65816Routine, value: &Mir65816Value) -> bool {
    match value {
        Mir65816Value::U8(_) | Mir65816Value::U16(_) => true,
        Mir65816Value::Temp(id, width) => width.get() == 2 && unsigned(r, *id, 2),
        Mir65816Value::Param(id) => r.frame.parameters.iter().any(|p| {
            p.param == *id && matches!(p.incoming, Mir65816AbiHome::StackArgument { size, .. } if size.get() == 2)
        }),
        _ => false,
    }
}

pub(super) fn small_shift(op: NirBinaryOp, right: &Mir65816Value) -> Option<u8> {
    if !matches!(op, NirBinaryOp::Lsh | NirBinaryOp::Rsh) {
        return None;
    }
    let n = match right {
        Mir65816Value::U8(n) => u32::from(*n),
        Mir65816Value::U16(n) => u32::from(*n),
        Mir65816Value::U24(n) | Mir65816Value::U32(n) => *n,
        _ => return None,
    };
    (1..=3).contains(&n).then_some(n as u8)
}

impl Plan {
    pub fn new(r: &Mir65816Routine) -> Self {
        let counts = liveness::input_counts(r);
        let mut definitions = BTreeMap::<TempId, usize>::new();
        for block in &r.blocks {
            for (id, _) in &block.params {
                *definitions.entry(*id).or_default() += 1;
            }
            for op in &block.ops {
                if let Some(id) = liveness::operation_output(op) {
                    *definitions.entry(id).or_default() += 1;
                }
            }
        }
        let mut plan = Self {
            decisions: r
                .temps
                .iter()
                .map(|(id, _)| (*id, Decision::Memory(MemoryReason::UnsupportedProducer)))
                .collect(),
            producers: BTreeMap::new(),
            consumers: BTreeMap::new(),
        };
        for block in &r.blocks {
            for (index, op) in block.ops.iter().enumerate() {
                let Some(id) = liveness::operation_output(op) else {
                    continue;
                };
                let selected = (|| {
                    if definitions.get(&id) != Some(&1) {
                        return Err(MemoryReason::MultipleDefinitions);
                    }
                    if counts.get(&id) != Some(&1) {
                        return Err(MemoryReason::NotSingleUse);
                    }
                    let bytes = match op {
                        Mir65816Op::Load {
                            width,
                            address,
                            volatile,
                            ..
                        } => {
                            if *volatile
                                || address.index.is_some()
                                || !matches!(
                                    address.base,
                                    Mir65816AddressBase::AutomaticFrame(_)
                                        | Mir65816AddressBase::Parameter(_)
                                        | Mir65816AddressBase::Static(NirStorageId::Global(_))
                                        | Mir65816AddressBase::External(
                                            Mir65816ExternalAddress::Global(_)
                                        )
                                )
                            {
                                return Err(MemoryReason::VolatileOrAddress);
                            }
                            width.get()
                        }
                        Mir65816Op::Binary {
                            width,
                            signed: false,
                            operation,
                            left,
                            right,
                            ..
                        } if width.get() == 2
                            && (matches!(
                                operation,
                                NirBinaryOp::Add
                                    | NirBinaryOp::Sub
                                    | NirBinaryOp::And
                                    | NirBinaryOp::Or
                                    | NirBinaryOp::Xor
                            ) && word_value(r, right)
                                || small_shift(*operation, right).is_some())
                            && word_value(r, left) =>
                        {
                            2
                        }
                        _ => return Err(MemoryReason::UnsupportedProducer),
                    };
                    if !matches!(bytes, 1 | 2) || !unsigned(r, id, bytes) {
                        return Err(MemoryReason::UnsupportedWidth);
                    }
                    let Some(Mir65816Op::Cast {
                        dest,
                        from,
                        to,
                        from_signed: false,
                        kind: NirCastKind::Integer,
                        value: Mir65816Value::Temp(input, actual),
                    }) = block.ops.get(index + 1)
                    else {
                        return Err(MemoryReason::UnsupportedConsumer);
                    };
                    if *input != id
                        || actual != from
                        || from.get() != bytes
                        || !(bytes + 1..=4).contains(&to.get())
                        || !unsigned(r, *dest, to.get())
                    {
                        return Err(MemoryReason::UnsupportedConsumer);
                    }
                    Ok(Accumulator {
                        block: block.id,
                        producer: index,
                        consumer: index + 1,
                        bytes: bytes as u8,
                    })
                })();
                let decision = match selected {
                    Ok(range) => {
                        plan.producers.insert((block.id, index), id);
                        plan.consumers.insert((block.id, index + 1), id);
                        Decision::Accumulator(range)
                    }
                    Err(reason) => Decision::Memory(reason),
                };
                plan.decisions.insert(id, decision);
            }
        }
        plan
    }

    pub fn accumulator(&self, id: TempId) -> Option<Accumulator> {
        match self.decisions.get(&id) {
            Some(Decision::Accumulator(a)) => Some(*a),
            _ => None,
        }
    }

    pub fn count(&self) -> usize {
        self.producers.len()
    }

    pub fn producer(&self, block: BlockId, index: usize) -> Option<Accumulator> {
        self.accumulator(*self.producers.get(&(block, index))?)
    }

    pub fn consumer(&self, block: BlockId, index: usize) -> Option<Accumulator> {
        self.accumulator(*self.consumers.get(&(block, index))?)
    }
}
