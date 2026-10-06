//! Complete captures. Every incoming edge establishes block parameters;
//! a surviving identity is retained only through its whole closed live region.
use super::*;

fn cyclic_core(r: &Mir65816Routine, reachable: &BTreeSet<BlockId>) -> BTreeSet<BlockId> {
    let mut next = BTreeMap::<BlockId, BTreeSet<BlockId>>::new();
    let mut previous = BTreeMap::<BlockId, BTreeSet<BlockId>>::new();
    for (i, block) in r.blocks.iter().enumerate() {
        if !reachable.contains(&block.id) {
            continue;
        }
        let successors: BTreeSet<_> = operands::successors(r, i).unwrap().into_iter().collect();
        for &target in &successors {
            previous.entry(target).or_default().insert(block.id);
        }
        next.insert(block.id, successors);
    }
    let mut core = reachable.clone();
    let mut pending: Vec<_> = core
        .iter()
        .copied()
        .filter(|b| next[b].is_empty() || previous.get(b).is_none_or(BTreeSet::is_empty))
        .collect();
    while let Some(id) = pending.pop() {
        if !core.remove(&id) {
            continue;
        }
        for target in &next[&id] {
            if let Some(preds) = previous.get_mut(target) {
                preds.remove(&id);
                if preds.is_empty() {
                    pending.push(*target);
                }
            }
        }
        for pred in previous.get(&id).into_iter().flatten() {
            if let Some(succs) = next.get_mut(pred) {
                succs.remove(&id);
                if succs.is_empty() {
                    pending.push(*pred);
                }
            }
        }
    }
    // Conservatively retain paths between cycles too. The closed fixed-point
    // lifetime proof applies to this whole region, not lexical block order.
    core
}

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
    if r.blocks.len() < 2 {
        return;
    }
    let Ok((graph, facts)) = liveness::residence_facts(r) else {
        return;
    };
    let core = cyclic_core(r, reachable);
    extend_regions(
        r,
        demand,
        resources,
        definitions,
        uses,
        counts,
        reachable,
        plan,
        &graph,
        &facts,
        &core,
        false,
    );
    if core.is_empty() {
        return;
    }
    extend_regions(
        r,
        demand,
        resources,
        definitions,
        uses,
        counts,
        reachable,
        plan,
        &graph,
        &facts,
        &core,
        true,
    );
}

