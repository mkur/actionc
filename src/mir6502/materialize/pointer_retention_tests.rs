use super::*;
use crate::mir6502::MirEdge;

fn fixture(text: &str) -> MirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(text).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    let nir = crate::nir::optimize_program(&crate::nir::lower_program(
        &crate::semantic::ir::lower_program(&ast, &model),
    ))
    .unwrap();
    let mut config = Mir6502Config::optimized();
    config.enable_pointer_call_retention = false;
    config.enable_pointer_loop_retention = false;
    config.enable_pointer_promotion = false;
    crate::mir6502::materialize_program_with_origin(
        crate::mir6502::lower_program(&nir).unwrap(),
        &config,
        0x3000,
    )
    .unwrap()
}
fn probe(body: &str) -> MirProgram {
    fixture(&format!(
        "BYTE observed\nPROC Touch=*()\nASM\n{body}\nENDASM\nPROC Probe(BYTE POINTER p)\nobserved=p(1)\nTouch()\nobserved=p(2)\nRETURN\nPROC Main()\nProbe(BYTE POINTER($6000))\nRETURN\n"
    ))
}
fn changed(p: &MirProgram) -> bool {
    let map = memory(p, p, 0x3000).unwrap();
    apply(p, p, &map).unwrap().0 != *p
}
#[test]
fn pointer_retention_calls_preserve_exact_pairs_and_reject_each_clobbered_lane() {
    assert!(changed(&probe("lda #3\nsta.z $A0\nrts")));
    for address in [0xAC, 0xAD] {
        assert!(!changed(&probe(&format!(
            "lda #3\nsta.z ${address:02X}\nrts"
        ))));
    }
    assert!(!changed(&probe("lda ($A0),y\nsta ($A2),y\nrts")));
}
#[test]
fn pointer_retention_calls_keep_source_mutations_and_opaque_barriers() {
    let mut p = probe("rts");
    let caller = p.routines.iter().find(|r| r.name == "Probe").unwrap();
    let source = caller.frame.params[0].base.clone();
    let MirStorageBase::Param(id) = source else {
        panic!()
    };
    let address = memory(&p, &p, 0x3000)
        .unwrap()
        .address(caller.id, &MirMem::Param { id, offset: 0 })
        .unwrap();
    let touch = p.routines.iter_mut().find(|r| r.name == "Touch").unwrap();
    // A separate absolute spelling of the caller's parameter must invalidate it.
    touch.blocks[0].ops = vec![
        MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 9,
            width: MirWidth::Byte,
        },
        MirOp::Store {
            dst: MirAddr::Direct(MirMem::Absolute(address)),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        },
    ];
    touch.blocks[0].terminator = MirTerminator::Return;
    assert!(!changed(&p));
    let mut p = probe("rts");
    let touch = p.routines.iter_mut().find(|r| r.name == "Touch").unwrap();
    if let MirOp::MachineBlock { effects, .. } = &mut touch.blocks[0].ops[0] {
        effects.opaque = true;
    }
    assert!(!changed(&p));
}
#[test]
fn pointer_retention_calls_include_transitive_and_argument_writes() {
    let p = fixture(
        "BYTE observed\nPROC Leaf=*()\nASM\nlda #5\nsta.z $AD\nrts\nENDASM\nPROC Touch()\nLeaf()\nRETURN\nPROC Probe(BYTE POINTER p)\nobserved=p(1)\nTouch()\nobserved=p(2)\nRETURN\nPROC Main()\nProbe(BYTE POINTER($6000))\nRETURN\n",
    );
    assert!(!changed(&p));
    let mut p = probe("rts");
    let map = memory(&p, &p, 0x3000).unwrap();
    let calls = CallWrites::analyze(&p, &map);
    let r = p.routines.iter_mut().find(|r| r.name == "Probe").unwrap();
    for op in &mut r.blocks[0].ops {
        if let MirOp::Call { args, .. } = op {
            args.push(MirCallArg {
                value: MirValue::ConstU8(7),
                width: MirWidth::Byte,
                home: MirArgHome::FixedZeroPage(MirFixedZpSlot(0xAC)),
            });
        }
        if matches!(op, MirOp::Call { .. }) {
            assert!(calls.for_call(op, r.id, &map).unwrap().contains(&0xAC));
        }
    }
}
#[test]
fn pointer_retention_calls_replay_cost_and_keep_frames() {
    let p = probe("lda #3\nsta.z $A0\nrts");
    let selected = select_costed(p.clone(), &Mir6502Config::optimized(), 0x3000, false).unwrap();
    assert_ne!(selected, p);
    assert!(
        address_values::Cost::measure(&selected, 0x3000)
            .unwrap()
            .bytes
            < address_values::Cost::measure(&p, 0x3000).unwrap().bytes
    );
    for (a, b) in selected.routines.iter().zip(&p.routines) {
        assert_eq!(a.frame, b.frame);
    }
    crate::mir6502::verify_program(&selected, MirPhase::PreEmission).unwrap();
}

