//! A stored scalar can remain unread across a complete, exact callee graph.
//! The proof is local to its lifetime; unrelated program effects are not facts
//! about this definition. Unexposed frames and ordinary bounded accesses remain
//! mandatory, including in every owning callee frame.
use super::*;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    effects::classify_terminator,
    known_callees::{MirKnownCalleeSummaries, MirKnownMemoryReads, MirKnownMemoryWrites},
};

const MAX_OPS: usize = 512;
const MAX_CALL_DEPTH: usize = 16;

pub(super) fn improve(
    selected: &mut MirProgram,
    emitted: &mut MirProgram,
    config: &Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
) -> super::super::MirPeepholeStats {
    let mut stats = super::super::MirPeepholeStats::default();
    // Forwarding alone can be cost-neutral. Retry only value forwarding on
    // the accepted input, then cost it together with independently proved
    // home deletion. Never reuse a previously rejected deletion proposal.
    let mut proposal = selected.clone();
    let forwarded = forward_loads(&mut proposal, config);
    if !proposal.routines.iter().any(|r| r.frame.locals.iter().any(|slot| {
        enabled(slot, config) && slot.offset == 0 && slot.init.is_none()
            && matches!(slot.base, MirStorageBase::Local(id) if private_store_candidate(&proposal, r, id))
    })) { return stats; }
    let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&proposal);
    let mut proof = Proof {
        program: &proposal,
        summaries,
        callees: BTreeMap::new(),
    };
    let mut candidates = BTreeMap::<RoutineId, Vec<MirMem>>::new();
    for r in &proposal.routines {
        for slot in &r.frame.locals {
            let MirStorageBase::Local(id) = slot.base else {
                continue;
            };
            if !enabled(slot, config) || slot.offset != 0 || slot.init.is_some() {
                continue;
            }
            if proof.unobserved(r, id) {
                candidates
                    .entry(r.id)
                    .or_default()
                    .extend((0..slot.storage_size).map(|offset| MirMem::Local { id, offset }));
            }
        }
    }
    if candidates.is_empty() {
        return stats;
    }
    for r in &mut proposal.routines {
        let Some(cells) = candidates.get(&r.id) else {
            // Unrelated forwarding must pass its own existing cost gate.
            *r = selected
                .routines
                .iter()
                .find(|old| old.id == r.id)
                .unwrap()
                .clone();
            continue;
        };
        let ids: BTreeSet<_> = cells
            .iter()
            .filter_map(|m| match m {
                MirMem::Local { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        for b in &mut r.blocks {
            b.ops.retain(|op| !matches!(op,
                MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, .. }), .. } if ids.contains(id)));
        }
        r.frame
            .locals
            .retain(|s| !matches!(s.base, MirStorageBase::Local(id) if ids.contains(&id)));
    }
    let materialized = crate::mir6502::verify_program(&proposal, MirPhase::PreMaterialization)
        .ok()
        .and_then(|()| {
            crate::mir6502::materialize_resolved_program_without_scalar_costing(
                proposal.clone(),
                config,
                origin,
                runtime,
                false,
            )
            .ok()
        });
    let accepted = materialized.as_ref().is_some_and(|new| {
        Cost::measure(emitted, origin)
            .zip(Cost::measure(new, origin))
            .is_some_and(|(old, new)| {
                new.bytes < old.bytes && candidates.keys().all(|&id| new.does_not_grow(&old, id))
            })
            && occupied(new).is_subset(&occupied(emitted))
            && private_cells::physical_cells_unaliased(emitted, origin, &candidates)
    });
    for (id, cells) in candidates {
        if accepted {
            stats.record_many(
                id,
                "scalar-capture-load-forwarded",
                forwarded.get(&id).copied().unwrap_or(0),
            );
        }
        stats.record_many(
            id,
            if accepted {
                "scalar-capture-call-home-bytes-removed"
            } else {
                "scalar-capture-call-declined-cost-or-alias"
            },
            cells.len(),
        );
    }
    if accepted {
        *selected = proposal;
        *emitted = materialized.unwrap();
    }
    stats
}

struct Proof<'a> {
    program: &'a MirProgram,
    summaries: MirKnownCalleeSummaries,
    callees: BTreeMap<RoutineId, bool>,
}

