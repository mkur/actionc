//! Invocation-backed residence between resource barriers.
use super::*;

pub(super) fn extend(
    r: &Mir65816Routine,
    demand: &home_demand::Plan,
    resources: &resources::Plan,
    definitions: &BTreeMap<TempId, Vec<ProgramPoint>>,
    uses: &BTreeMap<TempId, Vec<ProgramPoint>>,
    counts: &BTreeMap<TempId, usize>,
    reachable: &BTreeSet<BlockId>,
    plan: &mut Plan,
) {
    if !r
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .any(|op| matches!(op, Mir65816Op::Call { .. }))
    {
        return;
    }
    let Ok((graph, facts)) = liveness::residence_facts(r) else {
        return;
    };
    for (id, ty) in &r.temps {
        if demand.omits(*id) || plan.home(*id).is_some() || ty.width != Some(ByteSize::new(3)) {
            continue;
        }
        let Some([definition]) = definitions.get(id).map(Vec::as_slice) else {
            continue;
        };
        let Some(reads) = uses.get(id) else {
            continue;
        };
        // Only values whose complete lifetime encounters a call need the split
        // strategy. Unsupported uses remain canonical stack reads as well.
        if !facts.points.get(id).is_some_and(|points| {
            points.iter().any(|p| {
                r.blocks
                    .iter()
                    .find(|b| b.id == p.block)
                    .unwrap()
                    .ops
                    .get(p.index)
                    .is_some_and(|op| matches!(op, Mir65816Op::Call { .. }))
            })
        }) {
            continue;
        }
        let mut ranges = vec![];
        for block in &r.blocks {
            if !reachable.contains(&block.id) {
                continue;
            }
            let mut start = 0;
            for end in 0..=block.ops.len() {
                let point = ProgramPoint {
                    block: block.id,
                    index: end,
                };
                let stop = block.ops.get(end).is_none_or(|op| {
                    resources.windows[&point].form == resources::Form::Barrier
                        || select::top_bits::owns_mask(block, end, counts)
                        || demand.consumer(block.id, end).is_some()
                        || operands::operation(op)
                            .inputs
                            .iter()
                            .any(|v| matches!(v, Mir65816Value::Temp(t, _) if t == id))
                            && !supported(demand, point, op, *id, 3)
                });
                if !stop {
                    continue;
                }
                let local: Vec<_> = reads
                    .iter()
                    .filter(|p| {
                        p.block == block.id
                            && p.index >= start
                            && p.index < end
                            && (definition.block != block.id
                                || definition.index == usize::MAX
                                || p.index > definition.index)
                    })
                    .collect();
                if let (Some(first), Some(last)) = (
                    local.iter().map(|p| p.index).min(),
                    local.iter().map(|p| p.index).max(),
                ) {
                    let covered = plan.values.get(id).is_some_and(|v| {
                        v.backed && v.block == block.id && first <= v.last && v.first <= last
                    });
                    // A cached pointer must pay for its complete stack reload.
                    // Count preparations, not repeated fields of one staged base.
                    let mut previous = None;
                    let mut misses = 0;
                    for (offset, op) in block.ops[first..=last].iter().enumerate() {
                        match op {
                            Mir65816Op::Load { address, width, .. }
                            | Mir65816Op::Store { address, width, .. } => {
                                if let Mir65816AddressBase::Indirect(base) = &address.base {
                                    // An already resident alternative base does
                                    // not overwrite the old PTR workspace. Its
                                    // field access cannot establish a saving.
                                    if matches!(base, Mir65816Value::Temp(t, _) if t != id &&
                                        (plan.home(*t).is_some() || plan.cache(*t, ProgramPoint {
                                            block: block.id, index: first + offset,
                                        }).is_some()))
                                    {
                                        continue;
                                    }
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
                    if !covered && misses >= 3 {
                        ranges.push(Residence {
                            block: block.id,
                            first,
                            last,
                            slot: Slot {
                                offset: 0,
                                width: 3,
                            },
                            backed: true,
                        });
                    }
                }
                start = end + 1;
            }
        }
        if ranges.is_empty() {
            continue;
        }
        let preferred = plan.values.get(id).map(|v| v.slot.offset);
        let free = preferred
            .into_iter()
            .chain(scalar::START..scalar::END)
            .find(|&offset| {
                offset % 2 == 0
                    && offset + 3 <= scalar::END
                    && plan.values.iter().all(|(other, v)| {
                        other == id
                            || !graph[id].contains(other)
                            || offset + 3 <= v.slot.offset
                            || v.slot.offset + u16::from(v.slot.width) <= offset
                    })
                    && plan.segments.iter().all(|(other, list)| {
                        other == id
                            || !graph[id].contains(other)
                            || list.iter().all(|v| {
                                offset + 3 <= v.slot.offset
                                    || v.slot.offset + u16::from(v.slot.width) <= offset
                            })
                    })
            });
        if let Some(offset) = free {
            for v in &mut ranges {
                v.slot.offset = offset;
            }
            plan.segments.insert(*id, ranges);
        }
    }
}
