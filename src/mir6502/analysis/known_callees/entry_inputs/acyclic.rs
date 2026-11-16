//! Actual native entry composition, scoped to one immutable program.
use super::*;

pub(in crate::mir6502) struct MirAcyclicEntryAnalysis<'a> {
    program: &'a MirProgram,
    routines: BTreeMap<RoutineId, &'a MirRoutine>,
    entries: BTreeMap<RoutineId, Option<Arc<MirCalleeEntryFact>>>,
    visiting: BTreeSet<RoutineId>,
}

impl<'a> MirAcyclicEntryAnalysis<'a> {
    pub fn new(program: &'a MirProgram) -> Self {
        Self {
            program,
            routines: program.routines.iter().map(|r| (r.id, r)).collect(),
            entries: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    pub fn analyze(&mut self, target: RoutineId) -> Option<Arc<MirCalleeEntryFact>> {
        if let Some(entry) = self.entries.get(&target) { return entry.clone(); }
        if !self.visiting.insert(target) { return None; }
        let result = self.compose(target);
        self.visiting.remove(&target);
        self.entries.insert(target, result.clone());
        result
    }

    fn compose(&mut self, target: RoutineId) -> Option<Arc<MirCalleeEntryFact>> {
        let routine = *self.routines.get(&target)?;
        if !matches!(routine.abi, MirRoutineAbi::Action | MirRoutineAbi::ActionObservable) {
            return None;
        }
        let mut targets = BTreeSet::new();
        for op in routine.blocks.iter().flat_map(|b| &b.ops) {
            match op {
                MirOp::Call { target: MirCallTarget::Routine(id), args, .. } => {
                    crate::mir6502::abi::canonical_call_lanes(self.routines.get(id)?, args).ok()?;
                    targets.insert(*id);
                }
                MirOp::Call { .. } | MirOp::RuntimeHelper { .. } | MirOp::MachineBlock { .. }
                    | MirOp::SArgsCapture { .. } | MirOp::Barrier { .. } => return None,
                _ => {},
            }
        }
        let dependencies = targets.into_iter().map(|id| Some((id, self.analyze(id)?)))
            .collect::<Option<BTreeMap<_, _>>>()?;
        MirCalleeEntryFact::analyze_with_dependencies(self.program, target,
            MirCalleeEntryStage::ActualEntry, Some(dependencies)).map(Arc::new)
    }
}

/// Limit definite return definitions to the typed scalar ABI, then prove the
/// physical writes on every returning path. An arbitrary scratch store, an
/// untyped signature or a conditional result store grants no definition.
pub(super) fn defined_results(
    routine: &MirRoutine, cfg: &MirCfg,
    transfers: &BTreeMap<crate::mir6502::ir::MirBlockId, MirHomeBlockTransfers>,
) -> BTreeSet<u8> {
    let Some(signature) = &routine.scalar_signature else { return BTreeSet::new(); };
    let [width] = signature.result.as_slice() else { return BTreeSet::new(); };
    let slots: BTreeSet<_> = (0..crate::mir6502::abi::action_arg_width_bytes(*width))
        .map(|i| 0xa0 + i as u8).collect();
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
    slots.into_iter().filter(|s| !live.contains(MirHomeByte::FixedZeroPage(MirFixedZpSlot(*s)))).collect()
}

#[cfg(test)]
mod tests;
