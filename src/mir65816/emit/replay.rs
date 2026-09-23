//! Replay compiler-owned typed actions through a fresh tracked boundary.
//! Compound requests regenerate their children; those children are checked,
//! never submitted a second time. Stored states/decisions are observations only.
use super::{
    Code,
    analysis::{cfg::SelectedCfg, sites::Node},
    selected::*,
    tracked::TrackedEmitter65816,
};

fn same_action(a: &Record, b: &Record) -> bool {
    a.action == b.action
        && a.parent == b.parent
        && a.source == b.source
        && a.before == b.before
        && a.after == b.after
        && a.decision == b.decision
}

pub(super) fn emit(selected: &SelectedRoutine, _trace: bool) -> Result<Code, String> {
    #[cfg(any(test, feature = "native65816-state-proof"))]
    crate::mir65816::emit::work::add("full_replay", 1);
    let emitter = walk(selected, _trace, None)?;
    let code = emitter.finish_replayed(selected)?;
    let fresh = code.selected.as_ref().ok_or("missing replay selection")?;
    if fresh.records().len() != selected.records().len()
        || selected
            .records()
            .iter()
            .zip(fresh.records())
            .any(|(a, b)| !same_action(a, b))
    {
        return Err("replayed selection differs from its typed contract".into());
    }
    Ok(code)
}

pub(super) fn prefix(
    selected: &SelectedRoutine,
    stop: Node,
) -> Result<TrackedEmitter65816, String> {
    #[cfg(any(test, feature = "native65816-state-proof"))]
    super::work::add("prefix_replay", 1);
    selected.site(stop)?;
    walk(selected, false, Some(stop))
}

fn walk(
    selected: &SelectedRoutine,
    _trace: bool,
    stop: Option<Node>,
) -> Result<TrackedEmitter65816, String> {
    SelectedCfg::build(selected.records())?;
    let mut emitter = TrackedEmitter65816::default();
    #[cfg(feature = "native65816-state-proof")]
    if _trace {
        emitter.trace();
    }
    let records = selected.records();
    if !same_action(&records[0], &emitter.recorded()[0]) {
        return Err("replay entry is not the verified native entry contract".into());
    }
    let mut source = None;
    let mut index = 1;
    while index < records.len() {
        if stop == Some(Node(index)) {
            return Ok(emitter);
        }
        #[cfg(any(test, feature = "native65816-state-proof"))]
        super::work::add(
            if stop.is_some() {
                "prefix_actions"
            } else {
                "full_actions"
            },
            1,
        );
        let record = &records[index];
        if matches!(record.action, Action::ReturnExit | Action::FaultExit) {
            index += 1;
            continue;
        }
        if record.parent.is_some() || emitter.recorded().len() != index {
            return Err("replay requires a complete top-level action".into());
        }
        if emitter.boundary() != record.before {
            return Err(format!(
                "fresh replay environment differs before site {index}"
            ));
        }
        let end = if matches!(record.action, Action::Request(_)) {
            records
                .iter()
                .enumerate()
                .skip(index + 1)
                .find_map(|(i, r)| {
                    matches!(r.action,Action::EndRequest(n) if n==Node(index)).then_some(i + 1)
                })
                .ok_or("unterminated replay request")?
        } else {
            index + 1
        };
        match &record.action {
            Action::Instruction { form, .. } => emitter.instruction(form.clone())?,
            Action::Request(request) => request.replay(&mut emitter),
            Action::Allocate(label) => {
                if emitter.label() != *label {
                    return Err("replay label identity mismatch".into());
                }
            }
            Action::Bind(label) => emitter.mark(*label),
            Action::SourceStart(s) => {
                if source.replace((*s, emitter.position())).is_some() {
                    return Err("nested replay source".into());
                }
                emitter.begin_source(s.block, s.index);
            }
            Action::SourceEnd {
                source: s,
                fused_terminator,
            } => {
                let (start, pc) = source.take().ok_or("missing replay source start")?;
                if start != *s {
                    return Err("replay source identity mismatch".into());
                }
                if let Some(t) = fused_terminator {
                    emitter.fused_span(s.block, s.index, pc, *t);
                } else {
                    emitter.span(s.block, s.index, pc);
                }
            }
            Action::Entry | Action::EndRequest(_) | Action::ReturnExit | Action::FaultExit => {
                return Err("unexpected replay action".into());
            }
        }
        if emitter.recorded().len() != end
            || records[index..end]
                .iter()
                .zip(&emitter.recorded()[index..])
                .any(|(a, b)| !same_action(a, b))
        {
            return Err(format!(
                "fresh replay actions or consume decisions differ at site {index}"
            ));
        }
        index = end;
    }
    if source.is_some() {
        return Err("unfinished replay source".into());
    }
    if stop.is_some() {
        return Err("replay prefix is not a top-level action".into());
    }
    Ok(emitter)
}

impl Request {
    fn replay(&self, e: &mut TrackedEmitter65816) {
        match self {
            Self::Mode(super::state::Width::Byte) => e.a8(),
            Self::Mode(super::state::Width::Word) => e.a16(),
            Self::EstablishBody => e.establish_body(),
            Self::Barrier => e.barrier(),
            Self::RegisterHome(h) => e.register_home(*h),
            Self::DeclareBlocks(labels) => e.declare_blocks(labels.iter().copied()),
            Self::ProveEntries {
                predecessors,
                reachable,
            } => e.prove_entries(predecessors.clone(), reachable.clone()),
            Self::RememberWord(t, h) => e.remember_word(*t, *h),
            Self::ConsumeWord(t, h, o) => {
                e.consume_word(*t, *h, *o);
            }
            Self::RememberFrame(o, b, h) => e.remember_frame_word(*o, *b, *h),
            Self::ConsumeFrame(o, b, h, d) => {
                e.consume_frame_word(*o, *b, *h, *d);
            }
            Self::CaptureIncoming(p, s, t, d) => e.capture_incoming_word(*p, *s, *t, *d),
            Self::StoreIncoming(t, s, d) => {
                e.store_incoming_capture(*t, *s, *d);
            }
            Self::ProveX(c) => e.prove_x(c.clone()),
            Self::RefreshX => e.refresh_x(),
            Self::LoadX(t, h) => {
                e.load_x_word(*t, *h);
            }
            Self::CompareX(t, h, n) => e.compare_x_word(*t, *h, *n),
            Self::IncrementX(t, h, u, d) => e.increment_x_word(*t, *h, *u, *d),
            Self::Jump(l) => e.jump(*l),
            Self::Fallthrough(l) => e.fallthrough(*l),
            Self::Dispatch(b, l) => e.dispatch(*b, *l),
        }
    }
}

/// Exact output check for the direct reference path and replay qualification.
/// Selected identities are checked separately; no semantic field is normalized.
pub(super) fn equivalent(a: &Code, b: &Code) -> Result<(), String> {
    if a.bytes != b.bytes
        || a.fixups != b.fixups
        || a.labels != b.labels
        || a.return_fixups != b.return_fixups
        || a.mir_spans != b.mir_spans
        || a.mir_transfers != b.mir_transfers
        || a.conditional_branches != b.conditional_branches
        || a.boundaries != b.boundaries
    {
        return Err("replay encoding or position metadata differs".into());
    }
    #[cfg(feature = "native65816-state-proof")]
    if a.state_trace != b.state_trace || a.instruction_effects != b.instruction_effects {
        return Err("replay proof observations differ".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
