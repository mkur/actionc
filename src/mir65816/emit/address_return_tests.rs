use super::*;

fn program(source: &str) -> Mir65816Program {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::mir65816::lower_program(&nir).unwrap()
}

const SOURCE: &str = "TYPE Parcel=[BYTE ARRAY prefix(82) BYTE ARRAY storage(32)] \
    TYPE Cursor=[BYTE value] Cursor POINTER FUNC Field(Parcel POINTER item) \
    RETURN(Cursor POINTER(@item.storage)) PROC Main() RETURN";

#[test]
fn direct_field_return_has_one_checked_expression_and_no_intermediate_homes() {
    let p = program(SOURCE);
    let r = &p.routines[0];
    let plan = placement::Plan::new(r, &p.data).unwrap();
    let expression = &plan.demand.address_returns[&r.blocks[0].id];
    assert_eq!(expression.offset, 82);
    assert_eq!(plan.frame.extent, 0);
    assert!(plan.frame.temps.is_empty());
    plan.verify().unwrap();
    let m = select::routine(r, false).unwrap();
    for (_, index) in &expression.nodes {
        assert!(m.code.mir_spans[&(expression.block, *index)].is_empty());
    }
    assert_eq!(
        m.code.bytes,
        [
            0xa3, 4, 0x18, 0x69, 82, 0, 0xa8, 0xe2, 0x20, 0xa3, 6, 0x69, 0, 0xc2, 0x20, 0x29, 0xff,
            0, 0xaa, 0x98, 0x6b
        ]
    );
}

#[test]
fn address_return_admission_requires_types_complete_uses_and_final_source_geometry() {
    let p = program(SOURCE);
    for case in 0..6 {
        let mut r = p.routines[0].clone();
        let counts = liveness::input_counts(&r);
        let definitions: BTreeMap<_, _> = r.temps.iter().map(|(t, _)| (*t, 1)).collect();
        let b = &r.blocks[0];
        let expression = candidate(&r, b, &counts, &definitions).unwrap();
        match case {
            0 => {
                let mut uses = counts.clone();
                uses.insert(expression.nodes[0].0, 2);
                assert!(candidate(&r, b, &uses, &definitions).is_none());
            }
            1 => {
                let mut defs = definitions.clone();
                defs.insert(expression.nodes[0].0, 2);
                assert!(candidate(&r, b, &counts, &defs).is_none());
            }
            2 => {
                r.temps
                    .iter_mut()
                    .find(|(t, _)| *t == expression.source)
                    .unwrap()
                    .1
                    .kind = crate::nir::NirTypeKind::Error;
                assert!(candidate(&r, &r.blocks[0], &counts, &definitions).is_none());
            }
            3 => {
                let Mir65816Op::Cast { kind, .. } = r.blocks[0].ops.last_mut().unwrap() else {
                    panic!()
                };
                *kind = NirCastKind::Integer;
                assert!(candidate(&r, &r.blocks[0], &counts, &definitions).is_none());
            }
            _ => {
                let mut demand = home_demand::Plan::new(&r);
                let mut frame = AllocatedFrame::with_demand(&r, &demand).unwrap();
                demand.pointers = Default::default();
                demand.decisions.insert(
                    expression.source,
                    home_demand::Decision::Memory(home_demand::MemoryReason::UnsupportedProducer),
                );
                frame.temps.insert(
                    expression.source,
                    Location::Stack(Slot {
                        offset: if case == 4 { 254 } else { 4 },
                        width: if case == 4 { 3 } else { 2 },
                    }),
                );
                assert!(expression.resolve(&r, &demand, &frame).is_err());
            }
        }
    }
}

#[test]
fn observable_pointer_load_is_captured_before_deferred_address_return() {
    let p = program(
        "TYPE Parcel=[BYTE tag CARD amount] Parcel POINTER input=$7100 \
        CARD POINTER FUNC Field() RETURN(@input.amount) PROC Main() RETURN",
    );
    let r = &p.routines[0];
    let plan = placement::Plan::new(r, &p.data).unwrap();
    let expression = &plan.demand.address_returns[&r.blocks[0].id];
    assert!(!plan.demand.omits(expression.source));
    let m = select::routine(r, false).unwrap();
    assert!(!m.code.mir_spans[&(expression.block, 0)].is_empty());
    plan.verify().unwrap();
}

