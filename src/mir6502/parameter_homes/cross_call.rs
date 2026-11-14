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
            // Keep already-elided captures on their selected path. This first
            // scope targets existing inline parameter storage.
            let old = selected.routines.iter().find(|r| r.id == routine.id)?;
            if !old.frame.params.iter().any(|p| matches!(p.base, MirStorageBase::Param(id)
                if old.frame.parameter_has_memory(id) && matches!(old.frame.parameter_home(id), None | Some(MirParamHome::Inline)))) {
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
        let Some(bindings) = zero_page::bindings_avoiding(input, routine, &forbidden) else {
            continue;
        };
        trace.event(
            "cross-call-candidate",
            || serde_json::json!({"routine": routine.name, "bindings": format!("{bindings:?}")}),
        );
        let r = &mut proposal.get_or_insert_with(|| input.clone()).routines[index];
        install_bindings(r, bindings);
    }
    proposal
}