fn extend_regions(
    r: &Mir65816Routine,
    demand: &home_demand::Plan,
    resources: &resources::Plan,
    definitions: &BTreeMap<TempId, Vec<ProgramPoint>>,
    uses: &BTreeMap<TempId, Vec<ProgramPoint>>,
    counts: &BTreeMap<TempId, usize>,
    reachable: &BTreeSet<BlockId>,
    plan: &mut Plan,
    graph: &liveness::Interference,
    facts: &liveness::ResidenceFacts,
    core: &BTreeSet<BlockId>,
    loops: bool,
) {
    let blocks: BTreeMap<_, _> = r.blocks.iter().map(|b| (b.id, b)).collect();
    let before = plan.clone();
    for (id, ty) in &r.temps {
        if demand.omits(*id) || plan.home(*id).is_some() {
            continue;
        }
        let Some(bytes) = ty.width.map(ByteSize::get).filter(|w| matches!(w, 2 | 3)) else {
            continue;
        };
        let Some([definition]) = definitions.get(id).map(Vec::as_slice) else {
            continue;
        };
        let Some(reads) = uses.get(id).filter(|u| !u.is_empty()) else {
            continue;
        };
        let block = blocks[&definition.block];
        let parameter = definition.index == usize::MAX;
        // Do not move an edge-only preheader capture away from its stack
        // affinity with a loop-carried parameter. The target cannot retain DP
        // residence in this stage, so this would add a whole-pointer copy (and
        // its A-save) without a resident operation consumer.
        if !loops
            && !parameter
            && reads
                .iter()
                .all(|p| p.block == block.id && p.index == block.ops.len())
            && operands::edges(&block.terminator).iter().any(|e| {
                core.contains(&e.target)
                    && e.args
                        .iter()
                        .any(|v| matches!(v, Mir65816Value::Temp(t, _) if t == id))
            })
        {
            continue;
        }
        if !parameter
            && !reads.iter().any(|p| {
                p.block != block.id
                    || p.index == block.ops.len() && !operands::edges(&block.terminator).is_empty()
            })
        {
            continue;
        }
        if !parameter {
            let Some(producer) = block.ops.get(definition.index) else {
                continue;
            };
            if matches!(producer, Mir65816Op::Compare { .. })
                || !supported(demand, *definition, producer, *id, bytes)
                || demand.consumer(block.id, definition.index).is_some()
            {
                continue;
            }
        }
        let Some(points) = facts.points.get(id) else {
            continue;
        };
        if points.iter().any(|p| {
            !reachable.contains(&p.block)
                || !loops && core.contains(&p.block)
                || blocks[&p.block].ops.get(p.index).is_some_and(|op| {
                    resources.windows[p].form == resources::Form::Barrier
                        || select::top_bits::owns_mask(blocks[&p.block], p.index, counts)
                        || (operands::operation(op)
                            .inputs
                            .iter()
                            .any(|v| matches!(v, Mir65816Value::Temp(t, _) if t == id))
                            && !supported(demand, *p, op, *id, bytes))
                })
        }) {
            continue;
        }
        // Closed CFG interference includes every simultaneous parameter write,
        // every operation output and all logical occurrences, even dead edges.
        let offset = (scalar::START..scalar::END).find(|&offset| {
            offset % 2 == 0
                && offset + bytes as u16 <= scalar::END
                && plan.values.iter().all(|(other, v)| {
                    other == id
                        || !graph[id].contains(other)
                        || offset + bytes as u16 <= v.slot.offset
                        || v.slot.offset + u16::from(v.slot.width) <= offset
                })
        });
        let Some(offset) = offset else {
            continue;
        };
        plan.values.insert(
            *id,
            Residence {
                block: definition.block,
                first: definition.index,
                last: definition.index,
                slot: Slot {
                    offset,
                    width: bytes as u8,
                },
                backed: false,
            },
        );
        plan.regions.insert(*id, points.clone());
        if points.iter().any(|p| core.contains(&p.block)) {
            plan.loops.insert(*id);
        }
    }
    if loops {
        // Edge-only preheader captures are useful only when every cyclic
        // destination retains a complete DP home too. Otherwise keep the
        // previous stack affinity rather than adding a DP-to-stack transfer.
        let refused: Vec<_> = plan
            .regions
            .keys()
            .copied()
            .filter(|id| {
                if before.regions.contains_key(id) {
                    return false;
                }
                let Some([definition]) = definitions.get(id).map(Vec::as_slice) else {
                    return false;
                };
                let block = blocks[&definition.block];
                definition.index != usize::MAX
                    && uses[id]
                        .iter()
                        .all(|p| p.block == block.id && p.index == block.ops.len())
                    && operands::edges(&block.terminator).iter().any(|e| {
                        core.contains(&e.target)
                            && e.args.iter().enumerate().any(|(i, v)| {
                                matches!(v, Mir65816Value::Temp(t, _) if t == id)
                                    && plan.home(blocks[&e.target].params[i].0).is_none()
                            })
                    })
            })
            .collect();
        for id in refused {
            plan.regions.remove(&id);
            plan.loops.remove(&id);
            plan.values.remove(&id);
            if let Some(v) = before.values.get(&id) {
                plan.values.insert(id, *v);
            }
        }
    }
    if plan.regions.is_empty() || loops && *plan == before {
        return;
    }
    // Parallel-copy demand is part of the trial. A residence admission must not
    // turn a previously direct edge into a larger fixed frame or local peak.
    let previous = AllocatedFrame::layout_for_residence(r, demand, &before);
    let candidate = AllocatedFrame::layout_for_residence(r, demand, plan);
    if !matches!((previous, candidate), (Ok(a), Ok(b)) if b.extent <= a.extent && b.peak_below_entry <= a.peak_below_entry
        && (!loops || no_transfer_growth(r, &a, &b, &before).unwrap_or(false)))
    {
        *plan = before;
        return;
    }
    plan.entries.clear();
    for (&block, live) in &facts.entries {
        let homes: BTreeMap<_, _> = live
            .iter()
            .copied()
            .filter_map(|id| plan.home(id).map(|h| (id, h)))
            .collect();
        if !homes.is_empty() {
            plan.entries.insert(block, homes);
        }
    }
}

