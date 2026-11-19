//! Candidate-local call facts for resident storage. The borrowed program fixes
//! the representation; construction never imports another candidate's summaries.
use super::*;
use crate::mir6502::analysis::known_callees::{
    MirAcyclicEntryAnalysis, MirCalleeEntryFact, MirKnownCalleeSummaries, MirKnownMemoryWrites,
};
use std::collections::BTreeMap;

pub(in crate::mir6502) struct ResidenceCalls<'a> {
    program: &'a MirProgram,
    known: MirKnownCalleeSummaries,
    entries: BTreeMap<RoutineId, Option<Arc<MirCalleeEntryFact>>>,
    reviewed_fingerprints: BTreeMap<RoutineId, Option<u64>>,
    acyclic: Option<MirAcyclicEntryAnalysis<'a>>,
    stage: MirCalleeEntryStage,
}

impl<'a> ResidenceCalls<'a> {
    pub fn new(program: &'a MirProgram, stage: MirCalleeEntryStage) -> Self {
        Self::with_entries(program, stage, None)
    }

    fn with_entries(program: &'a MirProgram, stage: MirCalleeEntryStage, origin: Option<u16>) -> Self {
        Self {
            program,
            known: if let Some(origin) = origin { MirKnownCalleeSummaries::analyze_for_source_homes(program, origin) }
                else { MirKnownCalleeSummaries::analyze_for_residence(program) },
            entries: BTreeMap::new(),
            reviewed_fingerprints: BTreeMap::new(),
            acyclic: origin.map(|origin| MirAcyclicEntryAnalysis::at_origin(program, origin)),
            stage,
        }
    }

    /// Explicit opt-in for source-home discovery and validation of supplied
    /// bindings. Spill discovery and planned handoff retain the leaf contract.
    #[cfg(test)]
    pub fn with_acyclic(program: &'a MirProgram, stage: MirCalleeEntryStage) -> Self {
        Self::at_origin(program, stage, 0)
    }

    pub fn at_origin(program: &'a MirProgram, stage: MirCalleeEntryStage, origin: u16) -> Self {
        Self::with_entries(program, stage, Some(origin))
    }

    pub fn sargs_capture(&self, routine: &MirRoutine, op: &MirOp) -> Option<crate::mir6502::runtime_contracts::CaptureSummary> {
        if !self.program.routines.iter().any(|r| std::ptr::eq(r, routine)) { return None; }
        crate::mir6502::runtime_contracts::sargs_capture(self.program, routine, op)
    }

    fn entry(&mut self, target: RoutineId) -> Option<Arc<MirCalleeEntryFact>> {
        if let Some(analysis) = &mut self.acyclic { return analysis.analyze(target); }
        self.entries.entry(target).or_insert_with(||
            MirCalleeEntryFact::analyze(self.program, target, self.stage).map(Arc::new)).clone()
    }

    /// Diagnostics read already established facts; tracing does not run analysis.
    pub fn entry_description(&self, call: &MirOp) -> Option<String> {
        let MirOp::Call { target: MirCallTarget::Routine(target), .. } = call else { return None; };
        if let Some(current) = self.reviewed_fingerprints.get(target) {
            return Some(match current {
                Some(hash) => format!("{} current fingerprint {hash:016x}", if self.program.routines.iter().any(|r| r.id == *target && r.frame.callee_review.is_some()) { "reviewed-6502" } else { "abi-inputs-only" }),
                None => "body outside abi-inputs-only review scope".into(),
            });
        }
        if let Some(analysis) = &self.acyclic {
            if let Some((id, site, reason)) = analysis.failure(*target) {
                let name = self.program.routines.iter().find(|r| r.id == id).map(|r| r.name.as_str());
                return Some(format!("{name:?} ({id:?}) at {site:?}: {reason}"));
            }
            return analysis.cached(*target).map(|f| format!("{}: {:?}", f.source.name, f.inputs));
        }
        self.entries.get(target)?.as_ref().map(|f| format!("{}: {:?}", f.source.name, f.inputs))
    }

