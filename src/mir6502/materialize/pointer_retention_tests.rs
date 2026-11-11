use super::*;
use crate::mir6502::MirEdge;

#[test]
fn bounded_call_pointer_retention_uses_owned_destinations_and_keeps_unknown_fallback() {
    let p = fixture(include_str!(
        "../../../fixtures/runtime/bounded_call_pointer_retention.act"
    ));
    let mut config = Mir6502Config::optimized();
    config.enable_pointer_promotion = false;
    config.enable_bounded_call_pointer_retention = false;
    let old = select_costed(p.clone(), &config, 0x3000, false).unwrap();
    config.enable_bounded_call_pointer_retention = true;
    let new = select_costed(p.clone(), &config, 0x3000, false).unwrap();
    let routine =
        |p: &MirProgram, name: &str| p.routines.iter().find(|r| r.name == name).unwrap().clone();
    assert_ne!(
        routine(&old, "ReadAcrossCopy"),
        routine(&new, "ReadAcrossCopy")
    );
    assert_eq!(routine(&old, "UnknownCopy"), routine(&new, "UnknownCopy"));
    assert_eq!(
        address_values::Cost::measure(&old, 0x3000).unwrap().bytes
            - address_values::Cost::measure(&new, 0x3000).unwrap().bytes,
        10
    );
    assert!(
        old.routines
            .iter()
            .zip(&new.routines)
            .all(|(a, b)| a.frame == b.frame)
    );
    assert_eq!(
        select_costed(new.clone(), &config, 0x3000, false).unwrap(),
        new
    );
    assert_eq!(select_costed(p, &config, 0x3000, true).unwrap(), new);
}

#[test]
fn bounded_call_pointer_retention_rejects_parameter_writes_and_unsafe_callees() {
    let source = include_str!("../../../fixtures/runtime/bounded_call_pointer_retention.act")
        .replace("\r\n", "\n");
    for text in [
        source.replace(
            "CopyBytes(@dstBuf, @srcBuf, count)",
            "CopyBytes(@p, @srcBuf, 1)",
        ),
        source.replace(
            "CopyBytes(@dstBuf, @srcBuf, count)",
            "CopyBytes(@dstBuf, CARD(p), count)",
        ),
        source.replace("  rts\n", "  sta $D301\n  rts\n"),
        source.replace("  rts\n", "  pha\n  pla\n  rts\n"),
    ] {
        let p = fixture(&text);
        let mut config = Mir6502Config::optimized();
        config.enable_pointer_promotion = false;
        config.enable_bounded_call_pointer_retention = false;
        let old = select_costed(p.clone(), &config, 0x3000, false).unwrap();
        config.enable_bounded_call_pointer_retention = true;
        assert_eq!(select_costed(p, &config, 0x3000, false).unwrap(), old);
    }
}

