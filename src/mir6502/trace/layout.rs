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
const MAX_CANDIDATES: usize = 256;

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
    FitsFixedZeroPagePair,
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

    #[test]
    fn candidate_queries_follow_exact_outcomes_without_claiming_shared_analysis() {
        let session = new_session();
        let trace = session.context();
        {
            let _round = trace.layout_round(RoutineId(2), 3);
            query(&trace, 0x4000); // Shared analysis, not either candidate.
            {
                let scope = trace.layout_candidate(RoutineId(2), "loser", MirBlockId(4), 1);
                query(&trace, 0x3000);
                scope.shape(3);
            }
            {
                let scope = trace.layout_candidate(RoutineId(2), "winner", MirBlockId(4), 1);
                scope.shape(3); // A selected shape can have no address queries.
            }
            trace.layout_plan_outcome("winner", MirBlockId(4), 1, 4, "applied");
            trace.layout_plan_outcome("winner", MirBlockId(4), 1, 3, "candidate");
            trace.layout_plan_outcome("loser", MirBlockId(4), 1, 3, "overlap-rejected");
            trace.layout_plan_outcome("winner", MirBlockId(4), 1, 3, "applied");
            trace.layout_applied("winner", MirBlockId(4), 1, 3);
        }
        let events = session.events();
        let data = &events[0].data;
        assert_eq!(data["candidates"][0]["outcome"], "overlap-rejected");
        assert_eq!(data["candidates"][1]["outcome"], "applied");
        let observations = data["observations"].as_array().unwrap();
        assert!(
            observations
                .iter()
                .any(|o| o["observation"]["candidate"].is_null())
        );
        assert!(
            observations
                .iter()
                .any(|o| o["observation"]["candidate"]["family"] == "loser")
        );
        assert!(
            !observations
                .iter()
                .any(|o| o["observation"]["candidate"]["family"] == "winner")
        );
        {
            let _round = trace.layout_round(RoutineId(2), 4);
            let scope = trace.layout_candidate(RoutineId(2), "winner", MirBlockId(4), 1);
            scope.shape(3);
        }
        assert_eq!(
            session.events()[1].data["candidates"][0]["outcome"],
            "shape-only"
        );
    }

    #[test]
    fn candidate_caps_are_explicit_and_unscoped_candidates_grant_no_acceptance() {
        let session = new_session();
        let trace = session.context();
        trace
            .layout_candidate(RoutineId(2), "outside", MirBlockId(0), 0)
            .shape(1);
        assert!(session.events().is_empty());
        {
            let _round = trace.layout_round(RoutineId(2), 0);
            for start in 0..=MAX_CANDIDATES {
                trace
                    .layout_candidate(RoutineId(2), "bounded", MirBlockId(0), start)
                    .shape(start + 1);
            }
        }
        assert_eq!(
            session.events()[0].data["candidates"]
                .as_array()
                .unwrap()
                .len(),
            MAX_CANDIDATES
        );
        assert_eq!(session.events()[0].data["omitted"], 1);
        assert_eq!(
            session.manifest()["core_layout_census"]["status"],
            "truncated"
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
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<CandidateKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct CandidateKey {
    family: &'static str,
    block: u32,
    start: usize,
}

#[derive(Serialize)]
struct Candidate {
    key: CandidateKey,
    end: Option<usize>,
    outcome: &'static str,
    #[serde(skip)]
    observed: bool,
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
    active_candidate: Option<Candidate>,
    candidates: BTreeMap<CandidateKey, Candidate>,
    omitted_candidates: usize,
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

/// Diagnostics only. A scope covers local candidate checks, not inherited
/// dataflow facts. Scope identity is local to the active driver round.
#[derive(Default)]
pub(crate) struct CandidateGuard(Option<Trace>);

impl CandidateGuard {
    pub(crate) fn shape(&self, end: usize) {
        if let Some(trace) = &self.0 {
            let mut state = trace.session.as_ref().unwrap().state.borrow_mut();
            let candidate = state
                .layout
                .rounds
                .last_mut()
                .unwrap()
                .active_candidate
                .as_mut()
                .unwrap();
            candidate.end = Some(end);
            candidate.outcome = "shape-only";
        }
    }
}

impl Drop for CandidateGuard {
    fn drop(&mut self) {
        let Some(trace) = &self.0 else {
            return;
        };
        let mut state = trace.session.as_ref().unwrap().state.borrow_mut();
        let round = state.layout.rounds.last_mut().unwrap();
        let candidate = round.active_candidate.take().expect("candidate scope");
        if !candidate.observed && candidate.end.is_none() {
            return;
        }
        if round.candidates.len() < MAX_CANDIDATES {
            round.candidates.insert(candidate.key.clone(), candidate);
        } else {
            round.omitted_candidates += 1;
        }
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
        debug_assert!(round.active_candidate.is_none());
        if round.journal.observations.is_empty()
            && round.journal.omitted == 0
            && round.candidates.is_empty()
            && round.omitted_candidates == 0
        {
            return;
        }
        {
            let mut state = session.state.borrow_mut();
            let census = &mut state.layout;
            census.observed_rounds += 1;
            census.applied_rounds += usize::from(round.applied_count > 0);
            census.omitted += round.journal.omitted
                + round.applied_count.saturating_sub(round.applied.len())
                + round.omitted_candidates;
        }
        trace.event("core-layout-round", || json!({
            "routine":round.routine.0,"generation":round.generation,
            "association":"discovery-round envelope; not per-rewrite proof",
            "selection_scope":"local; speculative until selected by every parent",
            "applied_count":round.applied_count,"applied":round.applied,
            "omitted":round.journal.omitted + round.applied_count.saturating_sub(round.applied.len()) + round.omitted_candidates,
            "observations":round.journal.values(),
            "candidates":round.candidates.values().collect::<Vec<_>>(),
            "candidate_coverage":"instrumented local checks only; inherited analysis and unscoped observations are separate",
        }));
    }
}

impl Trace {
    pub(crate) fn layout_candidate(
        &self,
        routine: RoutineId,
        family: &'static str,
        block: MirBlockId,
        start: usize,
    ) -> CandidateGuard {
        if !self.layout_census_enabled() || !self.wants(routine) {
            return CandidateGuard::default();
        }
        let mut state = self.session.as_ref().unwrap().state.borrow_mut();
        let Some(round) = state
            .layout
            .rounds
            .last_mut()
            .filter(|r| r.routine == routine)
        else {
            return CandidateGuard::default();
        };
        assert!(
            round.active_candidate.is_none(),
            "candidate scopes must not overlap"
        );
        round.active_candidate = Some(Candidate {
            key: CandidateKey {
                family,
                block: block.0,
                start,
            },
            end: None,
            outcome: "no-shape",
            observed: false,
        });
        CandidateGuard(Some(self.clone()))
    }

    pub(crate) fn layout_plan_outcome(
        &self,
        family: &'static str,
        block: MirBlockId,
        start: usize,
        end: usize,
        outcome: &'static str,
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
        let key = CandidateKey {
            family,
            block: block.0,
            start,
        };
        if let Some(candidate) = round
            .candidates
            .get_mut(&key)
            .filter(|c| c.end == Some(end))
        {
            candidate.outcome = outcome;
        }
    }

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
                active_candidate: None,
                candidates: BTreeMap::new(),
                omitted_candidates: 0,
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
        let mut observation = Observation {
            epoch,
            file: caller.file(),
            line: caller.line(),
            predicate,
            result,
            inputs: inputs(),
            candidate: None,
        };
        let mut state = self.session.as_ref().unwrap().state.borrow_mut();
        let census = &mut state.layout;
        if let Some(round) = census
            .rounds
            .last_mut()
            .filter(|r| routine.is_none_or(|id| r.routine == id))
        {
            if let Some(candidate) = &mut round.active_candidate {
                candidate.observed = true;
                observation.candidate = Some(candidate.key.clone());
            }
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
