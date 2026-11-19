//! One parameter binding owns entry capture, working storage, and layout.
//! Each placement batch resumes one core continuation and requires a smaller image.
use super::analysis::known_callees::MirCalleeEntryStage;
use super::analysis::storage_ownership::ObjectAddressControl;
use super::preparation::{Resume, SelectedPlacement};
use super::{diagnostics::MirDiagnostic, ir::*};
use std::collections::BTreeSet;
use std::sync::Arc;
mod admission;
mod lifetimes;
mod logical;
mod observations;
pub(super) use observations::InvocationHomes;
mod locals;
mod calls;
mod cross_call;
pub(super) mod spills;
mod handoff;
#[cfg(test)]
mod mixed_tests;
#[cfg(test)]
mod handoff_tests;
mod zero_page;
mod physical;
pub(super) mod persistent;
pub(super) use physical::verify as verify_physical;
#[cfg(test)]
pub(super) use lifetimes::verify as verify_lifetimes;
pub(super) use lifetimes::verify_with_address_control as verify_residence;
pub(super) use calls::ResidenceCalls;

pub(super) fn select_output(
    nir: &crate::nir::NirProgram,
    baseline: super::preparation::Executable,
    resume: Option<super::preparation::Resume>,
    config: &super::Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    trace: &super::trace::Trace,
) -> super::preparation::Executable {
    if !config.enable_peepholes || !config.enable_parameter_home_selection {
        return baseline;
    }
    let Some(resume) = resume else {
        trace.skipped("parameter-homes", "no-current-checkpoint");
        return baseline;
    };
    let mut selected = SelectedPlacement {
        input: Arc::new(resume.core.checkpoint.input.program.clone()),
        executable: baseline,
    };
    if let Some(proposal) = propose(&selected.input, &selected.executable.program, config.enable_zp_home_lifetimes, origin) {
        selected = try_proposal(
            nir,
            selected,
            &resume,
            proposal,
            "parameter-homes",
            config,
            origin,
            runtime,
            trace,
        );
    } else {
        trace.skipped("parameter-homes", "no-candidates");
    }
    if config.enable_parameter_argument_handoff {
        if let Some(proposal) = handoff::propose(&selected.input, &selected.executable.program) {
            selected = try_proposal(nir, selected, &resume, proposal,
                "parameter-argument-handoff", config, origin, runtime, trace);
        } else {
            trace.skipped("parameter-argument-handoff", "no-candidates");
        }
    }
    if config.enable_cross_call_home_selection {
        if let Some(proposal) = cross_call::propose_configured(&selected.input, &selected.executable.program,
            &resume.core.checkpoint.input.config, origin, trace).or_else(|| {
                (!spills::discover(&selected.executable.program, config.enable_multiple_leaf_call_homes).is_empty())
                    .then(|| (*selected.input).clone())
            }) {
            selected = try_proposal(nir, selected, &resume, proposal,
                "cross-call-homes", config, origin, runtime, trace);
        } else {
            trace.skipped("cross-call-homes", "no-candidates");
        }
    }
    selected.executable
}

fn propose(input: &MirProgram, selected: &MirProgram, mixed: bool, origin: u16) -> Option<MirProgram> {
    let addresses = ObjectAddressControl::for_home_placement(input);
    let observations = physical::observations_complete(input);
    let mut proposal = input.clone();
    let mut changed = false;
    let layout = std::cell::LazyCell::new(|| super::materialize::MaterializeLayout::new(input, origin));
    for routine in &mut proposal.routines {
        let admitted = admission::check(&addresses, routine, false, mixed, Some(selected)).is_ok()
            || super::materialize::arithmetic_homes::reduced_constants(routine, &layout)
                .is_some_and(|view| admission::check(&addresses, &view, false, mixed, Some(selected)).is_ok());
        if !admitted
            || (!observations && physical::has_pointee_writes(input, routine)) {
            continue;
        }
        if let Some(bindings) =
            routine.frame.params.iter().all(|p| eligible_parameter(&addresses, routine, p))
                .then(|| register_bindings(routine)).flatten()
                .or_else(|| zero_page::bindings(input, routine, &addresses))
        {
            install_bindings(routine, bindings);
            changed = true;
        }
    }
    changed.then_some(proposal)
}

