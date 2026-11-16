//! One bounded proposal for homes across ordinary leaf calls. Selected callee facts
//! filter candidates; the resumed executable must establish its own proofs.
use super::*;
use crate::mir6502::analysis::sites::MirSite;

pub(super) fn propose(
    input: &MirProgram,
    selected: &MirProgram,
    multiple: bool,
    lifetimes: bool,
    trace: &super::super::trace::Trace,
) -> Option<MirProgram> {
    let addresses = ObjectAddressControl::new(input);
    let unresolved_observation = physical::unresolved_observation(input);
    let candidates: Vec<_> = input
        .routines
        .iter()
        .enumerate()
        .filter_map(|(index, routine)| {
            if !eligible_with_calls(&addresses, routine, true, lifetimes) {
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
                .flat_map(|b| b.ops.iter().enumerate().filter_map(|(op_index, op)| {
                    matches!(op, MirOp::Call { .. }).then_some((MirSite::Op { block: b.id, op_index }, op))
                }))
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
        let logical = if lifetimes {
            let Some(logical) = logical::LogicalHomes::new(routine) else { continue; };
            Some(logical)
        } else { None };
        let mut call_writes = Vec::new();
        for (call_index, (site, call)) in calls.iter().enumerate() {
            if lifetimes && let Err(reason) = facts.inputs(call) {
                trace.event("cross-call-candidate", || serde_json::json!({
                    "routine": routine.name, "reason": reason,
                    "call_count": calls.len(), "call_index": call_index,
                }));
                continue 'caller;
            }
            let writes = facts.writes(call).map(|writes| (0..=255u8)
                .filter(|slot| !writes.preserves_fixed_byte(MirFixedZpSlot(*slot)))
                .collect::<BTreeSet<_>>());
            if !lifetimes && let Err(reason) = writes {
                trace.event("cross-call-candidate", || serde_json::json!({
                    "routine": routine.name, "reason": reason,
                    "call_count": calls.len(), "call_index": call_index,
                }));
                continue 'caller;
            }
            call_writes.push((*site, writes));
        }
        let forbidden = |mem: &MirMem, width: u16| {
            let mut forbidden = BTreeSet::new();
            for (site, writes) in &call_writes {
                let crosses = (0..width).any(|offset| {
                    let lane = match *mem {
                        MirMem::Param { id, .. } => MirMem::Param { id, offset },
                        MirMem::Local { id, .. } => MirMem::Local { id, offset },
                        _ => return true,
                    };
                    logical.as_ref().is_none_or(|l| l.live_after(&lane, *site) != Some(false))
                });
                if !crosses {
                    // Reusing a return slot needs an explicit value handoff:
                    // later physical result reads are not reads of this object.
                    // Keep that separate from ordinary lifetime-based sharing.
                    forbidden.extend(super::super::analysis::home_liveness::action_return_home_uses()
                        .iter().filter_map(|h| match h {
                            super::super::analysis::effects::MirHomeByte::FixedZeroPage(s) => Some(s.0),
                            _ => None,
                        }));
                    continue;
                }
                match writes {
                    Ok(writes) => forbidden.extend(writes),
                    Err(reason) => {
                        trace.event("cross-call-home-ineligible", || serde_json::json!({
                            "routine": routine.name, "home": format!("{mem:?}"),
                            "site": format!("{site:?}"), "reason": reason,
                        }));
                        return None;
                    }
                }
            }
            Some(forbidden)
        };
        let mut candidate = routine.clone();
        // Do not recreate parameter memory already eliminated by the selected
        // path. Such a routine may still have an eligible source local.
        let ordinary_params = selected.routines.iter().find(|r| r.id == routine.id).is_some_and(|r|
            r.frame.params.iter().any(|p| matches!(p.base, MirStorageBase::Param(id)
                if r.frame.parameter_has_memory(id) && matches!(r.frame.parameter_home(id), None | Some(MirParamHome::Inline)))));
        let mut changed = false;
        if ordinary_params
            && let Some(bindings) = zero_page::bindings_avoiding(input, &candidate, &forbidden, &addresses)
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
