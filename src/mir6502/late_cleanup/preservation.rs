//! Compilation-local witnesses, independent of tracing and speculative trials.
use crate::mir6502::analysis::{address_ranges::MirAddressRanges, record_bases::MirPhysicalMemory};
use crate::mir6502::{Mir6502Config, MirDiagnostic, ir::*, materialize, trace::Trace};
use std::collections::BTreeSet;

/// Closed inventory of semantic decisions after core allocation. Adding a new
/// family requires an explicit preservation validator at this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum Family {
    RecordBases,
    RegisterCopies,
    PointerRetention,
    PointerPromotion,
    ArgumentStaging,
    PointerBytePlusOne,
    PointerLoadSharing,
    ArgumentXForwarding,
    WordLoadSharing,
    CheckedRanges,
}

struct SelectedDecision {
    family: Family,
    before: MirProgram,
    selected: MirProgram,
    routines: BTreeSet<RoutineId>,
}

pub(in crate::mir6502) struct Preservation {
    enabled: bool,
    decisions: Vec<SelectedDecision>,
}

impl Preservation {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            decisions: Vec::new(),
        }
    }

    pub fn select(
        &mut self,
        family: Family,
        before: MirProgram,
        select: impl FnOnce(MirProgram) -> Result<MirProgram, Vec<MirDiagnostic>>,
    ) -> Result<MirProgram, Vec<MirDiagnostic>> {
        if !self.enabled {
            return select(before);
        }
        let witness = before.clone();
        let selected = select(before)?;
        if witness != selected {
            let routines = witness
                .routines
                .iter()
                .zip(&selected.routines)
                .filter_map(|(a, b)| (a != b).then_some(a.id))
                .collect();
            self.decisions.push(SelectedDecision {
                family,
                before: witness,
                selected: selected.clone(),
                routines,
            });
        }
        Ok(selected)
    }

    /// Removed references still need addresses, but never contribute placement
    /// or executable operations. All frames are retained by the late pipeline.
    pub fn references<'a>(&'a self, baseline: &'a MirProgram) -> Vec<&'a MirProgram> {
        std::iter::once(baseline)
            .chain(self.decisions.iter().flat_map(|d| [&d.before, &d.selected]))
            .collect()
    }

    pub fn validate(
        &self,
        memory: &MirPhysicalMemory,
        ranges: Option<&MirAddressRanges<'_>>,
        config: &Mir6502Config,
        origin: u16,
        trace: &Trace,
    ) -> Result<(), Family> {
        use Family::*;
        for d in &self.decisions {
            let t = trace.child("preservation-replay", "selected-late-decision");
            t.event(
                "preservation-family",
                || serde_json::json!({"family": format!("{:?}", d.family)}),
            );
            t.count("late_cleanup_preservation_replays", 1);
            let (a, b, ids) = (&d.before, &d.selected, &d.routines);
            let valid = match d.family {
                RecordBases => materialize::record_bases::replay_selected(a, b, memory, config, &t),
                RegisterCopies => {
                    materialize::register_copies::replay_selected(a, b, memory, origin, &t)
                }
                PointerRetention | PointerPromotion => {
                    materialize::pointer_retention::replay_selected(
                        a,
                        b,
                        memory,
                        config,
                        d.family == PointerPromotion,
                        &t,
                    )
                }
                ArgumentStaging => {
                    materialize::argument_staging::replay_selected(a, b, memory, ids, &t)
                }
                PointerBytePlusOne => {
                    materialize::pointer_byte_plus_one::replay_selected(a, b, ids, &t)
                }
                PointerLoadSharing => {
                    materialize::pointer_load_sharing::replay_selected(a, b, memory, ids, &t)
                }
                ArgumentXForwarding => {
                    materialize::argument_staging::x_forwarding::replay_selected(
                        a, b, memory, ids, config, &t,
                    )
                }
                WordLoadSharing => {
                    materialize::word_load_sharing::replay_selected(a, b, memory, ids, &t)
                }
                CheckedRanges => ranges.is_some_and(|ranges| {
                    materialize::pointer_retention::checked_ranges::replay_selected(
                        a, b, memory, ranges,
                    )
                }),
            };
            if !valid {
                return Err(d.family);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::word_load_sharing::tests::{baseline, memory};
    use crate::mir6502::trace::{Mir6502Trace, TraceOptions};

    #[test]
    fn selected_replay_preserves_partial_selection_without_emission() {
        let before = baseline();
        let config = Mir6502Config::optimized();
        let mut selected = materialize::word_load_sharing::select_costed(
            before.clone(),
            &config,
            0x3000,
            crate::runtime::Runtime::ActionCart,
        )
        .unwrap();
        let changed = before
            .routines
            .iter()
            .zip(&selected.routines)
            .filter_map(|(a, b)| (a != b).then_some(a.id))
            .collect::<Vec<_>>();
        assert_eq!(changed.len(), 2);
        let restored = changed[0];
        *selected
            .routines
            .iter_mut()
            .find(|r| r.id == restored)
            .unwrap() = before
            .routines
            .iter()
            .find(|r| r.id == restored)
            .unwrap()
            .clone();
        let mut log = Preservation::new(true);
        let selected = log
            .select(Family::WordLoadSharing, before.clone(), |_| Ok(selected))
            .unwrap();
        assert_eq!(log.decisions.len(), 1);
        assert_eq!(log.decisions[0].routines.len(), 1);
        let session = Mir6502Trace::new(TraceOptions::default());
        assert_eq!(
            log.validate(&memory(&before), None, &config, 0x3000, &session.context()),
            Ok(())
        );
        assert!(session.manifest()["work"]["emission_invocations"].is_null());
        assert!(session.manifest()["work"]["materializations"].is_null());
        assert_eq!(
            log.validate(
                &MirPhysicalMemory::default(),
                None,
                &config,
                0x3000,
                &Trace::disabled()
            ),
            Err(Family::WordLoadSharing)
        );
        assert_eq!(log.references(&selected).len(), 3);
    }

    #[test]
    fn rejected_and_disabled_selections_keep_no_witnesses() {
        let before = baseline();
        for enabled in [false, true] {
            let mut log = Preservation::new(enabled);
            assert_eq!(
                log.select(Family::WordLoadSharing, before.clone(), Ok)
                    .unwrap(),
                before
            );
            assert!(log.decisions.is_empty());
        }
    }
}
