//! One bounded proposal for homes across supported native calls. Selected callee facts
//! filter candidates; the resumed executable must establish its own proofs.
use super::*;
use crate::mir6502::analysis::sites::MirSite;
use std::collections::BTreeMap;
mod chains;

#[cfg(test)]
pub(super) fn propose(
    input: &MirProgram,
    selected: &MirProgram,
    multiple: bool,
    lifetimes: bool,
    trace: &super::super::trace::Trace,
) -> Option<MirProgram> {
    propose_with_scope(input, selected, multiple, lifetimes, false, trace)
}

#[cfg(test)]
pub(super) fn propose_with_scope(
    input: &MirProgram, selected: &MirProgram, multiple: bool, lifetimes: bool,
    acyclic: bool, trace: &super::super::trace::Trace,
) -> Option<MirProgram> {
    let config = crate::mir6502::Mir6502Config {
        enable_multiple_leaf_call_homes: multiple,
        enable_zp_home_lifetimes: lifetimes,
        enable_acyclic_call_homes: acyclic,
        ..crate::mir6502::Mir6502Config::optimized()
    };
    propose_configured(input, selected, &config, 0x3000, trace)
}

pub(super) fn propose_configured(
    input: &MirProgram, selected: &MirProgram, config: &crate::mir6502::Mir6502Config,
    origin: u16, trace: &super::super::trace::Trace,
) -> Option<MirProgram> {
    let multiple = config.enable_multiple_leaf_call_homes;
    let lifetimes = config.enable_zp_home_lifetimes;
    let acyclic = config.acyclic_call_homes_enabled();
    let addresses = ObjectAddressControl::new(input);
    let unresolved_observation = physical::unresolved_observation(input);
    // Layout-independent numeric observations are already authoritative. Avoid
    // known conflicts before the one combined trial; symbolic/old-home overlaps
    // still require final-layout verification and exact rollback.
    let observed_zp: BTreeSet<u8> = if lifetimes {
        let observations = physical::numeric_observations(input, None).unwrap_or_default();
        (0..=255u16).filter(|byte| observations.iter().any(|(start, bytes)|
            byte.wrapping_sub(*start) < *bytes)).map(|b| b as u8).collect()
    } else { BTreeSet::new() };
    // Bindings and source exposure belong to the retained input. Only call
    // enumeration and site-based analyses use the expanded planning routine.
    let mut views = BTreeMap::new();
    if lifetimes && config.select_runtime_helpers {
        let layout = std::cell::LazyCell::new(|| super::super::materialize::MaterializeLayout::new(input, origin));
        for actual in &selected.routines {
            let Some(retained) = input.routines.iter().find(|r| r.id == actual.id) else { continue };
            if let Some(plan) = super::super::materialize::arithmetic_homes::plan(retained, actual, selected, config, &layout) {
                views.insert(retained.id, plan);
            }
        }
    }
    let candidates: Vec<_> = input
        .routines
        .iter()
        .enumerate()
        .filter_map(|(index, retained)| {
            let routine = views.get(&retained.id).map_or(retained, |plan| &plan.routine);
            if let Err(rejection) = admission::check(&addresses, routine, true, lifetimes, lifetimes.then_some(selected)) {
                if trace.enabled() && routine.blocks.iter().flat_map(|b| &b.ops)
                    .any(|op| matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. }))
                {
                    trace.event("cross-call-ineligible", || serde_json::json!({
                        "routine": routine.name,
                        "reason": rejection.reason.description(),
                        "site": rejection.site.map(|site| format!("{site:?}")),
                        "operation": rejection.site.and_then(|site| match site {
                            MirSite::Op { block, op_index } => routine.blocks.iter().find(|b| b.id == block)
                                .and_then(|b| b.ops.get(op_index)).map(|op| format!("{op:?}")),
                            _ => None,
                        }),
                        "indirect_writes": routine.blocks.iter().flat_map(|b| &b.ops)
                            .any(|op| super::super::analysis::effects::classify_op(op).memory.indirect_writes),
                        "machine_blocks": routine.blocks.iter().flat_map(|b| &b.ops)
                            .any(|op| matches!(op, MirOp::MachineBlock { .. })),
                    }));
                }
                return None;
            }
            if unresolved_observation.is_some() && physical::has_pointee_writes(input, retained) {
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
                    matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. }).then_some((MirSite::Op { block: b.id, op_index }, op))
                }))
                .collect();
            (!calls.is_empty() && (multiple || calls.len() == 1))
                .then_some((index, routine, calls))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let chains = acyclic.then(|| chains::CallChains::new(input, selected));
    let order = chains.as_ref().map(|c| c.order.clone())
        .unwrap_or_else(|| candidates.iter().map(|(_, r, _)| r.id).collect());
    let mut candidates: BTreeMap<_, _> = candidates.into_iter().map(|c| (c.1.id, c)).collect();
    let mut planned = BTreeMap::new();
    let mut facts = if acyclic { ResidenceCalls::at_origin(selected, MirCalleeEntryStage::ActualEntry, origin) }
        else { ResidenceCalls::new(selected, MirCalleeEntryStage::ActualEntry) };
    let fixed = facts.fixed_storage();
    let mut proposal = None;
    'caller: for id in order {
        // Propagate through inline/otherwise ineligible middle routines too.
        if let Some(chains) = &chains { planned.insert(id, chains.inherited(id, &planned)); }
        let Some((index, routine, calls)) = candidates.remove(&id) else { continue; };
        let logical = if lifetimes {
            let Some(logical) = logical::LogicalHomes::new(routine) else { continue; };
            Some(logical)
        } else { None };
        let mut call_writes = Vec::new();
        let mut fixed_inputs = BTreeSet::new();
        for (call_index, (site, call)) in calls.iter().enumerate() {
            if lifetimes {
                match facts.inputs(call) {
                    Ok(inputs) => fixed_inputs.extend(inputs),
                    Err(reason) => {
                        trace.event("cross-call-candidate", || serde_json::json!({
                            "routine": routine.name, "reason": reason,
                            "entry": facts.entry_description(call),
                            "call_count": calls.len(), "call_index": call_index,
                        }));
                        continue 'caller;
                    }
                }
            }
            // Candidate objects are checked individually below by the existing
            // ownership-aware binding selectors. Explicit ranges remain here.
            let writes = facts.writes_at(call, Some((routine, *site))).map(|writes| (0..=255u8)
                .filter(|slot| !writes.preserves_private_fixed_byte(MirFixedZpSlot(*slot), &fixed))
                .collect::<BTreeSet<_>>()).and_then(|mut writes| {
                    // The caller stages arguments before the callee begins. An
                    // outgoing ZP lane can clobber a live object even if the
                    // reviewed routine preserves that byte on return.
                    if let MirOp::Call { args, .. } = call {
                        for arg in args {
                            let lanes = super::super::abi::physical_lanes(&arg.home, arg.width)
                                .ok_or("unsupported outgoing argument lanes")?;
                            writes.extend(lanes.into_iter().filter_map(|lane| match lane {
                                MirParamLane::ZeroPage(slot) => Some(slot.0),
                                MirParamLane::Reg(_) => None,
                            }));
                        }
                    }
                    if let Some(chains) = &chains
                        && matches!(call, MirOp::Call { .. })
                        && MirAtariFppService::from_canonical_call(call).is_none()
                        && super::super::runtime_contracts::library_invocation(selected, call).is_none() {
                        let MirOp::Call { target: MirCallTarget::Routine(target), .. } = call else {
                            return Err("unknown planning dependency");
                        };
                        if !chains.supports(*target) { return Err("recursive or missing retained call dependency"); }
                        if let Some(bytes) = planned.get(target) { writes.extend(bytes); }
                    }
                    Ok(writes)
                });
            if acyclic && writes.is_err() {
                trace.event("cross-call-dependency", || serde_json::json!({
                    "routine": routine.name, "site": format!("{site:?}"),
                    "reason": writes.as_ref().err(), "entry": facts.entry_description(call),
                }));
            }
            if !lifetimes && let Err(reason) = writes {
                trace.event("cross-call-candidate", || serde_json::json!({
                    "routine": routine.name, "reason": reason,
                    "call_count": calls.len(), "call_index": call_index,
                }));
                continue 'caller;
            }
            let results: BTreeSet<_> = if matches!(call, MirOp::RuntimeHelper { .. }) {
                // Admission checked this retained operation against the selected
                // implementation. Candidate evidence is rebuilt after expansion.
                super::super::runtime_contracts::helper_invocation(selected,call)
                    .expect("admitted helper contract").definitions.into_iter().filter_map(|m| match m {
                        MirMem::FixedZeroPage(s) => Some(s.0), _ => None,
                    }).collect()
            } else {
                super::super::analysis::home_liveness::call_return_home_uses(input,call)
                    .iter().filter_map(|h| match h {
                        super::super::analysis::effects::MirHomeByte::FixedZeroPage(s) => Some(s.0), _ => None,
                    }).collect()
            };
            call_writes.push((*site, writes, results));
        }
        let retained = &input.routines[index];
        let forbidden = |mem: &MirMem, width: u16| {
            if !addresses.compiler_controls_address(routine.id, mem) { return None; }
            let mut forbidden = if physical::has_pointee_writes(input, retained) {
                observed_zp.clone()
            } else { BTreeSet::new() };
            forbidden.extend(&fixed_inputs);
            if let Some(plan) = views.get(&id) {
                forbidden.extend(&plan.setup_writes);
            }
            for (site, writes, results) in &call_writes {
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
                    forbidden.extend(results);
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
        let mut candidate = retained.clone();
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
            if let Some(bytes) = planned.get_mut(&id) {
                bytes.extend(candidate.frame.resident_bytes().iter().map(|(_, slot)| slot.0));
                trace.event("cross-call-planned-subtree", || serde_json::json!({
                    "routine": routine.name, "routine_id": id.0, "resident_bytes": bytes,
                }));
            }
            proposal.get_or_insert_with(|| input.clone()).routines[index] = candidate;
        }
    }
    proposal
}

#[cfg(test)]
mod tests;