fn install_bindings(routine: &mut MirRoutine, bindings: Vec<MirParamBinding>) {
    let absent: BTreeSet<_> = bindings
        .iter()
        .filter_map(|b| (b.home == MirParamHome::Absent).then_some(b.param))
        .collect();
    for block in &mut routine.blocks {
        block.ops.retain(|op| {
            !matches!(op,
            MirOp::Store { dst: MirAddr::Direct(MirMem::Param { id, .. }), .. }
            if absent.contains(id))
        });
    }
    for binding in &bindings {
        if let MirParamHome::ZeroPage(start) = binding.home {
            let width = routine
                .frame
                .params
                .iter()
                .find(|p| p.base == MirStorageBase::Param(binding.param))
                .unwrap()
                .storage_size;
            for lane in 0..width {
                let slot = MirFixedZpSlot(start.0 + lane as u8);
                if !routine.frame.fixed_zero_page.contains(&slot) {
                    routine.frame.fixed_zero_page.push(slot);
                }
            }
        }
    }
    routine.frame.parameter_homes = Some(bindings);
}

#[cfg(test)]
fn eligible_with_calls(addresses: &ObjectAddressControl<'_>, routine: &MirRoutine, calls: bool, mixed: bool) -> bool {
    admission::check(addresses, routine, calls, mixed, None).is_ok()
}

fn eligible_parameter(addresses: &ObjectAddressControl<'_>, routine: &MirRoutine, p: &MirStorageSlot) -> bool {
    matches!(p.base, MirStorageBase::Param(id) if addresses.compiler_controls_address(routine.id, &MirMem::Param { id, offset: 0 }))
        && scalar_parameter_cell(p) && p.offset == 0 && p.init.is_none()
        && matches!((p.scalar_width, p.storage_size),
            (Some(MirWidth::Byte), 1) | (Some(MirWidth::Word), 2))
}

fn scalar_parameter_cell(slot: &MirStorageSlot) -> bool {
    slot.storage == MirStorageClass::Scalar
        // An array parameter with a scalar word lane is its ABI pointer cell,
        // not the array object. No inline array storage gains eligibility.
        || (slot.storage == MirStorageClass::Array
            && slot.scalar_width == Some(MirWidth::Word) && slot.storage_size == 2)
}

fn native_address(addr: &MirAddr) -> bool {
    matches!(addr, MirAddr::Direct(_) | MirAddr::Deref { .. } | MirAddr::PointerCell { .. }
        | MirAddr::PointerIndex { .. } | MirAddr::ComputedIndex { .. }
        | MirAddr::AbsoluteIndexedX { base: MirMem::Global { .. } | MirMem::Static { .. } }
        | MirAddr::AbsoluteIndexedY { base: MirMem::Global { .. } | MirMem::Static { .. } })
}

fn has_pointer_reads(routine: &MirRoutine) -> bool {
    routine.blocks.iter().flat_map(|b| &b.ops).any(|op| {
        matches!(
            op,
            MirOp::Load {
                src: MirAddr::Deref { .. } | MirAddr::PointerCell { .. }
                    | MirAddr::PointerIndex { .. } | MirAddr::ComputedIndex { .. },
                ..
            }
        )
    })
}

fn register_bindings(routine: &MirRoutine) -> Option<Vec<MirParamBinding>> {
    use super::analysis::effects::{classify_op, classify_terminator};
    // One entry consumer ends the incoming parameter lifetime. This avoids
    // reserving registers through a temporary plan that has not yet expanded.
    if routine.blocks.len() != 1
        || !routine.frame.abi_parameter_bytes().is_some_and(|n|
            (3..=crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES).contains(&n))
    {
        return None;
    }
    let block = &routine.blocks[0];
    let mut offset = 0;
    let mut live = 0;
    let mut bindings = Vec::new();
    for slot in &routine.frame.params {
        let MirStorageBase::Param(id) = slot.base else {
            return None;
        };
        let home = if absent_parameter(routine, id) {
            MirParamHome::Absent
        } else {
            live += 1;
            if live > 1 {
                return None;
            }
            let first = block.ops.first()?;
            if !matches!(first, MirOp::Load { src: MirAddr::Direct(MirMem::Param { id: p, offset: 0 }), width, .. }
                if *p == id && Some(*width) == slot.scalar_width)
            {
                return None;
            }
            let refers = |e: super::analysis::effects::MirMemoryEffects| {
                e.direct_references
                    .iter()
                    .chain(e.direct_reads.iter().map(|r| &r.base))
                    .chain(e.direct_writes.iter().map(|r| &r.base))
                    .any(|m| matches!(m, MirMem::Param { id: p, .. } if *p == id))
            };
            if block
                .ops
                .iter()
                .skip(1)
                .any(|o| refers(classify_op(o).memory))
                || refers(classify_terminator(&block.terminator).memory)
            {
                return None;
            }
            let regs = (0..slot.storage_size)
                .map(
                    |lane| match super::abi::action_arg_home(offset + lane, MirWidth::Byte) {
                        MirArgHome::Reg(r) => Some(r),
                        _ => None,
                    },
                )
                .collect::<Option<Vec<_>>>()?;
            MirParamHome::Registers(regs)
        };
        offset += slot.abi_byte_width()?;
        bindings.push(MirParamBinding { param: id, home });
    }
    Some(bindings)
}