    /// Validate the ordinary call boundary even when no resident value survives
    /// invocation. This grants no preservation or entry facts to other consumers.
    pub fn inputs(&mut self, call: &MirOp) -> Result<BTreeSet<u8>, &'static str> {
        self.check_inputs(call, false)
    }

    fn check_inputs(&mut self, call: &MirOp, require_entry: bool) -> Result<BTreeSet<u8>, &'static str> {
        if self.stage != MirCalleeEntryStage::ActualEntry {
            return Err("residence inputs require expanded callee facts");
        }
        if MirAtariFppService::from_canonical_call(call).is_some()
            || crate::mir6502::runtime_contracts::library_invocation(self.program, call).is_some() {
            return Ok(BTreeSet::new());
        }
        if matches!(call, MirOp::RuntimeHelper { .. }) {
            return crate::mir6502::runtime_contracts::helper_invocation(self.program, call)
                .map(|_| BTreeSet::new()).ok_or("unsupported residence helper contract");
        }
        let MirOp::Call { target: MirCallTarget::Routine(target), args, .. } = call else {
            return Err("unknown or indirect residence call");
        };
        let callee = self.program.routines.iter().find(|r| r.id == *target)
            .ok_or("missing residence callee")?;
        let machine = callee.blocks.iter().flat_map(|b| &b.ops)
            .any(|op| matches!(op, MirOp::MachineBlock { .. }));
        if !matches!(callee.abi, MirRoutineAbi::Action | MirRoutineAbi::ActionObservable)
            || (machine && self.acyclic.is_none())
            || callee.blocks.iter().flat_map(|b| &b.ops)
            .any(|op| matches!(op, MirOp::Barrier { .. }) || (matches!(op, MirOp::SArgsCapture { .. }) && crate::mir6502::runtime_contracts::sargs_capture(self.program, callee, op).is_none())) {
            return Err("unsupported residence callee");
        }
        if self.acyclic.is_none() && callee.abi == MirRoutineAbi::ActionObservable
            && callee.blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(op, MirOp::Call { .. })) {
            return Err("nonleaf observable residence entry");
        }
        let outgoing = super::super::abi::canonical_call_lanes(callee, args)?;
        super::super::abi::validate_direct_call_result(callee, call)?;
        let observable = callee.abi == MirRoutineAbi::ActionObservable;
        match self.entry(*target) {
            // An ordinary native call already admits an unknown entry when all
            // resident values die. If the summary identifies fixed inputs, keep
            // those bytes out of the caller's storage even at this dead boundary.
            Some(entry) if !require_entry && !observable && !machine && self.acyclic.is_some() => {
                handoff::register_inputs_supplied(&entry, &outgoing)?;
                return Ok(handoff::extra_fixed_inputs(&entry, &outgoing).collect());
            }
            Some(entry) => handoff::inputs_supplied(&entry, &outgoing)?,
            None if callee.frame.callee_review.is_some() => {
                let review = callee.frame.callee_review.as_ref().unwrap();
                self.reviewed_fingerprints.entry(*target).or_insert_with(||
                    crate::mir6502::analysis::known_callees::body_fingerprint(self.program, callee, review.version()));
                return Err("missing reviewed assembly entry guarantee");
            }
            None if callee.frame.abi_inputs_only_review.is_some() => {
                self.reviewed_fingerprints.entry(*target).or_insert_with(||
                    crate::mir6502::analysis::known_callees::abi_inputs_review_fingerprint(self.program, callee));
                return Err("unknown residence callee entry");
            }
            // Observable and machine entries require an actual certificate,
            // including at dead boundaries. The absence of MIR call edges cannot
            // exclude hidden calls or implicit inputs in a machine payload.
            None if require_entry || observable || machine =>
                return Err("unknown residence callee entry"),
            None => {},
        }
        Ok(BTreeSet::new())
    }

    pub fn fixed_storage(&self) -> Arc<crate::mir6502::analysis::known_callees::MirFixedStorage> {
        self.known.fixed_storage().clone()
    }

    pub fn writes(&mut self, call: &MirOp) -> Result<MirKnownMemoryWrites, &'static str> {
        self.writes_at(call, None)
    }

    pub fn writes_at(&mut self, call: &MirOp,
        site: Option<(&MirRoutine, crate::mir6502::analysis::sites::MirSite)>,
    ) -> Result<MirKnownMemoryWrites, &'static str> {
        if self.stage != MirCalleeEntryStage::ActualEntry {
            return Err("residence writes require expanded callee facts");
        }
        if MirAtariFppService::from_canonical_call(call).is_some() {
            return Ok(MirKnownMemoryWrites::Exact(MirAtariFppService::workspace()
                .map(|byte| MirMem::FixedZeroPage(MirFixedZpSlot(byte))).collect()));
        }
        if let Some(s) = crate::mir6502::runtime_contracts::library_invocation(self.program, call) {
            let values = site.and_then(|(routine, site)| {
                Some((crate::mir6502::runtime_contracts::buffers::argument_values(self.program, routine)?, site))
            });
            return Ok(s.normal_writes(self.program, call, values.as_ref().map(|(v, site)| (v, *site))));
        }
        if matches!(call, MirOp::RuntimeHelper { .. }) {
            return crate::mir6502::runtime_contracts::helper_invocation(self.program, call)
                .map(|s| MirKnownMemoryWrites::Exact(s.writes)).ok_or("unsupported residence helper contract");
        }
        let MirOp::Call {
            target: MirCallTarget::Routine(target),
            ..
        } = call
        else {
            return Err("unknown or indirect residence call");
        };
        let callee = self
            .program
            .routines
            .iter()
            .find(|r| r.id == *target)
            .ok_or("missing residence callee")?;
        if !matches!(callee.abi, MirRoutineAbi::Action | MirRoutineAbi::ActionObservable)
            || callee.blocks.iter().flat_map(|b| &b.ops).any(|op| {
                matches!(
                    op,
                    MirOp::Barrier { .. }
                ) || (matches!(op, MirOp::RuntimeHelper { .. }) && crate::mir6502::runtime_contracts::helper_invocation(self.program, op).is_none()) || (matches!(op, MirOp::SArgsCapture { .. }) && crate::mir6502::runtime_contracts::sargs_capture(self.program, callee, op).is_none()) || (self.acyclic.is_none() && matches!(op, MirOp::Call { .. } | MirOp::MachineBlock { .. }))
            })
        {
            return Err("unsupported residence callee");
        }
        self.check_inputs(call, true)?;
        let summary = self.known.get(*target).ok_or("unknown residence callee writes")?;
        let projected;
        let writes = if self.acyclic.is_some() && summary.needs_write_arguments() {
            let (routine, site) = site.ok_or("unresolved residence pointer argument")?;
            let values = crate::mir6502::runtime_contracts::buffers::argument_values(self.program, routine)
                .ok_or("unresolved residence pointer argument")?;
            projected = self.known.writes_at_site(self.program, routine, call, &values, site);
            &projected
        } else { summary.writes() };
        if writes.has_formal_writes() || matches!(writes, MirKnownMemoryWrites::Unknown) {
            return Err("unknown residence callee writes");
        }
        Ok(writes.clone())
    }
}

