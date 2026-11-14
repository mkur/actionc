//! One parameter binding owns entry capture, working storage, and layout.
//! Selection resumes one core continuation and accepts only a smaller image.
use super::{diagnostics::MirDiagnostic, ir::*};
use std::collections::BTreeSet;
mod lifetimes;
mod zero_page;
pub(super) use lifetimes::verify as verify_lifetimes;

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
    let Some(proposal) = propose(&resume.core.checkpoint.input.program) else {
        trace.skipped("parameter-homes", "no-candidates");
        return baseline;
    };
    try_proposal(
        nir, baseline, resume, proposal, config, origin, runtime, trace,
    )
}

fn propose(input: &MirProgram) -> Option<MirProgram> {
    let mut proposal = input.clone();
    let mut changed = false;
    for routine in &mut proposal.routines {
        if !eligible(input, routine) {
            continue;
        }
        if let Some(bindings) =
            register_bindings(routine).or_else(|| zero_page::bindings(input, routine))
        {
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
            changed = true;
        }
    }
    changed.then_some(proposal)
}

/// Deliberately closed expansion set. Additional shapes must establish their
/// hidden helper, scratch, and observable-memory behavior before admission.
fn eligible(program: &MirProgram, routine: &MirRoutine) -> bool {
    use super::analysis::effects::classify_op;
    routine.abi == MirRoutineAbi::Action
        && routine.frame.parameter_homes.is_none()
        && !routine.frame.params.is_empty()
        && routine
            .frame
            .params
            .iter()
            .map(|p| p.abi_byte_width().unwrap_or(5))
            .sum::<u16>()
            <= 4
        && routine.frame.params.iter().all(|p| {
            matches!(p.base, MirStorageBase::Param(id) if !exposed(program, routine, id))
                && p.storage == MirStorageClass::Scalar
                && p.offset == 0
                && p.init.is_none()
                && matches!(
                    (p.scalar_width, p.storage_size),
                    (Some(MirWidth::Byte), 1) | (Some(MirWidth::Word), 2)
                )
        })
        // Data-dependent pointer arithmetic has a separate fused staging
        // strategy. Keep it on the legacy path until that consumer cooperates
        // with resident homes. This first pointer slice admits immediate word
        // arithmetic only; scalar routines retain their existing native scope.
        && (!has_pointer_reads(routine) || routine.blocks.iter().flat_map(|b| &b.ops).all(|op| {
            !matches!(op, MirOp::Binary { left, right, width: MirWidth::Word, .. }
                if !matches!(left, MirValue::ConstU8(_) | MirValue::ConstU16(_))
                && !matches!(right, MirValue::ConstU8(_) | MirValue::ConstU16(_)))
        }))
        && routine.blocks.iter().all(|b| {
            b.ops.iter().all(|op| {
                let effects = classify_op(op);
                !effects.memory.opaque
                    && !effects.memory.indirect_writes
                    && matches!(
                        op,
                        MirOp::LoadImm { .. }
                            | MirOp::Move { .. }
                            | MirOp::Extend { .. }
                            | MirOp::Truncate { .. }
                            | MirOp::Compare { .. }
                            | MirOp::UpdateMem { .. }
                            | MirOp::Binary {
                                op: MirBinaryOp::Add
                                    | MirBinaryOp::Sub
                                    | MirBinaryOp::And
                                    | MirBinaryOp::Or
                                    | MirBinaryOp::Xor,
                                ..
                            }
                            | MirOp::Load {
                                src: MirAddr::Direct(_)
                                    | MirAddr::Deref { .. }
                                    | MirAddr::PointerCell { .. },
                                ..
                            }
                            | MirOp::Store {
                                dst: MirAddr::Direct(_),
                                ..
                            }
                    )
            })
        })
}