fn absent_parameter(routine: &MirRoutine, id: crate::nir::ParamId) -> bool {
    // Reuse the legacy proof, but never delete an observable source read along
    // with an unused destination. Producers remain in the executable body.
    super::materialize::absent_parameter_stores(routine, id)
}

fn try_proposal(
    nir: &crate::nir::NirProgram,
    selected: SelectedPlacement,
    resume: &Resume,
    proposal: MirProgram,
    stage: &'static str,
    config: &super::Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    trace: &super::trace::Trace,
) -> SelectedPlacement {
    let child = trace.child(stage, "batch-trial");
    if stage == "parameter-argument-handoff" {
        child.count("parameter_argument_handoff_trials", 1);
    }
    if stage == "cross-call-homes" {
        child.count("cross_call_home_trials", 1);
    }
    let input = Arc::new(proposal);
    let baseline = &selected.executable;
    let trial = super::preparation::resume_output(
        nir,
        resume,
        (*input).clone(),
        config,
        origin,
        runtime,
        &child,
        (stage == "cross-call-homes").then_some(baseline),
    );
    let trial = trial.and_then(|candidate| {
        physical::verify_relocation(baseline, &candidate, &selected.input, &input)?;
        Ok(candidate)
    });
    let accepted = trial.as_ref().is_ok_and(|p| p.improves(baseline));
    child.event(match stage { "parameter-homes" => "parameter-home-result", "cross-call-homes" => "cross-call-home-result", _ => "parameter-argument-handoff-result" }, || serde_json::json!({
        "accepted": accepted,
        "routine_cost_changes": trial.as_ref().ok().map(|p| p.program.routines.iter().filter_map(|r| {
            let before = baseline.costs.get(&r.id)?;
            let after = p.costs.get(&r.id)?;
            (before != after).then(|| serde_json::json!({"routine": r.name, "before": before, "after": after}))
        }).collect::<Vec<_>>()),
        "diagnostics": trial.as_ref().err().map(|errors| format!("{errors:?}")),
        "before_bytes": crate::codegen::format_load_file(&baseline.output).len(),
        "after_bytes": trial.as_ref().ok().map(|p| crate::codegen::format_load_file(&p.output).len()),
        "reason": match &trial {
            Err(_) => "invalid-trial",
            Ok(p) if !p.costs.keys().eq(baseline.costs.keys()) => "incompatible-routine-inventory",
            Ok(_) if accepted => "smaller-image",
            Ok(_) => "no-final-saving",
        },
    }));
    if accepted {
        SelectedPlacement {
            input,
            executable: trial.unwrap(),
        }
    } else {
        selected
    }
}

