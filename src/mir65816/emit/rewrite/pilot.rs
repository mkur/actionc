//! Typed load candidates exist before omission. The private planning projection
//! cannot publish output; the driver must validate each actual load removal.
use super::super::{
    Code, Location, TempId,
    analysis::sites::Node,
    layout, replay,
    selected::{Action, Instruction},
};
use super::{
    context::Context,
    driver::{self, Driver},
    rules,
};

#[derive(Clone, Debug)]
pub(in crate::mir65816::emit) struct Candidate {
    pub request: Node,
    pub temp: Option<TempId>,
    pub home: Option<Location>,
    pub load: Instruction,
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
pub(in crate::mir65816::emit) fn shadow(
    code: &Code,
    candidates: &[Candidate],
    trace: bool,
) -> Result<Vec<Observation>, String> {
    let selected = code.selected.as_ref().ok_or("missing planned selection")?;
    let context = Context::new(selected)?;
    let reference = layout::finalize(code.clone(), true)?;
    let mut observations = Vec::new();
    for candidate in candidates {
        let request = selected.site(candidate.request)?;
        let old = decision(selected, candidate)?;
        let proof = context
            .adjacent_load(request, candidate.temp, candidate.home, &candidate.load)
            .into_result();
        if old != proof.is_ok() {
            return Err(format!(
                "adjacent shadow mismatch at {}: old={old}, checked={proof:?}",
                candidate.request.0
            ));
        }
        if old {
            // Analyze an actual original LDA, never infer read removability from
            // the projection in which the old predicate already omitted it.
            let load_node = Node(candidate.request.0 + 2);
            let original = driver::insert_load(selected, load_node, &candidate.load)?;
            let mut scratch = layout::finalize(replay::emit(&original, trace)?, true)?;
            let s = scratch
                .selected
                .as_ref()
                .ok_or("missing original candidate")?;
            let plan = rules::adjacent(&Context::new(s)?, s.site(load_node)?)
                .into_result()
                .map_err(|b| b.reason)?;
            Driver::new(1)
                .apply(&mut scratch, &plan, trace)
                .into_result()
                .map_err(|b| b.reason)?;
            replay::equivalent(&reference, &scratch)?;
        }
        observations.push(Observation {
            request_ordinal: candidate.request.0,
            accepted: old,
            reason: proof
                .err()
                .map_or_else(|| "proven A16/home/NZ equivalence".into(), |b| b.reason),
            original_load: kind(&candidate.load),
            private_read_bytes_removed: if old { 2 } else { 0 },
        });
    }
    Ok(observations)
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
    if !candidates.is_empty() {
        let context = Context::new(selected)?;
        for (index, candidate) in candidates.iter().enumerate() {
            let request = selected.site(candidate.request)?;
            let projected = decision(selected, candidate)?;
            let proof = context
                .adjacent_load(request, candidate.temp, candidate.home, &candidate.load)
                .into_result();
            if projected {
                pending.push(index);
            } else if proof.is_ok() {
                return Err("planning rejected a proved adjacent load".into());
            }
            observations.push(Observation {
                request_ordinal: candidate.request.0,
                accepted: false,
                reason: proof
                    .err()
                    .map_or_else(|| "awaiting checked transaction".into(), |b| b.reason),
                original_load: kind(&candidate.load),
                private_read_bytes_removed: 0,
            });
        }
    }
    // The complete original planned stream includes every projected-away LDA.
    // Insert backwards so its existing symbolic request ordinals remain usable.
    let mut original = (**selected).clone();
    for &index in pending.iter().rev() {
        let candidate = &candidates[index];
        original = driver::insert_load(&original, Node(candidate.request.0 + 2), &candidate.load)?;
    }
    let mut scratch = layout::finalize(replay::emit(&original, trace)?, true)?;
    let mut driver = Driver::new(pending.len());
    let mut retained = 0;
    for index in pending {
        let candidate = &candidates[index];
        let s = scratch
            .selected
            .as_ref()
            .ok_or("missing original candidate selection")?;
        let load = s.site(Node(candidate.request.0 + 2 + retained))?;
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
