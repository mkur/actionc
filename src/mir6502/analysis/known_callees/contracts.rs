//! Compatibility of published guarantees, not a freshness certificate for sites.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) struct MirCalleeContractChange {
    pub routine: Option<RoutineId>,
    pub fact: &'static str,
}

impl MirKnownCalleeSummaries {
    /// `new` must be analyzed independently from `proposed` under the same
    /// publication policy. Owners separately validate interfaces, identity domains
    /// and placement/replay; this query does not make old source witnesses current.
    pub(in crate::mir6502) fn contracts_preserved_by(
        &self, proposed: &MirProgram, new: &Self,
    ) -> Result<(), MirCalleeContractChange> {
        let changed = |routine, fact| Err(MirCalleeContractChange { routine, fact });
        if self.mutable_return_flags != new.mutable_return_flags {
            return changed(None, "publication-policy");
        }
        if self.fixed_storage != new.fixed_storage {
            return changed(None, "fixed-backing");
        }
        if !self.scalar_entries_ignoring_flags.is_subset(&new.scalar_entries_ignoring_flags) {
            return changed(None, "entry-flags");
        }
        for (id, fact) in &self.entry_flags {
            if let super::super::entry_flags::MirKnownCalleeEntryFlags::Known { may_observe } = fact.summary {
                let unobserved = super::super::effects::MirFlagSet {
                    c: !may_observe.c, z: !may_observe.z, n: !may_observe.n, v: !may_observe.v,
                };
                if !proposed.routines.iter().find(|r| r.id == *id).is_some_and(|r|
                    super::super::entry_flags::MirKnownCalleeEntryFlags::analyze(r).unobserved(unobserved))
                {
                    return changed(Some(*id), "precise-entry-flags");
                }
            }
        }
        for (id, old) in &self.routines {
            let Some(next) = new.get(*id) else { return changed(Some(*id), "missing-callee"); };
            if let MirKnownMemoryReads::Exact(a) = old.reads() {
                let MirKnownMemoryReads::Exact(b) = next.reads() else { return changed(Some(*id), "unknown-reads"); };
                if !b.iter().all(|m| a.contains(m)) { return changed(Some(*id), "expanded-reads"); }
            }
            if let MirKnownMemoryWrites::Exact(a) = old.writes() {
                let MirKnownMemoryWrites::Exact(b) = next.writes() else { return changed(Some(*id), "unknown-writes"); };
                if !b.iter().all(|m| a.contains(m)) { return changed(Some(*id), "expanded-writes"); }
            }
            if self.accumulator_summary_is_observable(*id) && old.accumulator().is_some()
                && old.accumulator() != next.accumulator()
            { return changed(Some(*id), "accumulator"); }
            if old.zn().is_some() && old.zn() != next.zn() { return changed(Some(*id), "exit-flags"); }
            if [MirReg::A, MirReg::X, MirReg::Y].into_iter().any(|r|
                old.preserves_register(r) && !next.preserves_register(r))
            { return changed(Some(*id), "register-preservation"); }
            // Pairs alone miss an isolated preserved byte between two writes.
            if (0..=u8::MAX).any(|byte| old.writes().preserves_fixed_byte(MirFixedZpSlot(byte), self.fixed_storage())
                && !next.writes().preserves_fixed_byte(MirFixedZpSlot(byte), new.fixed_storage()))
            { return changed(Some(*id), "fixed-byte-preservation"); }
        }
        Ok(())
    }
}
