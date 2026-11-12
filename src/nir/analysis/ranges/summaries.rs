//! Normal-return summaries. Failure-only effects are excluded from a caller's
//! continuation proof; their executable operations are never removed.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ScopedStorage {
    owner: Option<RoutineId>,
    id: NirStorageId,
}
impl ScopedStorage {
    fn new(owner: RoutineId, id: NirStorageId) -> Self {
        Self {
            owner: (!matches!(id, NirStorageId::Global(_))).then_some(owner),
            id,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ReturnSummary {
    pub range: Option<NirUnsignedRange>,
    writes: Option<BTreeSet<ScopedStorage>>,
    pub height: usize,
    opaque: bool,
}
impl ReturnSummary {
    pub fn apply_writes(&self, owner: RoutineId, state: &mut State) {
        if let Some(writes) = &self.writes {
            state
                .cells
                .retain(|id, _| !writes.contains(&ScopedStorage::new(owner, *id)));
        } else {
            state.cells.clear();
        }
    }
}

pub(super) fn collect(
    routine: &NirRoutine,
    facts: &RoutineFacts,
    children: &BTreeMap<RoutineId, ReturnSummary>,
    non_returning: &BTreeSet<RoutineId>,
    height: usize,
) -> ReturnSummary {
    let cfg = NirCfg::from_routine(routine);
    // Only blocks that can reach a normal return with executable output may
    // affect the resumed caller. A sink block has input but no normal output.
    let mut returning = BTreeSet::new();
    let mut todo = routine
        .blocks
        .iter()
        .filter(|b| {
            facts.outputs.contains_key(&b.id) && matches!(b.terminator, NirTerminator::Return(_))
        })
        .map(|b| b.id)
        .collect::<Vec<_>>();
    while let Some(id) = todo.pop() {
        if !returning.insert(id) {
            continue;
        }
        todo.extend(
            cfg.predecessors(id)
                .iter()
                .copied()
                .filter(|p| facts.outputs.contains_key(p)),
        );
    }
    let mut writes = Some(BTreeSet::new());
    let mut opaque = false;
    let mut range: Option<NirUnsignedRange> = None;
    let mut range_complete = true;
    let mut any_return = false;
    let result_bits = routine.signature.result.as_ref().and_then(type_bits);
    for block in &routine.blocks {
        // Opaque code can return early, before the source CFG's terminator.
        // It cannot be hidden by a later apparent source-level failure sink.
        if facts.inputs.contains_key(&block.id)
            && block
                .ops
                .iter()
                .any(|op| matches!(op, NirOp::ForeignCode { .. } | NirOp::Unsupported { .. }))
        {
            opaque = true;
            writes = None;
        }
        if !returning.contains(&block.id) {
            continue;
        }
        if let NirTerminator::Return(value) = &block.terminator {
            any_return = true;
            let value = value
                .as_ref()
                .and_then(|v| value_fact(routine, &facts.outputs[&block.id], v));
            match (value, result_bits) {
                (Some(mut f), Some(bits)) if f.range.max <= NirUnsignedRange::top(bits).max => {
                    f.range.bits = bits;
                    range = Some(range.map_or(f.range, |old| old.hull(f.range)));
                }
                _ => range_complete = false,
            }
        }
        for op in &block.ops {
            match op {
                NirOp::Store { place, .. } => {
                    if let Some(id) = direct_storage_id(place) {
                        if let Some(writes) = &mut writes {
                            writes.insert(ScopedStorage::new(routine.id, id));
                        }
                    } else {
                        writes = None;
                        opaque = true;
                    }
                }
                NirOp::Call {
                    callee: NirCallee::User { id, .. },
                    ..
                } if !non_returning.contains(id) => {
                    if let Some(summary) = children.get(id) {
                        opaque |= summary.opaque;
                        match (&mut writes, &summary.writes) {
                            (Some(writes), Some(other)) => writes.extend(other),
                            _ => writes = None,
                        }
                    } else {
                        writes = None;
                        opaque = true;
                    }
                }
                NirOp::Call { .. }
                | NirOp::VolatileStore { .. }
                | NirOp::CopyBytes { .. }
                | NirOp::Real(_) => {
                    writes = None;
                    opaque = true;
                }
                _ if crate::nir::optimizer::binary_may_fault(op) => {
                    writes = None;
                    opaque = true;
                }
                _ => {}
            }
        }
    }
    // The ABI writes parameter homes even when the source body never assigns
    // them. IDs are scoped by owner, independently of repeated p0/l0 spellings.
    if let Some(writes) = &mut writes {
        writes.extend(
            routine
                .params
                .iter()
                .map(|p| ScopedStorage::new(routine.id, NirStorageId::Param(p.id))),
        );
    }
    ReturnSummary {
        range: (any_return && range_complete && !opaque)
            .then_some(range)
            .flatten(),
        writes,
        height,
        opaque,
    }
}

pub(super) struct Resolver<'a> {
    pub program: &'a NirProgram,
    pub non_returning: BTreeSet<RoutineId>,
    pub trackable: BTreeMap<RoutineId, BTreeMap<NirStorageId, u8>>,
    pub routines: BTreeMap<RoutineId, RoutineFacts>,
    pub summaries: BTreeMap<RoutineId, ReturnSummary>,
    active: BTreeSet<RoutineId>,
    done: BTreeSet<RoutineId>,
}
impl<'a> Resolver<'a> {
    pub fn new(program: &'a NirProgram) -> Self {
        let storage = analyze_program_storage(program);
        let trackable = program
            .routines
            .iter()
            .zip(&storage.routines)
            .map(|(r, s)| {
                let homes = s
                    .homes
                    .iter()
                    .filter_map(|(id, f)| {
                        f.is_value_trackable()
                            .then(|| Some((*id, type_bits(f.ty.as_ref()?)?)))
                            .flatten()
                    })
                    .collect();
                (r.id, homes)
            })
            .collect();
        Self {
            program,
            non_returning: non_returning_routines(program),
            trackable,
            routines: BTreeMap::new(),
            summaries: BTreeMap::new(),
            active: BTreeSet::new(),
            done: BTreeSet::new(),
        }
    }
    pub fn resolve(&mut self, id: RoutineId) -> Option<usize> {
        if self.done.contains(&id) {
            return self.summaries.get(&id).map(|s| s.height);
        }
        if self.active.len() >= 16 || !self.active.insert(id) {
            return None;
        }
        let program = self.program;
        let Some(routine) = program.routines.iter().find(|r| r.id == id) else {
            self.active.remove(&id);
            return None;
        };
        let mut height = 1;
        let mut closed = !routine.entry.external
            && routine.entry.external_symbol.is_none()
            && !matches!(routine.entry.placement, NirRoutinePlacement::Absolute(_));
        for op in routine.blocks.iter().flat_map(|b| &b.ops) {
            if let NirOp::Call { callee, .. } = op {
                match callee {
                    NirCallee::User { id, .. } => {
                        if let Some(child) = self.resolve(*id) {
                            height = height.max(child + 1);
                        } else {
                            closed = false;
                        }
                    }
                    NirCallee::Fault(_) => {}
                    _ => closed = false,
                }
            }
        }
        if let Some(facts) = analyze_routine(
            routine,
            &self.trackable[&id],
            &self.non_returning,
            &self.summaries,
        ) {
            if closed && height <= 16 {
                let summary = collect(
                    routine,
                    &facts,
                    &self.summaries,
                    &self.non_returning,
                    height,
                );
                self.summaries.insert(id, summary);
            }
            self.routines.insert(id, facts);
        }
        self.active.remove(&id);
        self.done.insert(id);
        self.summaries.get(&id).map(|s| s.height)
    }
}