/// Explicit source storage wins over compiler reservations, including absolute
/// locals and numeric direct accesses that do not have a global declaration.
fn source_zero_page(program: &MirProgram, routine: &MirRoutine) -> BTreeSet<u8> {
    use super::analysis::effects::{classify_op, classify_terminator};
    let declarations = program.globals.iter().filter_map(|g| match g.backing {
        MirGlobalBacking::Absolute(start) => Some((start, g.storage_size)), _ => None,
    }).chain(program.routines.iter().flat_map(|r| r.frame.params.iter().chain(&r.frame.locals))
        .filter_map(|slot| match slot.base {
            MirStorageBase::Absolute(start) => Some((start, slot.storage_size)), _ => None,
        }));
    let mut owned = BTreeSet::new();
    for (start, bytes) in declarations {
        let end = u32::from(start) + u32::from(bytes);
        owned.extend((u32::from(start)..end.min(256)).map(|byte| byte as u8));
        owned.extend((0..end.saturating_sub(65536).min(256)).map(|byte| byte as u8));
    }
    for block in &routine.blocks {
        for op in &block.ops {
            match op {
                // FR0/FR1 contain implicit operands and results between calls.
                MirOp::Call { target: MirCallTarget::AtariFpp(_), .. } =>
                    owned.extend(MirAtariFppService::workspace()),
                MirOp::PackedRealCopy { .. } => {
                    let memory = classify_op(op).memory;
                    for range in memory.direct_reads.iter().chain(&memory.direct_writes) {
                        if let MirMem::FixedZeroPage(slot) = range.base {
                            owned.extend((0..range.bytes).map(|i| slot.0.wrapping_add(i as u8)));
                        }
                    }
                }
                _ => {}
            }
        }
        for memory in block.ops.iter().map(|op| classify_op(op).memory)
            .chain(std::iter::once(classify_terminator(&block.terminator).memory)) {
            for range in memory.direct_reads.iter().chain(&memory.direct_writes) {
                if let MirMem::Absolute(address) = range.base {
                    for offset in 0..range.bytes {
                        let byte = address.wrapping_add(offset);
                        if byte < 256 {
                            owned.insert(byte as u8);
                        }
                    }
                }
            }
            for mem in memory.address_references {
                if let MirMem::Absolute(address) = mem
                    && address < 256
                {
                    owned.insert(address as u8);
                }
            }
        }
    }
    owned
}

#[cfg(test)]
pub(super) fn verify_storage(program: &MirProgram, routine: &MirRoutine, stage: MirCalleeEntryStage) -> Vec<MirDiagnostic> {
    verify_storage_with_address_control(program, routine, stage, &ObjectAddressControl::new(program))
}

pub(super) fn verify_storage_with_address_control(program: &MirProgram, routine: &MirRoutine,
    stage: MirCalleeEntryStage, addresses: &ObjectAddressControl<'_>) -> Vec<MirDiagnostic> {
    if !addresses.is_current(program) {
        return vec![MirDiagnostic::routine(&routine.name, "stale source-object address facts")];
    }
    let mut errors = handoff::verify(program, routine, stage);
    errors.extend(locals::verify_storage_with_address_control(program, routine, addresses));
    let Some(bindings) = &routine.frame.parameter_homes else {
        return errors;
    };
    let owned = source_zero_page(program, routine);
    for b in bindings {
        if b.home == MirParamHome::Inline {
            continue;
        }
        if !addresses.compiler_controls_address(routine.id, &MirMem::Param { id: b.param, offset: 0 })
            || !matches!(
                routine.abi,
                MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry
            )
        {
            errors.push(MirDiagnostic::routine(
                &routine.name,
                "exposed parameter cannot change its inline home",
            ));
        }
        if routine.frame.parameter_lanes(b.param).is_some_and(|lanes| {
            lanes
                .iter()
                .any(|lane| matches!(lane, MirParamLane::ZeroPage(slot) if owned.contains(&slot.0)))
        }) {
            errors.push(MirDiagnostic::routine(
                &routine.name,
                "parameter binding overlaps source-owned storage",
            ));
        }
    }
    errors
}