impl Proof<'_> {
    fn unobserved(&mut self, r: &MirRoutine, id: crate::nir::LocalId) -> bool {
        if !private_store_candidate(self.program, r, id) {
            return false;
        }
        let Ok(cfg) = MirCfg::from_routine(r) else {
            return false;
        };
        let mut budget = MAX_OPS;
        for b in &r.blocks {
            for (i, op) in b.ops.iter().enumerate() {
                if let MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Local { id: target, offset }),
                    width,
                    ..
                } = op
                {
                    if *target == id {
                        for byte in 0..bytes(*width) {
                            if !self.lifetime(r, &cfg, id, offset + byte, b.id, i + 1, &mut budget)
                            {
                                return false;
                            }
                        }
                    }
                }
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn lifetime(
        &mut self,
        r: &MirRoutine,
        cfg: &MirCfg,
        local: crate::nir::LocalId,
        byte: u16,
        block: MirBlockId,
        start: usize,
        budget: &mut usize,
    ) -> bool {
        let mut pending = vec![(block, start)];
        let mut seen = BTreeSet::new();
        while let Some((block, first)) = pending.pop() {
            if !seen.insert((block, first)) {
                continue;
            }
            let Some(b) = r.blocks.iter().find(|b| b.id == block) else {
                return false;
            };
            let mut overwritten = false;
            for op in b.ops.iter().skip(first) {
                if *budget == 0 {
                    return false;
                }
                *budget -= 1;
                if let MirOp::Call {
                    target: MirCallTarget::Routine(id),
                    ..
                } = op
                {
                    // A recursive invocation could read or rewrite this same
                    // static frame. The complete callee audit rejects cycles.
                    if !plain_call(self.program, r, op)
                        || *id == r.id
                        || !self.safe_callee(*id, r.id)
                    {
                        return false;
                    }
                    continue;
                }
                if !ordinary_op(self.program, r, op) {
                    return false;
                }
                if matches!(op, MirOp::Store { dst: MirAddr::Direct(MirMem::Local { id, offset }), width, .. }
                    if *id == local && byte.checked_sub(*offset).is_some_and(|n| n < bytes(*width)))
                {
                    overwritten = true;
                    break;
                }
            }
            if !overwritten {
                if matches!(b.terminator, MirTerminator::Unreachable) {
                    return false;
                }
                let e = classify_terminator(&b.terminator).memory;
                if e.indirect_reads
                    || e.indirect_writes
                    || e.opaque
                    || e.has_unknown_effects
                    || e.direct_reads
                        .iter()
                        .any(|a| !ordinary_range(self.program, r, &a.base, a.bytes))
                {
                    return false;
                }
                pending.extend(cfg.successors(block).iter().map(|&next| (next, 0)));
            }
        }
        true
    }

    fn safe_callee(&mut self, callee: RoutineId, owner: RoutineId) -> bool {
        // Cache only audits independent of the candidate: every admitted graph
        // is acyclic. An explicit owner check still prevents reentry.
        if callee == owner {
            return false;
        }
        if let Some(&safe) = self.callees.get(&callee) {
            return safe;
        }
        let mut budget = MAX_OPS;
        let safe = audit_callee(
            self.program,
            &self.summaries,
            callee,
            &mut BTreeSet::new(),
            &mut budget,
        );
        self.callees.insert(callee, safe);
        safe
    }
}

fn private_store_candidate(program: &MirProgram, r: &MirRoutine, id: crate::nir::LocalId) -> bool {
    private_cells::home_has_only_captured_stores(r, id)
        && !private_cells::frame_exposed_in_data(program, r.id)
        && !r
            .frame
            .locals
            .iter()
            .any(|s| matches!(s.base, MirStorageBase::LocalAlias { .. }))
        && !r.blocks.iter().any(|b| {
            b.ops
                .iter()
                .map(|op| classify_op(op).memory)
                .chain(std::iter::once(classify_terminator(&b.terminator).memory))
                .any(|e| {
                    e.address_references
                        .iter()
                        .any(|m| matches!(m, MirMem::Local { .. } | MirMem::Param { .. }))
                })
        })
}

