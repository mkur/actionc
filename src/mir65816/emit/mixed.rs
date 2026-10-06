//! Closed residence in the existing domain scratch pool. Captured
//! values are immutable; this never caches pointee contents or storage versions.
use super::*;
mod branches;
mod segments;
#[cfg(test)]
pub(super) mod tests;
use crate::mir65816::analysis::{ProgramPoint, operands};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Residence {
    pub block: BlockId,
    pub first: usize,
    pub last: usize,
    pub slot: Slot,
    pub backed: bool,
}
impl Residence {
    pub fn contains(self, point: ProgramPoint) -> bool {
        self.block == point.block && self.first <= point.index && point.index <= self.last
    }
}
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub(super) struct Plan {
    pub values: BTreeMap<TempId, Residence>,
    /// Complete call-free lifetimes, separate from a bounded block-local cache.
    pub regions: BTreeMap<TempId, BTreeSet<ProgramPoint>>,
    pub entries: BTreeMap<BlockId, BTreeMap<TempId, Location>>,
    pub loops: BTreeSet<TempId>,
    /// Each range is reloaded from the authoritative invocation home before
    /// its first consumer. No cached copy crosses a resource barrier or edge.
    pub segments: BTreeMap<TempId, Vec<Residence>>,
}

fn supported(
    demand: &home_demand::Plan,
    point: ProgramPoint,
    op: &Mir65816Op,
    id: TempId,
    bytes: u32,
) -> bool {
    // The existing deferred three-byte A/X producer and component-store chain
    // owns stack input bindings. Its later physical consume is not the ordinary
    // logical address occurrence, so a DP capture needs a separate contract.
    if bytes == 3
        && demand
            .producer(point.block, point.index)
            .is_some_and(|interval| interval.bytes == 3)
        && operands::operation(op)
            .inputs
            .iter()
            .any(|v| matches!(v, Mir65816Value::Temp(t, _) if *t == id))
    {
        return false;
    }
    match op {
        Mir65816Op::Load { address, .. }
        | Mir65816Op::Store { address, .. }
        | Mir65816Op::AddressOf { address, .. } => address.displacement.get() < 1 << 24,
        Mir65816Op::Copy {
            source_volatile: false,
            destination_volatile: false,
            bytes,
            ..
        } => bytes.get() < 1 << 24,
        Mir65816Op::Binary {
            width,
            operation,
            right,
            ..
        } => {
            bytes == 2
                && width.get() == 2
                && matches!(
                    operation,
                    NirBinaryOp::Add
                        | NirBinaryOp::Sub
                        | NirBinaryOp::And
                        | NirBinaryOp::Or
                        | NirBinaryOp::Xor
                )
                && matches!(
                    right,
                    Mir65816Value::U8(_)
                        | Mir65816Value::U16(_)
                        | Mir65816Value::Temp(_, _)
                        | Mir65816Value::Param(_)
                )
        }
        Mir65816Op::Compare { width, .. } => bytes == 2 && width.get() == 2,
        // Preserve the existing wide arithmetic and cast selectors. Qualified
        // complete pointer identities still use their exact private transfer.
        Mir65816Op::Cast {
            from,
            to,
            from_signed: false,
            ..
        } => from == to && from.get() == bytes,
        _ => false,
    }
}

