use super::*;
use crate::mir6502::ir::*;

fn lower(source: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze(&ast).unwrap();
    crate::mir6502::lower_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap()
}

fn materialize(program: MirProgram) -> MirProgram {
    crate::mir6502::materialize_program(program, &crate::mir6502::Mir6502Config::default()).unwrap()
}

fn reader(backing: u16) -> MirProgram {
    lower(&format!(
        "CARD state=${backing:04X}
         CARD FUNC ReadState() RETURN(state)
         CARD FUNC Outer() RETURN(ReadState())
         PROC Main() RETURN"
    ))
}

fn input(program: &MirProgram, stage: MirCalleeEntryStage) -> MirCalleeEntryFact {
    MirCalleeEntryFact::analyze(program, program.routines[0].id, stage).unwrap()
}

#[test]
fn global_state_reads_keep_their_footprint_without_becoming_entry_inputs() {
    for address in [0xe8, 0x600] {
        let raw = reader(address);
        let selected = materialize(raw.clone());
        for (program, stage) in [
            (&raw, MirCalleeEntryStage::PlannedCapture),
            (&selected, MirCalleeEntryStage::ActualEntry),
        ] {
            let fact = input(program, stage);
            assert!(
                fact.inputs.fixed.is_empty(),
                "{address:x}: {:?}",
                fact.inputs
            );
            assert!(!fact.inputs.registers.a && !fact.inputs.flags.any());
            let reads = MirKnownCalleeSummaries::analyze(program);
            let MirKnownMemoryReads::Exact(reads) = reads.get(fact.source.id).unwrap().reads()
            else {
                panic!("expected bounded global reads");
            };
            for offset in 0..2 {
                assert!(reads.contains(&MirMem::Global {
                    id: program.globals[0].id,
                    offset
                }));
            }
        }
        let mut entries = MirAcyclicEntryAnalysis::new(&selected);
        let outer = entries.analyze(selected.routines[1].id).unwrap();
        assert!(outer.inputs.fixed.is_empty(), "{:?}", outer.inputs);
        let known = MirKnownCalleeSummaries::analyze(&selected);
        let MirKnownMemoryReads::Exact(reads) = known.get(outer.source.id).unwrap().reads() else {
            panic!("expected transitive global reads");
        };
        assert!(reads.contains(&MirMem::Global {
            id: selected.globals[0].id,
            offset: 1
        }));
        let mut changed = selected.clone();
        changed.globals[0].backing = MirGlobalBacking::Absolute(0x700);
        assert!(!outer.is_current(&changed, MirCalleeEntryStage::ActualEntry));
    }
}

#[test]
fn global_state_aliases_preserve_raw_inputs_and_exact_definition_proofs() {
    let mut p = reader(0xe8);
    let mut backing = p.globals[0].clone();
    backing.id = crate::nir::SymbolId(backing.id.0 + 1);
    backing.name = "backing".into();
    backing.storage_size = 3;
    backing.backing = MirGlobalBacking::Absolute(0xe7);
    p.globals[0].backing = MirGlobalBacking::Alias {
        target: backing.id,
        offset: 1,
    };
    p.globals.push(backing);
    let mut p = materialize(p);
    assert!(
        input(&p, MirCalleeEntryStage::ActualEntry)
            .inputs
            .fixed
            .is_empty()
    );

    let raw_read = MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xe8))),
        width: MirWidth::Byte,
    };
    p.routines[0].blocks[0].ops.insert(0, raw_read);
    let raw = input(&p, MirCalleeEntryStage::ActualEntry);
    assert_eq!(raw.inputs.fixed, BTreeSet::from([0xe8]));
    p.routines[0].blocks[0].ops.insert(
        0,
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Global {
                id: p.globals[0].id,
                offset: 0,
            }),
            src: MirValue::ConstU8(7),
            width: MirWidth::Byte,
        },
    );
    assert!(
        input(&p, MirCalleeEntryStage::ActualEntry)
            .inputs
            .fixed
            .is_empty()
    );

    // A logical store with unknown backing cannot establish a physical input.
    p.globals[1].backing = MirGlobalBacking::Ordinary { offset: 0 };
    assert_eq!(
        input(&p, MirCalleeEntryStage::ActualEntry).inputs.fixed,
        BTreeSet::from([0xe8])
    );
    assert!(!raw.is_current(&p, MirCalleeEntryStage::ActualEntry));
}

