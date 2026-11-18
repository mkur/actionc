use super::*;
use crate::mir6502::{self, analysis::known_callees::MirKnownCalleeSummaries};

fn source(text: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    mir6502::lower_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap()
}

fn bind(p: &mut MirProgram, index: usize, start: u8) -> LocalId {
    let r = &mut p.routines[index];
    let mut offset = 0;
    r.frame.local_homes = Some(
        r.frame
            .locals
            .iter()
            .filter_map(|s| {
                let MirStorageBase::Local(local) = s.base else {
                    return None;
                };
                let first = start + offset;
                offset += s.storage_size as u8;
                Some(MirLocalBinding {
                    local,
                    home: MirLocalHome::ZeroPage(MirFixedZpSlot(first)),
                })
            })
            .collect(),
    );
    r.frame
        .fixed_zero_page
        .extend((start..start + offset).map(MirFixedZpSlot));
    r.frame.local_homes.as_ref().unwrap()[0].local
}

fn materialize(p: MirProgram) -> MirProgram {
    mir6502::materialize_program_with_origin_and_runtime(
        p,
        &mir6502::Mir6502Config::default(),
        0x3000,
        crate::compiler::Runtime::ActionCart,
    )
    .unwrap()
}

fn emit(p: &MirProgram) -> (Vec<u8>, crate::mir6502::emit::MirEmissionSummary) {
    let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x3000);
    let summary =
        crate::mir6502::emit::emit_program_with_physical_memory(p, 0x3000, &mut emitter, true)
            .unwrap();
    (emitter.finish_with_relocations().unwrap().bytes, summary)
}

#[test]
fn explicit_local_binding_controls_layout_cost_maps_and_physical_effects() {
    let mut p = source("CARD FUNC Keep(CARD v) CARD kept kept=v kept==+1 RETURN(kept)");
    let old = materialize(p.clone());
    let (old_bytes, old_summary) = emit(&old);
    let id = bind(&mut p, 0, 0xe4);
    let mut inline = p.clone();
    inline.routines[0].frame.local_homes.as_mut().unwrap()[0].home = MirLocalHome::Inline;
    inline.routines[0]
        .frame
        .fixed_zero_page
        .retain(|s| ![0xe4, 0xe5].contains(&s.0));
    let inline = emit(&materialize(inline));
    assert_eq!(old_bytes, inline.0);
    assert_eq!(old_summary.storage_symbols, inline.1.storage_symbols);
    mir6502::verify_program(&p, MirPhase::PreMaterialization).unwrap();
    let new = materialize(p);
    let (new_bytes, summary) = emit(&new);
    let layout = mir6502::materialize::MaterializeLayout::new(&new, 0x3000);
    for offset in 0..2 {
        let mem = MirMem::Local { id, offset };
        assert_eq!(
            layout.mem_address(new.routines[0].id, &mem),
            Some(0xe4 + offset)
        );
        assert_eq!(
            summary.physical_memory.address(new.routines[0].id, &mem),
            Some(0xe4 + offset)
        );
    }
    let symbol = summary
        .storage_symbols
        .iter()
        .find(|s| s.name == "kept")
        .unwrap();
    assert_eq!((symbol.address, symbol.size), (0xe4, 2));
    assert_eq!(
        old_bytes.len() - new_bytes.len(),
        mir6502::emit::routine_costs(&old, &old_summary)[&old.routines[0].id]
            - mir6502::emit::routine_costs(&new, &summary)[&new.routines[0].id]
    );
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&new);
    let writes = known.get(new.routines[0].id).unwrap().writes();
    for slot in [0xe4, 0xe5] {
        assert!(!writes.preserves_fixed_byte(MirFixedZpSlot(slot), known.fixed_storage()));
    }
    assert!(writes.preserves_fixed_byte(MirFixedZpSlot(0xe6), known.fixed_storage()));
    let crate::mir6502::analysis::known_callees::MirKnownMemoryReads::Exact(reads) =
        known.get(new.routines[0].id).unwrap().reads()
    else {
        panic!("expected exact reads")
    };
    assert!(reads.contains(&MirMem::FixedZeroPage(MirFixedZpSlot(0xe4))));
    assert!(reads.contains(&MirMem::FixedZeroPage(MirFixedZpSlot(0xe5))));
    assert!(mir6502::format_program(&new).contains("kept -> ZeroPage"));
    // A damaged authoritative table cannot silently recover legacy placement
    // in a callee summary, even before structural verification reports it.
    let mut missing = new.clone();
    missing.routines[0].frame.local_homes = Some(vec![]);
    let layout = mir6502::materialize::MaterializeLayout::new(&missing, 0x3000);
    assert_eq!(
        layout.mem_address(missing.routines[0].id, &MirMem::Local { id, offset: 0 }),
        None
    );
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&missing);
    assert!(matches!(
        known.get(missing.routines[0].id).unwrap().writes(),
        crate::mir6502::analysis::known_callees::MirKnownMemoryWrites::Unknown
    ));
}

