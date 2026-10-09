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
