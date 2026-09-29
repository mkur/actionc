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
            assert_eq!(plan.count(), 1, "{from}/{to}/{optimize}");
            let emitted = &machine.routines[0];
            emitted.frame.verify_stack(r).unwrap();
            // Only the widened result remains, aligned at S+2. An isolated
            // CARD -> LONGCARD used to require an eight-byte frame.
            // Raw SIZE returns also contain a pre-existing identity cast;
            // its coalescer deliberately retains reserved frame capacity.
            if to != "SIZE" {
                assert_eq!(emitted.frame.extent, if to == "LONGCARD" { 6 } else { 4 });
            }
            for (id, decision) in &plan.decisions {
                if let Decision::Accumulator(a) = decision {
                    assert!(!emitted.frame.temps.contains_key(id));
                    let span = emitted.code.mir_spans[&(a.block, a.producer)].clone();
                    let bytes = &emitted.code.bytes[span];
                    assert_eq!(*bytes.last().unwrap(), emitted.frame.extent as u8 + 4);
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
            assert_eq!(plan.count(), 1, "{op}/{optimize}");
            let (&id, _) = plan
                .decisions
                .iter()
                .find(|(_, d)| matches!(d, Decision::Accumulator(_)))
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
        assert_eq!(Plan::new(&r).count(), 0, "variant {variant}");
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
    assert_eq!(Plan::new(&r).count(), 0);
}

#[test]
fn home_demand_sparse_map_verification_rejects_unapproved_omissions() {
    let p = program(
        "SIZE FUNC Work(CARD value) RETURN(SIZE(value)) PROC Main() RETURN",
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