#[test]
fn bounded_call_pointer_retention_rebuilds_changed_callee_and_physical_facts() {
    let p = fixture(include_str!(
        "../../../fixtures/runtime/bounded_call_pointer_retention.act"
    ));
    let map = memory(&p, &p, 0x3000).unwrap();
    assert_ne!(
        apply_with_bounded_calls(&p, &p, &map, true, false, true)
            .unwrap()
            .0,
        p
    );
    let mut changed = p.clone();
    let items = &mut changed.machine_blocks[0].items;
    items.splice(
        items.len() - 1..items.len() - 1,
        [MirMachineItem::Byte(0x85), MirMachineItem::Byte(0xAC)],
    );
    assert_eq!(
        apply_with_bounded_calls(&p, &changed, &map, true, false, true)
            .unwrap()
            .0,
        p
    );
    assert_eq!(
        apply_with_bounded_calls(&p, &p, &MirPhysicalMemory::default(), true, false, true)
            .unwrap()
            .0,
        p
    );
}

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
    apply(p, p, &map, false, false).unwrap().0 != *p
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
fn pointer_retention_calls_keep_hardware_mapping_and_unowned_access_barriers() {
    for body in [
        "lda.a $BFFA\nrts",
        "lda.a $D20F\nrts",
        "sta.a $D301\nrts",
        "sta.a $6000\nrts",
        "lda ($A0),y\nrts",
    ] {
        assert!(!changed(&probe(body)), "{body}");
    }
    assert!(changed(&probe("lda.z $E0\nrts")));
    let p = fixture(
        "BYTE ARRAY table(4)\nBYTE observed\nPROC Touch=*()\nASM\nldx #1\nlda.a table,x\nsta.z $A0\nrts\nENDASM\nPROC Probe(BYTE POINTER p)\nobserved=p(1)\nTouch()\nobserved=p(2)\nRETURN\nPROC Main()\nProbe(BYTE POINTER($6000))\nRETURN\n",
    );
    assert!(
        changed(&p),
        "an inspectable indexed RAM table read must stay usable"
    );
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

#[test]
fn pointer_retention_selects_entered_loop_seeding_and_keeps_both_reads() {
    let p = fixture(
        &include_str!("../../../fixtures/runtime/pointer_invariant_loop.act").replace("\r\n", "\n"),
    );
    let map = memory(&p, &p, 0x3000).unwrap();
    let calls = CallWrites::analyze(&p, &map);
    let r = p.routines.iter().find(|r| r.name == "ReadFixed").unwrap();
    assert!(!retention::entered_loops(r, &map, &calls).is_empty());
    let seeded = seed_loop(
        r,
        &map,
        &calls,
        &MirKnownCalleeSummaries::analyze_for_rewriting(&p),
    )
    .expect("safe entered loop");
    assert!(seeded.blocks[0].ops.len() > r.blocks[0].ops.len());
    let selected = select_costed(p.clone(), &Mir6502Config::optimized(), 0x3000, false).unwrap();
    assert_ne!(selected, p);
    let indirect_reads = |p: &MirProgram| {
        p.routines
            .iter()
            .flat_map(|r| &r.blocks)
            .flat_map(|b| &b.ops)
            .filter(|op| {
                matches!(
                    op,
                    MirOp::Load {
                        src: MirAddr::FixedIndirectIndexedY { .. },
                        ..
                    } | MirOp::LoadIndirect { .. }
                )
            })
            .count()
    };
    assert_eq!(indirect_reads(&p), indirect_reads(&selected));
}

#[test]
fn pointer_retention_indexed_store_requires_dead_pair_y_and_flags() {
    let p = fixture(
        &include_str!("../../../fixtures/runtime/pointer_retention.act").replace("\r\n", "\n"),
    );
    let map = memory(&p, &p, 0x3000).unwrap();
    let (trial, _, _) = apply(&p, &p, &map, true, false).unwrap();
    let count = |p: &MirProgram| {
        p.routines
            .iter()
            .flat_map(|r| &r.blocks)
            .flat_map(|b| &b.ops)
            .filter(|op| matches!(op, MirOp::MaterializeIndexedAddress { .. }))
            .count()
    };
    assert!(count(&trial) < count(&p));
    for flag in [
        MirFlagTest::CSet,
        MirFlagTest::VSet,
        MirFlagTest::NSet,
        MirFlagTest::ZSet,
    ] {
        let mut negative = p.clone();
        let r = negative
            .routines
            .iter_mut()
            .find(|r| r.name == "ConvertSpan")
            .unwrap();
        let exit = r.blocks.last().unwrap().id;
        let b = r
            .blocks
            .iter_mut()
            .find(|b| {
                b.ops
                    .iter()
                    .any(|op| matches!(op, MirOp::MaterializeIndexedAddress { .. }))
            })
            .unwrap();
        let last = b
            .ops
            .iter()
            .position(|op| matches!(op, MirOp::StoreIndirect { .. }))
            .unwrap();
        b.ops.truncate(last + 1);
        b.terminator = MirTerminator::Branch {
            cond: MirCond::FlagTest(flag),
            then_edge: MirEdge::plain(exit),
            else_edge: MirEdge::plain(exit),
        };
        let map = memory(&negative, &negative, 0x3000).unwrap();
        let (selected, _, _) = apply(&negative, &negative, &map, true, false).unwrap();
        assert_eq!(count(&negative), count(&selected));
    }
}

#[test]
fn pointer_retention_indexed_store_keeps_live_y_pair_and_aliased_result_reads() {
    let original = fixture(
        &include_str!("../../../fixtures/runtime/pointer_retention.act").replace("\r\n", "\n"),
    );
    for kind in 0..3 {
        let mut p = original.clone();
        let observed = p.globals.iter().find(|g| g.name == "observed").unwrap().id;
        let r = p
            .routines
            .iter_mut()
            .find(|r| r.name == "ConvertSpan")
            .unwrap();
        let b = r
            .blocks
            .iter_mut()
            .find(|b| {
                b.ops
                    .iter()
                    .any(|op| matches!(op, MirOp::MaterializeIndexedAddress { .. }))
            })
            .unwrap();
        let index = b
            .ops
            .iter()
            .position(|op| matches!(op, MirOp::MaterializeIndexedAddress { .. }))
            .unwrap();
        match kind {
            0 => b.ops.insert(
                index + 3,
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Global {
                        id: observed,
                        offset: 0,
                    }),
                    src: MirValue::Def(MirDef::Reg(MirReg::Y)),
                    width: MirWidth::Byte,
                },
            ),
            1 => b.ops.insert(
                index + 3,
                MirOp::LoadIndirect {
                    consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                        lo: MirFixedZpSlot(0xAC),
                    }),
                    dst: MirDef::Reg(MirReg::A),
                    offset: 0,
                },
            ),
            _ => {
                b.ops[index + 1] = MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAC))),
                    width: MirWidth::Byte,
                }
            }
        }
        let map = memory(&p, &p, 0x3000).unwrap();
        let (trial, _, _) = apply(&p, &p, &map, true, false).unwrap();
        assert!(
            trial
                .routines
                .iter()
                .find(|r| r.name == "ConvertSpan")
                .unwrap()
                .blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(op, MirOp::MaterializeIndexedAddress { .. })),
            "kind={kind}"
        );
    }
}