#[test]
fn local_initialization_requires_every_path_and_every_byte() {
    for (body, initialized) in [
        ("n=0 IF x THEN n=1 FI", true),
        ("IF x THEN n=1 ELSE n=2 FI", true),
        ("IF x THEN n=1 FI", false),
        ("WHILE x DO n=1 x==-1 OD", false),
        ("n==+1", false),
        ("", false),
    ] {
        let mut p = source(&format!("CARD FUNC Keep(BYTE x) CARD n {body} RETURN(n)"));
        let id = bind(&mut p, 0, 0xe4);
        assert_eq!(
            !entry_dependencies(&p.routines[0]).unwrap().contains(&id),
            initialized,
            "{body}"
        );
        assert_eq!(
            verify_storage(&p, &p.routines[0]).is_empty(),
            initialized,
            "{body}"
        );
    }
    let mut p = source("CARD FUNC Keep() CARD n n=$1234 RETURN(n)");
    let id = bind(&mut p, 0, 0xe4);
    let mut partial = p.clone();
    for op in partial.routines[0]
        .blocks
        .iter_mut()
        .flat_map(|b| &mut b.ops)
    {
        if let MirOp::Store {
            dst: MirAddr::Direct(MirMem::Local { .. }),
            width,
            ..
        } = op
        {
            *width = MirWidth::Byte;
        }
    }
    assert!(
        entry_dependencies(&partial.routines[0])
            .unwrap()
            .contains(&id)
    );
    for op in p.routines[0].blocks.iter_mut().flat_map(|b| &mut b.ops) {
        if let MirOp::Store { dst, .. } = op
            && matches!(dst, MirAddr::Direct(MirMem::Local { .. }))
        {
            *dst = MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xe4)));
        }
    }
    assert!(entry_dependencies(&p.routines[0]).unwrap().contains(&id));
}

#[test]
fn local_residence_checks_expanded_callees_and_current_physical_aliases() {
    let mut p = source(
        "BYTE FUNC Touch(BYTE x) RETURN(x+1) CARD FUNC Keep(CARD v) CARD n n=v Touch(7) RETURN(n)",
    );
    bind(&mut p, 1, 0xe4);
    let mut p = materialize(p);
    mir6502::verify_program(&p, MirPhase::PreEmission).unwrap();
    p.routines[0].blocks[0].ops.insert(
        0,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(0xe5)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    );
    assert!(
        mir6502::verify_program(&p, MirPhase::PreEmission)
            .unwrap_err()
            .iter()
            .any(|e| e.message.contains("clobbers live"))
    );
}

#[test]
fn local_pointer_binding_reaches_resident_consumers_and_rewrite_proofs() {
    let mut p = source("BYTE FUNC Read(BYTE POINTER p) BYTE POINTER q q=p RETURN(q^)");
    let id = bind(&mut p, 0, 0xe4);
    assert_eq!(
        p.routines[0]
            .frame
            .object_pointer_pair(&MirMem::Local { id, offset: 0 }),
        Some(MirFixedZpSlot(0xe4))
    );
    let p = materialize(p);
    let text = mir6502::format_program(&p);
    assert!(text.contains("(zp$E4)"), "{text}");
    emit(&p);
}

#[test]
fn invalid_or_observable_local_bindings_are_rejected() {
    let mut p = source("CARD FUNC Keep(CARD x) CARD n n=x RETURN(n)");
    let id = bind(&mut p, 0, 0xe4);
    for mode in 0..6 {
        let mut bad = p.clone();
        let r = &mut bad.routines[0];
        match mode {
            0 => r.frame.local_homes.as_mut().unwrap().clear(),
            1 => r.frame.local_homes.as_mut().unwrap().push(MirLocalBinding {
                local: id,
                home: MirLocalHome::Inline,
            }),
            2 => {
                r.frame.local_homes.as_mut().unwrap()[0].home =
                    MirLocalHome::ZeroPage(MirFixedZpSlot(0xff))
            }
            3 => r.frame.fixed_zero_page.clear(),
            4 => r.frame.zero_page_allocations.push(MirZpAllocation {
                slot: MirZpSlot(99),
                start: MirFixedZpSlot(0xe5),
                size: 1,
            }),
            _ => {
                let MirStorageBase::Param(param) = r.frame.params[0].base else {
                    panic!()
                };
                r.frame.parameter_homes = Some(vec![MirParamBinding {
                    param,
                    home: MirParamHome::ZeroPage(MirFixedZpSlot(0xe4)),
                }]);
            }
        }
        assert!(!super::verify_bindings(r).is_empty(), "mode {mode}");
    }
    let mut initialized = p.clone();
    initialized.routines[0].frame.locals[0].init = Some(MirStorageInit::ZeroFill {
        bytes: 2,
        mutable: true,
        section: "local".into(),
    });
    assert!(!verify_storage(&initialized, &initialized.routines[0]).is_empty());
    let mut alias = p.clone();
    let mut slot = alias.routines[0].frame.locals[0].clone();
    slot.base = MirStorageBase::LocalAlias {
        id: LocalId(99),
        target: id,
    };
    alias.routines[0].frame.locals.push(slot);
    assert!(!verify_storage(&alias, &alias.routines[0]).is_empty());
    for source_text in [
        "CARD FUNC Keep() CARD n n=1 RETURN(@n)",
        "BYTE z=$E4 CARD FUNC Keep() CARD n n=1 RETURN(n)",
    ] {
        let mut p = source(source_text);
        bind(&mut p, 0, 0xe4);
        assert!(
            !verify_storage(&p, &p.routines[0]).is_empty(),
            "{source_text}"
        );
    }
}