#[test]
fn field_and_literal_element_chains_share_one_complete_return_owner() {
    for (index, expected) in [(0, 82), (3, 85), (31, 113)] {
        let p = program(&SOURCE.replace("@item.storage", &format!("@item.storage({index})")));
        let r = &p.routines[0];
        let plan = placement::Plan::new(r, &p.data).unwrap();
        let expression = &plan.demand.address_returns[&r.blocks[0].id];
        assert_eq!(expression.offset, expected);
        assert_eq!(expression.nodes.len(), 3);
        assert!(plan.frame.temps.is_empty());
        assert_eq!(
            (
                plan.frame.extent,
                plan.frame.spill_bytes,
                plan.frame.peak_below_entry
            ),
            (0, 0, 0)
        );
        let m = select::routine(r, false).unwrap();
        assert_eq!(m.code.bytes.len(), 21);
        for (_, index) in &expression.nodes {
            assert!(m.code.mir_spans[&(expression.block, *index)].is_empty());
        }
    }
}

#[test]
fn address_composition_refuses_dynamic_nonnumeric_out_of_range_and_nonpointer_links_atomically() {
    let p = program(&SOURCE.replace("@item.storage", "@item.storage(0)"));
    for case in 0..6 {
        let mut r = p.routines[0].clone();
        let address = match &mut r.blocks[0].ops[2] {
            Mir65816Op::AddressOf { address, .. } => address,
            _ => panic!(),
        };
        match case {
            0 => {
                address.index.as_mut().unwrap().value =
                    Mir65816Value::Temp(TempId(0), ByteSize::new(3))
            }
            1 => address.index.as_mut().unwrap().value = Mir65816Value::Null(ByteSize::new(2)),
            2 => address.displacement = ByteOffset::new(65535),
            3 => address.index.as_mut().unwrap().stride = ByteSize::ZERO,
            4 => address.index.as_mut().unwrap().value = Mir65816Value::U32(u32::MAX),
            _ => {
                if let crate::nir::NirTypeKind::Pointer { address_space, .. } =
                    &mut r.temps[1].1.kind
                {
                    *address_space = crate::target::TargetLayout::CODE_ADDRESS_SPACE;
                } else {
                    panic!()
                }
            }
        }
        let counts = liveness::input_counts(&r);
        let definitions = r.temps.iter().map(|(t, _)| (*t, 1)).collect();
        assert!(
            candidate(&r, &r.blocks[0], &counts, &definitions).is_none(),
            "case {case}"
        );
        let demand = home_demand::Plan::new(&r);
        assert!(demand.address_returns.is_empty(), "case {case}");
        assert!(
            r.temps
                .iter()
                .all(|(t, _)| demand.deferred_address(*t).is_none())
        );
    }
}

#[test]
fn complete_census_counts_unreachable_uses_and_chain_length_is_bounded() {
    let p = program(&SOURCE.replace("@item.storage", "@item.storage(0)"));
    let mut r = p.routines[0].clone();
    let mut unreachable = r.blocks[0].clone();
    unreachable.id = BlockId(99);
    unreachable.ops = vec![r.blocks[0].ops[2].clone()];
    unreachable.terminator = Mir65816Terminator::Exit;
    r.blocks.push(unreachable);
    let definitions = r.temps.iter().map(|(t, _)| (*t, 1)).collect();
    assert!(candidate(&r, &r.blocks[0], &liveness::input_counts(&r), &definitions).is_none());

    let mut r = p.routines[0].clone();
    let ty = r.temps.last().unwrap().1.clone();
    let mut source = TempId(3);
    for i in 4..21 {
        let dest = TempId(i);
        r.temps.push((dest, ty.clone()));
        r.blocks[0].ops.push(Mir65816Op::Cast {
            dest,
            from: ByteSize::new(3),
            to: ByteSize::new(3),
            from_signed: false,
            kind: NirCastKind::Pointer,
            value: Mir65816Value::Temp(source, ByteSize::new(3)),
        });
        source = dest;
    }
    let Mir65816Terminator::Return { value, .. } = &mut r.blocks[0].terminator else {
        panic!()
    };
    *value = Some(Mir65816Value::Temp(source, ByteSize::new(3)));
    let definitions = r.temps.iter().map(|(t, _)| (*t, 1)).collect();
    assert!(candidate(&r, &r.blocks[0], &liveness::input_counts(&r), &definitions).is_none());
    assert!(home_demand::Plan::new(&r).address_returns.is_empty());
}