#[test]
fn pointer_retention_indexed_store_rejects_stale_generation_and_window() {
    use crate::mir6502::analysis::{
        posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration,
    };
    let p = fixture(
        &include_str!("../../../fixtures/runtime/pointer_retention.act").replace("\r\n", "\n"),
    );
    let map = memory(&p, &p, 0x3000).unwrap();
    let calls = CallWrites::analyze(&p, &map);
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    for changed_window in [false, true] {
        let r = p.routines.iter().find(|r| r.name == "ConvertSpan").unwrap();
        let generation = MirRoutineGeneration::initial();
        let s = PostHomeAnalysisSnapshot::new_with_known_callees(r, generation, &known).unwrap();
        let facts = retention::ResidentFacts::analyze(r, &map, &calls).unwrap();
        let plan =
            indexed_store_plan(r, &PostHomeRewriteContext::new(&s), &facts, &map, &calls).unwrap();
        let mut r = r.clone();
        if changed_window {
            r.blocks
                .iter_mut()
                .find(|b| b.id == plan.block)
                .unwrap()
                .ops[plan.index + 1] = MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: 42,
                width: MirWidth::Byte,
            };
        }
        assert!(!plan.apply(
            &mut r,
            if changed_window {
                generation
            } else {
                generation.next()
            }
        ));
    }
}

#[test]
fn pointer_retention_result_forwarding_keeps_live_flags_and_raw_readbacks() {
    use crate::mir6502::analysis::{
        posthome::PostHomeAnalysisSnapshot, sites::MirRoutineGeneration,
    };
    let p = fixture(
        &include_str!("../../../fixtures/runtime/pointer_retention.act").replace("\r\n", "\n"),
    );
    let map = memory(&p, &p, 0x3000).unwrap();
    let calls = CallWrites::analyze(&p, &map);
    let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
    let original = p.routines.iter().find(|r| r.name == "ConvertSpan").unwrap();
    let generation = MirRoutineGeneration::initial();
    let snapshot =
        PostHomeAnalysisSnapshot::new_with_known_callees(original, generation, &known).unwrap();
    let facts = retention::ResidentFacts::analyze(original, &map, &calls).unwrap();
    let plan = indexed_store_plan(
        original,
        &PostHomeRewriteContext::new(&snapshot),
        &facts,
        &map,
        &calls,
    )
    .unwrap();
    let mut selected = original.clone();
    assert!(plan.apply(&mut selected, generation));
    for kind in 0..3 {
        let mut r = selected.clone();
        let exit = r.blocks.last().unwrap().id;
        let b = r
            .blocks
            .iter_mut()
            .find(|b| {
                b.ops.iter().any(|op| {
                    matches!(
                        op,
                        MirOp::Load {
                            src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let reload = b
            .ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    MirOp::Load {
                        src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0))),
                        ..
                    }
                )
            })
            .unwrap();
        if kind == 1 {
            b.ops.truncate(reload + 1);
            b.terminator = MirTerminator::Branch {
                cond: MirCond::FlagTest(MirFlagTest::ZSet),
                then_edge: MirEdge::plain(exit),
                else_edge: MirEdge::plain(exit),
            };
        }
        if kind == 2 {
            if let MirOp::Load { src, .. } = &mut b.ops[reload] {
                *src = MirAddr::Direct(MirMem::Absolute(0xA0));
            }
        }
        let snapshot =
            PostHomeAnalysisSnapshot::new_with_known_callees(&r, generation, &known).unwrap();
        assert_eq!(
            result_reload_plan(&r, &PostHomeRewriteContext::new(&snapshot)).is_some(),
            kind == 0
        );
    }
}