#[test]
fn object_storage_native_pointees_keep_lifetime_and_physical_obligations() {
    use crate::mir6502::analysis::{effects::writes_ordinary_pointee,
        known_callees::MirKnownMemoryWrites, storage_ownership::ObjectAddressControl};
    for ty in ["BYTE", "CARD"] {
        let text = format!("CARD FUNC Keep({ty} ARRAY v BYTE x) CARD n n=$1234 v(x)=n RETURN(n+v(x))");
        let mut p = source(&text);
        bind(&mut p, 0, 0xe4);
        let p = materialize(p);
        mir6502::verify_program(&p, MirPhase::PreEmission).unwrap();
        let r = &p.routines[0];
        assert!(r.blocks.iter().flat_map(|b| &b.ops).any(writes_ordinary_pointee));
        let addresses = ObjectAddressControl::new(&p);
        assert!(verify_residence(r, None, Some(&addresses)).is_empty());
        let stale = p.clone();
        assert!(!verify_residence(&stale.routines[0], None, Some(&addresses)).is_empty());
        assert!(matches!(MirKnownCalleeSummaries::analyze_for_rewriting(&p).get(r.id).unwrap().writes(), MirKnownMemoryWrites::Unknown));
        let (_, summary) = emit(&p);
        super::super::physical::verify(&p, &summary.physical_memory).unwrap();
        for lane in [0xe4, 0xe5] {
            let mut clobber = p.clone();
            let ops = &mut clobber.routines[0].blocks[0].ops;
            let index = ops.iter().position(writes_ordinary_pointee).unwrap();
            ops.insert(index, MirOp::Store { dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(lane))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)), width: MirWidth::Byte });
            assert!(mir6502::verify_program(&clobber, MirPhase::PreEmission).unwrap_err()
                .iter().any(|e| e.message.contains("clobbers live")));
        }
        // Explicit numeric observations are separate from logical exposure.
        let mut numeric = p.clone();
        numeric.routines[0].blocks[0].ops.insert(0, MirOp::Load { dst: MirDef::Reg(MirReg::A),
            src: MirAddr::Direct(MirMem::Absolute(0xe5)), width: MirWidth::Byte });
        assert!(super::super::physical::verify(&numeric, &summary.physical_memory).is_err());
        // Keep the complete declaration inventory, including fixed aliases.
        let mut aliases = summary.physical_memory.clone();
        let mut records = aliases.source_storage().unwrap().to_vec();
        let mut alias = records.iter().find(|s| s.address == 0xe4).unwrap().clone();
        alias.object = crate::mir6502::analysis::record_bases::SourceObject::Global(crate::nir::SymbolId(999));
        alias.binding = None;
        records.push(alias);
        aliases.protect_source_storage(Some(records));
        assert!(super::super::physical::verify(&p, &aliases).is_err());
    }
    let mut persistent = source("CARD FUNC Keep(CARD ARRAY v) CARD n v(0)=1 RETURN(n)");
    bind(&mut persistent, 0, 0xe4);
    assert!(!verify_storage(&persistent, &persistent.routines[0]).is_empty());
}

#[test]
fn object_storage_admission_keeps_shift_and_helper_boundaries() {
    for op in ["LSH", "RSH"] {
        for count in 0..=9 {
            let p = source(&format!("BYTE FUNC Shift(BYTE x,keep) x=x {op} {count} RETURN(x XOR keep)"));
            assert!(super::super::eligible_with_calls(&ObjectAddressControl::new(&p), &p.routines[0], false, false));
        }
    }
    for text in [
        "BYTE FUNC Shift(BYTE x,keep) x=x LSH keep RETURN(x)",
        "CARD FUNC Shift(CARD x BYTE keep) x=x RSH 5 RETURN(x)",
        "CARD FUNC Mul(CARD x,y) RETURN(x*y)",
        "CARD FUNC Exposed(CARD x) CARD p p=@x RETURN(p)",
    ] {
        let p = source(text);
        assert!(!super::super::eligible_with_calls(&ObjectAddressControl::new(&p), &p.routines[0], false, false), "{text}");
    }
}
