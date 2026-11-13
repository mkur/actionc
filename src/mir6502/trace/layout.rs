//! A bounded census of planning-address observations, never rewrite authority.
//!
//! A discovery round can inspect several candidates. Its observations form a
//! conservative envelope, not minimal dependencies of each applied rewrite.
use super::{Trace, homes};
use crate::mir6502::{MirBlockId, MirMem, RoutineId};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAX_ROUND_OBSERVATIONS: usize = 512;
const MAX_OUTSIDE_OBSERVATIONS: usize = 8192;
const MAX_APPLIED_SITES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Predicate {
    AddressQuery,
    EqualAddress,
    DisjointRanges,
    InRange,
    AboveZeroPageAndNoWrap,
    /// Declaration/extent proof; no numeric address was queried.
    OrdinaryRangeOutsideZeroPage,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::{
        MirSpillId,
        trace::{Mir6502Trace, TraceOptions},
    };

    fn new_session() -> Mir6502Trace {
        Mir6502Trace::new(TraceOptions {
            core_layout_census: true,
            ..Default::default()
        })
    }
    fn query(trace: &Trace, n: u16) {
        trace.layout_observation(
            "test",
            Some(RoutineId(2)),
            Predicate::AddressQuery,
            None,
            || {
                vec![Input::new(
                    RoutineId(2),
                    &MirMem::Spill {
                        id: MirSpillId(7),
                        offset: 0,
                    },
                    Some(n),
                    1,
                    true,
                )]
            },
        );
    }

    #[test]
    fn rounds_distinguish_rejection_and_applied_sites_and_deduplicate_queries() {
        let session = new_session();
        let trace = session.context();
        for applied in [false, true] {
            let _round = trace.layout_round(RoutineId(2), 3);
            query(&trace, 0x3000);
            query(&trace, 0x3000);
            if applied {
                trace.layout_applied("control", MirBlockId(4), 1, 3);
            }
        }
        let events = session.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data["applied_count"], 0);
        assert_eq!(events[1].data["applied_count"], 1);
        assert_eq!(events[1].data["applied"][0]["range"], json!([1, 3]));
        assert_eq!(events[1].data["observations"].as_array().unwrap().len(), 1);
        assert_eq!(events[1].data["observations"][0]["evaluations"], 2);
        assert_eq!(
            events[1].data["observations"][0]["observation"]["inputs"][0]["home"]["id"],
            7
        );
    }

    #[test]
    fn journals_are_bounded_and_disabled_observers_are_lazy() {
        Trace::disabled().layout_observation("unused", None, Predicate::AddressQuery, None, || {
            panic!("disabled")
        });
        let session = new_session();
        let trace = session.context();
        {
            let _round = trace.layout_round(RoutineId(2), 0);
            for n in 0..=MAX_ROUND_OBSERVATIONS as u16 {
                query(&trace, n);
            }
        }
        assert_eq!(session.events()[0].data["omitted"], 1);
        assert_eq!(
            session.manifest()["core_layout_census"]["status"],
            "truncated"
        );
        assert!(new_session().events().is_empty());
    }

    #[test]
    fn unscoped_queries_never_acquire_rewrite_acceptance() {
        let session = new_session();
        let trace = session.context();
        query(&trace, 0x3000);
        trace.layout_applied("outside", MirBlockId(0), 0, 1);
        trace.finish_layout_census();
        assert_eq!(session.events()[0].kind, "core-layout-unattributed");
        assert!(session.events()[0].data["applied_count"].is_null());
        assert_eq!(
            session.manifest()["core_layout_census"]["rounds_with_applied_rewrites"],
            0
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Input {
    home: homes::Home,
    address: Option<u16>,
    bytes: Option<u16>,
    planning_address: bool,
}

impl Input {
    pub(crate) fn new(
        owner: RoutineId,
        mem: &MirMem,
        address: Option<u16>,
        bytes: u16,
        planning_address: bool,
    ) -> Self {
        Self {
            home: homes::memory(owner, mem),
            address,
            bytes: (bytes > 0).then_some(bytes),
            planning_address,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Observation {
    epoch: &'static str,
    file: &'static str,
    line: u32,
    predicate: Predicate,
    result: Option<bool>,
    inputs: Vec<Input>,
}

#[derive(Default)]
struct Journal {
    observations: BTreeMap<Observation, usize>,
    omitted: usize,
}

impl Journal {
    fn record(&mut self, observation: Observation, limit: usize) {
        if let Some(n) = self.observations.get_mut(&observation) {
            *n += 1;
        } else if self.observations.len() < limit {
            self.observations.insert(observation, 1);
        } else {
            self.omitted += 1;
        }
    }
    fn values(&self) -> Value {
        json!(
            self.observations
                .iter()
                .map(|(fact, count)| json!({"observation":fact,"evaluations":count}))
                .collect::<Vec<_>>()
        )
    }
}

struct Round {
    attempt: u64,
    routine: RoutineId,
    generation: u64,
    journal: Journal,
    applied: Vec<Value>,
    applied_count: usize,
}

#[derive(Default)]
pub(super) struct Census {
    rounds: Vec<Round>,
    outside: BTreeMap<(u64, Option<RoutineId>), Journal>,
    outside_observations: usize,
    omitted: usize,
    observed_rounds: usize,
    applied_rounds: usize,
}

impl Census {
    pub(super) fn summary(&self, enabled: bool) -> Value {
        json!({"enabled":enabled,"status":if !enabled {"disabled"} else if self.omitted>0 {"truncated"} else {"complete"},
            "observed_rounds":self.observed_rounds,"rounds_with_applied_rewrites":self.applied_rounds,
            "omitted_observations_or_sites":self.omitted,
            "association":"discovery-round envelope; includes rejected-candidate and analysis queries",
            "coverage":"traced core MaterializeLayout instances; outside-driver queries are unattributed"})
    }
}

pub(crate) struct RoundGuard(Option<Trace>);
impl Drop for RoundGuard {
    fn drop(&mut self) {
        let Some(trace) = &self.0 else {
            return;
        };
        let Some(session) = &trace.session else {
            return;
        };
        let round = session
            .state
            .borrow_mut()
            .layout
            .rounds
            .pop()
            .expect("layout census round");
        debug_assert_eq!(round.attempt, trace.attempt);
        if round.journal.observations.is_empty() && round.journal.omitted == 0 {
            return;
        }
        {
            let mut state = session.state.borrow_mut();
            let census = &mut state.layout;
            census.observed_rounds += 1;
            census.applied_rounds += usize::from(round.applied_count > 0);
            census.omitted +=
                round.journal.omitted + round.applied_count.saturating_sub(round.applied.len());
        }
        trace.event("core-layout-round", || json!({
            "routine":round.routine.0,"generation":round.generation,
            "association":"discovery-round envelope; not per-rewrite proof",
            "selection_scope":"local; speculative until selected by every parent",
            "applied_count":round.applied_count,"applied":round.applied,
            "omitted":round.journal.omitted + round.applied_count.saturating_sub(round.applied.len()),
            "observations":round.journal.values(),
        }));
    }
}

impl Trace {
    pub(crate) fn layout_census_enabled(&self) -> bool {
        self.enabled()
            && self
                .session
                .as_ref()
                .is_some_and(|s| s.options.core_layout_census)
    }

    pub(crate) fn layout_round(&self, routine: RoutineId, generation: u64) -> RoundGuard {
        if !self.layout_census_enabled() || !self.wants(routine) {
            return RoundGuard(None);
        }
        self.session
            .as_ref()
            .unwrap()
            .state
            .borrow_mut()
            .layout
            .rounds
            .push(Round {
                attempt: self.attempt,
                routine,
                generation,
                journal: Journal::default(),
                applied: Vec::new(),
                applied_count: 0,
            });
        RoundGuard(Some(self.clone()))
    }

    pub(crate) fn layout_applied(
        &self,
        family: &'static str,
        block: MirBlockId,
        start: usize,
        end: usize,
    ) {
        if !self.layout_census_enabled() {
            return;
        }
        let mut state = self.session.as_ref().unwrap().state.borrow_mut();
        let Some(round) = state
            .layout
            .rounds
            .last_mut()
            .filter(|r| r.attempt == self.attempt)
        else {
            return;
        };
        round.applied_count += 1;
        if round.applied.len() < MAX_APPLIED_SITES {
            round
                .applied
                .push(json!({"family":family,"block":block.0,"range":[start,end]}));
        }
    }

    #[track_caller]
    pub(crate) fn layout_observation(
        &self,
        epoch: &'static str,
        routine: Option<RoutineId>,
        predicate: Predicate,
        result: Option<bool>,
        inputs: impl FnOnce() -> Vec<Input>,
    ) {
        if !self.layout_census_enabled() || routine.is_some_and(|r| !self.wants(r)) {
            return;
        }
        let caller = std::panic::Location::caller();
        let observation = Observation {
            epoch,
            file: caller.file(),
            line: caller.line(),
            predicate,
            result,
            inputs: inputs(),
        };
        let mut state = self.session.as_ref().unwrap().state.borrow_mut();
        let census = &mut state.layout;
        if let Some(round) = census
            .rounds
            .last_mut()
            .filter(|r| routine.is_none_or(|id| r.routine == id))
        {
            round.journal.record(observation, MAX_ROUND_OBSERVATIONS);
        } else {
            let key = (self.attempt, routine);
            let existing = census
                .outside
                .get(&key)
                .is_some_and(|j| j.observations.contains_key(&observation));
            if !existing && census.outside_observations >= MAX_OUTSIDE_OBSERVATIONS {
                census.omitted += 1;
                return;
            }
            census.outside_observations += usize::from(!existing);
            census
                .outside
                .entry(key)
                .or_default()
                .record(observation, MAX_OUTSIDE_OBSERVATIONS);
        }
    }

    pub(super) fn finish_layout_census(&self) {
        let Some(session) = &self.session else {
            return;
        };
        let outside = std::mem::take(&mut session.state.borrow_mut().layout.outside);
        for ((attempt, routine), journal) in outside {
            let trace = Trace {
                attempt,
                ..self.clone()
            };
            trace.event("core-layout-unattributed", || {
                json!({"routine":routine.map(|r|r.0),
                "association":"outside a shared rewrite driver; no acceptance inferred",
                "observations":journal.values()})
            });
        }
    }
}