#[test]
fn pointer_promotion_reconstructs_exits_without_growing_frames() {
    let p = fixture(
        &include_str!("../../../fixtures/runtime/pointer_retention.act").replace("\r\n", "\n"),
    );
    let selected =
        select_promotion_costed(p.clone(), &Mir6502Config::optimized(), 0x3000, false).unwrap();
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
    let r = selected
        .routines
        .iter()
        .find(|r| r.name == "ScanFixed")
        .unwrap();
    assert!(r.blocks.iter().flat_map(|b| &b.ops).any(|op| matches!(
        op,
        MirOp::UpdateMem {
            op: MirUpdateOp::Inc,
            mem: MirMem::FixedZeroPage(_),
            width: MirWidth::Word
        }
    )));
    crate::mir6502::verify_program(&selected, MirPhase::PreEmission).unwrap();
}

#[test]
fn pointer_promotion_rejects_observation_escape_interference_and_live_exit_a() {
    let original = fixture(
        &include_str!("../../../fixtures/runtime/pointer_retention.act").replace("\r\n", "\n"),
    );
    for kind in 0..5 {
        let mut p = original.clone();
        let map = memory(&p, &p, 0x3000).unwrap();
        let observed = p.globals.iter().find(|g| g.name == "observed").unwrap().id;
        let r = p
            .routines
            .iter_mut()
            .find(|r| r.name == "ScanFixed")
            .unwrap();
        let (pointer, word_index, body_id) = r
            .blocks
            .iter()
            .find_map(|b| {
                b.ops.iter().enumerate().find_map(|(i, op)| match op {
                    MirOp::UpdateMem {
                        op: MirUpdateOp::Inc,
                        mem,
                        width: MirWidth::Word,
                    } => Some((mem.clone(), i, b.id)),
                    _ => None,
                })
            })
            .unwrap();
        let address = map.address(r.id, &pointer).unwrap();
        let b = r.blocks.iter_mut().find(|b| b.id == body_id).unwrap();
        match kind {
            0 => b.ops.insert(
                word_index + 1,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(pointer.clone()),
                    width: MirWidth::Byte,
                },
            ),
            1 => b.ops.insert(
                word_index + 1,
                MirOp::Move {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirValue::StorageAddrByte {
                        mem: pointer.clone(),
                        byte: 0,
                    },
                    width: MirWidth::Byte,
                },
            ),
            2 => b.ops.insert(
                word_index + 1,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xAC))),
                    width: MirWidth::Byte,
                },
            ),
            3 => b.ops.insert(
                word_index + 1,
                MirOp::Load {
                    dst: MirDef::Reg(MirReg::A),
                    src: MirAddr::Direct(MirMem::Absolute(address)),
                    width: MirWidth::Byte,
                },
            ),
            _ => r.blocks.last_mut().unwrap().ops.insert(
                0,
                MirOp::Store {
                    dst: MirAddr::Direct(MirMem::Global {
                        id: observed,
                        offset: 0,
                    }),
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                },
            ),
        }
        let map = memory(&p, &p, 0x3000).unwrap();
        let calls = CallWrites::analyze(&p, &map);
        let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        let r = p.routines.iter().find(|r| r.name == "ScanFixed").unwrap();
        assert!(
            promotion::try_promote(&p, r, &map, &calls, &known).is_none(),
            "kind={kind}"
        );
    }
}

#[test]
fn pointer_promotion_keeps_incoming_and_competing_pointer_scans() {
    for source in [
        "BYTE ARRAY buffer(512)\nBYTE observed\nPROC Scan(BYTE POINTER p)\nBYTE i\nFOR i=0 TO 31 DO\nobserved=p^\np==+1\nOD\nRETURN\nPROC Main()\nScan(buffer)\nRETURN\n",
        "BYTE ARRAY buffer(512)\nBYTE observed\nPROC Scan()\nBYTE POINTER p,q\nBYTE i\np=buffer q=buffer\nFOR i=0 TO 31 DO\nobserved=p^\nobserved=q^\np==+1 q==+1\nOD\nRETURN\nPROC Main()\nScan()\nRETURN\n",
        "BYTE ARRAY buffer(512)\nBYTE observed\nPROC Scan(BYTE count)\nBYTE POINTER p\nBYTE i\np=buffer i=0\nWHILE i<count DO\nobserved=p^\np==+1 i==+1\nOD\nRETURN\nPROC Main()\nScan(0)\nRETURN\n",
    ] {
        let p = fixture(source);
        let map = memory(&p, &p, 0x3000).unwrap();
        let calls = CallWrites::analyze(&p, &map);
        let known = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        let r = p.routines.iter().find(|r| r.name == "Scan").unwrap();
        assert!(promotion::try_promote(&p, r, &map, &calls, &known).is_none());
    }
}