fn bytes(width: MirWidth) -> u16 {
    if width == MirWidth::Byte { 1 } else { 2 }
}

fn ordinary_range(p: &MirProgram, r: &MirRoutine, mem: &MirMem, bytes: u16) -> bool {
    let (offset, size) = match mem {
        MirMem::Local { id, offset } => (
            *offset,
            r.frame
                .locals
                .iter()
                .find(|s| s.base == MirStorageBase::Local(*id) && s.offset == 0)
                .map(|s| s.storage_size),
        ),
        MirMem::Param { id, offset } => (
            *offset,
            r.frame
                .params
                .iter()
                .find(|s| s.base == MirStorageBase::Param(*id) && s.offset == 0)
                .map(|s| s.storage_size),
        ),
        MirMem::Global { id, offset } => (
            *offset,
            p.globals
                .iter()
                .find(|g| g.id == *id && matches!(g.backing, MirGlobalBacking::Ordinary { .. }))
                .map(|g| g.storage_size),
        ),
        MirMem::Static { id, offset } => (
            *offset,
            p.statics
                .iter()
                .find(|s| s.id == *id)
                .and_then(|s| u16::try_from(s.image.bytes.len()).ok()),
        ),
        _ => return false,
    };
    offset
        .checked_add(bytes)
        .zip(size)
        .is_some_and(|(end, size)| end <= size)
}

fn ordinary_op(p: &MirProgram, r: &MirRoutine, op: &MirOp) -> bool {
    if matches!(
        op,
        MirOp::Call { .. }
            | MirOp::RuntimeHelper { .. }
            | MirOp::MachineBlock { .. }
            | MirOp::Barrier { .. }
            | MirOp::CopyBytes {
                source_volatile: true,
                ..
            }
            | MirOp::CopyBytes {
                destination_volatile: true,
                ..
            }
    ) {
        return false;
    }
    ordinary_memory(p, r, op)
}

fn ordinary_memory(p: &MirProgram, r: &MirRoutine, op: &MirOp) -> bool {
    let e = classify_op(op).memory;
    !e.indirect_reads
        && !e.indirect_writes
        && !e.opaque
        && !e.has_unknown_effects
        && !e.may_write_any
        && e.structured_reads == MirMemoryEffect::None
        && e.structured_writes == MirMemoryEffect::None
        && e.direct_reads
            .iter()
            .chain(&e.direct_writes)
            .all(|a| ordinary_range(p, r, &a.base, a.bytes))
}

fn plain_call(p: &MirProgram, r: &MirRoutine, op: &MirOp) -> bool {
    // The body summary does not describe argument packing or result homes.
    // Keep those transfers outside ordinary RAM, and audit value operands in
    // the caller's frame before consulting the callee's effects.
    fn argument(home: &MirArgHome) -> bool {
        match home {
            MirArgHome::Reg(_)
            | MirArgHome::RegisterPair { .. }
            | MirArgHome::ZeroPage(_)
            | MirArgHome::FixedZeroPage(_) => true,
            MirArgHome::BytePair { lo, hi } => argument(lo) && argument(hi),
            MirArgHome::Absolute(_) | MirArgHome::StackFrame { .. } => false,
        }
    }
    fn result(home: &MirResultHome) -> bool {
        matches!(
            home,
            MirResultHome::Reg(_)
                | MirResultHome::RegisterPair { .. }
                | MirResultHome::ZeroPage(_)
                | MirResultHome::FixedZeroPage(_)
        )
    }
    matches!(op, MirOp::Call { abi, args, result: value, additional_results, effects, .. }
        if !effects.may_call_os
        && abi.params.iter().all(argument)
        && args.iter().all(|a| argument(&a.home))
        && abi.result.iter().all(result)
        && abi.additional_results.iter().all(|a| result(&a.home))
        && value.iter().chain(additional_results).all(|v| result(&v.home))
        && ordinary_memory(p, r, op))
}

