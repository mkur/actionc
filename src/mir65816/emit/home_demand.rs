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
    // One/two bytes use A8/A16. Three/four bytes use native A/X result
    // lanes and may only flow through identity casts into a return.
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

fn byte_value(r: &Mir65816Routine, value: &Mir65816Value) -> bool {
    match value {
        Mir65816Value::U8(_) => true,
        Mir65816Value::Temp(id, width) => width.get() == 1 && unsigned(r, *id, 1),
        Mir65816Value::Param(id) => r.frame.parameters.iter().any(|p| {
            p.param == *id && matches!(p.incoming, Mir65816AbiHome::StackArgument { size, .. } if size.get() == 1)
        }),
        _ => false,
    }
}

fn wide_value(r: &Mir65816Routine, value: &Mir65816Value, bytes: u32) -> bool {
    match value {
        Mir65816Value::U8(_) | Mir65816Value::U16(_) | Mir65816Value::U24(_) | Mir65816Value::U32(_) => true,
        Mir65816Value::Temp(id, width) => width.get() == bytes && unsigned(r, *id, bytes),
        Mir65816Value::Param(id) => r.frame.parameters.iter().any(|p| p.param == *id && matches!(p.incoming, Mir65816AbiHome::StackArgument { size, .. } if size.get() == bytes)),
        _ => false,
    }
}

fn frame_store(r: &Mir65816Routine, address: &Mir65816Address, bytes: u32) -> bool {
    if address.index.is_some() {
        return false;
    }
    let Some(end) = address.displacement.get().checked_add(bytes) else {
        return false;
    };
    match address.base {
        Mir65816AddressBase::AutomaticFrame(id) => r.frame.objects.iter().any(|o| o.id == id && o.mutable && end <= o.size.get()),
        Mir65816AddressBase::Parameter(id) => r.frame.parameters.iter().any(|p| p.param == id && p.frame_object.is_some() && matches!(p.incoming, Mir65816AbiHome::StackArgument { size, .. } if end <= size.get())),
        _ => false,
    }
}

