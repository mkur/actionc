use super::*;
use crate::mir6502::trace::{Mir6502Trace, TraceOptions};

fn program() -> MirProgram {
    super::word_result_proof_tests::program()
}
fn session() -> Mir6502Trace {
    Mir6502Trace::new(TraceOptions {
        core_layout_census: true,
        ..Default::default()
    })
}
fn local(offset: u16) -> MirMem {
    MirMem::Local {
        id: LocalId(0),
        offset,
    }
}
fn pointer_ops(source: MirMem) -> Vec<MirOp> {
    vec![
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(1)),
            src: MirAddr::Direct(source),
            width: MirWidth::Word,
        },
        MirOp::Load {
            dst: MirDef::VTemp(MirTempId(2)),
            src: MirAddr::Deref {
                ptr: MirValue::Def(MirDef::VTemp(MirTempId(1))),
                offset: 0,
            },
            width: MirWidth::Byte,
        },
    ]
}
fn has_queries(trace: &Mir6502Trace) -> bool {
    trace
        .events()
        .iter()
        .filter(|e| e.kind == "core-layout-round")
        .flat_map(|e| e.data["observations"].as_array().unwrap())
        .any(|o| o["observation"]["predicate"] == "address-query")
}

#[test]
fn pointer_setup_ordinary_ranges_choose_scratch_without_numeric_queries() {
    for origin in [0x3000, 0x41c7] {
        for source in [
            local(0),
            MirMem::Global {
                id: SymbolId(0),
                offset: 0,
            },
            MirMem::Param {
                id: ParamId(0),
                offset: 0,
            },
        ] {
            let mut p = program();
            p.routines[0].blocks[0].ops = pointer_ops(source);
            let trace = session();
            let t = trace.context();
            let layout = MaterializeLayout::new(&p, origin).with_census(&t, "test");
            {
                let _round = t.layout_round(RoutineId(0), 0);
                let candidates = analyzed_pointer_temp_deref_candidates(
                    &p.routines[0].blocks[0],
                    RoutineId(0),
                    &layout,
                );
                assert_eq!(candidates.len(), 1);
                assert!(candidates[0].1.replacement.iter().any(|op| matches!(op,
                    MirOp::MaterializeAddress { consumer, .. } if *consumer == DEFAULT_POINTER_PAIR)));
            }
            assert!(!has_queries(&trace));
        }
    }
}

#[test]
fn pointer_setup_fixed_pair_boundaries_aliases_and_unknowns_keep_numeric_fallbacks() {
    let mut p = program();
    let mut alias = p.globals[0].clone();
    alias.id = SymbolId(1);
    alias.backing = MirGlobalBacking::Alias {
        target: SymbolId(0),
        offset: 0,
    };
    p.globals.push(alias);
    for (source, fixed) in [
        (MirMem::Absolute(0x80), Some(0x80)),
        (MirMem::Absolute(0xfe), Some(0xfe)),
        (MirMem::Absolute(0xff), None),
        (MirMem::Absolute(0x100), None),
        (
            MirMem::Global {
                id: SymbolId(1),
                offset: 0,
            },
            None,
        ),
        (
            MirMem::Local {
                id: LocalId(99),
                offset: 0,
            },
            None,
        ),
    ] {
        p.routines[0].blocks[0].ops = pointer_ops(source);
        let trace = session();
        let t = trace.context();
        let layout = MaterializeLayout::new(&p, 0x3000).with_census(&t, "test");
        {
            let _round = t.layout_round(RoutineId(0), 0);
            let c = analyzed_pointer_temp_deref_candidates(
                &p.routines[0].blocks[0],
                RoutineId(0),
                &layout,
            );
            assert_eq!(c.len(), 1);
            let expected = fixed.map_or(DEFAULT_POINTER_PAIR, |lo| {
                MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
                    lo: MirFixedZpSlot(lo),
                })
            });
            assert!(c[0].1.replacement.iter().any(
                |op| matches!(op, MirOp::LoadIndirect { consumer, .. } if *consumer == expected)
            ));
        }
        assert!(has_queries(&trace));
    }
}