fn audit_callee(
    p: &MirProgram,
    summaries: &MirKnownCalleeSummaries,
    id: RoutineId,
    active: &mut BTreeSet<RoutineId>,
    budget: &mut usize,
) -> bool {
    if active.len() == MAX_CALL_DEPTH || !active.insert(id) {
        return false;
    }
    let Some(r) = p.routines.iter().find(|r| r.id == id) else {
        return false;
    };
    let Some(summary) = summaries.for_target(&MirCallTarget::Routine(id)) else {
        return false;
    };
    if r.abi != MirRoutineAbi::Action
        || r.blocks.is_empty()
        || !matches!(summary.reads(), MirKnownMemoryReads::Exact(_))
        || !matches!(summary.writes(), MirKnownMemoryWrites::Exact(_))
    {
        return false;
    }
    for b in &r.blocks {
        if matches!(b.terminator, MirTerminator::Unreachable) {
            return false;
        }
        let e = classify_terminator(&b.terminator).memory;
        if e.indirect_reads
            || e.indirect_writes
            || e.opaque
            || e.has_unknown_effects
            || e.direct_reads
                .iter()
                .any(|a| !ordinary_range(p, r, &a.base, a.bytes))
        {
            return false;
        }
        for op in &b.ops {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            match op {
                MirOp::Call {
                    target: MirCallTarget::Routine(next),
                    ..
                } => {
                    if !plain_call(p, r, op) || !audit_callee(p, summaries, *next, active, budget) {
                        return false;
                    }
                }
                _ if ordinary_op(p, r, op) => {}
                _ => return false,
            }
        }
    }
    active.remove(&id);
    true
}

#[cfg(test)]
mod tests {
    use super::super::tests::lower;
    use super::*;

    const SOURCE: &str = "TYPE Choice=VARIANT [NONE SOME [CARD value]]\nChoice input\nCARD answer\nPROC Save(CARD n)\nanswer=n\nRETURN\nPROC Relay(CARD n)\nSave(n)\nRETURN\nBYTE FUNC Unsafe()\nBYTE POINTER p=$9000\nRETURN(p^)\nPROC Run()\nCASE input OF\nWHEN Choice.SOME(value) THEN\nRelay(value)\nWHEN Choice.NONE THEN\nanswer=0\nESAC\nRETURN\nPROC Main()\nRETURN\n";

    fn forwarded(source: &str) -> MirProgram {
        let mut p = lower(source, false);
        super::super::select(&mut p, &Mir6502Config::default());
        crate::mir6502::verify_program(&p, MirPhase::PreMaterialization).unwrap();
        p
    }

    fn binding(p: &MirProgram) -> (&MirRoutine, crate::nir::LocalId) {
        let r = p.routines.iter().find(|r| r.name == "Run").unwrap();
        let id = r
            .frame
            .locals
            .iter()
            .find_map(|s| match s.base {
                MirStorageBase::Local(id) if s.purpose == MirStoragePurpose::PatternBinding => {
                    Some(id)
                }
                _ => None,
            })
            .unwrap();
        (r, id)
    }

    fn proven(p: &MirProgram) -> bool {
        let (r, id) = binding(p);
        let mut proof = Proof {
            program: p,
            summaries: MirKnownCalleeSummaries::analyze_for_rewriting(p),
            callees: BTreeMap::new(),
        };
        proof.unobserved(r, id)
    }

    #[test]
    fn scalar_call_elision_is_path_sensitive_and_costed_after_forwarding() {
        for source in [
            SOURCE.to_string(),
            SOURCE.replace("answer=0\nESAC", "answer=Unsafe()\nESAC"),
        ] {
            let p = forwarded(&source);
            assert!(!private_cells::has_only_named_reads(&p));
            let (r, id) = binding(&p);
            assert!(!private_cells::scalar_home_unobserved(&p, r, id));
            assert!(private_store_candidate(&p, r, id), "private stores");
            let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
            for callee in p
                .routines
                .iter()
                .filter(|r| matches!(r.name.as_str(), "Relay" | "Save"))
            {
                let mut budget = MAX_OPS;
                assert!(
                    audit_callee(&p, &summaries, callee.id, &mut BTreeSet::new(), &mut budget),
                    "callee {} {:?}",
                    callee.name,
                    summaries.get(callee.id)
                );
            }
            assert!(proven(&p), "lifetime");
            let raw = lower(&source, false);
            let mut off = Mir6502Config::default();
            off.enable_scalar_capture_call_elision = false;
            let old =
                crate::mir6502::materialize_program_with_origin(raw.clone(), &off, 0x3000).unwrap();
            let new = crate::mir6502::materialize_program_with_origin(
                raw,
                &Mir6502Config::default(),
                0x3000,
            )
            .unwrap();
            assert!(
                Cost::measure(&new, 0x3000).unwrap().bytes
                    < Cost::measure(&old, 0x3000).unwrap().bytes
            );
            assert!(
                !new.routines
                    .iter()
                    .find(|n| n.id == r.id)
                    .unwrap()
                    .frame
                    .locals
                    .iter()
                    .any(|s| s.base == MirStorageBase::Local(id))
            );
            assert!(occupied(&new).is_subset(&occupied(&old)));
        }
    }

