//! One bounded proposal for homes across ordinary leaf calls. Selected callee facts
//! filter candidates; the resumed executable must establish its own proofs.
use super::*;

pub(super) fn propose(
    input: &MirProgram,
    selected: &MirProgram,
    multiple: bool,
    trace: &super::super::trace::Trace,
) -> Option<MirProgram> {
    let addresses = ObjectAddressControl::new(input);
    let unresolved_observation = physical::unresolved_observation(input);
    let candidates: Vec<_> = input
        .routines
        .iter()
        .enumerate()
        .filter_map(|(index, routine)| {
            if !eligible_with_calls(&addresses, routine, true) {
                if trace.enabled() && routine.blocks.iter().flat_map(|b| &b.ops)
                    .any(|op| matches!(op, MirOp::Call { .. }))
                {
                    trace.event("cross-call-ineligible", || serde_json::json!({
                        "routine": routine.name,
                        "reason": "unsupported caller body or signature",
                        "indirect_writes": routine.blocks.iter().flat_map(|b| &b.ops)
                            .any(|op| super::super::analysis::effects::classify_op(op).memory.indirect_writes),
                        "machine_blocks": routine.blocks.iter().flat_map(|b| &b.ops)
                            .any(|op| matches!(op, MirOp::MachineBlock { .. })),
                    }));
                }
                return None;
            }
            if unresolved_observation.is_some() && physical::has_pointee_writes(routine) {
                trace.event("cross-call-ineligible", || serde_json::json!({
                    "routine": routine.name, "reason": "unresolved explicit observation",
                    "observation": unresolved_observation,
                }));
                return None;
            }
            let calls: Vec<_> = routine
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(op, MirOp::Call { .. }))
                .collect();
            (!calls.is_empty() && (multiple || calls.len() == 1))
                .then_some((index, routine, calls))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let mut facts = ResidenceCalls::new(selected, MirCalleeEntryStage::ActualEntry);
    let mut proposal = None;
    'caller: for (index, routine, calls) in candidates {
        let mut forbidden = BTreeSet::new();
        for (call_index, call) in calls.iter().enumerate() {
            // Entry facts may be cached by target; argument supply is checked
            // for each site, including repeated targets, before placing anything.
            let writes = match facts.writes(call) {
                Ok(writes) => writes,
                Err(reason) => {
                    trace.event("cross-call-candidate", || serde_json::json!({
                        "routine": routine.name, "reason": reason,
                        "call_count": calls.len(), "call_index": call_index,
                    }));
                    continue 'caller;
                }
            };
            forbidden.extend((0..=255u8)
                .filter(|slot| !writes.preserves_fixed_byte(MirFixedZpSlot(*slot))));
        }
        let mut candidate = routine.clone();
        // Do not recreate parameter memory already eliminated by the selected
        // path. Such a routine may still have an eligible source local.
        let ordinary_params = selected.routines.iter().find(|r| r.id == routine.id).is_some_and(|r|
            r.frame.params.iter().any(|p| matches!(p.base, MirStorageBase::Param(id)
                if r.frame.parameter_has_memory(id) && matches!(r.frame.parameter_home(id), None | Some(MirParamHome::Inline)))));
        let mut changed = false;
        if ordinary_params
            && let Some(bindings) = zero_page::bindings_avoiding(input, &candidate, &forbidden)
        {
            install_bindings(&mut candidate, bindings);
            changed = true;
        }
        if let Some(bindings) = zero_page::local_bindings(input, &candidate, &forbidden, &addresses) {
            candidate.frame.local_homes = Some(bindings);
            for (_, slot) in candidate.frame.resident_bytes() {
                if !candidate.frame.fixed_zero_page.contains(&slot) {
                    candidate.frame.fixed_zero_page.push(slot);
                }
            }
            changed = true;
        }
        if changed {
            trace.event("cross-call-candidate", || serde_json::json!({
                "routine": routine.name, "bindings": format!("{:?}", candidate.frame.parameter_homes),
                "local_bindings": format!("{:?}", candidate.frame.local_homes),
                "call_count": calls.len(),
            }));
            proposal.get_or_insert_with(|| input.clone()).routines[index] = candidate;
        }
    }
    proposal
}

#[cfg(test)]
mod tests;
