//! Typed load candidates exist before omission. The private planning projection
//! cannot publish output; the driver must validate each actual load removal.
use super::super::{
    Code, Location, TempId,
    analysis::sites::Node,
    layout, replay,
    selected::{Action, Instruction},
};
use super::{
    driver::{self, Driver},
    rules,
};

#[derive(Clone, Debug)]
pub(in crate::mir65816::emit) struct Candidate {
    pub request: Node,
    pub temp: Option<TempId>,
    pub home: Option<Location>,
    pub load: Instruction,
    /// Diagnostic from selection only; never a permission to delete a load.
    pub planning_blocker: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub request_ordinal: usize,
    pub accepted: bool,
    pub reason: String,
    pub original_load: &'static str,
    pub private_read_bytes_removed: u8,
}
fn kind(load: &Instruction) -> &'static str {
    use super::super::selected::{ByteOp, WordOp};
    match load {
        Instruction::Byte(ByteOp::LdaStack, _) => "stack-word",
        Instruction::Byte(ByteOp::LdaDp, _) => "dp-word",
        Instruction::Word(WordOp::LdaImm, _) => "immediate-word",
        _ => "unsupported",
    }
}
fn decision(
    selected: &super::super::selected::SelectedRoutine,
    candidate: &Candidate,
) -> Result<bool, String> {
    use super::super::{
        copies::WordHome,
        selected::{ByteOp, Request, WordOp},
    };
    let offset = match candidate.load {
        Instruction::Byte(ByteOp::LdaStack, n) => Some(WordHome::Stack(n)),
        Instruction::Byte(ByteOp::LdaDp, n) => Some(WordHome::DirectPage(n)),
        Instruction::Word(WordOp::LdaImm, _) => None,
        _ => return Err("unsupported planned load".into()),
    };
    selected.site(candidate.request)?;
    let record = &selected.records()[candidate.request.0];
    if record.parent.is_some()
        || record.action
            != Action::Request(Request::ConsumeWord(candidate.temp, candidate.home, offset))
    {
        return Err("planned load does not match its original consume inputs".into());
    }
    let end = selected
        .records()
        .get(candidate.request.0 + 1)
        .ok_or("missing planned consume end")?;
    if end.action != Action::EndRequest(candidate.request) {
        return Err("incomplete planned consume request".into());
    }
    end.decision
        .ok_or_else(|| "missing projected consume observation".into())
}
pub(in crate::mir65816::emit) fn apply(
    code: &Code,
    candidates: &[Candidate],
    trace: bool,
) -> Result<Code, String> {
    let selected = code.selected.as_ref().ok_or("missing planned selection")?;
    selected.reconcile(code)?;
    if candidates
        .windows(2)
        .any(|pair| pair[0].request.0 >= pair[1].request.0)
    {
        return Err("planned candidates are duplicated or out of order".into());
    }
    let mut observations = Vec::new();
    let mut pending = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if decision(selected, candidate)? {
            pending.push(index);
        }
        observations.push(Observation {
            request_ordinal: candidate.request.0,
            accepted: false,
            reason: candidate
                .planning_blocker
                .clone()
                .unwrap_or_else(|| "awaiting checked transaction".into()),
            original_load: kind(&candidate.load),
            private_read_bytes_removed: 0,
        });
    }
    // Restore all actual loads in one traversal. The projection supplies no
    // final permission: each removal below needs a fresh checked transaction.
    let loads = pending
        .iter()
        .map(|&index| {
            let candidate = &candidates[index];
            (Node(candidate.request.0 + 2), &candidate.load)
        })
        .collect::<Vec<_>>();
    let original = driver::expand_loads(selected, &loads)?;
    let mut scratch = layout::finalize(replay::emit(&original, trace)?, true)?;
    let mut driver = Driver::new(pending.len());
    let mut retained = 0;
    for index in pending {
        let candidate = &candidates[index];
        let s = scratch
            .selected
            .as_ref()
            .ok_or("missing original candidate selection")?;
        let load = rediscover(s, candidate, retained)?;
        // Rebuild context and rediscover after every accepted transaction; no
        // earlier generation's plan can authorize a later edit.
        let outcome = rules::candidate(s, load)
            .into_result()
            .and_then(|plan| driver.apply(&mut scratch, &plan, trace).into_result());
        let observation = &mut observations[index];
        match outcome {
            Ok(()) => {
                observation.accepted = true;
                observation.reason = "proven A16/home/NZ equivalence".into();
                observation.private_read_bytes_removed = 2;
            }
            Err(blocker) => {
                // The already-replayed original keeps its actual load and
                // verified continuation. Other candidates are rediscovered.
                retained += 1;
                observation.reason = blocker.reason;
            }
        }
    }
    #[cfg(feature = "native65816-state-proof")]
    {
        scratch.rewrite_observations = observations;
    }
    #[cfg(not(feature = "native65816-state-proof"))]
    let _ = observations;
    Ok(scratch)
}

/// Projected ordinals locate requests only. Check their exact correspondence
/// and mint a site from the current generation before proposing any removal.
fn rediscover(
    selected: &super::super::selected::SelectedRoutine,
    candidate: &Candidate,
    retained: usize,
) -> Result<super::super::analysis::sites::SelectedSite, String> {
    let request = candidate
        .request
        .0
        .checked_add(retained)
        .ok_or("candidate ordinal overflow")?;
    let current = Candidate {
        request: Node(request),
        ..candidate.clone()
    };
    if !decision(selected, &current)? {
        return Err("restored candidate lost its projected consume".into());
    }
    let node = Node(request + 2);
    let site = selected.site(node)?;
    let record = &selected.records()[node.0];
    if record.parent.is_some()
        || !matches!(
            &record.action, Action::Instruction { form, .. } if form == &candidate.load
        )
    {
        return Err("restored candidate does not match its actual load".into());
    }
    Ok(site)
}