#[test]
fn invalid_global_state_evidence_declines_the_entry_certificate() {
    let p = reader(0xe8);
    for mutation in 0..4 {
        let mut changed = p.clone();
        match mutation {
            0 => changed.globals.clear(),
            1 => changed.globals[0].storage_size = 1,
            2 => changed.globals[0].backing = MirGlobalBacking::Absolute(0xffff),
            _ => {
                changed.globals[0].backing = MirGlobalBacking::Alias {
                    target: changed.globals[0].id,
                    offset: 0,
                }
            }
        }
        assert!(
            MirCalleeEntryFact::analyze(
                &changed,
                changed.routines[0].id,
                MirCalleeEntryStage::PlannedCapture
            )
            .is_none(),
            "{mutation}"
        );
    }
}

fn assembly(declaration: &str, body: &str) -> MirProgram {
    // Exercise the isolated decoder on typed operands. The production query
    // must decline the same unreviewed source body.
    let assembled = lower(&format!(
        "{declaration}\nBYTE FUNC Observe=*()\nASM\n{body}\nsta $A0\nrts\nENDASM\nPROC Main() RETURN"
    ));
    assert!(MirCalleeEntryFact::analyze(&assembled, assembled.routines[0].id,
        MirCalleeEntryStage::ActualEntry).is_none());
    assembled
}

fn decoded_inputs(program: &MirProgram) -> Option<MirEntryRequirements> {
    BoundedMachine::decode(&program.routines[0], program)?.entry_inputs(program)
}

#[test]
fn assembly_state_reads_preserve_typed_identity_and_raw_requirements() {
    for (declaration, body, expected) in [
        ("CARD state=$E8", "lda state", vec![]),
        ("CARD state=$0600", "lda state", vec![]),
        ("CARD state", "lda state", vec![]),
        ("CARD state=$E8", "lda $E8", vec![0xe8]),
        ("CARD state=$E8", "lda state\nlda $E8", vec![0xe8]),
        ("CARD state=$E8", "ldy #0\nlda (state),y", vec![]),
        ("CARD state=$E8", "ldy #0\nlda ($E8),y", vec![0xe8, 0xe9]),
    ] {
        let p = assembly(declaration, body);
        let inputs = decoded_inputs(&p).unwrap();
        assert_eq!(inputs.fixed, expected.iter().copied().collect(), "{body}");
        assert!(!inputs.registers.a && !inputs.registers.y && !inputs.flags.any());
        let summary = summarize_bounded_machine_block(&p.machine_blocks[0], &p.routines[0], &p).unwrap();
        let reads = summary.reads();
        if body.contains("(state)") {
            assert_eq!(reads, &MirKnownMemoryReads::Unknown);
        } else if body.contains("lda state") {
            let MirKnownMemoryReads::Exact(reads) = reads else {
                panic!("{reads:?}");
            };
            assert!(reads.contains(&MirMem::Global {
                id: p.globals[0].id,
                offset: 0
            }));
        }
    }
}

#[test]
fn assembly_global_stores_require_fixed_backing_and_all_path_definitions() {
    for (declaration, body, defined) in [
        ("CARD state=$E8", "lda #7\nsta state\nlda $E8", true),
        ("CARD state", "lda #7\nsta state\nlda $E8", false),
        (
            "CARD state=$E8",
            "lda #7\nbeq skip\nsta state\nskip:\nlda $E8",
            false,
        ),
    ] {
        let p = assembly(declaration, body);
        assert_eq!(
            decoded_inputs(&p).unwrap()
                .fixed
                .contains(&0xe8),
            !defined,
            "{declaration}: {body}"
        );
    }
}

#[test]
fn decoded_entry_requirements_follow_current_aliases_and_operand_evidence() {
    let mut p = assembly("CARD backing=$E7 CARD state=backing", "lda state");
    let backing = p.globals.iter().find(|g| g.name == "backing").unwrap().id;
    let state = p.globals.iter_mut().find(|g| g.name == "state").unwrap();
    state.backing = MirGlobalBacking::Alias {
        target: backing,
        offset: 1,
    };
    let state_id = state.id;
    assert!(decoded_inputs(&p).unwrap().fixed.is_empty());
    let mut literal = p.clone();
    for item in &mut literal.machine_blocks[0].items {
        if let MirMachineItem::Relocation { target, .. } = item
            && *target
                == MirInlineAsmTarget::Memory(MirMem::Global {
                    id: state_id,
                    offset: 0,
                })
        {
            *target = MirInlineAsmTarget::Absolute(0xe8);
        }
    }
    assert_eq!(
        decoded_inputs(&literal).unwrap()
            .fixed,
        BTreeSet::from([0xe8])
    );
    let state = p.globals.iter_mut().find(|g| g.id == state_id).unwrap();
    state.backing = MirGlobalBacking::Alias {
        target: state_id,
        offset: 0,
    };
    assert!(decoded_inputs(&p).is_none());
    assert!(
        MirCalleeEntryFact::analyze(&p, p.routines[0].id, MirCalleeEntryStage::ActualEntry)
            .is_none()
    );
}
