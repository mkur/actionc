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
fn home_demand_removes_memory_homes_before_emitting_unsigned_widening() {
    for optimize in [false, true] {
        for (from, to) in [
            ("BYTE", "CARD"),
            ("BYTE", "SIZE"),
            ("BYTE", "LONGCARD"),
            ("CARD", "SIZE"),
            ("CARD", "LONGCARD"),
        ] {
            let p = program(
                &format!("{to} FUNC Work({from} value) RETURN({to}(value)) PROC Main() RETURN"),
                optimize,
            );
            let machine = materialize(&p).unwrap();
            let r = &machine.prepared.routines[0];
            let plan = Plan::new(r);
            assert_eq!(
                plan.count(),
                if to == "CARD" {
                    1
                } else if to == "SIZE" {
                    3
                } else {
                    2
                },
                "{from}/{to}/{optimize}"
            );
            let emitted = &machine.routines[0];
            emitted.frame.verify_stack(r).unwrap();
            // Wide returns keep A/X throughout; BYTE -> CARD still materializes
            // its widened word because narrow cast producers are not admitted.
            assert_eq!(emitted.frame.extent, if to == "CARD" { 4 } else { 0 });
            for (id, decision) in &plan.decisions {
                if let Decision::Accumulator(a) = decision {
                    assert!(!emitted.frame.temps.contains_key(id));
                    let span = emitted.code.mir_spans[&(a.block, a.producer)].clone();
                    let bytes = &emitted.code.bytes[span];
                    if matches!(r.blocks[0].ops[a.producer], Mir65816Op::Load { .. }) {
                        assert_eq!(*bytes.last().unwrap(), emitted.frame.extent as u8 + 4);
                    }
                    assert!(!bytes.contains(&0x83), "producer must not store to stack");
                }
            }
            #[cfg(feature = "native65816-state-proof")]
            {
                let (reference, _) = super::super::proof::materialize_reference(&p, false).unwrap();
                super::super::proof::compare_replay_output(
                    &reference.routines[0].code,
                    &emitted.code,
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn home_demand_keeps_native_word_arithmetic_in_a_until_its_cast() {
    for op in ["+", "-", "&", "OR", "XOR", "LSH", "RSH"] {
        let rhs = if matches!(op, "LSH" | "RSH") {
            "2"
        } else {
            "CARD(2)"
        };
        for optimize in [false, true] {
            let p = program(
                &format!(
                    "SIZE FUNC Work(CARD value) RETURN(SIZE(value {op} {rhs})) PROC Main() RETURN"
                ),
                optimize,
            );
            let m = materialize(&p).unwrap();
            let r = &m.prepared.routines[0];
            let plan = Plan::new(r);
            let chained = optimize || matches!(op, "LSH" | "RSH");
            assert_eq!(
                plan.count(),
                (if chained { 2 } else { 1 }) + 2,
                "{op}/{optimize}"
            );
            let (&id, _) = plan
                .decisions
                .iter()
                .find(|(_, d)| {
                    matches!(d, Decision::Accumulator(a)
                    if matches!(r.blocks[0].ops[a.producer], Mir65816Op::Binary { .. }))
                })
                .unwrap();
            let a = plan.accumulator(id).unwrap();
            assert!(matches!(
                r.blocks[0].ops[a.producer],
                Mir65816Op::Binary { .. }
            ));
            assert!(!m.routines[0].frame.temps.contains_key(&id));
            m.routines[0].frame.verify_stack(r).unwrap();
        }
    }
}

#[test]
fn expression_chains_select_every_link_before_assigning_homes() {
    for optimize in [false, true] {
        let p = program(
            "SIZE FUNC Work(CARD value) RETURN(SIZE((value RSH 1) LSH 2)) PROC Main() RETURN",
            optimize,
        );
        let m = materialize(&p).unwrap();
        let r = &m.prepared.routines[0];
        let plan = Plan::new(r);
        assert_eq!(plan.count(), 5);
        let emitted = &m.routines[0];
        emitted.frame.verify_stack(r).unwrap();
        for (id, decision) in &plan.decisions {
            let Decision::Accumulator(a) = decision else {
                continue;
            };
            assert!(!emitted.frame.temps.contains_key(id));
            let span = emitted.code.mir_spans[&(a.block, a.producer)].clone();
            let bytes = &emitted.code.bytes[span];
            match &r.blocks[0].ops[a.producer] {
                Mir65816Op::Load { .. } => {
                    assert_eq!(bytes, [0xa3, emitted.frame.extent as u8 + 4]);
                    assert!(plan.consumer(a.block, a.producer).is_none());
                }
                Mir65816Op::Binary { operation, .. } => {
                    assert!(plan.consumer(a.block, a.producer).is_some());
                    assert_eq!(
                        bytes,
                        if *operation == NirBinaryOp::Rsh {
                            &[0x4a][..]
                        } else {
                            &[0x0a, 0x0a][..]
                        }
                    );
                }
                Mir65816Op::Cast { .. } => assert!(!bytes.contains(&0x83)),
                _ => panic!("unexpected expression producer"),
            }
        }
        #[cfg(feature = "native65816-state-proof")]
        {
            let (reference, _) = super::super::proof::materialize_reference(&p, false).unwrap();
            super::super::proof::compare_replay_output(&reference.routines[0].code, &emitted.code)
                .unwrap();
        }
    }
}

#[test]
fn expression_chains_stop_at_unsupported_links_without_losing_the_safe_suffix() {
    let p = program(
        "SIZE FUNC Work(CARD value) RETURN(SIZE((value RSH 1) LSH 2)) PROC Main() RETURN",
        false,
    );
    let original = &p.routines[0];
    let load = liveness::operation_output(&original.blocks[0].ops[0]).unwrap();
    for variant in 0..6 {
        let mut r = original.clone();
        if variant == 0 {
            if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[0] {
                *volatile = true;
            }
        } else if let Mir65816Op::Binary {
            operation,
            signed,
            left,
            right,
            ..
        } = &mut r.blocks[0].ops[1]
        {
            match variant {
                1 => *signed = true,
                2 => *right = Mir65816Value::U8(4),
                3 => *operation = NirBinaryOp::Mul,
                4 => {
                    // No commutation: A would hold the right-hand operand.
                    *operation = NirBinaryOp::Sub;
                    std::mem::swap(left, right);
                }
                5 => {
                    // Two operand occurrences are two uses, even in one op.
                    *operation = NirBinaryOp::Add;
                    *right = left.clone();
                }
                _ => unreachable!(),
            }
        }
        let plan = Plan::new(&r);
        assert!(plan.accumulator(load).is_none(), "variant {variant}");
        assert!(
            plan.producer(r.blocks[0].id, 2).is_some(),
            "safe suffix {variant}"
        );
    }
}

#[test]
fn expression_terminal_consumers_share_sparse_allocation_and_replay() {
    for source in [
        "CARD FUNC Work(CARD value) RETURN((value RSH 1) LSH 2)",
        "BYTE FUNC Work(BYTE value) BYTE local local=value+7 RETURN(local)",
        "BYTE FUNC Work(CARD value) IF (value RSH 1)<$4000 THEN RETURN(1) FI RETURN(0)",
        "CARD FUNC Echo(CARD value) RETURN(value) CARD FUNC Work(CARD value) RETURN(Echo(value RSH 1))",
        "SIZE FUNC Work(SIZE a,b) RETURN(a+b)",
        "LONGCARD FUNC Work(LONGCARD a,b) RETURN(a-b)",
    ] {
        for optimize in [false, true] {
            let p = program(&format!("{source} PROC Main() RETURN"), optimize);
            let m = materialize(&p).unwrap();
            let index = m
                .prepared
                .routines
                .iter()
                .position(|r| r.name == "Work")
                .unwrap();
            let r = &m.prepared.routines[index];
            let plan = Plan::new(r);
            assert!(plan.count() >= 1, "{source}/{optimize}");
            m.routines[index].frame.verify_stack(r).unwrap();
            for id in plan.producers.values() {
                assert!(!m.routines[index].frame.temps.contains_key(id));
            }
            #[cfg(feature = "native65816-state-proof")]
            {
                let (reference, _) = super::super::proof::materialize_reference(&p, false).unwrap();
                super::super::proof::compare_replay_output(
                    &reference.routines[index].code,
                    &m.routines[index].code,
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn expression_call_admission_keeps_multiple_arguments_and_indirect_transfers_materialized() {
    let p = program(
        "PROC Use(CARD a,b) RETURN PROC Work(CARD value) Use(value RSH 1, value RSH 2) RETURN PROC Main() RETURN",
        true,
    );
    let r = p.routines.iter().find(|r| r.name == "Work").unwrap();
    let call = r.blocks[0]
        .ops
        .iter()
        .position(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap();
    assert!(Plan::new(r).consumer(r.blocks[0].id, call).is_none());
    let p = program(
        "PROC Use(CARD a) RETURN PROC Work(CARD value) Use(value RSH 1) RETURN PROC Main() RETURN",
        true,
    );
    let mut r = p
        .routines
        .iter()
        .find(|r| r.name == "Work")
        .unwrap()
        .clone();
    let call = r.blocks[0]
        .ops
        .iter()
        .position(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap();
    assert!(Plan::new(&r).consumer(r.blocks[0].id, call).is_some());
    if let Mir65816Op::Call { target, .. } = &mut r.blocks[0].ops[call] {
        *target = Mir65816CallTarget::Indirect(Mir65816Value::U24(0x18000), ByteSize::new(3));
    }
    assert!(Plan::new(&r).consumer(r.blocks[0].id, call).is_none());
}

#[test]
fn home_demand_requires_a_complete_adjacent_single_use_lifetime() {
    let original = program(
        "SIZE FUNC Work(CARD value) RETURN(SIZE(value)) PROC Main() RETURN",
        false,
    );
    let r = &original.routines[0];
    let plan = Plan::new(r);
    let (&id, _) = plan
        .decisions
        .iter()
        .find(|(_, d)| matches!(d, Decision::Accumulator(_)))
        .unwrap();
    let a = plan.accumulator(id).unwrap();
    for variant in 0..10 {
        let mut r = r.clone();
        match variant {
            0 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[a.producer] {
                    *volatile = true;
                }
            }
            1 => {
                if let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[a.producer] {
                    address.base = Mir65816AddressBase::Indirect(Mir65816Value::U24(0x120000));
                }
            }
            2 => {
                if let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[a.producer] {
                    address.index = Some(Mir65816Index {
                        value: Mir65816Value::U8(0),
                        stride: ByteSize::ONE,
                    });
                }
            }
            3 => {
                if let Mir65816Op::Cast { from_signed, .. } = &mut r.blocks[0].ops[a.consumer] {
                    *from_signed = true;
                }
            }
            4 => {
                let extra = r.blocks[0].ops[a.consumer].clone();
                r.blocks[0].ops.push(extra);
            }
            5 => {
                let extra = r.blocks[0].ops[a.producer].clone();
                r.blocks[0].ops.insert(a.consumer, extra);
            }
            6 => {
                if let Mir65816Op::Cast { kind, .. } = &mut r.blocks[0].ops[a.consumer] {
                    *kind = NirCastKind::Pointer;
                }
            }
            7 => {
                if let Mir65816Op::Cast { to, .. } = &mut r.blocks[0].ops[a.consumer] {
                    *to = ByteSize::new(2);
                }
            }
            8 => {
                r.blocks[0].ops.insert(
                    a.consumer,
                    Mir65816Op::Store {
                        address: Mir65816Address {
                            base: Mir65816AddressBase::External(Mir65816ExternalAddress::Absolute(
                                AddressValue::new(AddressSpaceId(0), 0x7000),
                            )),
                            displacement: ByteOffset::ZERO,
                            index: None,
                            mode: Mir65816AddressMode::External,
                        },
                        value: Mir65816Value::U8(1),
                        width: ByteSize::ONE,
                        volatile: false,
                    },
                );
            }
            9 => {
                let mut successor = r.blocks[0].clone();
                successor.id = BlockId(99);
                successor.params.clear();
                successor.ops = r.blocks[0].ops.split_off(a.consumer);
                r.blocks[0].terminator = Mir65816Terminator::Goto(Mir65816Edge {
                    target: successor.id,
                    args: vec![],
                });
                r.blocks.push(successor);
            }
            _ => unreachable!(),
        }
        assert!(Plan::new(&r).accumulator(id).is_none(), "variant {variant}");
    }
}

#[test]
fn home_demand_does_not_carry_a_value_across_a_call() {
    let p = program(
        "PROC Touch() RETURN SIZE FUNC Work(CARD value) Touch() RETURN(SIZE(value)) PROC Main() RETURN",
        false,
    );
    let mut r = p
        .routines
        .iter()
        .find(|r| r.name == "Work")
        .unwrap()
        .clone();
    let a = Plan::new(&r)
        .decisions
        .values()
        .find_map(|d| match d {
            Decision::Accumulator(a) => Some(*a),
            _ => None,
        })
        .unwrap();
    let call = r.blocks[0]
        .ops
        .iter()
        .position(|op| matches!(op, Mir65816Op::Call { .. }))
        .unwrap();
    let call = r.blocks[0].ops.remove(call);
    r.blocks[0].ops.insert(a.consumer - 1, call);
    let id = liveness::operation_output(&r.blocks[0].ops[a.producer - 1]).unwrap();
    assert!(Plan::new(&r).accumulator(id).is_none());
}

#[test]
fn home_demand_sparse_map_verification_rejects_unapproved_omissions() {
    let p = program(
        "CARD FUNC Work(BYTE value) RETURN(CARD(value)) PROC Main() RETURN",
        false,
    );
    let r = &p.routines[0];
    let f = AllocatedFrame::new(r).unwrap();
    let plan = Plan::new(r);
    let (&register, _) = plan
        .decisions
        .iter()
        .find(|(_, d)| matches!(d, Decision::Accumulator(_)))
        .unwrap();
    let (&memory, &home) = f.temps.first_key_value().unwrap();
    let mut bad = f.clone();
    bad.temps.remove(&memory);
    assert!(bad.verify_stack(r).is_err());
    bad.temps.insert(register, home);
    assert!(bad.verify_stack(r).is_err());
    let mut changed = r.clone();
    if let Mir65816Op::Load { volatile, .. } = &mut changed.blocks[0].ops[0] {
        *volatile = true;
    }
    assert!(f.verify_stack(&changed).is_err());
}

#[test]
fn borrowed_pointer_aliases_have_no_fictitious_owned_homes() {
    // The byte field store keeps this on the borrowed-home path; pointer-only
    // casts can now use the closed resident allocator instead.
    for optimize in [false, true] {
        let p = program(
            "TYPE Box=[BYTE tag BYTE POINTER link] PROC Work(Box POINTER target BYTE POINTER ptr) target.tag=1 target.link=BYTE POINTER(ADDRESS(ptr)) RETURN PROC Main() RETURN",
            optimize,
        );
        let m = materialize(&p).unwrap();
        let r = &m.prepared.routines[0];
        let plan = Plan::new(r);
        assert!(
            plan.decisions
                .values()
                .any(|d| matches!(d, Decision::Borrowed))
        );
        let f = &m.routines[0].frame;
        assert_eq!(f.extent, 0);
        f.verify_stack(r).unwrap();
        let id = *plan
            .decisions
            .iter()
            .find(|(_, d)| matches!(d, Decision::Borrowed))
            .unwrap()
            .0;
        let mut forged = f.clone();
        forged.temps.insert(
            id,
            Location::Stack(Slot {
                offset: 2,
                width: 3,
            }),
        );
        assert!(forged.verify_stack(r).is_err());
    }
}

#[test]
fn pointer_alias_demand_keeps_mutable_snapshots_across_writes() {
    for optimize in [false, true] {
        let p = program(
            "BYTE POINTER shared PROC Work(BYTE POINTER target) BYTE POINTER saved saved=shared shared=target target^=saved^ RETURN PROC Main() RETURN",
            optimize,
        );
        let m = materialize(&p).unwrap();
        let r = &m.prepared.routines[0];
        let plan = Plan::new(r);
        for op in r.blocks.iter().flat_map(|b| &b.ops) {
            if let Mir65816Op::Load {
                dest,
                address,
                width,
                ..
            } = op
                && width.get() == 3
                && matches!(
                    address.base,
                    Mir65816AddressBase::Static(_) | Mir65816AddressBase::External(_)
                )
            {
                // Keep the snapshot in owned storage. It may now be captured
                // directly in saved, but may never borrow the mutable global.
                assert!(matches!(
                    plan.decisions[dest],
                    Decision::Memory(_) | Decision::LocalLoad
                ));
            }
        }
        m.routines[0].frame.verify_stack(r).unwrap();
    }
}

#[test]
fn address_results_flow_through_casts_and_borrowed_definitions_into_stores() {
    for optimize in [false, true] {
        let p = program(
            "TYPE Node=[Node POINTER next,prev] TYPE Header=[Node POINTER head,tail,last] PROC Work(Header POINTER chain) chain.head=Node POINTER(@chain.tail) chain.tail=NULL chain.last=Node POINTER(@chain.head) RETURN PROC Main() RETURN",
            optimize,
        );
        let m = materialize(&p).unwrap();
        let r = &m.prepared.routines[0];
        let f = &m.routines[0].frame;
        assert_eq!(f.extent, 0, "{optimize}: {r:#?}");
        assert!(f.temps.is_empty());
        f.verify_stack(r).unwrap();
        let demand = Plan::new(r);
        assert!(
            demand
                .decisions
                .values()
                .any(|d| matches!(d,Decision::Accumulator(a) if a.bytes==3))
        );
        #[cfg(feature = "native65816-state-proof")]
        {
            let (reference, _) = super::super::proof::materialize_reference(&p, false).unwrap();
            super::super::proof::compare_replay_output(
                &reference.routines[0].code,
                &m.routines[0].code,
            )
            .unwrap();
        }
    }
}

#[test]
fn sole_address_store_consumes_word_and_bank_without_building_a_register_result() {
    use super::super::selected::{Action, ByteOp, Implied, Instruction};
    for optimize in [false, true] {
        let p = program(
            "TYPE Item=[Item POINTER next,previous] PROC Work(Item POINTER p) p.next=Item POINTER(@p.previous) RETURN PROC Main() RETURN",
            optimize,
        );
        let m = materialize(&p).unwrap();
        let work = &m.routines[0];
        assert_eq!(work.frame.extent, 0);
        let instructions: Vec<_> = work
            .code
            .selected
            .as_ref()
            .unwrap()
            .records()
            .iter()
            .filter_map(|r| {
                if let Action::Instruction { form, .. } = &r.action {
                    Some(form)
                } else {
                    None
                }
            })
            .collect();
        assert!(!instructions.iter().any(|op| matches!(
            op,
            Instruction::Implied(Implied::Tax | Implied::Tay | Implied::Txa | Implied::Tya)
        )));
        let word_store = instructions
            .iter()
            .position(|op| {
                matches!(
                    op,
                    Instruction::Byte(ByteOp::StaIndirect | ByteOp::StaIndirectY, _)
                )
            })
            .unwrap();
        let bank_add = instructions
            .iter()
            .position(|op| matches!(op, Instruction::Byte(ByteOp::AdcImm, 0)))
            .unwrap();
        assert!(word_store < bank_add);
        assert!(
            instructions[bank_add + 1..]
                .iter()
                .any(|op| matches!(op, Instruction::Byte(ByteOp::StaIndirectY, _)))
        );
        for (block, index) in m.prepared.routines[0].blocks.iter().flat_map(|b| {
            b.ops
                .iter()
                .enumerate()
                .filter(|(_, op)| matches!(op, Mir65816Op::AddressOf { .. }))
                .map(|(i, _)| (b.id, i))
        }) {
            assert!(work.code.mir_spans[&(block, index)].is_empty());
        }
    }
}

#[test]
fn prepared_bases_are_reused_only_until_a_possible_clobber() {
    for optimize in [false, true] {
        for (body, reuse) in [
            ("RETURN(p.a+p.b)", true),
            ("CARD first first=p.a p.a=17 RETURN(first+p.a)", true),
            (
                "CARD first BYTE raw=$7200 first=p.a raw=17 RETURN(first+p.a)",
                false,
            ),
        ] {
            let p = program(
                &format!(
                    "TYPE Packet=[CARD a,b] CARD FUNC Work(Packet POINTER p) {body} PROC Main() RETURN"
                ),
                optimize,
            );
            let m = materialize(&p).unwrap();
            let records = m.routines[0].code.selected.as_ref().unwrap().records();
            let decisions: Vec<_> = records
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    matches!(
                        r.action,
                        selected::Action::Request(selected::Request::StagePointer(..))
                    )
                })
                .map(|(i, _)| {
                    records
                        .iter()
                        .find(|r| matches!(r.action,selected::Action::EndRequest(n) if n.0==i))
                        .unwrap()
                        .decision
                })
                .collect();
            assert!(decisions.contains(&Some(false)));
            assert_eq!(
                decisions.last(),
                Some(&Some(reuse)),
                "{optimize}/{body}: {decisions:?}"
            );
        }
    }
}

#[test]
fn ordinary_stores_keep_one_base_and_volatile_stores_keep_the_fallback() {
    use super::super::selected::{Action, Request};
    for optimize in [false, true] {
        for volatile in [false, true] {
            let mut p = program(
                "TYPE Links=[Links POINTER head,tail,previous] \
                 PROC Work(Links POINTER p) p.head=Links POINTER(@p.tail) \
                 p.tail=NULL p.previous=Links POINTER(@p.head) RETURN PROC Main() RETURN",
                optimize,
            );
            for op in p.routines[0].blocks.iter_mut().flat_map(|b| &mut b.ops) {
                if let Mir65816Op::Store { volatile: v, .. } = op {
                    *v = volatile;
                }
            }
            let m = materialize(&p).unwrap();
            let work = &m.routines[0];
            let records = work.code.selected.as_ref().unwrap().records();
            let decisions: Vec<_> = records
                .iter()
                .enumerate()
                .filter_map(|(i, r)| {
                    matches!(r.action, Action::Request(Request::StagePointer(..))).then(|| {
                        records
                            .iter()
                            .find(|r| matches!(r.action,Action::EndRequest(n) if n.0==i))
                            .unwrap()
                            .decision
                            .unwrap()
                    })
                })
                .collect();
            assert_eq!(decisions, [false, !volatile, !volatile]);
            assert_eq!(
                records
                    .iter()
                    .filter(|r| matches!(r.action, Action::Request(Request::AllowPointerStore(_))))
                    .count(),
                if volatile { 0 } else { 3 }
            );
            if !volatile {
                assert_eq!(work.frame.extent, 0);
                assert_eq!(work.code.bytes.len(), 63);
            }
        }
    }
}

#[test]
fn checked_store_permission_matches_source_extent_and_operation_scope() {
    use super::super::selected::{Action, ByteOp, Implied, Request, WordOp};
    use super::super::tracked::TrackedEmitter65816;
    let p = program(
        "TYPE Pair=[CARD a,b] PROC Work(Pair POINTER p) p.a=7 RETURN PROC Main() RETURN",
        false,
    );
    let m = materialize(&p).unwrap();
    let work = &m.routines[0];
    let records = work.code.selected.as_ref().unwrap().records();
    let contract = records
        .iter()
        .find_map(|r| match r.action {
            Action::Request(Request::AllowPointerStore(c)) => Some(c),
            _ => None,
        })
        .unwrap();
    let (origin, home, ptr) = records
        .iter()
        .find_map(|r| match r.action {
            Action::Request(Request::StagePointer(o, h, p)) => Some((o, h, p)),
            _ => None,
        })
        .unwrap();
    for variant in 0..7 {
        let mut e = TrackedEmitter65816::default();
        e.test_frame(work.frame.extent);
        let site = contract.site();
        e.begin_source(site.block, site.index);
        e.allow_pointer_store(contract);
        let other = Slot {
            offset: home.offset + 3,
            width: 3,
        };
        let staged = if variant == 4 { other } else { home };
        assert!(!e.stage_pointer(origin, staged, ptr));
        match variant {
            0 => e.byte(ByteOp::StaIndirect, ptr),
            1 => {
                e.word(WordOp::LdyImm, 2);
                e.byte(ByteOp::StaIndirectY, ptr);
            }
            2 => e.byte(ByteOp::StaIndirect, ptr + 3),
            3 => {
                e.byte(ByteOp::LdaStack, home.offset as u8);
                e.op(Implied::Tay);
                e.byte(ByteOp::StaIndirectY, ptr);
            }
            4 => e.byte(ByteOp::StaIndirect, ptr),
            5 => e.byte(ByteOp::StaStack, home.offset as u8),
            6 => e.byte(ByteOp::StaDp, ptr),
            _ => unreachable!(),
        }
        assert_eq!(
            e.stage_pointer(origin, staged, ptr),
            variant == 0,
            "variant {variant}"
        );
        e.span(site.block, site.index, 0);
        // The next raw operation cannot inherit the preceding store permission.
        e.byte(ByteOp::StaIndirect, ptr);
        assert!(!e.stage_pointer(origin, staged, ptr));
    }
    let mut wrong_site = TrackedEmitter65816::default();
    wrong_site.test_frame(work.frame.extent);
    wrong_site.begin_source(contract.site().block, contract.site().index + 1);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wrong_site.allow_pointer_store(contract);
        }))
        .is_err()
    );
}

#[test]
fn immediate_pointer_load_initializes_local_without_a_capture_home() {
    for optimize in [false, true] {
        let p = program(
            "TYPE Node=[Node POINTER next,previous] TYPE Chain=[Node POINTER head,tail,previous] \
             PROC Work(Chain POINTER chain Node POINTER item) LET first=chain.head \
             item.previous=Node POINTER(@chain.head) item.next=first \
             first.previous=item chain.head=item RETURN PROC Main() RETURN",
            optimize,
        );
        let m = materialize(&p).unwrap();
        let work = &m.routines[0];
        let r = &m.prepared.routines[0];
        let plan = Plan::new(r);
        let captures: Vec<_> = plan.locals.temps().collect();
        assert_eq!(captures.len(), 1);
        assert_eq!(plan.decisions[&captures[0]], Decision::LocalLoad);
        assert!(!work.frame.temps.contains_key(&captures[0]));
        assert_eq!((work.frame.extent, work.frame.spill_bytes), (4, 0));
        assert_eq!(work.code.bytes.len(), 146);
        work.frame.verify_stack(r).unwrap();
        for b in &r.blocks {
            for (i, op) in b.ops.iter().enumerate() {
                if matches!(op, Mir65816Op::Load { dest, .. } if captures.contains(dest)) {
                    assert!(!work.code.mir_spans[&(b.id, i)].is_empty());
                    assert!(work.code.mir_spans[&(b.id, i + 1)].is_empty());
                }
            }
        }
    }
}

#[test]
fn direct_local_loads_keep_captures_for_escape_volatility_and_multiple_uses() {
    // Multiple later uses keep the local present in both raw and optimized MIR.
    let source = "TYPE Node=[Node POINTER next,previous] TYPE Chain=[Node POINTER head,tail,previous] \
        PROC Work(Chain POINTER chain Node POINTER item) LET first=chain.head \
        item.previous=Node POINTER(@chain.head) item.next=first \
        first.previous=item chain.head=item RETURN PROC Main() RETURN";
    for optimize in [false, true] {
        let original = program(source, optimize);
        assert_eq!(Plan::new(&original.routines[0]).locals.temps().count(), 1);
        for variant in 0..4 {
            let mut p = original.clone();
            let r = &mut p.routines[0];
            let index = r.blocks[0]
                .ops
                .windows(2)
                .position(|ops| {
                    matches!(
                        (&ops[0], &ops[1]),
                        (
                            Mir65816Op::Load {
                                address: Mir65816Address {
                                    base: Mir65816AddressBase::Indirect(_),
                                    ..
                                },
                                ..
                            },
                            Mir65816Op::Store {
                                address: Mir65816Address {
                                    base: Mir65816AddressBase::AutomaticFrame(_),
                                    ..
                                },
                                ..
                            }
                        )
                    )
                })
                .unwrap();
            match variant {
                0 => r.frame.objects[0].addressable = true,
                1 => {
                    if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[index] {
                        *volatile = true;
                    }
                }
                2 => {
                    if let Mir65816Op::Store { volatile, .. } = &mut r.blocks[0].ops[index + 1] {
                        *volatile = true;
                    }
                }
                3 => {
                    let extra = r.blocks[0].ops[index + 1].clone();
                    r.blocks[0].ops.insert(index + 2, extra);
                }
                _ => unreachable!(),
            }
            let plan = Plan::new(r);
            assert_eq!(plan.locals.temps().count(), 0, "{optimize}/{variant}");
            materialize(&p).unwrap();
        }
    }
}
