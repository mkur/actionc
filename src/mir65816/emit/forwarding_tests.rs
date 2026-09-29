use super::*;

fn program(source: &str, optimize: bool) -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    let nir = if optimize {
        crate::nir::optimize_program(&nir).unwrap()
    } else {
        nir
    };
    crate::mir65816::lower_program(&nir).unwrap()
}

#[test]
fn one_argument_wrappers_have_only_a_typed_far_jump_and_no_homes() {
    for optimize in [false, true] {
        for ty in [
            "BYTE",
            "CARD",
            "INT",
            "SIZE",
            "LONGCARD",
            "LONGINT",
            "BYTE POINTER",
        ] {
            let p = program(
                &format!(
                    "{ty} FUNC Sink({ty} value) RETURN(value) {ty} FUNC Wrap({ty} value) RETURN(Sink(value)) PROC Main() RETURN"
                ),
                optimize,
            );
            let m = materialize(&p).unwrap();
            let r = m
                .prepared
                .routines
                .iter()
                .find(|r| r.name == "Wrap")
                .unwrap();
            let proof = plans(&m.prepared)
                .remove(&r.id)
                .unwrap_or_else(|| panic!("{ty}/{optimize}: {r:#?}"));
            let emitted = m.routines.iter().find(|m| m.id == r.id).unwrap();
            emitted
                .frame
                .verify_forwarding(&m.prepared, r, &proof)
                .unwrap();
            assert_eq!(emitted.code.bytes, [0x5c, 0, 0, 0]);
            assert_eq!(emitted.code.forwarding_target(), Some(p.routines[0].id));
            assert_eq!(
                emitted.code.fixups,
                [Fixup {
                    offset: 1,
                    target: Target::Routine(p.routines[0].id),
                    addend: 0,
                    byte: None
                }]
            );
            assert_eq!(emitted.code.mir_spans.len(), r.blocks[0].ops.len() + 1);
            assert!(emitted.frame.temps.is_empty());
            assert_eq!(
                (emitted.frame.extent, emitted.frame.peak_below_entry),
                (0, 0)
            );
            let selected = emitted.code.selected.as_ref().unwrap();
            let replayed = replay::emit(selected, false).unwrap();
            assert_eq!(replayed.bytes, emitted.code.bytes);
            let fx = selected
                .records()
                .iter()
                .find_map(|r| match &r.action {
                    selected::Action::Instruction { effects, .. } => Some(effects),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                fx.control,
                effects::Control::Forward(Target::Routine(proof.target()))
            );
            assert_eq!(fx.environment_writes, effects::env::PC | effects::env::PBR);
            assert_eq!(fx.writes, effects::Registers::default());
            assert!(fx.memory.iter().all(|m| m.access == effects::Access::Read));
        }
    }
}

#[test]
fn pointer_reinterpretation_qualifies_but_extra_work_does_not() {
    for optimize in [false, true] {
        let p = program(
            "TYPE A=[BYTE field] TYPE B=[CARD field] BYTE FUNC Sink(A POINTER p) RETURN(p.field) BYTE FUNC Wrap(B POINTER p) RETURN(Sink(A POINTER(p))) PROC Main() RETURN",
            optimize,
        );
        assert!(plans(&p).contains_key(&p.routines[1].id));
        for source in [
            "CARD FUNC Sink(CARD value) RETURN(value) CARD FUNC Wrap(CARD value) RETURN(Sink(value+1)) PROC Main() RETURN",
            "CARD FUNC Sink(CARD value) RETURN(value) CARD FUNC Wrap(CARD value) RETURN(Sink(7)) PROC Main() RETURN",
            "CARD FUNC Sink(CARD value) RETURN(value) BYTE FUNC Wrap(CARD value) RETURN(BYTE(Sink(value))) PROC Main() RETURN",
            "CARD FUNC Sink(CARD value) RETURN(value) CARD FUNC Wrap(CARD value) BYTE ARRAY local(3) local(0)=1 RETURN(Sink(value)) PROC Main() RETURN",
            "CARD FUNC Sink(CARD value) RETURN(value) CARD FUNC Wrap(CARD value) value==+1 RETURN(Sink(value)) PROC Main() RETURN",
        ] {
            let p = program(source, optimize);
            assert!(
                !plans(&p).contains_key(&p.routines[1].id),
                "{source}/{optimize}"
            );
            let m = materialize(&p).unwrap();
            assert!(m.routines[1].code.forwarding_target().is_none());
        }
    }
}

#[test]
fn forwarding_rejects_cycles_and_rechecks_corrupted_proofs_and_frames() {
    let mut p = program(
        "CARD FUNC Sink(CARD a) RETURN(a) CARD FUNC First(CARD a) RETURN(Sink(a)) CARD FUNC Second(CARD a) RETURN(First(a)) PROC Main() RETURN",
        false,
    );
    let r = &p.routines[1];
    let proof = plans(&p).remove(&r.id).unwrap();
    let mut frame = AllocatedFrame::forwarding(&p, r, &proof).unwrap();
    frame.peak_below_entry = 6;
    assert!(frame.verify_forwarding(&p, r, &proof).is_err());
    let mut corrupt = proof.clone();
    corrupt.target = p.routines[2].id;
    assert!(corrupt.verify(&p, r).is_err());
    // First -> Second -> First. Both ordinary calls must retain their guards.
    let id = p.routines[2].id;
    let signature_id = p.routines[2].signature;
    for op in &mut p.routines[1].blocks[0].ops {
        if let Mir65816Op::Call {
            target, signature, ..
        } = op
        {
            *target = Mir65816CallTarget::Direct(id.0);
            *signature = Some(signature_id);
        }
    }
    verify_program(&p).unwrap();
    assert!(plans(&p).is_empty());
    assert!(proof.verify(&p, &p.routines[1]).is_err());
    assert!(
        materialize(&p)
            .unwrap()
            .routines
            .iter()
            .all(|r| r.code.forwarding_target().is_none())
    );
}

#[test]
fn forwarding_matches_zero_and_mixed_arguments_but_never_reorders_them() {
    for optimize in [false, true] {
        for source in [
            "BYTE FUNC Sink() RETURN(129) BYTE FUNC Wrap() RETURN(Sink()) PROC Main() RETURN",
            "BYTE out PROC Sink() out=1 RETURN PROC Wrap() Sink() RETURN PROC Main() RETURN",
            "CARD out PROC Sink(BYTE a CARD b BYTE c SIZE d) out=b RETURN PROC Wrap(BYTE a CARD b BYTE c SIZE d) Sink(a,b,c,d) RETURN PROC Main() RETURN",
        ] {
            let p = program(source, optimize);
            let m = materialize(&p).unwrap();
            assert_eq!(m.routines[1].code.bytes, [0x5c, 0, 0, 0]);
        }
        for args in ["b,a", "a,a", "a,CARD(0)"] {
            let p = program(
                &format!(
                    "CARD FUNC Sink(CARD a,b) RETURN(a+b) CARD FUNC Wrap(CARD a,b) RETURN(Sink({args})) PROC Main() RETURN"
                ),
                optimize,
            );
            assert!(!plans(&p).contains_key(&p.routines[1].id));
        }
        let mut p = program(
            "CARD FUNC Sink(CARD a) RETURN(a) CARD FUNC Wrap(CARD a) RETURN(Sink(a)) PROC Main() RETURN",
            optimize,
        );
        // A normalized direct Param still forwards without inventing a load.
        let r = &mut p.routines[1];
        let param = r.frame.parameters[0].param;
        r.blocks[0]
            .ops
            .retain(|op| !matches!(op, Mir65816Op::Load { .. }));
        r.temps.retain(|(id, _)| {
            r.blocks[0]
                .ops
                .iter()
                .any(|op| super::super::liveness::operation_output(op) == Some(*id))
        });
        for op in &mut r.blocks[0].ops {
            if let Mir65816Op::Call { args, .. } = op {
                args[0] = Mir65816Value::Param(param);
            }
        }
        verify_program(&p).unwrap();
        assert!(plans(&p).contains_key(&p.routines[1].id));
    }
}

#[test]
fn forwarding_cannot_erase_volatile_reads_or_unknown_callees() {
    for variant in 0..4 {
        let mut p = program(
            "CARD FUNC Sink(CARD a) RETURN(a) CARD FUNC Wrap(CARD a) RETURN(Sink(a)) PROC Main() RETURN",
            false,
        );
        let proof = plans(&p).remove(&p.routines[1].id).unwrap();
        match variant {
            0 => {
                if let Mir65816Op::Load { volatile, .. } = &mut p.routines[1].blocks[0].ops[0] {
                    *volatile = true;
                }
            }
            1 => p.routines[0].entry.external = true,
            2 => {
                if let Mir65816Op::Call { plan, .. } =
                    p.routines[1].blocks[0].ops.last_mut().unwrap()
                {
                    plan.outgoing_bytes = ByteSize::new(5);
                }
            }
            _ => p.routines[0].result_home = None,
        }
        assert!(proof.verify(&p, &p.routines[1]).is_err());
        assert!(!plans(&p).contains_key(&p.routines[1].id));
    }
}