impl Plan {
    pub fn new(r: &Mir65816Routine, demand: &home_demand::Plan) -> Self {
        if scalar::admitted(r) || AllocatedFrame::pointer_leaf(r).ok().flatten().is_some() {
            return Self::default();
        }
        let Some(entry) = r.blocks.first() else {
            return Self::default();
        };
        let indices: BTreeMap<_, _> = r
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id, i))
            .collect();
        let mut reachable = BTreeSet::new();
        let mut pending = vec![entry.id];
        while let Some(id) = pending.pop() {
            if !reachable.insert(id) {
                continue;
            }
            let Some(&index) = indices.get(&id) else {
                return Self::default();
            };
            let Ok(successors) = operands::successors(r, index) else {
                return Self::default();
            };
            pending.extend(successors);
        }
        let resources = resources::Plan::new(r);
        let mut defs = BTreeMap::<TempId, Vec<ProgramPoint>>::new();
        let mut uses = BTreeMap::<TempId, Vec<ProgramPoint>>::new();
        let mut address_uses = BTreeMap::<TempId, BTreeSet<ProgramPoint>>::new();
        for block in &r.blocks {
            for (id, _) in &block.params {
                defs.entry(*id).or_default().push(ProgramPoint {
                    block: block.id,
                    index: usize::MAX,
                });
            }
            for index in 0..=block.ops.len() {
                let point = ProgramPoint {
                    block: block.id,
                    index,
                };
                let census = block
                    .ops
                    .get(index)
                    .map(operands::operation)
                    .unwrap_or_else(|| operands::terminator(&block.terminator));
                if let Some((id, _)) = census.definition {
                    defs.entry(id).or_default().push(point);
                }
                for input in census.inputs {
                    if let Mir65816Value::Temp(id, _) = input {
                        uses.entry(*id).or_default().push(point);
                    }
                }
                let addresses: Vec<_> = match block.ops.get(index) {
                    Some(
                        Mir65816Op::Load { address, .. }
                        | Mir65816Op::Store { address, .. }
                        | Mir65816Op::AddressOf { address, .. },
                    ) => vec![address],
                    Some(Mir65816Op::Copy {
                        destination,
                        source,
                        ..
                    }) => vec![destination, source],
                    _ => vec![],
                };
                for address in addresses {
                    if let Mir65816AddressBase::Indirect(Mir65816Value::Temp(id, _)) = &address.base
                    {
                        address_uses.entry(*id).or_default().insert(point);
                    }
                }
            }
        }
        let counts = uses.iter().map(|(&id, reads)| (id, reads.len())).collect();
        let mut candidates = vec![];
        for (id, ty) in &r.temps {
            if demand.omits(*id) {
                continue;
            }
            let Some(bytes) = ty.width.map(ByteSize::get).filter(|w| matches!(w, 2 | 3)) else {
                continue;
            };
            let Some([definition]) = defs.get(id).map(Vec::as_slice) else {
                continue;
            };
            let Some(reads) = uses.get(id).filter(|u| !u.is_empty()) else {
                continue;
            };
            let block = r.blocks.iter().find(|b| b.id == definition.block).unwrap();
            if !reachable.contains(&block.id) {
                continue;
            }
            let Some(producer) = block.ops.get(definition.index) else {
                continue;
            };
            if matches!(producer, Mir65816Op::Compare { .. })
                || !supported(demand, *definition, producer, *id, bytes)
                || select::top_bits::owns_mask(block, definition.index, &counts)
                || resources.windows[definition].form == resources::Form::Barrier
                // A widening consumer owns a stack result in the existing plan.
                || demand.consumer(block.id, definition.index).is_some()
            {
                continue;
            }
            let end = (definition.index + 1..block.ops.len())
                .find(|&i| {
                    resources.windows[&ProgramPoint {
                        block: block.id,
                        index: i,
                    }]
                        .form
                        == resources::Form::Barrier
                        // This selector owns the complete mask/compare region
                        // and currently requires its original stack captures.
                        || select::top_bits::owns_mask(block, i, &counts)
                        || reads.iter().any(|p| p.block == block.id && p.index == i)
                            && !supported(demand, ProgramPoint { block: block.id, index: i }, &block.ops[i], *id, bytes)
                })
                .unwrap_or(block.ops.len());
            let local: Vec<_> = reads
                .iter()
                .filter(|p| p.block == block.id && definition.index < p.index && p.index < end)
                .collect();
            let Some(last) = local.iter().map(|p| p.index).max() else {
                continue;
            };
            let backed = local.len() != reads.len();
            let addresses = address_uses.get(id).map_or(0, |points| {
                points
                    .iter()
                    .filter(|p| {
                        p.block == block.id && definition.index < p.index && p.index <= last
                    })
                    .count()
            });
            // Count conservative address-cache misses, not repeated uses of
            // the same prepared base. A stack-backed cache must pay for its copy.
            let mut previous = None;
            let mut misses = 0;
            for op in &block.ops[definition.index + 1..=last] {
                match op {
                    Mir65816Op::Load { address, width, .. }
                    | Mir65816Op::Store { address, width, .. } => {
                        if let Mir65816AddressBase::Indirect(base) = &address.base {
                            if matches!(base, Mir65816Value::Temp(t, _) if t == id)
                                && previous != Some(base)
                                && select::addresses::direct_resident_base(
                                    r,
                                    address,
                                    width.get() as u8,
                                )
                            {
                                misses += 1;
                            }
                            previous = Some(base);
                        } else if matches!(op, Mir65816Op::Store { .. })
                            && !matches!(
                                address.base,
                                Mir65816AddressBase::AutomaticFrame(_)
                                    | Mir65816AddressBase::Parameter(_)
                            )
                        {
                            previous = None;
                        }
                    }
                    Mir65816Op::AddressOf { .. }
                    | Mir65816Op::PointerOffset { .. }
                    | Mir65816Op::Copy { .. } => previous = None,
                    _ => (),
                }
            }
            // Size first: a stack-backed pointer cache pays one complete private
            // transfer; at least three proved preparations cover it. Pure
            // DP words retain equal-size native instructions and can shrink frames.
            if (bytes == 3 && addresses == 0) || (backed && (bytes != 3 || misses < 3)) {
                continue;
            }
            candidates.push((
                *id,
                Residence {
                    block: block.id,
                    first: definition.index,
                    last,
                    slot: Slot {
                        offset: 0,
                        width: bytes as u8,
                    },
                    backed,
                },
            ));
        }
        candidates.sort_by_key(|(id, v)| (v.block, v.first, *id));
        let mut plan = Self::default();
        for (id, mut residence) in candidates {
            let free = (scalar::START..scalar::END).find(|&offset| {
                offset % 2 == 0
                    && offset + u16::from(residence.slot.width) <= scalar::END
                    && plan.values.values().all(|v| {
                        v.block != residence.block
                            || v.last < residence.first
                            || residence.last < v.first
                            || offset + u16::from(residence.slot.width) <= v.slot.offset
                            || v.slot.offset + u16::from(v.slot.width) <= offset
                    })
            });
            if let Some(offset) = free {
                residence.slot.offset = offset;
                plan.values.insert(id, residence);
            }
            // Pressure refuses this interval only. The ordinary stack capture
            // remains authoritative; other disjoint intervals remain available.
        }
        if !plan.values.is_empty() {
            let ordinary = Self::default();
            let before = AllocatedFrame::layout_for_residence(r, demand, &ordinary);
            let after = AllocatedFrame::layout_for_residence(r, demand, &plan);
            // Include edge staging and incoming geometry: greedy recoloring can
            // create a copy cycle even when fewer values need stack storage.
            match (before, after) {
                (_, Err(_)) => return ordinary,
                (Ok(before), Ok(after))
                    if after.extent > before.extent
                        || after.peak_below_entry > before.peak_below_entry =>
                {
                    return ordinary;
                }
                _ => (),
            }
        }
        branches::extend(
            r, demand, &resources, &defs, &uses, &counts, &reachable, &mut plan,
        );
        segments::extend(
            r, demand, &resources, &defs, &uses, &counts, &reachable, &mut plan,
        );
        plan
    }
    pub fn home(&self, id: TempId) -> Option<Location> {
        self.values
            .get(&id)
            .filter(|v| !v.backed)
            .map(|v| Location::DirectPage(v.slot))
    }
    pub fn cache(&self, id: TempId, point: ProgramPoint) -> Option<Slot> {
        if let Some(v) = self
            .segments
            .get(&id)
            .into_iter()
            .flatten()
            .find(|v| v.contains(point))
        {
            return Some(v.slot);
        }
        self.values
            .get(&id)
            .filter(|v| v.backed && v.contains(point) && point.index > v.first)
            .map(|v| v.slot)
    }
}
