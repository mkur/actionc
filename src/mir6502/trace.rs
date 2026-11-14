//! Bounded, compilation-local diagnostics. Nothing here grants rewrite permission.
pub(crate) mod homes;
pub(crate) mod rewrites;
pub(crate) mod layout;
mod summary;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use serde::Serialize;
use serde_json::{Value, json};

use super::{MirDiagnostic, MirProgram, RoutineId};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TraceOptions {
    /// Diagnostic observations only; never enables a transformation or proof.
    pub core_layout_census: bool,
    /// Exact display names or `r123` IDs; used only to filter diagnostics.
    pub routines: Vec<String>,
    pub max_bytes: usize,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self {
            core_layout_census: false,
            routines: Vec::new(),
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TraceEvent {
    pub id: u64,
    pub attempt: u64,
    pub kind: &'static str,
    pub data: Value,
}

#[derive(Default)]
struct State {
    layout: layout::Census,
    events: Vec<TraceEvent>,
    bytes: usize,
    next_id: u64,
    selected: BTreeSet<RoutineId>,
    filter_errors: Vec<String>,
    truncated_at: Option<String>,
    finished: bool,
    failed: bool,
    output_identity: Option<String>,
    output_sizes: Option<Value>,
    final_selection_recorded: bool,
    selected_artifact_snapshot: Option<u64>,
    next_attempt: u64,
    work: std::collections::BTreeMap<&'static str, usize>,
}

impl State {
    fn final_snapshot(&self) -> Option<u64> {
        if self.final_selection_recorded {
            self.selected_artifact_snapshot
        } else {
            self.events.iter().rev().find(|e| e.kind == "snapshot"
                && e.data["stage"] == "pre-emission").map(|e| e.id)
        }
    }
    fn final_layout(&self) -> Option<u64> {
        let snapshot = self.final_snapshot()?;
        self.events.iter().rev().find(|e| e.kind == "final-layout"
            && e.data["snapshot"] == snapshot).map(|e| e.id)
    }
}

/// A caller-owned bounded collector. A generation call resets it before use.
struct Collector {
    options: TraceOptions,
    state: RefCell<State>,
}

pub struct Mir6502Trace {
    inner: Rc<Collector>,
}

impl Mir6502Trace {
    pub fn new(options: TraceOptions) -> Self {
        Self {
            inner: Rc::new(Collector {
                options,
                state: RefCell::new(State::default()),
            }),
        }
    }

    pub fn events(&self) -> Vec<TraceEvent> {
        self.inner.state.borrow().events.clone()
    }

    pub fn manifest(&self) -> Value {
        let s = self.inner.state.borrow();
        json!({
            "schema_version": SCHEMA_VERSION,
            "status": if !s.finished { "incomplete" } else if s.failed { "compilation-failed" }
                else if !s.filter_errors.is_empty() { "filter-error" }
                else if s.truncated_at.is_some() { "truncated" } else { "complete" },
            "work": s.work, "analysis_counter_scope": "shared rewrite drivers only",
            "detail_bytes": s.bytes, "max_bytes": self.inner.options.max_bytes,
            "first_omitted_stage": s.truncated_at, "filter_errors": s.filter_errors,
            "compilation_event": s.events.iter().find(|e|e.kind == "compilation").map(|e|e.id),
            "output_sizes": s.output_sizes,
            "output_identity": s.output_identity, "identity_algorithm": "fnv1a64",
            "selected_routines": s.selected.iter().map(|r| r.0).collect::<Vec<_>>(),
            "core_layout_census": s.layout.summary(self.inner.options.core_layout_census),
            "final_layout": if s.finished && !s.failed { s.final_layout() } else { None },
            "final_snapshot": s.final_snapshot()
        })
    }

    pub(super) fn reset(&mut self) {
        *self.inner.state.borrow_mut() = State::default();
    }
    pub(super) fn context(&self) -> Trace {
        Trace {
            session: Some(Rc::clone(&self.inner)),
            attempt: 0,
            final_artifact: false,
            artifact_snapshot: None,
            #[cfg(test)]
            observer: None,
        }
    }
    pub(super) fn finish(
        &self,
        result: &Result<crate::codegen::CodegenOutput, Vec<MirDiagnostic>>,
    ) {
        self.context().finish_layout_census();
        let mut s = self.inner.state.borrow_mut();
        s.finished = true;
        s.failed = result.is_err();
        if let Ok(output) = result {
            let image = crate::codegen::format_load_file(output);
            s.output_identity = Some(identity_bytes(&image));
            s.output_sizes = Some(json!({"load_file_bytes":image.len(),"segment_bytes":output.bytes.len(),"relocations":output.relocations.len()}));
        }
    }
}

impl Default for Mir6502Trace {
    fn default() -> Self {
        Self::new(TraceOptions::default())
    }
}

/// Explicitly passed observation capability. Disabled contexts do not evaluate payloads.
#[derive(Clone, Default)]
pub(crate) struct Trace {
    session: Option<Rc<Collector>>,
    attempt: u64,
    final_artifact: bool,
    artifact_snapshot: Option<u64>,
    #[cfg(test)]
    observer: Option<Rc<RefCell<std::collections::BTreeMap<&'static str, usize>>>>,
}

impl Trace {
    pub(crate) const fn disabled() -> Self {
        Self {
            session: None,
            attempt: 0,
            final_artifact: false,
            artifact_snapshot: None,
            #[cfg(test)]
            observer: None,
        }
    }
    pub(crate) fn final_artifact(&self) -> bool {
        self.final_artifact && self.enabled()
    }
    pub(crate) fn with_artifact_snapshot(mut self, snapshot: Option<u64>) -> Self {
        self.artifact_snapshot = snapshot;
        self
    }
    pub(crate) fn select_final_artifact(&self, snapshot: Option<u64>) {
        self.event("final-artifact-selection", || json!({"snapshot":snapshot}));
        if let Some(session) = &self.session {
            let mut state = session.state.borrow_mut();
            state.final_selection_recorded = true;
            state.selected_artifact_snapshot = snapshot;
        }
    }
    pub(crate) fn final_snapshot(&self) -> Option<u64> {
        // Emission gets the snapshot from its own artifact, including None
        // when tracing was truncated. Never borrow a previous trial's ID.
        if self.final_artifact { return self.artifact_snapshot; }
        self.session.as_ref()?.state.borrow().final_snapshot()
    }
    pub(crate) fn emitted_range(
        &self,
        routine: RoutineId,
        block: super::MirBlockId,
        op: Option<usize>,
        start: usize,
        end: usize,
        tail_call: bool,
    ) {
        if self.wants(routine) {
            self.event("emitted-range", || {
                json!({"snapshot":self.final_snapshot(),"routine":routine.0,
                "block":block.0,"op":op,"offset_start":start,"offset_end":end,"tail_call":tail_call})
            });
        }
    }
    pub(crate) fn enabled(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.state.borrow().truncated_at.is_none())
    }

    pub(crate) fn event(&self, kind: &'static str, data: impl FnOnce() -> Value) -> Option<u64> {
        let session = self.session.as_ref()?;
        if !self.enabled() {
            return None;
        }
        let data = data();
        let mut s = session.state.borrow_mut();
        let id = s.next_id;
        let event = TraceEvent {
            id,
            attempt: self.attempt,
            kind,
            data,
        };
        let bytes = serde_json::to_vec(&event).expect("diagnostic JSON").len() + 1;
        if bytes > session.options.max_bytes.saturating_sub(s.bytes) {
            s.truncated_at = Some(
                event
                    .data
                    .get("stage")
                    .and_then(Value::as_str)
                    .unwrap_or(kind)
                    .to_owned(),
            );
            return None;
        }
        s.next_id += 1;
        s.bytes += bytes;
        s.events.push(event);
        Some(id)
    }

    pub(crate) fn count(&self, counter: &'static str, n: usize) {
        #[cfg(test)]
        if let Some(observer) = &self.observer {
            *observer.borrow_mut().entry(counter).or_default() += n;
        }

        if let Some(session) = &self.session {
            *session.state.borrow_mut().work.entry(counter).or_default() += n;
        }
    }

    pub(crate) fn child(&self, stage: &'static str, purpose: &'static str) -> Self {
        let Some(session) = &self.session else {
            return self.clone();
        };
        let id = {
            let mut s = session.state.borrow_mut();
            s.next_attempt += 1;
            s.next_attempt
        };
        let child = Self {
            attempt: id,
            final_artifact: self.final_artifact || purpose == "final-artifact",
            ..self.clone()
        };
        child.event(
            "attempt-start",
            || json!({ "parent": self.attempt, "stage": stage, "purpose": purpose,
                "scope": if matches!(stage,"emission-request"|"final-emission"|"inline-final-rematerialization") {"context"} else {"attempt"} }),
        );
        child
    }

    pub(crate) fn run<T: std::borrow::Borrow<MirProgram>>(
        &self,
        stage: &'static str,
        f: impl FnOnce(&Self) -> Result<T, Vec<MirDiagnostic>>,
    ) -> Result<T, Vec<MirDiagnostic>> {
        let child = self.child(stage, "pipeline-attempt");
        child.count("pipeline_attempts", 1);
        let result = f(&child);
        let snapshot = result
            .as_ref()
            .ok()
            .and_then(|p| child.snapshot("attempt-return", p.borrow()));
        child.event("attempt-end", || {
            let mut sources = Vec::new();
            let mut selection = None;
            let mut unchanged = None;
            if let (Some(session), Some(snapshot)) = (&child.session, snapshot) {
                let state = session.state.borrow();
                selection = state.events.iter().rev().find(|e| e.attempt == child.attempt && matches!(e.kind,"selection"|"inline-selection")).map(|e|e.id);
                let output = &state.events.iter().find(|e| e.id == snapshot).unwrap().data["program_identity"];
                unchanged = state.events.iter().find(|e|e.attempt == child.attempt && e.kind == "snapshot" && e.data["stage"] == "attempt-input").map(|e|&e.data["program_identity"] == output);
                let start = state.events.iter().find(|e| e.kind == "attempt-start" && e.attempt == child.attempt).unwrap().id;
                for e in state.events.iter().filter(|e| e.id > start && e.id < snapshot && e.attempt != child.attempt && e.kind == "snapshot" && e.data["stage"] == "attempt-return" && &e.data["program_identity"] == output) {
                    sources.push(e.id);
                }
            }
            json!({ "stage": stage,
                "status": if result.is_ok() { "returned" } else { "failed" },
                "returned_snapshot": snapshot, "equivalent_source_snapshots": sources,
                "selection_event":selection,"unchanged_from_input":unchanged,
                "selection_scope": "local; speculative until selected by every parent" })
        });
        result
    }

    pub(crate) fn decision(
        &self,
        stage: &'static str,
        candidates: impl IntoIterator<Item = RoutineId>,
        accepted: impl Fn(RoutineId) -> bool,
    ) {
        self.event("selection", || {
            let (yes, no): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|id| accepted(*id));
            json!({ "stage": stage,
                "accepted_routines": yes.iter().map(|r|r.0).collect::<Vec<_>>(),
                "rejected_routines": no.iter().map(|r|r.0).collect::<Vec<_>>(),
                "outcome": if yes.is_empty() { "rejected" } else if no.is_empty() { "accepted" } else { "partial" } })
        });
    }

    pub(crate) fn skipped(&self, stage: &'static str, reason: &'static str) {
        self.event(
            "stage",
            || json!({"stage":stage,"outcome":"skipped","reason":reason}),
        );
    }

    pub(crate) fn bind_routines(&self, program: &MirProgram) {
        let Some(session) = &self.session else {
            return;
        };
        let mut s = session.state.borrow_mut();
        for selector in &session.options.routines {
            let id = selector
                .strip_prefix('r')
                .and_then(|v| v.parse::<u32>().ok());
            let matches = program
                .routines
                .iter()
                .filter(|r| Some(r.id.0) == id || r.name == *selector)
                .collect::<Vec<_>>();
            if matches.len() == 1 {
                s.selected.insert(matches[0].id);
            } else {
                s.filter_errors.push(format!(
                    "routine filter {selector:?} matched {} routines",
                    matches.len()
                ));
            }
        }
    }

    pub(crate) fn wants(&self, routine: RoutineId) -> bool {
        self.enabled()
            && self.session.as_ref().is_some_and(|s| {
                s.options.routines.is_empty() || s.state.borrow().selected.contains(&routine)
            })
    }

    pub(crate) fn routine_snapshot(
        &self,
        stage: &'static str,
        routine: &super::MirRoutine,
    ) -> Option<u64> {
        if !self.wants(routine.id) {
            return None;
        }
        self.event("snapshot", || json!({"stage":stage,"scope":"routine",
            "routine":routine.id.0,"routines":[{"id":routine.id.0,"name":routine.name,"identity":identity(&format!("{routine:?}")),"blocks":routine.blocks.iter().map(|b|json!({"id":b.id.0,"ops":b.ops.len()})).collect::<Vec<_>>() }],
            "mir":super::printer::format_routine(routine),
            "frames":[homes::frame(routine)],"accesses":[homes::accesses(routine)]}))
    }

    pub(crate) fn latest_snapshot(&self, routine: RoutineId) -> Option<u64> {
        self.session
            .as_ref()?
            .state
            .borrow()
            .events
            .iter()
            .rev()
            .find(|e| {
                e.attempt == self.attempt
                    && e.kind == "snapshot"
                    && e.data["routines"]
                        .as_array()
                        .is_some_and(|rs| rs.iter().any(|r| r["id"] == routine.0))
            })
            .map(|e| e.id)
    }

    pub(crate) fn remap(
        &self,
        stage: &'static str,
        routine: RoutineId,
        before: Option<u64>,
        after: Option<u64>,
        entries: impl FnOnce() -> Value,
    ) {
        if self.wants(routine) {
            self.event("home-remap", ||json!({"stage":stage,"routine":routine.0,"from_snapshot":before,"to_snapshot":after,"entries":entries()}));
        }
    }

    pub(crate) fn snapshot(&self, stage: &'static str, program: &MirProgram) -> Option<u64> {
        self.event("snapshot", || {
            let routines = program.routines.iter().filter(|r|self.wants(r.id)).collect::<Vec<_>>();
            json!({ "stage": stage, "scope": "program", "program_identity": identity(&format!("{program:?}")),
                "mir": super::printer::format_program_filtered(program, |id|self.wants(id)),
                "routines": routines.iter().map(|r|json!({"id":r.id.0,"name":r.name,"identity":identity(&format!("{r:?}")),"blocks":r.blocks.iter().map(|b|json!({"id":b.id.0,"ops":b.ops.len()})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                "frames":routines.iter().map(|r|homes::frame(r)).collect::<Vec<_>>(),
                "accesses":routines.iter().map(|r|homes::accesses(r)).collect::<Vec<_>>() })
        })
    }
}

impl std::fmt::Debug for Trace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trace")
            .field("attempt", &self.attempt)
            .field("enabled", &self.enabled())
            .finish()
    }
}

/// Diagnostic identity only; never an optimization proof or cache key.
pub(crate) fn identity(text: &str) -> String {
    identity_bytes(text.as_bytes())
}
pub(crate) fn identity_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_does_not_build_payload_and_budget_is_explicit() {
        Trace::disabled().event("test", || panic!("disabled trace evaluated payload"));
        let s = Mir6502Trace::new(TraceOptions {
            max_bytes: 0,
            ..Default::default()
        });
        s.context().event("test", || json!({"stage":"first"}));
        assert!(s.events().is_empty());
        assert_eq!(s.manifest()["first_omitted_stage"], "first");
    }
    #[test]
    fn missing_trial_snapshot_never_inherits_a_previous_artifact() {
        let session = Mir6502Trace::default();
        let trace = session.context();
        let old = trace.event("snapshot", || json!({"stage":"pre-emission"}));
        assert!(old.is_some());
        let trial = trace.child("final-emission", "final-artifact")
            .with_artifact_snapshot(None);
        assert_eq!(trial.final_snapshot(), None);
        trial.emitted_range(RoutineId(0), super::super::MirBlockId(0), None, 0, 1, false);
        assert!(session.events().last().unwrap().data["snapshot"].is_null());
        trace.select_final_artifact(None);
        assert!(session.manifest()["final_snapshot"].is_null());
        assert!(session.manifest()["final_layout"].is_null());
    }

    #[test]
    fn trace_does_not_add_compiler_work() {
        fn nir(source: &str) -> crate::nir::NirProgram {
            let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
            let model = crate::semantic::analyze_with_options(
                &ast,
                crate::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            crate::nir::optimize_program(&crate::nir::lower_program(
                &crate::semantic::ir::lower_program(&ast, &model),
            ))
            .unwrap()
        }
        for source in [
            include_str!("../../fixtures/runtime/argument_staging.act"),
            include_str!("../../surveys/tn/reproducers/captured-call-arguments.act"),
            "BYTE input,output BYTE FUNC Map(BYTE value) IF (value AND $80)#0 THEN RETURN((value LSH 1) XOR $1B) FI RETURN(value LSH 1) PROC Main() output=Map(input) RETURN",
        ] {
            let input = nir(&source.replace("\r\n", "\n"));
            let session = Mir6502Trace::default();
            let on_work = Rc::new(RefCell::new(std::collections::BTreeMap::new()));
            let off_work = Rc::new(RefCell::new(std::collections::BTreeMap::new()));
            let mut on = session.context();
            on.observer = Some(on_work.clone());
            let mut off = Trace::disabled();
            off.observer = Some(off_work.clone());
            let config = super::super::Mir6502Config::optimized();
            let a = super::super::generate_output_impl(
                &input,
                0x3000,
                &config,
                crate::runtime::Runtime::ActionCart,
                &off,
            )
            .unwrap();
            let b = super::super::generate_output_impl(
                &input,
                0x3000,
                &config,
                crate::runtime::Runtime::ActionCart,
                &on,
            )
            .unwrap();
            assert_eq!(a, b);
            assert_eq!(*on_work.borrow(), *off_work.borrow());
            for key in [
                "materializations",
                "emission_invocations",
                "inner_emission_probes",
                "rewrite_analysis_builds",
            ] {
                assert!(on_work.borrow()[key] > 0, "{key}");
            }
            let events = session.events();
            let ids = events
                .iter()
                .filter(|e| e.kind == "attempt-start")
                .map(|e| e.attempt)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                ids.len(),
                events.iter().filter(|e| e.kind == "attempt-start").count()
            );
            for event in events.iter().filter(|e| e.kind == "attempt-start") {
                let parent = event.data["parent"].as_u64().unwrap();
                assert!(parent == 0 || ids.contains(&parent));
                assert!(parent < event.attempt);
            }
        }
    }
}