/// Loop placement budgets edge-copy bytes against proved address savings.
/// Word and whole-pointer plans
/// have exact costs; the legacy byte fallback uses a conservative lower bound
/// when its optional word expansion is available. Mode and final A/NZ repairs
/// count too. This deliberately leaves speculative hot-loop tradeoffs out.
fn no_transfer_growth(
    r: &Mir65816Routine,
    before: &AllocatedFrame,
    after: &AllocatedFrame,
    previous: &Plan,
) -> Result<bool, String> {
    let cost = |frame: &AllocatedFrame, edge: &Mir65816Edge| -> Result<usize, String> {
        if let Some(p) = frame.pointer_copies(r, edge, 0)? {
            let steps = frame
                .pointer_staging(&p, 0)?
                .map_or(0, |s| p.scheduled(s).count());
            return Ok(6 + usize::from(p.preserve_a()) * 4 + steps * 8);
        }
        if let Some(p) = frame.word_copies(r, edge, 0)? {
            return Ok(p.cost().0);
        }
        if let Some(p) = frame.mixed_copies(r, edge)? {
            let bytes: usize = p
                .steps
                .iter()
                .map(|s| match s {
                    mixed_copies::Step::Capture { source, .. } => usize::from(source.slot().width),
                    mixed_copies::Step::Move(m) => usize::from(m.bytes),
                })
                .sum();
            return Ok(4 + bytes * 4 + usize::from(!p.moves.is_empty()) * 2);
        }
        let widths = frame.edge_widths(r, edge)?;
        if widths.is_empty() {
            return Ok(0);
        }
        let block = r.blocks.iter().find(|b| b.id == edge.target).unwrap();
        let identity: Vec<_> = edge.args.iter().zip(&block.params).map(|(v, (id, _))|
            matches!(v, Mir65816Value::Temp(t, _) if frame.temps[t] == frame.temps[id])).collect();
        let copied: usize = widths
            .iter()
            .zip(&identity)
            .filter(|(_, same)| !**same)
            .map(|(&w, _)| usize::from(w))
            .sum();
        // A three-byte move disables the legacy fallback's wide strategy.
        let pairs = if widths.contains(&3) { 8 } else { 4 };
        Ok(4 + copied * pairs + usize::from(identity.last() == Some(&true)) * 2)
    };
    let mut credit = 0;
    for block in &r.blocks {
        let mut prepared = None;
        for (index, op) in block.ops.iter().enumerate() {
            let point = ProgramPoint {
                block: block.id,
                index,
            };
            match op {
                Mir65816Op::Load { address, width, .. }
                | Mir65816Op::Store { address, width, .. } => {
                    if let Mir65816AddressBase::Indirect(base) = &address.base {
                        let old_dp = matches!(base, Mir65816Value::Temp(id, _) if
                            matches!(before.temps.get(id), Some(Location::DirectPage(_))) || previous.cache(*id, point).is_some());
                        if old_dp {
                            continue;
                        }
                        if prepared != Some(base)
                            && select::addresses::direct_resident_base(
                                r,
                                address,
                                width.get() as u8,
                            )
                            && matches!(base, Mir65816Value::Temp(id, _) if
                            matches!(after.temps.get(id), Some(Location::DirectPage(_))))
                        {
                            // Two exact private word loads/stores prepare a
                            // stack pointer. Direct DP addressing omits them.
                            credit += 8;
                        }
                        prepared = Some(base);
                    }
                }
                // Assume other forms retain the workspace; this can only
                // undercount the stack preparations removed by residence.
                _ => (),
            }
        }
    }
    let mut old = 0;
    let mut new = 0;
    for block in &r.blocks {
        for edge in operands::edges(&block.terminator) {
            old += cost(before, edge)?;
            new += cost(after, edge)?;
        }
    }
    Ok(new <= old + credit)
}