#[cfg(test)]
mod assembly_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_program(observable: bool, inline_middle: bool, dead: bool) -> MirProgram {
        let source = format!("BYTE FUNC Touch(BYTE x) RETURN(x+1) BYTE FUNC Middle{}(BYTE x) RETURN(Touch(x)) CARD FUNC Hold(CARD keep) Middle(7) {} RETURN(keep)",
            if observable { "=*" } else { "" }, if dead { "keep=8" } else { "" });
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let mut p = crate::mir6502::lower_program(&nir).unwrap();
        for (r, slot) in p.routines.iter_mut().zip([0xe8, 0xea, 0xe4]) {
            if r.name == "Middle" && (observable || inline_middle) { continue; }
            let MirStorageBase::Param(param) = r.frame.params[0].base else { panic!() };
            r.frame.parameter_homes = Some(vec![MirParamBinding {
                param, home: MirParamHome::ZeroPage(MirFixedZpSlot(slot)) }]);
            r.frame.fixed_zero_page.extend((0..r.frame.params[0].storage_size)
                .map(|i| MirFixedZpSlot(slot + i as u8)));
        }
        crate::mir6502::materialize_program(p, &crate::mir6502::Mir6502Config::default()).unwrap()
    }

    fn verify_chain(p: &MirProgram) -> Vec<MirDiagnostic> {
        let hold = p.routines.iter().find(|r| r.name == "Hold").unwrap();
        verify_lifetimes(hold, Some(&mut ResidenceCalls::with_acyclic(p, MirCalleeEntryStage::ActualEntry)))
    }

    #[test]
    fn acyclic_residence_checks_descendant_lanes_and_keeps_leaf_scope_explicit() {
        for (observable, inline) in [(false, false), (false, true), (true, true)] {
            let p = chain_program(observable, inline, false);
            assert!(verify_chain(&p).is_empty());
            let call = p.routines.iter().find(|r| r.name == "Hold").unwrap()
                .blocks.iter().flat_map(|b| &b.ops).find(|o| matches!(o, MirOp::Call { .. })).unwrap();
            assert!(ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry).writes(call).is_err());
            assert!(ResidenceCalls::with_acyclic(&p, MirCalleeEntryStage::ActualEntry).writes(call).is_ok());
            for byte in [0xe4, 0xe5] {
                for absolute in [false, true] {
                    let mut bad = p.clone();
                    let mut op = store(byte);
                    if absolute {
                        let MirOp::Store { dst, .. } = &mut op else { unreachable!() };
                        *dst = MirAddr::Direct(MirMem::Absolute(u16::from(byte)));
                    }
                    bad.routines[0].blocks[0].ops.insert(0, op);
                    assert!(!verify_chain(&bad).is_empty(), "{observable} {inline} {byte:x} {absolute}");
                }
            }
            // Resolve the grandchild's symbolic object in its own frame.
            let mut moved = p.clone();
            let MirStorageBase::Param(param) = moved.routines[0].frame.params[0].base else { panic!() };
            moved.routines[0].frame.parameter_homes.as_mut().unwrap()[0].home =
                MirParamHome::ZeroPage(MirFixedZpSlot(0xe5));
            moved.routines[0].blocks[0].ops.insert(0, MirOp::Store {
                dst: MirAddr::Direct(MirMem::Param { id: param, offset: 0 }),
                src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte });
            assert!(!verify_chain(&moved).is_empty());
        }
    }

    #[test]
    fn dead_native_boundary_excludes_the_callees_extra_input_bytes() {
        for (dead, extra, allowed) in [(true, 0xe6, true), (true, 0xe5, false), (false, 0xe6, false)] {
            let mut p = chain_program(false, true, dead);
            let middle = p.routines.iter_mut().find(|r| r.name == "Middle").unwrap();
            middle.blocks[0].ops.insert(0, MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(extra))),
                width: MirWidth::Byte,
            });
            let diagnostics = verify_chain(&p);
            assert_eq!(diagnostics.is_empty(), allowed, "dead={dead} extra={extra:x} {diagnostics:?}");
        }
    }

    #[test]
    fn acyclic_residence_retains_ordinary_dead_boundary_fallback() {
        for observable in [false, true] {
            for dead in [false, true] {
                let mut p = chain_program(observable, true, dead);
                p.routines[0].effects.opaque = true;
                assert_eq!(verify_chain(&p).is_empty(), dead && !observable);
            }
        }
    }

    fn program() -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(
            "BYTE FUNC Touch(BYTE x) RETURN(x+1) CARD FUNC Hold(CARD keep) Touch(7) RETURN(keep)"
        ).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let n = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let mut p = crate::mir6502::lower_program(&n).unwrap();
        for (r, byte) in p.routines.iter_mut().zip([0xe8, 0xe4]) {
            let MirStorageBase::Param(param) = r.frame.params[0].base else {
                panic!()
            };
            r.frame.parameter_homes = Some(vec![MirParamBinding {
                param,
                home: MirParamHome::ZeroPage(MirFixedZpSlot(byte)),
            }]);
            r.frame.fixed_zero_page.extend(
                (0..r.frame.params[0].storage_size).map(|i| MirFixedZpSlot(byte + i as u8)),
            );
        }
        crate::mir6502::materialize_program(p, &crate::mir6502::Mir6502Config::default()).unwrap()
    }
    fn verify(p: &MirProgram) -> Vec<MirDiagnostic> {
        let r = p.routines.iter().find(|r| r.name == "Hold").unwrap();
        verify_lifetimes(
            r,
            Some(&mut ResidenceCalls::new(
                p,
                MirCalleeEntryStage::ActualEntry,
            )),
        )
    }
    fn store(byte: u8) -> MirOp {
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(byte))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }
    }
    #[test]
    fn caller_lifetime_rechecks_fixed_global_alias_backing_after_rebinding() {
        use crate::mir6502::ir::{MirGlobal, MirGlobalBacking};
        use crate::nir::SymbolId;
        let mut p = program();
        p.globals = vec![MirGlobal { id: SymbolId(90), name: "state".into(), kind: "word".into(),
            width: Some(MirWidth::Word), storage_size: 2, backing: MirGlobalBacking::Absolute(0x600), init: None },
            MirGlobal { id: SymbolId(91), name: "alias".into(), kind: "byte".into(),
            width: Some(MirWidth::Byte), storage_size: 1,
            backing: MirGlobalBacking::Alias { target: SymbolId(90), offset: 1 }, init: None }];
        p.routines[0].blocks[0].ops.insert(0, MirOp::Store {
            dst: MirAddr::Direct(MirMem::Global { id: SymbolId(91), offset: 0 }),
            src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte });
        assert!(verify(&p).is_empty());
        let routines = p.routines.clone();
        for address in [0xe3, 0xe4] {
            p.globals[0].backing = MirGlobalBacking::Absolute(address);
            assert!(!verify(&p).is_empty(), "alias reaches {:02x}", address + 1);
            assert_eq!(p.routines, routines);
        }
        p.globals[0].backing = MirGlobalBacking::Absolute(0x700);
        assert!(verify(&p).is_empty());
    }

    #[test]
    fn caller_lifetime_uses_current_callee_writes_and_absolute_aliases() {
        for abi in [MirRoutineAbi::Action, MirRoutineAbi::ActionObservable] {
            check_callee_writes(abi);
        }
    }
    fn check_callee_writes(abi: MirRoutineAbi) {
        let mut p = program();
        p.routines[0].abi = abi;
        assert!(verify(&p).is_empty());
        for slot in [0xe4, 0xe5] {
            for absolute in [false, true] {
                let mut bad = p.clone();
                let mut write = store(slot);
                if absolute {
                    let MirOp::Store { dst, .. } = &mut write else {
                        unreachable!()
                    };
                    *dst = MirAddr::Direct(MirMem::Absolute(slot as u16));
                }
                bad.routines[0].blocks[0].ops.insert(0, write);
                assert!(
                    !verify(&bad).is_empty(),
                    "slot={slot:x} absolute={absolute}"
                );
            }
        }
        let mut moved = p.clone();
        moved.routines[0].frame.parameter_homes.as_mut().unwrap()[0].home =
            MirParamHome::ZeroPage(MirFixedZpSlot(0xe5));
        let param = moved.routines[0].frame.parameter_homes.as_ref().unwrap()[0].param;
        // A symbolic callee capture must follow the changed binding.
        moved.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::Param {
                    id: param,
                    offset: 0,
                }),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify(&moved).is_empty());
        let mut virtual_write = p.clone();
        virtual_write.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(99))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify(&virtual_write).is_empty());
        virtual_write.routines[0]
            .frame
            .zero_page_allocations
            .push(MirZpAllocation {
                slot: MirZpSlot(99),
                start: MirFixedZpSlot(0xe9),
                size: 1,
            });
        assert!(verify(&virtual_write).is_empty());
        virtual_write.routines[0]
            .frame
            .zero_page_allocations
            .last_mut()
            .unwrap()
            .start = MirFixedZpSlot(0xe4);
        assert!(!verify(&virtual_write).is_empty());
    }
    #[test]
    fn setup_clobbers_and_extra_callee_inputs_are_not_preservation() {
        for abi in [MirRoutineAbi::Action, MirRoutineAbi::ActionObservable] {
            check_setup_and_inputs(abi);
        }
    }
    fn check_setup_and_inputs(abi: MirRoutineAbi) {
        let mut p = program();
        p.routines[0].abi = abi;
        let mut setup = p.clone();
        let r = &mut setup.routines[1];
        let (block, index) = r
            .blocks
            .iter()
            .enumerate()
            .find_map(|(b, block)| {
                block
                    .ops
                    .iter()
                    .position(|o| matches!(o, MirOp::Call { .. }))
                    .map(|i| (b, i))
            })
            .unwrap();
        r.blocks[block].ops.insert(index, store(0xe4));
        assert!(!verify(&setup).is_empty());
        let mut extra = p.clone();
        extra.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Load {
                dst: MirDef::Reg(MirReg::Y),
                src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xe6))),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify(&extra).is_empty());
        let mut unknown = p.clone();
        let MirOp::Call { target, .. } = &mut unknown.routines[1].blocks[block].ops[index] else {
            panic!()
        };
        *target = MirCallTarget::Routine(RoutineId(999));
        assert!(!verify(&unknown).is_empty());
    }

    #[test]
    fn native_current_location_calls_require_complete_inputs_at_every_boundary() {
        let mut p = program();
        p.routines[0].abi = MirRoutineAbi::ActionObservable;
        let call = p.routines[1].blocks.iter().flat_map(|b| &b.ops)
            .find(|o| matches!(o, MirOp::Call { .. })).unwrap().clone();
        let mut facts = ResidenceCalls::new(&p, MirCalleeEntryStage::ActualEntry);
        assert!(facts.inputs(&call).is_ok());
        assert!(facts.writes(&call).is_ok());
        // A cached target fact does not validate the next site's arguments.
        let mut bad_call = call.clone();
        let MirOp::Call { args, .. } = &mut bad_call else { panic!() };
        args.clear();
        assert_eq!(facts.inputs(&bad_call), Err("mismatched residence arguments"));
        assert!(facts.writes(&bad_call).is_err());
        for variant in 0..4 {
            let mut bad = p.clone();
            match variant {
                0 => bad.routines[0].frame.direct_parameter_abi = None,
                1 => bad.routines[0].blocks[0].ops.push(call.clone()),
                2 => bad.routines[0].blocks[0].ops.push(MirOp::Barrier { effects: Default::default() }),
                _ => bad.routines[0].effects.opaque = true,
            }
            let mut facts = ResidenceCalls::new(&bad, MirCalleeEntryStage::ActualEntry);
            assert!(facts.inputs(&call).is_err(), "dead boundary {variant}");
            assert!(facts.writes(&call).is_err(), "live boundary {variant}");
        }
    }
}
