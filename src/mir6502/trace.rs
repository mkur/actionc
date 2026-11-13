//! Bounded, compilation-local diagnostics. Nothing here grants rewrite permission.
use std::cell::RefCell;
use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{Value, json};

use super::{MirDiagnostic, MirProgram, RoutineId};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TraceOptions {
    /// Exact display names or `r123` IDs; used only to filter diagnostics.
    pub routines: Vec<String>,
    pub max_bytes: usize,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self {
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
    events: Vec<TraceEvent>,
    bytes: usize,
    next_id: u64,
    selected: BTreeSet<RoutineId>,
    filter_errors: Vec<String>,
    truncated_at: Option<String>,
    finished: bool,
    failed: bool,
    output_identity: Option<String>,
}

/// A caller-owned bounded collector. A generation call resets it before use.
pub struct Mir6502Trace {
    options: TraceOptions,
    state: RefCell<State>,
}

impl Mir6502Trace {
    pub fn new(options: TraceOptions) -> Self {
        Self {
            options,
            state: RefCell::new(State::default()),
        }
    }

    pub fn events(&self) -> Vec<TraceEvent> {
        self.state.borrow().events.clone()
    }

    pub fn manifest(&self) -> Value {
        let s = self.state.borrow();
        json!({
            "schema_version": SCHEMA_VERSION,
            "status": if !s.finished { "incomplete" } else if s.failed { "compilation-failed" }
                else if !s.filter_errors.is_empty() { "filter-error" }
                else if s.truncated_at.is_some() { "truncated" } else { "complete" },
            "detail_bytes": s.bytes, "max_bytes": self.options.max_bytes,
            "first_omitted_stage": s.truncated_at, "filter_errors": s.filter_errors,
            "output_identity": s.output_identity, "identity_algorithm": "fnv1a64",
            "selected_routines": s.selected.iter().map(|r| r.0).collect::<Vec<_>>(),
            "final_snapshot": s.events.iter().rev().find(|e| e.kind == "snapshot" && e.data["stage"] == "pre-emission").map(|e| e.id)
        })
    }

    pub(super) fn reset(&mut self) {
        *self.state.get_mut() = State::default();
    }
    pub(super) fn context(&self) -> Trace<'_> {
        Trace {
            session: Some(self),
            attempt: 0,
        }
    }
    pub(super) fn finish(
        &self,
        result: &Result<crate::codegen::CodegenOutput, Vec<MirDiagnostic>>,
    ) {
        let mut s = self.state.borrow_mut();
        s.finished = true;
        s.failed = result.is_err();
        s.output_identity = result
            .as_ref()
            .ok()
            .map(|o| identity_bytes(&crate::codegen::format_load_file(o)));
    }
}

impl Default for Mir6502Trace {
    fn default() -> Self {
        Self::new(TraceOptions::default())
    }
}

/// Explicitly passed observation capability. Disabled contexts do not evaluate payloads.
#[derive(Clone, Copy, Default)]
pub(crate) struct Trace<'a> {
    session: Option<&'a Mir6502Trace>,
    attempt: u64,
}

impl<'a> Trace<'a> {
    pub(crate) const fn disabled() -> Self {
        Self {
            session: None,
            attempt: 0,
        }
    }
    pub(crate) fn enabled(self) -> bool {
        self.session
            .is_some_and(|s| s.state.borrow().truncated_at.is_none())
    }

    pub(crate) fn event(self, kind: &'static str, data: impl FnOnce() -> Value) -> Option<u64> {
        let session = self.session?;
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

    pub(crate) fn bind_routines(self, program: &MirProgram) {
        let Some(session) = self.session else {
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

    pub(crate) fn wants(self, routine: RoutineId) -> bool {
        self.enabled()
            && self.session.is_some_and(|s| {
                s.options.routines.is_empty() || s.state.borrow().selected.contains(&routine)
            })
    }

    pub(crate) fn snapshot(self, stage: &'static str, program: &MirProgram) -> Option<u64> {
        self.event("snapshot", || {
            let mut selected = program.clone();
            selected.routines.retain(|r| self.wants(r.id));
            json!({ "stage": stage, "scope": "program", "mir": super::format_program(&selected),
                "routines": selected.routines.iter().map(|r| json!({"id":r.id.0,"name":r.name})).collect::<Vec<_>>() })
        })
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
}