#[test]
fn pointer_setup_competing_candidates_have_separate_applied_and_rejected_observations() {
    for only_deref in [false, true] {
        let mut p = program();
        p.routines[0].blocks[0].ops = pointer_ops(local(0));
        let trace = session();
        let layout = MaterializeLayout::new(&p, 0x3000).with_census(&trace.context(), "test");
        let result = MirPreHomeRewriteDriver::default()
            .with_trace(&trace.context())
            .run_fixed_point_by_key(
                &mut p.routines[0],
                |r, c| {
                    if only_deref {
                        super::super::super::rewrite::pilots::discover_pointer_temp_derefs(
                            r, c, &layout,
                        )
                    } else {
                        discover_pointer_rewrites(r, c, &layout)
                    }
                },
                super::super::super::rewrite::pilots::pointer_rewrite_rank,
            )
            .unwrap();
        assert_eq!(result.applied, 1);
        let events = trace.events();
        let round = events
            .iter()
            .find(|e| e.kind == "core-layout-round" && e.data["applied_count"] == 1)
            .unwrap();
        let c = round.data["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["key"]["family"] == "pointer-temp-deref")
            .unwrap();
        assert_eq!(
            c["outcome"],
            if only_deref {
                "applied"
            } else {
                "overlap-rejected"
            }
        );
        assert!(round.data["observations"].as_array().unwrap().iter().any(
            |o| o["observation"]["candidate"] == c["key"]
                && o["observation"]["predicate"] == "ordinary-range-outside-zero-page"
                && o["observation"]["result"] == true
        ));
        assert!(!has_queries(&trace));
    }
}

fn prepare(source: MirMem, pair: u8) -> MirOp {
    MirOp::MaterializeAddress {
        consumer: MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(pair),
        }),
        value: pointer_value_from_mem(&source),
    }
}

#[test]
fn pointer_source_separation_keeps_shared_facts_and_rejects_overlap_unknowns_and_clobbers() {
    for case in [
        "ordinary",
        "alias",
        "overlap",
        "unknown",
        "pair-crosses-zp",
        "source-write",
        "opaque",
        "live-a",
    ] {
        let mut p = program();
        let mut alias = p.globals[0].clone();
        alias.id = SymbolId(1);
        alias.backing = MirGlobalBacking::Alias {
            target: SymbolId(0),
            offset: 0,
        };
        p.globals.push(alias);
        let (source, pair, origin) = match case {
            "alias" => (
                MirMem::Global {
                    id: SymbolId(1),
                    offset: 0,
                },
                0xac,
                0x3000,
            ),
            "overlap" => (MirMem::Absolute(0xac), 0xac, 0x3000),
            "unknown" => (
                MirMem::Local {
                    id: LocalId(99),
                    offset: 0,
                },
                0xac,
                0x3000,
            ),
            "pair-crosses-zp" => (
                MirMem::Global {
                    id: SymbolId(0),
                    offset: 0,
                },
                0xff,
                0x100,
            ),
            _ => (local(0), 0xac, 0x3000),
        };
        let prep = prepare(source.clone(), pair);
        let ops = &mut p.routines[0].blocks[0].ops;
        *ops = vec![prep.clone()];
        if case == "source-write" {
            ops.push(MirOp::Store {
                dst: MirAddr::Direct(source),
                src: MirValue::ConstU8(0),
                width: MirWidth::Byte,
            });
        }
        if case == "opaque" {
            ops.push(MirOp::Barrier {
                effects: MirEffects {
                    opaque: true,
                    ..Default::default()
                },
            });
        }
        ops.push(prep);
        if case == "live-a" {
            ops.push(word_chain_store(MirMem::Absolute(0x600)));
        }
        ops.push(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: 0,
            width: MirWidth::Byte,
        });
        let original = p.routines[0].blocks[0].ops.clone();
        let trace = session();
        let layout = MaterializeLayout::new(&p, origin).with_census(&trace.context(), "test");
        let result = MirPostHomeRewriteDriver::default()
            .with_trace(&trace.context())
            .run_fixed_point(&mut p.routines[0], |r, c| {
                ssa_lite::discover_redundant_indexed_address_materializations(r, c, &layout)
            })
            .unwrap();
        assert_eq!(
            result.applied,
            usize::from(matches!(case, "ordinary" | "alias")),
            "{case}"
        );
        if result.applied == 0 {
            assert_eq!(p.routines[0].blocks[0].ops, original);
        }
        if case == "ordinary" {
            let events = trace.events();
            let round = events
                .iter()
                .find(|e| e.kind == "core-layout-round" && e.data["applied_count"] == 1)
                .unwrap();
            let c = round.data["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["outcome"] == "applied")
                .unwrap();
            assert!(
                round.data["observations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|o| o["observation"]["candidate"] == c["key"]
                        && o["observation"]["result"] == true)
            );
            assert!(
                round.data["observations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|o| o["observation"]["candidate"].is_null()),
                "prefix facts must stay separate"
            );
            assert!(!has_queries(&trace));
        }
    }
}