pub(super) fn result_home(bytes: u32) -> Option<Mir65816AbiHome> {
    Some(Mir65816AbiHome::NativeResult(match bytes {
        1 => abi::ResultLocation::A8ZeroExtended,
        2 => abi::ResultLocation::A16,
        3 => abi::ResultLocation::A16X8ZeroExtended,
        4 => abi::ResultLocation::A16X16,
        _ => return None,
    }))
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
        // The closed scalar-DP/X profile keeps ownership of its complete
        // home classes and loop-carried values. Do not mix allocators here.
        if scalar::admitted(r) {
            return plan;
        }
        for block in &r.blocks {
            // Start at a supported terminal consumer and extend its selected
            // suffix backward. Intermediate arithmetic must already produce
            // into registers; no value crosses an unrelated operation.
            for (index, op) in block.ops.iter().enumerate().rev() {
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
                    if select::top_bits::owns_mask(block, index, &counts) {
                        return Err(MemoryReason::UnsupportedConsumer);
                    }
                    let bytes = match op {
                        Mir65816Op::Load {
                            width,
                            address,
                            volatile,
                            ..
                        } => {
                            if !matches!(width.get(), 1 | 2) {
                                return Err(MemoryReason::UnsupportedWidth);
                            }
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
                        } if matches!(width.get(), 1 | 2)
                            && (matches!(
                                operation,
                                NirBinaryOp::Add
                                    | NirBinaryOp::Sub
                                    | NirBinaryOp::And
                                    | NirBinaryOp::Or
                                    | NirBinaryOp::Xor
                            ) && if width.get() == 2 {
                                word_value(r, right)
                            } else {
                                byte_value(r, right)
                            } || small_shift(*operation, right).is_some())
                            && if width.get() == 2 {
                                word_value(r, left)
                            } else {
                                byte_value(r, left)
                            } =>
                        {
                            width.get()
                        }
                        Mir65816Op::Binary {
                            width,
                            signed: false,
                            operation,
                            left,
                            right,
                            ..
                        } if matches!(width.get(), 3 | 4)
                            && matches!(
                                operation,
                                NirBinaryOp::Add
                                    | NirBinaryOp::Sub
                                    | NirBinaryOp::And
                                    | NirBinaryOp::Or
                                    | NirBinaryOp::Xor
                            )
                            && wide_value(r, left, width.get())
                            && wide_value(r, right, width.get()) =>
                        {
                            width.get()
                        }
                        Mir65816Op::Cast {
                            from,
                            to,
                            from_signed: false,
                            kind: NirCastKind::Integer,
                            value,
                            ..
                        } if matches!(to.get(), 3 | 4)
                            && ((matches!(from.get(), 1 | 2)
                                && from.get() < to.get()
                                && if from.get() == 1 {
                                    byte_value(r, value)
                                } else {
                                    word_value(r, value)
                                })
                                || (from == to && wide_value(r, value, from.get()))) =>
                        {
                            to.get()
                        }
                        _ => return Err(MemoryReason::UnsupportedProducer),
                    };
                    if !matches!(bytes, 1..=4) || !unsigned(r, id, bytes) {
                        return Err(MemoryReason::UnsupportedWidth);
                    }
                    if bytes > 2
                        && r.temps.iter().any(|(temp, ty)| {
                            *temp == id
                                && ty
                                    .kind
                                    .integer()
                                    .is_some_and(|i| i.role == crate::nir::NirIntegerRole::Address)
                        })
                    {
                        return Err(MemoryReason::UnsupportedWidth);
                    }
                    let consumes_a = match block.ops.get(index + 1) {
                        Some(Mir65816Op::Cast {
                            dest,
                            from,
                            to,
                            from_signed: false,
                            kind: NirCastKind::Integer,
                            value: Mir65816Value::Temp(input, actual),
                        }) => {
                            *input == id
                                && actual == from
                                && from.get() == bytes
                                && ((bytes <= 2 && (bytes + 1..=4).contains(&to.get()))
                                    || (bytes >= 3
                                        && to == from
                                        && plan.producer(block.id, index + 1).is_some()))
                                && unsigned(r, *dest, to.get())
                        }
                        Some(Mir65816Op::Binary {
                            left: Mir65816Value::Temp(input, actual),
                            ..
                        }) => {
                            bytes <= 2
                                && *input == id
                                && actual.get() == bytes
                                && plan.producer(block.id, index + 1).is_some()
                        }
                        Some(Mir65816Op::Store {
                            address,
                            value: Mir65816Value::Temp(input, actual),
                            width,
                            volatile: false,
                        }) => {
                            bytes <= 2
                                && *input == id
                                && actual == width
                                && width.get() == bytes
                                && frame_store(r, address, bytes)
                        }
                        Some(Mir65816Op::Compare {
                            left: Mir65816Value::Temp(input, actual),
                            right,
                            width,
                            signed: false,
                            operation,
                            ..
                        }) => {
                            bytes <= 2
                                && *input == id
                                && actual == width
                                && width.get() == bytes
                                && matches!(
                                    operation,
                                    NirCompareOp::Eq
                                        | NirCompareOp::Ne
                                        | NirCompareOp::Lt
                                        | NirCompareOp::Ge
                                )
                                && if bytes == 2 {
                                    word_value(r, right)
                                } else {
                                    byte_value(r, right)
                                }
                        }
                        Some(Mir65816Op::Call {
                            args,
                            target,
                            plan: call,
                            ..
                        }) => {
                            bytes <= 2
                                && matches!(
                                    target,
                                    Mir65816CallTarget::Direct(_)
                                        | Mir65816CallTarget::Helper(_)
                                        | Mir65816CallTarget::Runtime(_)
                                )
                                && matches!(args.as_slice(), [Mir65816Value::Temp(input, actual)] if *input == id && actual.get() == bytes)
                                && matches!(call.arguments.as_slice(), [Mir65816AbiHome::StackArgument { offset, size, .. }] if offset.get() == 0 && size.get() == bytes)
                                && call.outgoing_bytes.get() == (bytes | 1)
                        }
                        None => matches!(&block.terminator,
                            Mir65816Terminator::Return { value: Some(Mir65816Value::Temp(input, actual)), .. }
                            if *input == id && actual.get() == bytes && r.result_home == result_home(bytes)),
                        _ => false,
                    };
                    if !consumes_a {
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

    pub(super) fn call_returns(
        &self,
        r: &Mir65816Routine,
        block: &Mir65816Block,
        index: usize,
    ) -> bool {
        if index + 1 != block.ops.len() || self.consumer(block.id, index).is_none() {
            return false;
        }
        matches!((&block.ops[index], &block.terminator),
            (Mir65816Op::Call { result: Some((dest, bytes)), plan, .. },
             Mir65816Terminator::Return { value: Some(Mir65816Value::Temp(id, width)), .. })
             if dest == id && bytes == width && plan.result == r.result_home
                && liveness::input_counts(r).get(dest) == Some(&1))
    }
}