fn has_pointer_reads(routine: &MirRoutine) -> bool {
    routine.blocks.iter().flat_map(|b| &b.ops).any(|op| {
        matches!(
            op,
            MirOp::Load {
                src: MirAddr::Deref { .. } | MirAddr::PointerCell { .. },
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
        || routine
            .frame
            .params
            .iter()
            .map(|p| p.storage_size)
            .sum::<u16>()
            <= 2
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
    baseline: super::preparation::Executable,
    resume: super::preparation::Resume,
    proposal: MirProgram,
    config: &super::Mir6502Config,
    origin: u16,
    runtime: crate::runtime::Runtime,
    trace: &super::trace::Trace,
) -> super::preparation::Executable {
    let child = trace.child("parameter-homes", "batch-trial");
    let trial =
        super::preparation::resume_output(nir, resume, proposal, config, origin, runtime, &child);
    let accepted = trial.as_ref().is_ok_and(|p| p.improves(&baseline));
    child.event("parameter-home-result", || serde_json::json!({
        "accepted": accepted,
        "routine_cost_changes": trial.as_ref().ok().map(|p| p.program.routines.iter().filter_map(|r| {
            let before = baseline.costs.get(&r.id)?;
            let after = p.costs.get(&r.id)?;
            (before != after).then(|| serde_json::json!({"routine": r.name, "before": before, "after": after}))
        }).collect::<Vec<_>>()),
        "diagnostics": trial.as_ref().err().map(|errors| format!("{errors:?}")),
        "before_bytes": crate::codegen::format_load_file(&baseline.output).len(),
        "after_bytes": trial.as_ref().ok().map(|p| crate::codegen::format_load_file(&p.output).len()),
        "reason": if accepted {"smaller-image"} else if trial.is_err() {"invalid-trial"} else {"no-final-saving-or-routine-growth"},
    }));
    let selected = if accepted { trial.unwrap() } else { baseline };
    trace.snapshot("pre-emission", &selected.program);
    selected
}

/// Address uses expose the parameter object, unlike dereferencing its value.
pub(super) fn exposed(program: &MirProgram, routine: &MirRoutine, id: crate::nir::ParamId) -> bool {
    use super::analysis::effects::{classify_op, classify_terminator};
    let target = MirDataRelocationTarget::Param {
        routine: routine.id,
        id,
    };
    let image_has = |image: &MirDataImage| image.relocations.iter().any(|r| r.target == target);
    program.statics.iter().any(|s| image_has(&s.image))
        || program.globals.iter().any(|g| match &g.init {
            Some(MirGlobalInit::Bytes { image, .. }) => image_has(image),
            Some(MirGlobalInit::Descriptor { backing, .. }) => image_has(&backing.image),
            _ => false,
        })
        || program.routines.iter().flat_map(|r| r.frame.params.iter().chain(&r.frame.locals)).any(|s| match &s.init {
            Some(MirStorageInit::Bytes { image, .. }) => image_has(image),
            Some(MirStorageInit::Descriptor { backing, .. }) => image_has(&backing.image),
            _ => false,
        })
        || routine.blocks.iter().any(|b| {
            b.ops.iter().map(classify_op).map(|e| e.memory.address_references)
                .chain(std::iter::once(classify_terminator(&b.terminator).memory.address_references))
                .flatten().any(|m| matches!(m, MirMem::Param { id: p, .. } if p == id))
            // Machine payloads can refer to parameters by relocation or raw bytes.
            || b.ops.iter().any(|op| matches!(op, MirOp::MachineBlock { .. }))
        })
}

/// Explicit source storage wins over compiler reservations, including absolute
/// locals and numeric direct accesses that do not have a global declaration.
fn source_zero_page(program: &MirProgram, routine: &MirRoutine) -> BTreeSet<u8> {
    use super::analysis::effects::{classify_op, classify_terminator};
    let mut owned: BTreeSet<_> = (0..=255u16)
        .filter(|byte| {
            program.globals.iter().any(|g| {
                matches!(g.backing,
            MirGlobalBacking::Absolute(address) if byte.wrapping_sub(address) < g.storage_size)
            }) || program
                .routines
                .iter()
                .flat_map(|r| r.frame.params.iter().chain(&r.frame.locals))
                .any(|slot| {
                    matches!(slot.base, MirStorageBase::Absolute(address)
                if byte.wrapping_sub(address) < slot.storage_size)
                })
        })
        .map(|b| b as u8)
        .collect();
    for block in &routine.blocks {
        for memory in block
            .ops
            .iter()
            .map(|op| classify_op(op).memory)
            .chain(std::iter::once(
                classify_terminator(&block.terminator).memory,
            ))
        {
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

pub(super) fn verify_storage(program: &MirProgram, routine: &MirRoutine) -> Vec<MirDiagnostic> {
    let Some(bindings) = &routine.frame.parameter_homes else {
        return vec![];
    };
    let mut errors = Vec::new();
    let owned = source_zero_page(program, routine);
    for b in bindings {
        if b.home == MirParamHome::Inline {
            continue;
        }
        if exposed(program, routine, b.param)
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
        if let MirParamHome::ZeroPage(start) = b.home {
            let width = routine
                .frame
                .params
                .iter()
                .find(|s| s.base == MirStorageBase::Param(b.param))
                .map_or(0, |s| s.storage_size);
            if (0..width).any(|lane| owned.contains(&start.0.wrapping_add(lane as u8))) {
                errors.push(MirDiagnostic::routine(
                    &routine.name,
                    "parameter binding overlaps source-owned storage",
                ));
            }
        }
    }
    errors
}

pub(super) fn verify_bindings(routine: &MirRoutine) -> Vec<MirDiagnostic> {
    let Some(bindings) = &routine.frame.parameter_homes else {
        return vec![];
    };
    let mut errors = Vec::new();
    let mut fail = |message: &str| errors.push(MirDiagnostic::routine(&routine.name, message));
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
        if slot.offset != 0
            || slot.init.is_some()
            || slot.storage != MirStorageClass::Scalar
            || !matches!(
                (slot.scalar_width, width),
                (Some(MirWidth::Byte), 1) | (Some(MirWidth::Word), 2)
            )
        {
            fail("parameter binding requires an uninitialized scalar byte or word at offset zero");
        }
        match &binding.home {
            MirParamHome::Inline | MirParamHome::Absent => {}
            MirParamHome::Registers(regs) => {
                if regs.len() != usize::from(width)
                    || regs
                        .iter()
                        .any(|r| !matches!(r, MirReg::A | MirReg::X | MirReg::Y))
                {
                    fail("parameter register binding requires one A/X/Y register per byte");
                }
                let start = routine
                    .frame
                    .params
                    .iter()
                    .take_while(|p| p.base != slot.base)
                    .map(|p| p.abi_byte_width().unwrap_or(0))
                    .sum::<u16>();
                if regs.iter().enumerate().any(|(lane, reg)| {
                    super::abi::action_arg_home(start + lane as u16, MirWidth::Byte)
                        != MirArgHome::Reg(*reg)
                }) {
                    fail("register binding must retain its canonical incoming ABI lanes");
                }
                let mut lanes = Vec::new();
                for reg in regs {
                    if lanes.contains(reg) {
                        fail("parameter register lanes overlap");
                    }
                    lanes.push(*reg);
                }
            }
            MirParamHome::ZeroPage(start) => {
                if u16::from(start.0) + width > 256 {
                    fail("parameter pair crosses the zero-page boundary");
                }
                for byte in u16::from(start.0)..u16::from(start.0).saturating_add(width).min(256) {
                    if !occupied.insert(byte) {
                        fail("parameter zero-page bindings overlap");
                    }
                    if !routine
                        .frame
                        .fixed_zero_page
                        .contains(&MirFixedZpSlot(byte as u8))
                    {
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
        assert!(verify_lifetimes(&p.routines[0]).is_empty());
        p.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xa0))),
                src: MirValue::ConstU8(99),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify_lifetimes(&p.routines[0]).is_empty());
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
        assert!(!verify_storage(&p, &p.routines[0]).is_empty());
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
        assert!(verify_lifetimes(r).is_empty());
        r.blocks[0].ops.push(MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xab))),
            src: MirValue::ConstU8(99),
            width: MirWidth::Byte,
        });
        assert!(!verify_lifetimes(r).is_empty());
    }

    #[test]
    fn absolute_locals_and_numeric_accesses_exclude_parameter_storage() {
        let mut p = input();
        bind(&mut p, MirParamHome::ZeroPage(MirFixedZpSlot(0xa0)));
        let mut local = p.routines[0].frame.params[0].clone();
        local.base = MirStorageBase::Absolute(0xa0);
        p.routines[0].frame.locals.push(local);
        assert!(!verify_storage(&p, &p.routines[0]).is_empty());
        p.routines[0].frame.locals.clear();
        p.routines[0].blocks[0].ops.insert(
            0,
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::Absolute(0xa0)),
                width: MirWidth::Byte,
            },
        );
        assert!(!verify_storage(&p, &p.routines[0]).is_empty());
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
        assert!(zero_page::bindings(&p, &p.routines[0]).is_none());
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
                baseline,
                resume,
                proposal,
                &config,
                0x3000,
                crate::runtime::Runtime::ActionCart,
                &trace,
            );
            assert_eq!(result.output, expected);
        }
    }
}
