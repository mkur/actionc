//! Actual native entry composition with certified machine leaves, scoped to one program.
use super::*;

pub(in crate::mir6502) struct MirAcyclicEntryAnalysis<'a> {
    program: &'a MirProgram,
    ordinary_globals_above_zp: bool,
    routines: BTreeMap<RoutineId, &'a MirRoutine>,
    entries: BTreeMap<RoutineId, Option<Arc<MirCalleeEntryFact>>>,
    visiting: BTreeSet<RoutineId>,
    failures: BTreeMap<RoutineId, (RoutineId, Option<MirSite>, &'static str)>,
}

impl<'a> MirAcyclicEntryAnalysis<'a> {
    pub fn new(program: &'a MirProgram) -> Self {
        Self {
            program,
            ordinary_globals_above_zp: false,
            routines: program.routines.iter().map(|r| (r.id, r)).collect(),
            entries: BTreeMap::new(),
            visiting: BTreeSet::new(),
            failures: BTreeMap::new(),
        }
    }

    pub fn at_origin(program: &'a MirProgram, origin: u16) -> Self {
        Self { ordinary_globals_above_zp: origin >= 0x100, ..Self::new(program) }
    }

    pub fn analyze(&mut self, target: RoutineId) -> Option<Arc<MirCalleeEntryFact>> {
        if let Some(entry) = self.entries.get(&target) { return entry.clone(); }
        if !self.visiting.insert(target) {
            self.failures.insert(target, (target, None, "recursive native call dependency"));
            return None;
        }
        let result = self.compose(target);
        if result.is_none() {
            self.failures.entry(target).or_insert((target, None, "uncertified native entry"));
        }
        self.visiting.remove(&target);
        self.entries.insert(target, result.clone());
        result
    }

    pub fn failure(&self, target: RoutineId) -> Option<(RoutineId, Option<MirSite>, &'static str)> {
        self.failures.get(&target).copied()
    }

    pub fn cached(&self, target: RoutineId) -> Option<&MirCalleeEntryFact> {
        self.entries.get(&target)?.as_deref()
    }

    fn compose(&mut self, target: RoutineId) -> Option<Arc<MirCalleeEntryFact>> {
        let routine = *self.routines.get(&target)?;
        if !matches!(routine.abi, MirRoutineAbi::Action | MirRoutineAbi::ActionObservable) {
            return None;
        }
        if let Some(site) = routine.blocks.iter().find_map(|b| b.ops.iter().position(|op|
            matches!(op, MirOp::MachineBlock { .. })).map(|op_index|
                MirSite::Op { block: b.id, op_index }))
        {
            // The leaf contributes only its matched caller obligations. Its own
            // implementation matcher certifies that the complete body is covered.
            let fact = MirCalleeEntryFact::analyze_with_dependencies(self.program, target,
                MirCalleeEntryStage::ActualEntry, Some(BTreeMap::new()), self.ordinary_globals_above_zp).map(Arc::new);
            if fact.is_none() {
                self.failures.insert(target, (target, Some(site), "uncertified machine leaf entry"));
            }
            return fact;
        }
        let mut targets = BTreeSet::new();
        for (site, op) in routine.blocks.iter().flat_map(|b| b.ops.iter().enumerate()
            .map(|(op_index, op)| (MirSite::Op { block: b.id, op_index }, op))) {
            match op {
                _ if crate::mir6502::runtime_contracts::invocation(self.program, routine, op).is_some() => {},
                MirOp::Call { target: MirCallTarget::Routine(id), args, .. } => {
                    let callee = self.routines.get(id)?;
                    if let Err(reason) = crate::mir6502::abi::canonical_call_lanes(callee, args)
                        .and_then(|_| crate::mir6502::abi::validate_direct_call_result(callee, op)) {
                        self.failures.insert(target, (target, Some(site), reason));
                        return None;
                    }
                    targets.insert(*id);
                }
                MirOp::Call { .. } | MirOp::RuntimeHelper { .. } | MirOp::MachineBlock { .. }
                    | MirOp::SArgsCapture { .. } | MirOp::Barrier { .. } => {
                        self.failures.insert(target, (target, Some(site), "unsupported native call closure operation"));
                        return None;
                    },
                _ => {},
            }
        }
        let mut dependencies = BTreeMap::new();
        for id in targets {
            let Some(fact) = self.analyze(id) else {
                if let Some(failure) = self.failure(id) { self.failures.insert(target, failure); }
                return None;
            };
            dependencies.insert(id, fact);
        }
        MirCalleeEntryFact::analyze_with_dependencies(self.program, target,
            MirCalleeEntryStage::ActualEntry, Some(dependencies), self.ordinary_globals_above_zp).map(Arc::new)
    }
}

/// Return ABI slots supplied on every reachable normal return, using the
/// caller's transfer domain (physical definitions or available ABI values).
pub(super) fn result_slots_on_all_returns(
    routine: &MirRoutine, cfg: &MirCfg,
    transfers: &BTreeMap<crate::mir6502::ir::MirBlockId, MirHomeBlockTransfers>,
) -> BTreeSet<u8> {
    let Some(result) = routine.direct_result_abi else { return BTreeSet::new(); };
    let slots = crate::mir6502::abi::direct_result_slots(result);
    if slots.is_empty() || !routine.blocks.iter().any(|b|
        cfg.reachable().contains(&b.id) && matches!(b.terminator, MirTerminator::Return))
    {
        return BTreeSet::new();
    }
    let mut definitions = transfers.clone();
    for block in &routine.blocks {
        let transfers = definitions.get_mut(&block.id).unwrap();
        for op in &mut transfers.ops { op.reads.clear(); }
        transfers.terminator.reads.clear();
        if matches!(block.terminator, MirTerminator::Return) {
            transfers.terminator.reads.extend(slots.iter().map(|s|
                MirHomeByte::FixedZeroPage(MirFixedZpSlot(*s))));
        }
    }
    let proof = MirHomeLiveness::analyze_transfers(routine, cfg, definitions);
    let Some(live) = cfg.entry().and_then(|entry| proof.live_in(entry)) else { return BTreeSet::new(); };
    slots.iter().copied().filter(|s| !live.contains(MirHomeByte::FixedZeroPage(MirFixedZpSlot(*s)))).collect()
}

#[cfg(test)]
mod tests;
