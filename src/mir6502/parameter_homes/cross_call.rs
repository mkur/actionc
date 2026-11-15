//! One bounded proposal for homes across an ordinary call. Selected callee facts
//! filter candidates; the resumed executable must establish its own proofs.
use super::*;

pub(super) fn propose(
    input: &MirProgram,
    selected: &MirProgram,
    trace: &super::super::trace::Trace,
) -> Option<MirProgram> {
    let candidates: Vec<_> = input
        .routines
        .iter()
        .enumerate()
        .filter_map(|(index, routine)| {
            if !eligible_with_calls(input, routine, true) {
                return None;
            }
            let mut calls = routine
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .filter(|op| matches!(op, MirOp::Call { .. }));
            let call = calls.next()?;
            calls.next().is_none().then_some((index, routine, call))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let mut facts = ResidenceCalls::new(selected, MirCalleeEntryStage::ActualEntry);
    let mut proposal = None;
    for (index, routine, call) in candidates {
        let writes = match facts.writes(call) {
            Ok(writes) => writes,
            Err(reason) => {
                trace.event(
                    "cross-call-candidate",
                    || serde_json::json!({"routine": routine.name, "reason": reason}),
                );
                continue;
            }
        };
        let forbidden = (0..=255u8)
            .filter(|slot| !writes.preserves_fixed_byte(MirFixedZpSlot(*slot)))
            .collect();
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
        if let Some(bindings) = zero_page::local_bindings(input, &candidate, &forbidden) {
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
            }));
            proposal.get_or_insert_with(|| input.clone()).routines[index] = candidate;
        }
    }
    proposal
}
