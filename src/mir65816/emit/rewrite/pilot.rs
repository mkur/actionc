//! Typed load candidates exist before omission. Shadow qualification compares
//! the checked rule with the historical predicate for every planning request.
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
        let end = selected
            .records()
            .get(candidate.request.0 + 1)
            .ok_or("missing planned consume end")?;
        if end.action != Action::EndRequest(candidate.request) {
            return Err("candidate is not a complete consume request".into());
        }
        let old = end.decision.ok_or("candidate has no observed decision")?;
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