fn loop_fixture() -> MirProgram {
    fixture(
        "BYTE ARRAY data(512)\nBYTE observed\nPROC Touch=*()\nASM\nldx #$55\nrts\nENDASM\nPROC Probe(BYTE POINTER p)\nBYTE i\nobserved=p(0)\ni=0\nWHILE i<3 DO\nTouch()\nobserved=p(i)\ni==+1\nOD\nRETURN\nPROC Main()\nProbe(data)\nRETURN\n",
    )
}
fn cfg_proofs(p: &MirProgram) -> usize {
    use crate::mir6502::analysis::sites::MirRoutineGeneration;
    let map = memory(p, p, 0x3000).unwrap();
    let calls = CallWrites::analyze(p, &map);
    let r = p.routines.iter().find(|r| r.name == "Probe").unwrap();
    let facts = retention::ResidentFacts::analyze(r, &map, &calls).unwrap();
    r.blocks
        .iter()
        .map(|b| {
            b.ops
                .windows(4)
                .enumerate()
                .filter(|(i, _)| {
                    facts
                        .prove(
                            r,
                            MirRoutineGeneration::initial(),
                            MirSite::Op {
                                block: b.id,
                                op_index: *i,
                            },
                            &map,
                            &calls,
                        )
                        .is_some()
                })
                .count()
        })
        .sum()
}
#[test]
fn pointer_retention_cfg_requires_a_real_initial_definition_and_agreeing_edges() {
    let p = loop_fixture();
    assert!(cfg_proofs(&p) > 0);
    let mut no_entry = p.clone();
    let r = no_entry
        .routines
        .iter_mut()
        .find(|r| r.name == "Probe")
        .unwrap();
    r.blocks[0].ops.retain(|op| {
        !matches!(
            op,
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAC | 0xAD))),
                ..
            }
        )
    });
    assert_eq!(
        cfg_proofs(&no_entry),
        0,
        "a backedge must not prove first-entry residence"
    );
    let mut disagree = p.clone();
    let r = disagree
        .routines
        .iter_mut()
        .find(|r| r.name == "Probe")
        .unwrap();
    let MirTerminator::Jump(edge) = r.blocks[0].terminator.clone() else {
        panic!()
    };
    let id = MirBlockId(r.blocks.iter().map(|b| b.id.0).max().unwrap() + 1);
    r.blocks[0].terminator = MirTerminator::Branch {
        cond: MirCond::FlagTest(MirFlagTest::ZSet),
        then_edge: edge.clone(),
        else_edge: MirEdge::plain(id),
    };
    r.blocks.push(crate::mir6502::MirBlock {
        id,
        label: "clobber".into(),
        params: vec![],
        ops: vec![MirOp::Store {
            dst: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAD))),
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }],
        terminator: MirTerminator::Jump(edge),
    });
    assert_eq!(cfg_proofs(&disagree), 0);
}
#[test]
fn pointer_retention_cfg_bounds_the_entire_indexed_write_region() {
    let p = loop_fixture();
    let map = memory(&p, &p, 0x3000).unwrap();
    let data = p.globals.iter().find(|g| g.name == "data").unwrap();
    let address = map
        .address(
            p.routines[0].id,
            &MirMem::Global {
                id: data.id,
                offset: 0,
            },
        )
        .or_else(|| {
            p.routines.iter().find_map(|r| {
                map.address(
                    r.id,
                    &MirMem::Global {
                        id: data.id,
                        offset: 0,
                    },
                )
            })
        })
        .unwrap_or(0x3000);
    for (base, expected) in [(address, true), (0x6000, false), (0xFFF0, false)] {
        let mut p = p.clone();
        let r = p.routines.iter_mut().find(|r| r.name == "Probe").unwrap();
        r.blocks[0].ops.splice(
            0..0,
            [
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::A),
                    src: if expected {
                        MirValue::StorageAddrByte {
                            mem: MirMem::Global {
                                id: data.id,
                                offset: 0,
                            },
                            byte: 0,
                        }
                    } else {
                        MirValue::ConstU8(base as u8)
                    },
                    width: MirWidth::Byte,
                },
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::X),
                    src: if expected {
                        MirValue::StorageAddrByte {
                            mem: MirMem::Global {
                                id: data.id,
                                offset: 0,
                            },
                            byte: 1,
                        }
                    } else {
                        MirValue::ConstU8((base >> 8) as u8)
                    },
                    width: MirWidth::Byte,
                },
            ],
        );
        let entry = r.blocks[0].id;
        let body = r
            .blocks
            .iter_mut()
            .find(|b| {
                b.id != entry
                    && b.ops
                        .windows(4)
                        .any(|w| record_bases::setup_pair(w).is_some())
            })
            .unwrap();
        body.ops.push(MirOp::Store {
            dst: MirAddr::FixedIndirectIndexedY {
                zp: MirFixedZpSlot(0xAC),
            },
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        });
        assert_eq!(cfg_proofs(&p) > 0, expected, "base={base:04X}");
    }
}
#[test]
fn pointer_retention_cfg_rejects_excessive_graphs() {
    let mut p = loop_fixture();
    let r = p.routines.iter_mut().find(|r| r.name == "Probe").unwrap();
    let template = r.blocks.last().unwrap().clone();
    for n in 0..257 {
        let mut b = template.clone();
        b.id = MirBlockId(1000 + n);
        b.label = format!("oversize{n}");
        b.terminator = MirTerminator::Return;
        r.blocks.push(b);
    }
    let map = memory(&p, &p, 0x3000).unwrap();
    let calls = CallWrites::analyze(&p, &map);
    let r = p.routines.iter().find(|r| r.name == "Probe").unwrap();
    assert!(retention::ResidentFacts::analyze(r, &map, &calls).is_none());
}