pub(super) fn verify_bindings(routine: &MirRoutine) -> Vec<MirDiagnostic> {
    let mut errors = locals::verify_bindings(routine);
    let Some(bindings) = &routine.frame.parameter_homes else {
        return errors;
    };
    let mut fail = |message: &str| errors.push(MirDiagnostic::routine(&routine.name, message));
    if routine.frame.abi_parameter_bytes().is_none_or(|n| n > crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES)
        && !routine.frame.sargs_layout_valid() {
        fail("SArgs parameter bindings require one contiguous zero-page frame in ABI order");
    }
    let mut ids = BTreeSet::new();
    let mut occupied = BTreeSet::new();
    for binding in bindings {
        if !ids.insert(binding.param) {
            fail("duplicate parameter binding");
        }
        let Some(slot) = routine
            .frame
            .params
            .iter()
            .find(|s| s.base == MirStorageBase::Param(binding.param))
        else {
            fail(
                "parameter binding has no matching Param declaration or conflicts with legacy ABI-only metadata",
            );
            continue;
        };
        let width = slot.storage_size;
        if binding.home == MirParamHome::Inline { continue; }
        if slot.offset != 0
            || slot.init.is_some()
            || !scalar_parameter_cell(slot)
            || !matches!(
                (slot.scalar_width, width),
                (Some(MirWidth::Byte), 1) | (Some(MirWidth::Word), 2)
            )
        {
            fail("parameter binding requires an uninitialized scalar byte or word at offset zero");
        }
        if matches!(binding.home, MirParamHome::Inline | MirParamHome::Absent) {
            continue;
        }
        let Some(lanes) = binding.home.lanes(width) else {
            fail("parameter binding requires exactly one physical lane per byte within zero page");
            continue;
        };
        let start = routine
            .frame
            .params
            .iter()
            .take_while(|p| p.base != slot.base)
            .map(|p| p.abi_byte_width().unwrap_or(0))
            .sum::<u16>();
        let mut registers = Vec::new();
        for (offset, lane) in lanes.into_iter().enumerate() {
            match lane {
                MirParamLane::Reg(reg) => {
                    if super::abi::physical_lanes(
                        &super::abi::action_arg_home(start + offset as u16, MirWidth::Byte),
                        MirWidth::Byte,
                    ) != Some(vec![MirParamLane::Reg(reg)])
                    {
                        fail("register binding must retain its canonical incoming ABI lanes");
                    }
                    if registers.contains(&reg) {
                        fail("parameter register lanes overlap");
                    }
                    registers.push(reg);
                }
                MirParamLane::ZeroPage(slot) => {
                    let byte = u16::from(slot.0);
                    if !occupied.insert(byte) {
                        fail("parameter zero-page bindings overlap");
                    }
                    if !routine.frame.fixed_zero_page.contains(&slot) {
                        fail("parameter zero-page binding is not reserved");
                    }
                    if routine.frame.zero_page_allocations.iter().any(|a| {
                        byte >= u16::from(a.start.0)
                            && byte < u16::from(a.start.0) + u16::from(a.size)
                    }) {
                        fail("parameter zero-page binding overlaps a virtual allocation");
                    }
                }
            }
        }
    }
    for slot in &routine.frame.params {
        match slot.base {
            MirStorageBase::Param(id) if ids.contains(&id) => {}
            _ => {
                fail("bound routine has a missing parameter binding or legacy allocation metadata")
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::ParamId;
    fn input() -> MirProgram {
        source("CARD FUNC Echo(CARD x) x==+1 RETURN(x)")
    }
    fn source(text: &str) -> MirProgram {
        let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        super::super::lower_program(&nir).unwrap()
    }
    fn bind(p: &mut MirProgram, home: MirParamHome) -> ParamId {
        let r = &mut p.routines[0];
        let MirStorageBase::Param(id) = r.frame.params[0].base else {
            panic!()
        };
        r.frame.parameter_homes = Some(vec![MirParamBinding { param: id, home }]);
        id
    }
    #[test]
    fn inline_binding_preserves_planning_addresses_and_prints_identity() {
        let mut p = input();
        let old = super::super::materialize::MaterializeLayout::new(&p, 0x3000);
        let id = bind(&mut p, MirParamHome::Inline);
        assert!(verify_bindings(&p.routines[0]).is_empty());
        let new = super::super::materialize::MaterializeLayout::new(&p, 0x3000);
        let mem = MirMem::Param { id, offset: 0 };
        assert_eq!(
            old.mem_address(p.routines[0].id, &mem),
            new.mem_address(p.routines[0].id, &mem)
        );
        assert!(super::super::format_program(&p).contains("x -> Inline"));
    }
    #[test]
    fn physical_clobber_cannot_kill_its_own_parameter_liveness_proof() {
        let mut p = input();
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xa0)));
        assert!(verify_lifetimes(&p.routines[0], None).is_empty());
        p.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xa0))),
                src: MirValue::ConstU8(99),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify_lifetimes(&p.routines[0], None).is_empty());
    }

    #[test]
    fn source_owned_zero_page_excludes_wrapping_absolute_storage() {
        let mut p = input();
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xa0)));
        p.globals.push(MirGlobal {
            id: crate::nir::SymbolId(99),
            name: "wrapped".into(),
            kind: "bytes".into(),
            width: None,
            storage_size: 258,
            backing: MirGlobalBacking::Absolute(0xffff),
            init: None,
        });
        assert!(!verify_storage(&p, &p.routines[0], MirCalleeEntryStage::PlannedCapture).is_empty());
    }

    #[test]
    fn physical_clobber_is_rejected_when_parameter_is_live_on_a_backedge() {
        let mut p = input();
        let id = bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xaa)));
        let r = &mut p.routines[0];
        let entry = r.blocks[0].id;
        r.blocks.truncate(1);
        r.blocks[0].ops = vec![MirOp::Load {
            dst: MirDef::VTemp(MirTempId(0)),
            src: MirAddr::Direct(MirMem::Param { id, offset: 0 }),
            width: MirWidth::Word,
        }];
        r.blocks[0].terminator = MirTerminator::Jump(MirEdge {
            target: entry,
            args: vec![],
        });
        assert!(verify_lifetimes(r, None).is_empty());
        r.blocks[0].ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xab))),
            src: MirValue::ConstU8(99),
            width: MirWidth::Byte,
        });
        assert!(!verify_lifetimes(r, None).is_empty());
    }

    #[test]
    fn absolute_locals_and_numeric_accesses_exclude_parameter_storage() {
        let mut p = input();
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xa0)));
        let mut local = p.routines[0].frame.params[0].clone();
        local.base = MirStorageBase::Absolute(0xa0);
        p.routines[0].frame.locals.push(local);
        assert!(!verify_storage(&p, &p.routines[0], MirCalleeEntryStage::PlannedCapture).is_empty());
        p.routines[0].frame.locals.clear();
        p.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::Absolute(0xa0)),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify_storage(&p, &p.routines[0], MirCalleeEntryStage::PlannedCapture).is_empty());
    }

    #[test]
    fn fragmented_pool_does_not_split_a_word_home() {
        let mut p = input();
        p.routines[0].frame.fixed_zero_page.extend(
            (0xa0..=0xa3)
                .chain((0xe0..=0xef).step_by(2))
                .map(MirFixedZpSlot),
        );
        // The two result bytes may be reused only when source storage permits.
        p.globals.push(MirGlobal {
            id: crate::nir::SymbolId(99),
            name: "reserved_results".into(),
            kind: "word".into(),
            width: Some(MirWidth::Word),
            storage_size: 2,
            backing: MirGlobalBacking::Absolute(0xa0),
            init: None,
        });
        assert!(zero_page::bindings(&p, &p.routines[0], &ObjectAddressControl::new(&p)).is_none());
    }

    #[test]
    fn malformed_bindings_are_rejected() {
        let mut p = input();
        bind(&mut p, MirParamHome::Registers(vec![MirReg::A]));
        assert!(!verify_bindings(&p.routines[0]).is_empty());
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xff)));
        assert!(!verify_bindings(&p.routines[0]).is_empty());
        let id = bind(&mut p, MirParamHome::Inline);
        p.routines[0]
            .frame
            .parameter_homes
            .as_mut()
            .unwrap()
            .push(MirParamBinding {
                param: id,
                home: MirParamHome::Absent,
            });
        assert!(!verify_bindings(&p.routines[0]).is_empty());
        p.routines[0].frame.parameter_homes = Some(vec![]);
        assert!(!verify_bindings(&p.routines[0]).is_empty());
        bind(&mut p, MirParamHome::Inline);
        p.routines[0].frame.params[0].base = MirStorageBase::ParamAbiOnly(id);
        assert!(!verify_bindings(&p.routines[0]).is_empty());
    }

    fn materialize(p: MirProgram) -> MirProgram {
        super::super::materialize_program_with_origin_and_runtime(
            p,
            &super::super::Mir6502Config {
                enable_peepholes: false,
                ..Default::default()
            },
            0x3000,
            crate::compiler::Runtime::ActionCart,
        )
        .unwrap()
    }
    fn emit(p: &MirProgram) -> (Vec<u8>, super::super::emit::MirEmissionSummary) {
        let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
        let summary =
            super::super::emit::emit_program_with_physical_memory(p, 0x3000, &mut emitter, true)
                .unwrap();
        (emitter.finish_with_relocations().unwrap().bytes, summary)
    }
    #[test]
    fn inline_and_zero_page_layouts_agree_with_final_emission() {
        let mut p = input();
        let baseline = emit(&materialize(p.clone()));
        let id = bind(&mut p, MirParamHome::Inline);
        let inline = emit(&materialize(p.clone()));
        assert_eq!(baseline.0, inline.0);
        assert_eq!(baseline.1.storage_symbols, inline.1.storage_symbols);
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xe2)));
        p.routines[0]
            .frame
            .fixed_zero_page
            .extend([MirFixedZpSlot(0xe2), MirFixedZpSlot(0xe3)]);
        let p = materialize(p);
        let (_, summary) = emit(&p);
        let layout = super::super::materialize::MaterializeLayout::new(&p, 0x3000);
        for offset in 0..2 {
            let mem = MirMem::Param { id, offset };
            assert_eq!(
                layout.mem_address(p.routines[0].id, &mem),
                Some(0xe2 + offset)
            );
            assert_eq!(
                summary.physical_memory.address(p.routines[0].id, &mem),
                Some(0xe2 + offset)
            );
        }
        let symbol = summary
            .storage_symbols
            .iter()
            .find(|s| s.name == "x")
            .unwrap();
        assert_eq!((symbol.address, symbol.size), (0xe2, 2));
    }

    #[test]
    fn register_binding_never_invents_a_memory_address() {
        let mut p = input();
        let id = bind(&mut p, MirParamHome::Registers(vec![MirReg::A, MirReg::X]));
        let layout = super::super::materialize::MaterializeLayout::new(&p, 0x3000);
        assert_eq!(
            layout.mem_address(p.routines[0].id, &MirMem::Param { id, offset: 0 }),
            None
        );
        let errors = super::super::verify_program(&p, MirPhase::PostHome).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("residual memory reference"))
        );
    }

    #[test]
    fn exposure_and_source_owned_zero_page_reject_relocation() {
        let mut p = source("CARD FUNC Echo(CARD x) RETURN(@x)");
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xe2)));
        p.routines[0]
            .frame
            .fixed_zero_page
            .extend([MirFixedZpSlot(0xe2), MirFixedZpSlot(0xe3)]);
        let errors = super::super::verify_program(&p, MirPhase::PreMaterialization).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("exposed parameter"))
        );

        let mut p = input();
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xe2)));
        p.routines[0]
            .frame
            .fixed_zero_page
            .extend([MirFixedZpSlot(0xe2), MirFixedZpSlot(0xe3)]);
        p.globals.push(MirGlobal {
            id: crate::nir::SymbolId(999),
            name: "reserved".into(),
            kind: "byte".into(),
            width: Some(MirWidth::Byte),
            storage_size: 1,
            backing: MirGlobalBacking::Absolute(0xe3),
            init: None,
        });
        let errors = super::super::verify_program(&p, MirPhase::PreMaterialization).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("source-owned storage"))
        );
    }

    #[test]
    fn later_trials_keep_the_exact_selected_input_and_executable() {
        let ast = crate::parser::parse(
            &crate::lexer::tokenize(
                &include_str!("../../fixtures/runtime/parameter_registers.act")
                    .replace("\r\n", "\n"),
            )
            .unwrap(),
        )
        .unwrap();
        let model =
            crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
                .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(
            &crate::semantic::ir::lower_program(&ast, &model),
        ))
        .unwrap();
        let config = super::super::Mir6502Config {
            enable_small_leaf_inlining: false,
            enable_parameter_argument_handoff: true,
            ..super::super::Mir6502Config::optimized()
        };
        let runtime = crate::runtime::Runtime::ActionCart;
        for leaf_path in 0..3 {
            let session = super::super::trace::Mir6502Trace::default();
            let trace = session.context();
            let (program, resume) =
                super::super::prepare_program_for_emission(&nir, 0x3000, &config, runtime, &trace)
                    .unwrap();
            let resume = resume.unwrap();
            let baseline =
                super::super::finalize_output(&nir, program, 0x3000, runtime, &trace).unwrap();
            let original_size = baseline.output.bytes.len();
            let mut selected = SelectedPlacement {
                input: Arc::new(resume.core.checkpoint.input.program.clone()),
                executable: baseline,
            };
            if leaf_path != 0 {
                let mut proposal = propose(&selected.input, &selected.executable.program, false, 0x3000).expect("leaf controls qualify");
                if leaf_path == 2 {
                    proposal.routines[0].frame.parameter_homes = Some(vec![]);
                }
                selected = try_proposal(
                    &nir,
                    selected,
                    &resume,
                    proposal,
                    "parameter-homes",
                    &config,
                    0x3000,
                    runtime,
                    &trace,
                );
            }
            assert_eq!(
                selected.executable.output.bytes.len() < original_size,
                leaf_path == 1
            );
            // The accepted leaf input includes its removed absent stores, not
            // just a copied binding table. Exact resumption must reproduce it.
            for trial_kind in 0..3 {
                let expected_input = Arc::clone(&selected.input);
                let expected = selected.executable.clone();
                let mut proposal = (*selected.input).clone();
                if trial_kind == 1 {
                    proposal.routines[0].frame.parameter_homes = Some(vec![]);
                } else if trial_kind == 2 {
                    for i in 0..20 {
                        proposal.routines[0].blocks[0].ops.insert(
                            0,
                            MirOp::Store {
                                dst: MirAddr::Direct(MirMem::Absolute(0x600 + i)),
                                src: MirValue::ConstU8(i as u8),
                                width: MirWidth::Byte,
                            },
                        );
                    }
                }
                let before = session.manifest()["work"]["materializations"]
                    .as_u64()
                    .unwrap();
                let event_start = session.events().len();
                selected = try_proposal(
                    &nir,
                    selected,
                    &resume,
                    proposal,
                    "parameter-argument-handoff",
                    &config,
                    0x3000,
                    runtime,
                    &trace,
                );
                assert!(Arc::ptr_eq(&selected.input, &expected_input));
                assert_eq!(selected.executable.output, expected.output);
                assert_eq!(selected.executable.program, expected.program);
                assert_eq!(selected.executable.costs, expected.costs);
                assert_eq!(selected.executable.memory, expected.memory);
                assert_eq!(selected.executable.trace_snapshot, expected.trace_snapshot);
                let after = session.manifest()["work"]["materializations"]
                    .as_u64()
                    .unwrap();
                assert!(after <= before + 1);
                assert!(
                    !session.events()[event_start..]
                        .iter()
                        .any(|e| e.kind == "attempt-start"
                            && matches!(
                                e.data["stage"].as_str(),
                                Some(
                                    "inlining"
                                        | "loop_motion"
                                        | "comparison_loops"
                                        | "call_predicates"
                                        | "address_values"
                                        | "copy_values"
                                        | "scalar_captures"
                                        | "captured_comparisons"
                                )
                            ))
                );
            }
        }
    }

    #[test]
    fn invalid_and_growing_trials_restore_the_complete_fallback() {
        let ast = crate::parser::parse(
            &crate::lexer::tokenize("CARD FUNC Echo(CARD x) RETURN(x)").unwrap(),
        )
        .unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let config = super::super::Mir6502Config {
            enable_parameter_home_selection: true,
            ..Default::default()
        };
        let trace = super::super::trace::Trace::disabled();
        for invalid in [false, true] {
            let (program, resume) = super::super::prepare_program_for_emission(
                &nir,
                0x3000,
                &config,
                crate::runtime::Runtime::ActionCart,
                &trace,
            )
            .unwrap();
            let baseline = super::super::finalize_output(
                &nir,
                program,
                0x3000,
                crate::runtime::Runtime::ActionCart,
                &trace,
            )
            .unwrap();
            let expected = baseline.output.clone();
            let resume = resume.unwrap();
            let mut proposal = resume.core.checkpoint.input.program.clone();
            if invalid {
                proposal.routines[0].frame.parameter_homes = Some(vec![]);
            } else {
                bind(&mut proposal, MirParamHome::Inline);
            }
            let result = try_proposal(
                &nir,
                SelectedPlacement {
                    input: Arc::new(resume.core.checkpoint.input.program.clone()),
                    executable: baseline,
                },
                &resume,
                proposal,
                "parameter-homes",
                &config,
                0x3000,
                crate::runtime::Runtime::ActionCart,
                &trace,
            );
            assert_eq!(result.executable.output, expected);
        }
    }
}
