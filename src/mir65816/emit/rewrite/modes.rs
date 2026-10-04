//! A closed transformation of width requests, never an opcode-byte peephole.
use super::super::{
    Code, analysis::sites::Node, layout, selected::*, tracked::TrackedEmitter65816,
};
use std::collections::BTreeSet;
use std::ops::Range;

fn roots(records: &[Record]) -> Result<Vec<Range<usize>>, String> {
    let mut ranges = Vec::new();
    let mut start = 1;
    for (i, record) in records.iter().enumerate().skip(1) {
        if record.parent.is_none() && i != start {
            ranges.push(start..i);
            start = i;
        }
    }
    ranges.push(start..records.len());
    for range in &ranges {
        if let Action::Request(_) = records[range.start].action {
            if !matches!(records[range.end - 1].action, Action::EndRequest(n) if n == Node(range.start))
            {
                return Err("incomplete mode rewrite request".into());
            }
        } else if range.len() != 1 {
            return Err("invalid mode rewrite root".into());
        }
    }
    Ok(ranges)
}

fn bookkeeping(record: &Record) -> bool {
    matches!(
        record.action,
        Action::Allocate(_)
            | Action::SourceStart(_)
            | Action::SourceEnd { .. }
            | Action::Request(Request::Barrier | Request::ForgetPointer)
    )
}

fn overwritten(records: &[Record], roots: &[Range<usize>]) -> BTreeSet<usize> {
    let mut removed = BTreeSet::new();
    let mut pending = None;
    for range in roots {
        if matches!(
            records[range.start].action,
            Action::Request(Request::Mode(_))
        ) {
            if let Some(previous) = pending.take() {
                removed.insert(previous);
            }
            // Only a complete request containing exactly REP/SEP #$20 qualifies.
            // A zero-byte request can overwrite a preceding request too: fresh
            // emission will materialize its width if the deletion requires it.
            if range.len() == 3
                && matches!(
                    records[range.start + 1].action,
                    Action::Instruction {
                        form: Instruction::Byte(ByteOp::Rep | ByteOp::Sep, 0x20),
                        ..
                    }
                )
            {
                pending = Some(range.start);
            }
        } else if !bookkeeping(&records[range.start]) {
            pending = None;
        }
    }
    removed
}

fn validate_mode_requests(records: &[Record], roots: &[Range<usize>]) -> Result<(), String> {
    for range in roots {
        if let Action::Request(Request::Mode(width)) = records[range.start].action {
            let expected = match width {
                super::super::state::Width::Byte => ByteOp::Sep,
                super::super::state::Width::Word => ByteOp::Rep,
            };
            if !(range.len() == 2
                || range.len() == 3
                    && matches!(records[range.start + 1].action,
                Action::Instruction { form: Instruction::Byte(op, 0x20), .. } if op == expected))
            {
                return Err("mode rewrite requires a pure accumulator-width request".into());
            }
        }
    }
    Ok(())
}

pub(in crate::mir65816::emit) fn apply(code: Code, trace: bool) -> Result<Code, String> {
    let selected = code.selected.as_ref().ok_or("missing mode selection")?;
    selected.reconcile(&code)?;
    let records = selected.records();
    let roots = roots(records)?;
    validate_mode_requests(records, &roots)?;
    let removed = overwritten(records, &roots);
    if removed.is_empty() {
        return Ok(code);
    }
    let mut emitter = TrackedEmitter65816::default();
    #[cfg(feature = "native65816-state-proof")]
    if trace {
        emitter.trace();
    }
    let _ = trace;
    let mut source = None;
    for range in roots {
        let record = &records[range.start];
        if removed.contains(&range.start) {
            continue;
        }
        if matches!(record.action, Action::ReturnExit | Action::FaultExit) {
            continue;
        }
        // Only the erased interval may differ in M. All real consumers keep
        // their original widths/environment; compiler requests recompute their
        // own witnesses. Extra stack/value knowledge is not copied from records.
        let mut actual = emitter.boundary().env;
        if bookkeeping(record) || matches!(record.action, Action::Request(Request::Mode(_))) {
            actual.m = record.before.env.m;
        }
        if actual != record.before.env {
            return Err("mode rewrite changed an instruction environment".into());
        }
        match &record.action {
            Action::Instruction { form, .. } => emitter.instruction(form.clone())?,
            Action::Request(request) => request.replay(&mut emitter),
            Action::Allocate(label) => {
                if emitter.label() != *label {
                    return Err("mode rewrite changed label identity".into());
                }
            }
            Action::Bind(label) => emitter.mark(*label),
            Action::SourceStart(s) => {
                if source.replace((*s, emitter.position())).is_some() {
                    return Err("nested mode rewrite source".into());
                }
                emitter.begin_source(s.block, s.index);
            }
            Action::SourceEnd {
                source: s,
                fused_terminator,
            } => {
                let (start, pc) = source.take().ok_or("missing mode rewrite source")?;
                if start != *s {
                    return Err("mode rewrite changed source identity".into());
                }
                if let Some(t) = fused_terminator {
                    emitter.fused_span(s.block, s.index, pc, *t);
                } else {
                    emitter.span(s.block, s.index, pc);
                }
            }
            _ => return Err("unexpected mode rewrite action".into()),
        }
    }
    let mut scratch = layout::finalize(emitter.finish_reselected(selected)?, true)?;
    if scratch.bytes.len() >= code.bytes.len() {
        return Ok(code);
    }
    #[cfg(feature = "native65816-state-proof")]
    {
        scratch.rewrite_observations = code.rewrite_observations;
    }
    #[cfg(not(feature = "native65816-state-proof"))]
    let _ = &mut scratch;
    Ok(scratch)
}

#[cfg(test)]
mod tests;