    #[test]
    fn scalar_call_elision_audits_effects_and_bounds_in_each_callee_frame() {
        for reason in [
            "pointer read",
            "pointer write",
            "absolute read",
            "absolute global",
            "out of bounds read",
            "out of bounds write",
            "barrier",
            "unknown call",
            "recursion",
            "opaque call",
            "structured call",
            "indirect call",
            "absolute argument home",
            "absolute argument read",
            "budget",
        ] {
            let mut p = forwarded(SOURCE);
            assert!(proven(&p));
            if matches!(
                reason,
                "opaque call"
                    | "structured call"
                    | "indirect call"
                    | "absolute argument home"
                    | "absolute argument read"
            ) {
                let call = p
                    .routines
                    .iter_mut()
                    .find(|r| r.name == "Relay")
                    .unwrap()
                    .blocks
                    .iter_mut()
                    .flat_map(|b| &mut b.ops)
                    .find(|op| matches!(op, MirOp::Call { .. }))
                    .unwrap();
                let MirOp::Call {
                    effects,
                    abi,
                    args,
                    target,
                    ..
                } = call
                else {
                    unreachable!()
                };
                match reason {
                    "opaque call" => effects.opaque = true,
                    "structured call" => effects.memory_reads = MirMemoryEffect::All,
                    "indirect call" => {
                        *target = MirCallTarget::Indirect {
                            target: MirValue::ConstU16(0x9000),
                            width: MirWidth::Word,
                        }
                    }
                    "absolute argument home" => {
                        abi.params[0] = MirArgHome::Absolute(0x9000);
                        args[0].home = abi.params[0].clone();
                    }
                    "absolute argument read" => {
                        args[0].value = MirValue::PointerCell(MirMem::Absolute(0x9000))
                    }
                    _ => unreachable!(),
                }
                assert!(!proven(&p), "{reason}");
                continue;
            }
            let call = p
                .routines
                .iter()
                .find(|r| r.name == "Relay")
                .unwrap()
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .find(|op| matches!(op, MirOp::Call { .. }))
                .unwrap()
                .clone();
            let r = p.routines.iter_mut().find(|r| r.name == "Save").unwrap();
            r.temps.push(MirTemp { id: MirTempId(900) });
            let MirStorageBase::Param(param) = r.frame.params[0].base else {
                panic!()
            };
            let inserted = match reason {
                "pointer read" => Some(MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(900)),
                    src: MirAddr::Deref {
                        ptr: MirValue::ConstU16(0x9000),
                        offset: 0,
                    },
                    width: MirWidth::Byte,
                }),
                "pointer write" => Some(MirOp::Store {
                    dst: MirAddr::Deref {
                        ptr: MirValue::ConstU16(0x9000),
                        offset: 0,
                    },
                    src: MirValue::ConstU8(1),
                    width: MirWidth::Byte,
                }),
                "absolute read" => Some(MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(900)),
                    src: MirAddr::Direct(MirMem::Absolute(0xD20A)),
                    width: MirWidth::Byte,
                }),
                "out of bounds read" => Some(MirOp::Load {
                    dst: MirDef::VTemp(MirTempId(900)),
                    src: MirAddr::Direct(MirMem::Param {
                        id: param,
                        offset: 1,
                    }),
                    width: MirWidth::Word,
                }),
                "out of bounds write" => Some(MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Param {
                        id: param,
                        offset: 1,
                    }),
                    src: MirValue::ConstU16(1),
                    width: MirWidth::Word,
                }),
                "barrier" => Some(MirOp::Barrier {
                    effects: MirEffects::default(),
                }),
                "unknown call" | "recursion" => {
                    let mut op = call;
                    let MirOp::Call { target, .. } = &mut op else {
                        panic!()
                    };
                    match reason {
                        "unknown call" => *target = MirCallTarget::Routine(RoutineId(900)),
                        _ => {}
                    }
                    Some(op)
                }
                "absolute global" => {
                    p.globals
                        .iter_mut()
                        .find(|g| g.name == "answer")
                        .unwrap()
                        .backing = MirGlobalBacking::Absolute(0xD000);
                    None
                }
                "budget" => {
                    for n in 1..=513 {
                        let id = MirTempId(900 + n);
                        r.temps.push(MirTemp { id });
                        r.blocks[0].ops.push(MirOp::LoadImm {
                            dst: MirDef::VTemp(id),
                            value: 0,
                            width: MirWidth::Byte,
                        });
                    }
                    None
                }
                _ => unreachable!(),
            };
            if let Some(op) = inserted {
                r.blocks[0].ops.push(op);
            }
            assert!(!proven(&p), "{reason}");
        }
    }

    #[test]
    fn scalar_call_elision_checks_successors_and_frame_exposure() {
        for reason in ["later unknown call", "frame address", "unreachable"] {
            let mut p = forwarded(SOURCE);
            assert!(proven(&p));
            let (r, id) = binding(&p);
            let owner = r.id;
            let r = p.routines.iter_mut().find(|r| r.id == owner).unwrap();
            let b = r
                .blocks
                .iter_mut()
                .find(|b| matches!(b.terminator, MirTerminator::Return))
                .unwrap();
            match reason {
                "later unknown call" => b.ops.push(MirOp::Barrier {
                    effects: MirEffects::default(),
                }),
                "frame address" => {
                    r.temps.push(MirTemp { id: MirTempId(900) });
                    b.ops.push(MirOp::LeaAddr {
                        dst: MirDef::VTemp(MirTempId(900)),
                        target: MirMem::Local { id, offset: 0 },
                        width: MirWidth::Word,
                    });
                }
                "unreachable" => b.terminator = MirTerminator::Unreachable,
                _ => unreachable!(),
            }
            assert!(!proven(&p), "{reason}");
        }
    }

    #[test]
    fn scalar_call_elision_alias_rejection_preserves_existing_forwarding() {
        let source = SOURCE.replace("PROC Main()", "PROC ForwardOnly()\nLET sample=Choice.SOME(1234)\nCASE sample OF\nWHEN Choice.SOME(value) THEN\nanswer=value\nWHEN Choice.NONE THEN\nanswer=0\nESAC\nRETURN\nPROC Main()");
        let mut p = lower(&source, false);
        let mut off = Mir6502Config::default();
        off.enable_scalar_capture_call_elision = false;
        let old = crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
        let (r, id) = binding(&old);
        let address = super::super::super::MaterializeLayout::new(&old, 0x3000)
            .mem_address(r.id, &MirMem::Local { id, offset: 0 })
            .unwrap();
        let mut alias = p
            .globals
            .iter()
            .find(|g| g.storage_size > 0)
            .unwrap()
            .clone();
        alias.id = crate::nir::SymbolId(900);
        alias.name = "observer".into();
        alias.storage_size = 1;
        alias.width = Some(MirWidth::Byte);
        alias.backing = MirGlobalBacking::Absolute(address);
        alias.init = None;
        p.globals.push(alias);
        let before =
            crate::mir6502::materialize_program_with_origin(p.clone(), &off, 0x3000).unwrap();
        let after = crate::mir6502::materialize_program_with_origin(
            p.clone(),
            &Mir6502Config::default(),
            0x3000,
        )
        .unwrap();
        assert_eq!(before, after);
        off.enable_argument_capture_forwarding = false;
        off.enable_pattern_binding_forwarding = false;
        let unforwarded = crate::mir6502::materialize_program_with_origin(p, &off, 0x3000).unwrap();
        assert!(
            Cost::measure(&after, 0x3000).unwrap().bytes
                < Cost::measure(&unforwarded, 0x3000).unwrap().bytes
        );
    }
}
